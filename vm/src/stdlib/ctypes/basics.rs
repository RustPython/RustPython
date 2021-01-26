use std::{fmt, mem, os::raw::*, ptr, slice};

use widestring::WideChar;

use crate::builtins::int::PyInt;
use crate::builtins::memory::{try_buffer_from_object, Buffer, BufferOptions};
use crate::builtins::pystr::PyStrRef;
use crate::builtins::pytype::PyTypeRef;
use crate::common::borrow::{BorrowedValue, BorrowedValueMut};
use crate::common::lock::{PyRwLock, PyRwLockReadGuard, PyRwLockWriteGuard};
use crate::function::OptionalArg;
use crate::pyobject::{
    Either, PyObjectRef, PyRef, PyResult, PyValue, StaticType, TryFromObject,
    TypeProtocol,
};
use crate::slots::BufferProtocol;
use crate::VirtualMachine;

use crate::stdlib::ctypes::array::make_array_with_lenght;
use crate::stdlib::ctypes::dll::dlsym;
use crate::stdlib::ctypes::primitive::{new_simple_type, PySimpleType};

use crossbeam_utils::atomic::AtomicCell;

macro_rules! os_match_type {
    (
        $kind: expr,

        $(
            $($type: literal)|+ => $body: ident
        )+
    ) => {
        match $kind {
            $(
                $(
                    t if t == $type => { mem::size_of::<$body>() }
                )+
            )+
            _ => unreachable!()
        }
    }
}

pub fn get_size(ty: &str) -> usize {
    os_match_type!(
        ty,
        "u" => WideChar
        "c" | "b" => c_schar
        "h" => c_short
        "H" => c_ushort
        "i" => c_int
        "I" => c_uint
        "l" => c_long
        "q" => c_longlong
        "L" => c_ulong
        "Q" => c_ulonglong
        "f" => c_float
        "d" | "g" => c_double
        "?" | "B" => c_uchar
        "P" | "z" | "Z" => usize
    )
}

fn at_address(cls: &PyTypeRef, buf: usize, vm: &VirtualMachine) -> PyResult<RawBuffer> {
    match vm.get_attribute(cls.as_object().to_owned(), "__abstract__") {
        Ok(attr) => match bool::try_from_object(vm, attr) {
            Ok(false) => {
                let len = vm
                    .get_attribute(cls.as_object().to_owned(), "_length_")
                    .map_or(Ok(1), |o: PyObjectRef| match i64::try_from_object(vm, o) {
                        Ok(v_int) => {
                            if v_int < 0 {
                                Err(vm.new_type_error("'_length_' must positive".to_string()))
                            } else {
                                Ok(v_int as usize)
                            }
                        }
                        _ => Err(vm.new_type_error("'_length_' must be an integer".to_string())),
                    })?;

                Ok(RawBuffer {
                    inner: buf as *const u8 as *mut _,
                    size: len,
                })
            }
            Ok(_) => Err(vm.new_type_error("abstract class".to_string())),
            // @TODO: A sanity check
            Err(_) => Err(vm.new_type_error("attribute '__abstract__' must be bool".to_string())),
        },
        // @TODO: I think it's unreachable
        Err(_) => Err(vm.new_attribute_error("abstract class".to_string())),
    }
}

fn buffer_copy(
    cls: PyTypeRef,
    obj: PyObjectRef,
    offset: OptionalArg,
    vm: &VirtualMachine,
    copy: bool,
) -> PyResult<PyCData> {
    match vm.get_attribute(cls.as_object().to_owned(), "__abstract__") {
        Ok(attr) => {
            match bool::try_from_object(vm, attr) {
                Ok(b) if !b => {
                    let buffer = try_buffer_from_object(vm, &obj)?;
                    let opts = buffer.get_options().clone();

                    // @TODO: Fix the way the size of stored
                    // Would this be the a proper replacement?
                    // vm.call_method(cls.as_object().to_owned(), "size", ())?.
                    let cls_size = vm
                        .get_attribute(cls.as_object().to_owned(), "_size")
                        .map(|c_s| usize::try_from_object(vm, c_s))??;

                    let offset_int = offset
                        .into_option()
                        .map_or(Ok(0), |off| i64::try_from_object(vm, off))?;

                    if opts.readonly {
                        Err(vm.new_type_error("underlying buffer is not writable".to_string()))
                    } else if !opts.contiguous {
                        Err(vm.new_type_error("underlying buffer is not C contiguous".to_string()))
                    } else if offset_int < 0 {
                        Err(vm.new_value_error("offset cannot be negative".to_string()))
                    } else if cls_size > opts.len - (offset_int as usize) {
                        Err(vm.new_value_error(format!(
                            "Buffer size too small ({} instead of at least {} bytes)",
                            cls_size,
                            opts.len + (offset_int as usize)
                        )))
                    } else if let Some(mut buffer) = buffer.as_contiguous_mut() {
                        // @TODO: Is this copying?

                        let buffered = if copy {
                            unsafe { slice::from_raw_parts_mut(buffer.as_mut_ptr(), buffer.len()) }
                                .as_mut_ptr()
                        } else {
                            buffer.as_mut_ptr()
                        };

                        Ok(PyCData::new(
                            None,
                            Some(RawBuffer {
                                inner: buffered,
                                size: buffer.len(),
                            }),
                        ))
                    } else {
                        Err(vm.new_buffer_error("empty buffer".to_string()))
                    }
                }
                Ok(_) => Err(vm.new_type_error("abstract class".to_string())),
                Err(_) => {
                    // @TODO: A sanity check
                    Err(vm.new_type_error("attribute '__abstract__' must be bool".to_string()))
                }
            }
        }
        // @TODO: I think this is unreachable...
        Err(_) => Err(vm.new_type_error("abstract class".to_string())),
    }
}

pub fn default_from_param(
    cls: PyTypeRef,
    value: PyObjectRef,
    vm: &VirtualMachine,
) -> PyResult<PyObjectRef> {
    //@TODO: check if this behaves like it should
    if vm.isinstance(&value, &cls)? {
        Ok(value)
    } else if let Ok(parameter) = vm.get_attribute(value.clone(), "_as_parameter_") {
        default_from_param(cls, parameter, vm)
    } else {
        Err(vm.new_attribute_error(format!(
            "expected {} instance instead of {}",
            cls.name,
            value.class().name
        )))
    }
}
#[pyimpl]
pub trait PyCDataFunctions: PyValue {
    #[pymethod]
    fn size_of_instances(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyObjectRef>;

    #[pymethod]
    fn alignment_of_instances(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyObjectRef>;

    #[pymethod]
    fn ref_to(zelf: PyRef<Self>, offset: OptionalArg, vm: &VirtualMachine)
        -> PyResult<PyObjectRef>;

    #[pymethod]
    fn address_of(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyObjectRef>;
}
#[pyimpl]
pub trait PyCDataMethods: PyValue {
    // A lot of the logic goes in this trait
    // There's also other traits that should have different implementations for some functions
    // present here

    // The default methods (representing CDataType_methods) here are for:
    // StructType_Type
    // UnionType_Type
    // PyCArrayType_Type
    // PyCFuncPtrType_Type

    #[pymethod]
    fn from_param(
        zelf: PyRef<Self>,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef>;

    #[pyclassmethod]
    fn from_address(
        cls: PyTypeRef,
        address: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyCData> {
        if let Ok(obj) = address.downcast_exact::<PyInt>(vm) {
            if let Ok(v) = usize::try_from_object(vm, obj.into_object()) {
                let buffer = at_address(&cls, v, vm)?;
                Ok(PyCData::new(None, Some(buffer)))
            } else {
                Err(vm.new_runtime_error("casting pointer failed".to_string()))
            }
        } else {
            Err(vm.new_type_error("integer expected".to_string()))
        }
    }

    #[pyclassmethod]
    fn from_buffer(
        cls: PyTypeRef,
        obj: PyObjectRef,
        offset: OptionalArg,
        vm: &VirtualMachine,
    ) -> PyResult<PyCData> {
        buffer_copy(cls, obj, offset, vm, false)
    }

    #[pyclassmethod]
    fn from_buffer_copy(
        cls: PyTypeRef,
        obj: PyObjectRef,
        offset: OptionalArg,
        vm: &VirtualMachine,
    ) -> PyResult<PyCData> {
        buffer_copy(cls, obj, offset, vm, true)
    }

    #[pyclassmethod]
    fn in_dll(
        cls: PyTypeRef,
        dll: PyObjectRef,
        name: PyStrRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyCData> {
        if let Ok(h) = vm.get_attribute(cls.as_object().to_owned(), "_handle") {
            // This is something to be "CPython" like
            let raw_ptr = if let Ok(h_int) = h.downcast_exact::<PyInt>(vm) {
                dlsym(h_int, name, vm)
            } else {
                Err(vm.new_type_error(format!("_handle must be an int not {}", dll.class().name)))
            }?;

            let sym_ptr = usize::try_from_object(vm, raw_ptr)?;

            let buffer = at_address(&cls, sym_ptr, vm)?;
            Ok(PyCData::new(None, Some(buffer)))
        } else {
            Err(vm.new_attribute_error("atribute '_handle' not found".to_string()))
        }
    }
}

#[pyimpl]
pub trait PyCDataSequenceMethods: PyValue {
    // CDataType_as_sequence methods are default for all *Type_Type
    // Basically the sq_repeat slot is CDataType_repeat
    // which transforms into a Array

    #[pymethod(name = "__mul__")]
    #[pymethod(name = "__rmul__")]
    fn mul(zelf: PyRef<Self>, length: isize, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        if length < 0 {
            Err(vm.new_value_error(format!("Array length must be >= 0, not {} length", length)))
        } else {
            Ok(
                make_array_with_lenght(zelf.clone_class(), length as usize, vm)?
                    .as_object()
                    .clone(),
            )
        }
    }
}

pub fn generic_get_buffer<T>(zelf: &PyRef<T>, vm: &VirtualMachine) -> PyResult<Box<dyn Buffer>>
where
    for<'a> T: PyValue + fmt::Debug + BorrowValue<'a> + BorrowValueMut<'a>,
{
    if let Ok(buffer) = vm.get_attribute(zelf.as_object().clone(), "_buffer") {
        if let Ok(_buffer) = buffer.downcast_exact::<RawBuffer>(vm) {
            Ok(Box::new(PyCBuffer::<T> {
                data: zelf.clone(),
                options: BufferOptions {
                    readonly: false,
                    len: _buffer.size,
                    ..Default::default()
                },
            }))
        } else {
            Err(vm.new_attribute_error("_buffer attribute should be RawBuffer".to_string()))
        }
    } else {
        Err(vm.new_attribute_error("_buffer not found".to_string()))
    }
}

pub trait BorrowValueMut<'a> {
    fn borrow_value_mut(&'a self) -> PyRwLockWriteGuard<'a, RawBuffer>;
}

pub trait BorrowValue<'a> {
    fn borrow_value(&'a self) -> PyRwLockReadGuard<'a, RawBuffer>;
}

impl<'a> BorrowValue<'a> for PyCData {
    fn borrow_value(&'a self) -> PyRwLockReadGuard<'a, RawBuffer> {
        self._buffer.read()
    }
}

impl<'a> BorrowValueMut<'a> for PyCData {
    fn borrow_value_mut(&'a self) -> PyRwLockWriteGuard<'a, RawBuffer> {
        self._buffer.write()
    }
}

impl BufferProtocol for PyCData {
    fn get_buffer(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult<Box<dyn Buffer>> {
        generic_get_buffer::<Self>(zelf, vm)
    }
}

// This trait will be used by all types
impl<T> Buffer for PyCBuffer<T>
where
    for<'a> T: PyValue + fmt::Debug + BorrowValue<'a> + BorrowValueMut<'a>,
{
    fn obj_bytes(&self) -> BorrowedValue<[u8]> {
        PyRwLockReadGuard::map(self.data.borrow_value(), |x| unsafe {
            slice::from_raw_parts(x.inner, x.size)
        })
        .into()
    }

    fn obj_bytes_mut(&self) -> BorrowedValueMut<[u8]> {
        PyRwLockWriteGuard::map(self.data.borrow_value_mut(), |x| unsafe {
            slice::from_raw_parts_mut(x.inner, x.size)
        })
        .into()
    }

    fn release(&self) {}

    fn get_options(&self) -> &BufferOptions {
        &self.options
    }
}

#[derive(Debug)]
pub struct PyCBuffer<T>
where
    for<'a> T: PyValue + fmt::Debug + BorrowValue<'a> + BorrowValueMut<'a>,
{
    pub data: PyRef<T>,
    pub options: BufferOptions,
}

pub struct RawBuffer {
    pub inner: *mut u8,
    pub size: usize,
}

impl fmt::Debug for RawBuffer {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "RawBuffer {{ size: {} }}", self.size)
    }
}

impl PyValue for RawBuffer {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.object_type
    }
}

unsafe impl Send for RawBuffer {}
unsafe impl Sync for RawBuffer {}

// This Trait is the equivalent of PyCData_Type on tp_base for
// Struct_Type, Union_Type, PyCPointer_Type
// PyCArray_Type, PyCSimple_Type, PyCFuncPtr_Type
#[pyclass(module = "_ctypes", name = "_CData")]
pub struct PyCData {
    _objects: AtomicCell<Vec<PyObjectRef>>,
    _buffer: PyRwLock<RawBuffer>,
}

impl fmt::Debug for PyCData {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "PyCData {{ _objects: {{}}, _buffer: {{}}}}",)
    }
}

impl PyValue for PyCData {
    fn class(_vm: &VirtualMachine) -> &PyTypeRef {
        Self::static_type()
    }
}

impl PyCData {
    pub fn new(objs: Option<Vec<PyObjectRef>>, buffer: Option<RawBuffer>) -> Self {
        PyCData {
            _objects: AtomicCell::new(objs.unwrap_or_default()),
            _buffer: PyRwLock::new(buffer.unwrap_or(RawBuffer {
                inner: ptr::null::<u8>() as *mut _,
                size: 0,
            })),
        }
    }
}

#[pyimpl(flags(BASETYPE), with(BufferProtocol))]
impl PyCData {
    // PyCData_methods
    #[pymethod(magic)]
    pub fn ctypes_from_outparam(zelf: PyRef<Self>) {}

    #[pymethod(magic)]
    pub fn reduce(zelf: PyRef<Self>) {}

    #[pymethod(magic)]
    pub fn setstate(zelf: PyRef<Self>) {}
}

pub fn sizeof_func(tp: Either<PyTypeRef, PyObjectRef>, vm: &VirtualMachine) -> PyResult {
    match tp {
        Either::A(type_) if type_.issubclass(PySimpleType::static_type()) => {
            let zelf = new_simple_type(Either::B(&type_), vm)?;
            PyCDataFunctions::size_of_instances(zelf.into_ref(vm), vm)
        }
        Either::B(obj) if obj.has_class_attr("size_of_instances") => {
            let size_of = vm.get_attribute(obj, "size_of_instances").unwrap();
            vm.invoke(&size_of, ())
        }
        _ => Err(vm.new_type_error("this type has no size".to_string())),
    }
}

pub fn alignment(tp: Either<PyTypeRef, PyObjectRef>, vm: &VirtualMachine) -> PyResult {
    match tp {
        Either::A(type_) if type_.issubclass(PySimpleType::static_type()) => {
            let zelf = new_simple_type(Either::B(&type_), vm)?;
            PyCDataFunctions::alignment_of_instances(zelf.into_ref(vm), vm)
        }
        Either::B(obj) if obj.has_class_attr("alignment_of_instances") => {
            let size_of = vm.get_attribute(obj, "alignment_of_instances").unwrap();
            vm.invoke(&size_of, ())
        }
        _ => Err(vm.new_type_error("no alignment info".to_string())),
    }
}

pub fn byref(tp: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    //@TODO: Return a Pointer when Pointer implementation is ready
    let class = tp.clone_class();

    if class.issubclass(PyCData::static_type()) {
        if let Some(ref_to) = vm.get_method(tp, "ref_to") {
            return vm.invoke(&ref_to?, ());
        }
    };

    Err(vm.new_type_error(format!(
        "byref() argument must be a ctypes instance, not '{}'",
        class.name
    )))
}

pub fn addressof(tp: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    let class = tp.clone_class();

    if class.issubclass(PyCData::static_type()) {
        if let Some(address_of) = vm.get_method(tp, "address_of") {
            return vm.invoke(&address_of?, ());
        }
    };

    Err(vm.new_type_error(format!(
        "addressof() argument must be a ctypes instance, not '{}'",
        class.name
    )))
}

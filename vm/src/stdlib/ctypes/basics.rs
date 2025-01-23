use std::{fmt, os::raw::*, ptr, slice};

use widestring::WideChar;

use crate::builtins::int::PyInt;
use crate::builtins::pystr::PyStrRef;
use crate::common::lock::{PyRwLock, PyRwLockReadGuard, PyRwLockWriteGuard};
use crate::function::OptionalArg;
use crate::{AsObject, Py, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject, VirtualMachine};

use crate::stdlib::ctypes::array::make_array_with_length;
use crate::stdlib::ctypes::dll::dlsym;
use crate::stdlib::ctypes::primitive::{new_simple_type, PyCSimple};
use crate::function::Either;

use crate::builtins::PyTypeRef;
use crate::protocol::PyBuffer;
use crossbeam_utils::atomic::AtomicCell;
use crate::types::AsBuffer;

pub fn get_size(ty: &str) -> usize {
    match ty {
        "u" => size_of::<WideChar>(),
        "c" | "b" => size_of::<c_schar>(),
        "h" => size_of::<c_short>(),
        "H" => size_of::<c_short>(),
        "i" => size_of::<c_int>(),
        "I" => size_of::<c_uint>(),
        "l" => size_of::<c_long>(),
        "q" => size_of::<c_longlong>(),
        "L" => size_of::<c_ulong>(),
        "Q" => size_of::<c_ulonglong>(),
        "f" => size_of::<c_float>(),
        "d" | "g" => size_of::<c_double>(),
        "?" | "B" => size_of::<c_uchar>(),
        "P" | "z" | "Z" => size_of::<usize>(),
        _ => unreachable!(),
    }
}

fn at_address(cls: &PyTypeRef, buf: usize, vm: &VirtualMachine) -> PyResult<RawBuffer> {
    match vm.get_attribute_opt(cls.as_object().to_owned(), "__abstract__") {
        Ok(attr) => match bool::try_from_object(vm, attr.unwrap()) {
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
            // FIXME: A sanity check
            Err(_) => Err(vm.new_type_error("attribute '__abstract__' must be bool".to_string())),
        },
        // FIXME: I think it's unreachable
        Err(_) => Err(vm.new_attribute_error("abstract class".to_string())),
    }
}

// FIXME: rework this function
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
                    let buffer = PyBuffer::try_from_object(vm, &obj)?;
                    let opts = buffer.get_options().clone();

                    // TODO: Fix the way the size of stored
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
                        // FIXME: Perform copy
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
                    // TODO: A sanity check
                    Err(vm.new_type_error("attribute '__abstract__' must be bool".to_string()))
                }
            }
        }
        // TODO: I think this is unreachable...
        Err(_) => Err(vm.new_type_error("abstract class".to_string())),
    }
}

pub fn default_from_param<T>(zelf: PyRef<T>, value: PyObjectRef, vm: &VirtualMachine) -> PyResult
where
    T: PyCDataMethods + PyPayload,
{
    //TODO: check if this behaves like it should
    let cls = zelf.as_object().clone_class();
    if vm.isinstance(&value, &cls)? {
        Ok(value)
    } else if let Ok(parameter) = vm.get_attribute(value.clone(), "_as_parameter_") {
        T::from_param(zelf, parameter, vm)
    } else {
        Err(vm.new_attribute_error(format!(
            "expected {} instance instead of {}",
            cls.name,
            value.class().name
        )))
    }
}
#[pyclass]
pub trait PyCDataFunctions: PyPayload {
    #[pymethod]
    fn size_of_instances(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<usize>;

    #[pymethod]
    fn alignment_of_instances(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<usize>;

    #[pymethod]
    fn ref_to(zelf: PyRef<Self>, offset: OptionalArg, vm: &VirtualMachine) -> PyResult;

    #[pymethod]
    fn address_of(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult;
}
#[pyclass]
pub trait PyCDataMethods: PyPayload {
    // A lot of the logic goes in this trait
    // There's also other traits that should have different implementations for some functions
    // present here

    // The default methods (representing CDataType_methods) here are for:
    // StructType_Type
    // UnionType_Type
    // PyCArrayType_Type
    // PyCFuncPtrType_Type

    #[pymethod]
    fn from_param(zelf: PyRef<Self>, value: PyObjectRef, vm: &VirtualMachine) -> PyResult;

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

#[pyclass]
pub trait PyCDataSequenceMethods: PyPayload {
    // CDataType_as_sequence methods are default for all *Type_Type
    // Basically the sq_repeat slot is CDataType_repeat
    // which transforms into a Array

    #[pymethod(name = "__mul__")]
    #[pymethod(name = "__rmul__")]
    fn mul(zelf: PyRef<Self>, length: isize, vm: &VirtualMachine) -> PyResult {
        if length < 0 {
            Err(vm.new_value_error(format!("Array length must be >= 0, not {} length", length)))
        } else {
            Ok(
                make_array_with_length(zelf.clone_class(), length as usize, vm)?
                    .as_object()
                    .clone(),
            )
        }
    }
}

pub fn generic_get_buffer<T>(zelf: &Py<T>, vm: &VirtualMachine) -> PyResult<PyBuffer>
where
        for<'a> T: PyPayload + fmt::Debug + BorrowValue<'a> + BorrowValueMut<'a>,
{
    if let Ok(buffer) = vm.get_attribute(zelf.as_object().clone(), "_buffer") {
        if let Ok(_buffer) = buffer.downcast_exact::<RawBuffer>(vm) {
            Ok(Box::new(PyCBuffer::<T> {
                data: zelf.clone(),
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

impl AsBuffer for PyCData {
    fn as_buffer(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyBuffer> {
        generic_get_buffer::<Self>(zelf, vm)
    }
}

#[derive(Debug)]
pub struct PyCBuffer<T>
where
        for<'a> T: PyPayload + fmt::Debug + BorrowValue<'a> + BorrowValueMut<'a>,
{
    pub data: PyRef<T>,
}

// FIXME: Change this implementation
pub struct RawBuffer {
    pub inner: *mut u8,
    pub size: usize,
}

impl fmt::Debug for RawBuffer {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "RawBuffer {{ size: {} }}", self.size)
    }
}

unsafe impl Send for RawBuffer {}
unsafe impl Sync for RawBuffer {}

// This Trait is the equivalent of PyCData_Type on tp_base for
// Struct_Type, Union_Type, PyCPointer_Type
// PyCArray_Type, PyCSimple_Type, PyCFuncPtr_Type
#[derive(PyPayload)]
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

#[pyclass(flags(BASETYPE), with(AsBuffer))]
impl PyCData {
    // PyCData_methods
    #[pymethod(magic)]
    pub fn ctypes_from_outparam(zelf: PyRef<Self>) {}

    #[pymethod(magic)]
    pub fn reduce(zelf: PyRef<Self>) {}

    #[pymethod(magic)]
    pub fn setstate(zelf: PyRef<Self>) {}
}

// FIXME: this function is too hacky, work a better way of doing it
pub fn sizeof_func(tp: Either<PyTypeRef, PyObjectRef>, vm: &VirtualMachine) -> PyResult<usize> {
    match tp {
        Either::A(type_) if type_.issubclass(PyCSimple::static_type()) => {
            let zelf = new_simple_type(Either::B(&type_), vm)?;
            PyCDataFunctions::size_of_instances(zelf.into_ref(vm), vm)
        }
        Either::B(obj) if obj.has_class_attr("size_of_instances") => {
            let size_of_method = vm.get_attribute(obj, "size_of_instances").unwrap();
            let size_of_return = vm.invoke(&size_of_method, ())?;
            Ok(usize::try_from_object(vm, size_of_return)?)
        }
        _ => Err(vm.new_type_error("this type has no size".to_string())),
    }
}

// FIXME: this function is too hacky, work a better way of doing it
pub fn alignment(tp: Either<PyTypeRef, PyObjectRef>, vm: &VirtualMachine) -> PyResult<usize> {
    match tp {
        Either::A(type_) if type_.issubclass(PyCSimple::static_type()) => {
            let zelf = new_simple_type(Either::B(&type_), vm)?;
            PyCDataFunctions::alignment_of_instances(zelf.into_ref(vm), vm)
        }
        Either::B(obj) if obj.has_class_attr("alignment_of_instances") => {
            let alignment_of_m = vm.get_attribute(obj, "alignment_of_instances")?;
            let alignment_of_r = vm.invoke(&alignment_of_m, ())?;
            usize::try_from_object(vm, alignment_of_m)
        }
        _ => Err(vm.new_type_error("no alignment info".to_string())),
    }
}

pub fn byref(tp: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    //TODO: Return a Pointer when Pointer implementation is ready
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

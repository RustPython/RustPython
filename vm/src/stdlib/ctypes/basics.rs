use std::{fmt, ptr, slice};

use crate::builtins::int::PyInt;
use crate::builtins::memory::{try_buffer_from_object, Buffer, BufferOptions};
use crate::builtins::pystr::PyStrRef;
use crate::builtins::pytype::PyTypeRef;
use crate::common::borrow::{BorrowedValue, BorrowedValueMut};
use crate::common::lock::{PyRwLock, PyRwLockReadGuard, PyRwLockWriteGuard};
use crate::function::OptionalArg;
use crate::pyobject::{
    BorrowValue, PyObjectRef, PyRef, PyResult, PyValue, StaticType, TryFromObject, TypeProtocol,
};
use crate::slots::BufferProtocol;
use crate::VirtualMachine;

use crate::stdlib::ctypes::array::make_array_with_lenght;
use crate::stdlib::ctypes::dll::dlsym;

use crossbeam_utils::atomic::AtomicCell;

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

    #[pyclassmethod]
    fn from_param(cls: PyTypeRef, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyCData>;

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

impl<'a> BorrowValue<'a> for PyCData {
    type Borrowed = PyRwLockReadGuard<'a, RawBuffer>;

    fn borrow_value(&'a self) -> Self::Borrowed {
        self._buffer.read()
    }
}

impl BufferProtocol for PyCData {
    fn get_buffer(zelf: &PyRef<Self>, _vm: &VirtualMachine) -> PyResult<Box<dyn Buffer>> {
        Ok(Box::new(PyCDataBuffer {
            data: zelf.clone(),
            options: BufferOptions {
                readonly: false,
                len: zelf._buffer.read().size,
                ..Default::default()
            },
        }))
    }
}

#[derive(Debug)]
pub struct PyCDataBuffer {
    pub data: PyCDataRef,
    pub options: BufferOptions,
}

// This trait will be used by all types
impl Buffer for PyCDataBuffer {
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

pub type PyCDataRef = PyRef<PyCData>;

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

    pub fn borrow_value_mut(&self) -> PyRwLockWriteGuard<'_, RawBuffer> {
        self._buffer.write()
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

use std::{fmt, slice};

use crate::builtins::int::PyInt;
use crate::builtins::memory::{try_buffer_from_object, Buffer, BufferOptions};
use crate::builtins::pystr::PyStrRef;
use crate::builtins::pytype::PyTypeRef;
use crate::common::borrow::{BorrowedValue, BorrowedValueMut};
use crate::function::OptionalArg;
use crate::pyobject::{
    PyObjectRc, PyObjectRef, PyRef, PyResult, PyValue, StaticType, TryFromObject, TypeProtocol,
};
use crate::VirtualMachine;

use crate::stdlib::ctypes::dll::dlsym;

use crossbeam_utils::atomic::AtomicCell;

// GenericPyCData_new -> PyResult<PyObjectRef>
pub fn generic_pycdata_new(type_: PyTypeRef, vm: &VirtualMachine) {
    // @TODO: To be used on several places
}

fn at_address(cls: &PyTypeRef, buf: usize, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
    match vm.get_attribute(cls.as_object().to_owned(), "__abstract__") {
        Ok(attr) => match bool::try_from_object(vm, attr) {
            Ok(b) if b => {
                let len = vm
                    .get_attribute(cls.as_object().to_owned(), "_length_")
                    .map_or(Ok(1), |o: PyObjectRc| {
                        match i64::try_from_object(vm, o.clone()) {
                            Ok(v_int) => {
                                if v_int < 0 {
                                    Err(vm.new_type_error("'_length_' must positive".to_string()))
                                } else {
                                    Ok(v_int as usize)
                                }
                            }
                            _ => {
                                Err(vm.new_type_error("'_length_' must be an integer".to_string()))
                            }
                        }
                    })?;

                let slice = unsafe { slice::from_raw_parts(buf as *const u8, len) };
                Ok(slice.to_vec())
            }
            Ok(_) => Err(vm.new_type_error("abstract class".to_string())),
            Err(_) => Err(vm.new_type_error("attribute '__abstract__' must be bool".to_string())),
        },
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
                Ok(b) if b => {
                    let buffer = try_buffer_from_object(vm, &obj)?;
                    let opts = buffer.get_options().clone();

                    // @TODO: Fix the way the size of stored
                    let cls_size = vm
                        .get_attribute(cls.as_object().to_owned(), "_size")
                        .map(|c_s| usize::try_from_object(vm, c_s.clone()))??;

                    let offset_int = offset
                        .into_option()
                        .map_or(Ok(0), |off| i64::try_from_object(vm, off.clone()))?;

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
                        let buffered = if copy {
                            buffer.to_vec()
                        } else {
                            let b_ptr: *mut u8 = buffer.as_mut_ptr();
                            unsafe { Vec::from_raw_parts(b_ptr, buffer.len(), buffer.len()) }
                        };

                        Ok(PyCData::new(None, Some(buffered)))
                    } else {
                        Err(vm.new_buffer_error("empty buffer".to_string()))
                    }
                }
                Ok(_) => Err(vm.new_type_error("abstract class".to_string())),
                Err(_) => {
                    Err(vm.new_type_error("attribute '__abstract__' must be bool".to_string()))
                }
            }
        }
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
            let raw_ptr = dlsym(h.clone(), name.into_object(), vm).map_err(|e| {
                if vm
                    .isinstance(&e.clone().into(), &vm.ctx.exceptions.value_error)
                    .is_ok()
                {
                    vm.new_type_error(format!(
                        "_handle must be SharedLibrary not {}",
                        dll.clone().class().name
                    ))
                } else {
                    e
                }
            })?;

            let sym_ptr = usize::try_from_object(vm, raw_ptr.into_object(vm))?;

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

    // #[pymethod(name = "__mul__")]
    // fn mul(&self, counter: isize, vm: &VirtualMachine) -> PyObjectRef {
    // }

    // #[pymethod(name = "__rmul__")]
    // fn rmul(&self, counter: isize, vm: &VirtualMachine) -> PyObjectRef {
    //     self.mul(counter, vm)
    // }
}

// This trait will be used by all types
pub trait PyCDataBuffer: Buffer {
    fn obj_bytes(&self) -> BorrowedValue<[u8]>;

    fn obj_bytes_mut(&self) -> BorrowedValueMut<[u8]>;

    fn release(&self);

    fn get_options(&self) -> BorrowedValue<BufferOptions>;
}

// This Trait is the equivalent of PyCData_Type on tp_base for
// Struct_Type, Union_Type, PyCPointer_Type
// PyCArray_Type, PyCSimple_Type, PyCFuncPtr_Type
#[pyclass(module = "ctypes", name = "_CData")]
pub struct PyCData {
    _objects: AtomicCell<Vec<PyObjectRc>>,
    _buffer: AtomicCell<Vec<u8>>,
}

impl fmt::Debug for PyCData {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "PyCData {{ _objects: {{}}, _buffer: {{}}}}",)
    }
}

impl PyValue for PyCData {
    fn class(_vm: &VirtualMachine) -> &PyTypeRef {
        Self::init_bare_type()
    }
}

impl PyCData {
    fn new(objs: Option<Vec<PyObjectRc>>, buffer: Option<Vec<u8>>) -> Self {
        PyCData {
            _objects: AtomicCell::new(objs.unwrap_or_default()),
            _buffer: AtomicCell::new(buffer.unwrap_or_default()),
        }
    }
}

#[pyimpl]
impl PyCData {
    // PyCData_methods
    #[pymethod(name = "__ctypes_from_outparam__")]
    pub fn ctypes_from_outparam(zelf: PyRef<Self>) {}

    #[pymethod(name = "__reduce__")]
    pub fn reduce(zelf: PyRef<Self>) {}

    #[pymethod(name = "__setstate__")]
    pub fn setstate(zelf: PyRef<Self>) {}
}

// #[pyimpl]
// impl PyCDataBuffer for PyCData {

// }

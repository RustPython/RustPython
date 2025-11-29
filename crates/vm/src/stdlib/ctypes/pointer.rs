use crossbeam_utils::atomic::AtomicCell;
use num_traits::ToPrimitive;
use rustpython_common::lock::PyRwLock;

use crate::builtins::{PyType, PyTypeRef};
use crate::convert::ToPyObject;
use crate::protocol::PyNumberMethods;
use crate::stdlib::ctypes::PyCData;
use crate::stdlib::ctypes::base::CDataObject;
use crate::types::AsNumber;
use crate::{AsObject, PyObjectRef, PyPayload, PyResult, VirtualMachine};

#[pyclass(name = "PyCPointerType", base = PyType, module = "_ctypes")]
#[derive(PyPayload, Debug)]
pub struct PyCPointerType {
    #[allow(dead_code)]
    pub(crate) inner: PyCPointer,
}

#[pyclass(flags(IMMUTABLETYPE), with(AsNumber))]
impl PyCPointerType {
    #[pymethod]
    fn __mul__(cls: PyTypeRef, n: isize, vm: &VirtualMachine) -> PyResult {
        use super::array::{PyCArray, PyCArrayType};
        if n < 0 {
            return Err(vm.new_value_error(format!("Array length must be >= 0, not {n}")));
        }
        // Pointer size
        let element_size = std::mem::size_of::<usize>();
        Ok(PyCArrayType {
            inner: PyCArray {
                typ: PyRwLock::new(cls.as_object().to_owned()),
                length: AtomicCell::new(n as usize),
                element_size: AtomicCell::new(element_size),
                cdata: PyRwLock::new(CDataObject::new(0)),
            },
        }
        .to_pyobject(vm))
    }
}

impl AsNumber for PyCPointerType {
    fn as_number() -> &'static PyNumberMethods {
        static AS_NUMBER: PyNumberMethods = PyNumberMethods {
            multiply: Some(|a, b, vm| {
                let cls = a
                    .downcast_ref::<PyType>()
                    .ok_or_else(|| vm.new_type_error("expected type".to_owned()))?;
                let n = b
                    .try_index(vm)?
                    .as_bigint()
                    .to_isize()
                    .ok_or_else(|| vm.new_overflow_error("array size too large".to_owned()))?;
                PyCPointerType::__mul__(cls.to_owned(), n, vm)
            }),
            ..PyNumberMethods::NOT_IMPLEMENTED
        };
        &AS_NUMBER
    }
}

#[pyclass(
    name = "_Pointer",
    base = PyCData,
    metaclass = "PyCPointerType",
    module = "_ctypes"
)]
#[derive(Debug, PyPayload)]
pub struct PyCPointer {
    contents: PyRwLock<PyObjectRef>,
}

#[pyclass(flags(BASETYPE, IMMUTABLETYPE))]
impl PyCPointer {
    // TODO: not correct
    #[pygetset]
    fn contents(&self) -> PyResult<PyObjectRef> {
        let contents = self.contents.read().clone();
        Ok(contents)
    }
    #[pygetset(setter)]
    fn set_contents(&self, contents: PyObjectRef) -> PyResult<()> {
        *self.contents.write() = contents;
        Ok(())
    }

    #[pyclassmethod]
    fn from_address(cls: PyTypeRef, address: isize, vm: &VirtualMachine) -> PyResult {
        if address == 0 {
            return Err(vm.new_value_error("NULL pointer access".to_owned()));
        }
        // Pointer just stores the address value
        Ok(PyCPointer {
            contents: PyRwLock::new(vm.ctx.new_int(address).into()),
        }
        .into_ref_with_type(vm, cls)?
        .into())
    }

    #[pyclassmethod]
    fn from_buffer(
        cls: PyTypeRef,
        source: PyObjectRef,
        offset: crate::function::OptionalArg<isize>,
        vm: &VirtualMachine,
    ) -> PyResult {
        use crate::TryFromObject;
        use crate::protocol::PyBuffer;

        let offset = offset.unwrap_or(0);
        if offset < 0 {
            return Err(vm.new_value_error("offset cannot be negative".to_owned()));
        }
        let offset = offset as usize;
        let size = std::mem::size_of::<usize>();

        let buffer = PyBuffer::try_from_object(vm, source.clone())?;

        if buffer.desc.readonly {
            return Err(vm.new_type_error("underlying buffer is not writable".to_owned()));
        }

        let buffer_len = buffer.desc.len;
        if offset + size > buffer_len {
            return Err(vm.new_value_error(format!(
                "Buffer size too small ({} instead of at least {} bytes)",
                buffer_len,
                offset + size
            )));
        }

        // Read pointer value from buffer
        let bytes = buffer.obj_bytes();
        let ptr_bytes = &bytes[offset..offset + size];
        let ptr_val = usize::from_ne_bytes(ptr_bytes.try_into().expect("size is checked above"));

        Ok(PyCPointer {
            contents: PyRwLock::new(vm.ctx.new_int(ptr_val).into()),
        }
        .into_ref_with_type(vm, cls)?
        .into())
    }

    #[pyclassmethod]
    fn from_buffer_copy(
        cls: PyTypeRef,
        source: crate::function::ArgBytesLike,
        offset: crate::function::OptionalArg<isize>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let offset = offset.unwrap_or(0);
        if offset < 0 {
            return Err(vm.new_value_error("offset cannot be negative".to_owned()));
        }
        let offset = offset as usize;
        let size = std::mem::size_of::<usize>();

        let source_bytes = source.borrow_buf();
        let buffer_len = source_bytes.len();

        if offset + size > buffer_len {
            return Err(vm.new_value_error(format!(
                "Buffer size too small ({} instead of at least {} bytes)",
                buffer_len,
                offset + size
            )));
        }

        // Read pointer value from buffer
        let ptr_bytes = &source_bytes[offset..offset + size];
        let ptr_val = usize::from_ne_bytes(ptr_bytes.try_into().expect("size is checked above"));

        Ok(PyCPointer {
            contents: PyRwLock::new(vm.ctx.new_int(ptr_val).into()),
        }
        .into_ref_with_type(vm, cls)?
        .into())
    }
}

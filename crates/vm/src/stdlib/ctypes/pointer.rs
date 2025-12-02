use num_traits::ToPrimitive;
use rustpython_common::lock::PyRwLock;

use crate::builtins::{PyType, PyTypeRef};
use crate::function::FuncArgs;
use crate::protocol::PyNumberMethods;
use crate::stdlib::ctypes::PyCData;
use crate::types::{AsNumber, Constructor};
use crate::{AsObject, Py, PyObjectRef, PyPayload, PyResult, VirtualMachine};

#[pyclass(name = "PyCPointerType", base = PyType, module = "_ctypes")]
#[derive(Debug, Default)]
pub struct PyCPointerType {}

#[pyclass(flags(IMMUTABLETYPE), with(AsNumber))]
impl PyCPointerType {
    #[pymethod]
    fn __mul__(cls: PyTypeRef, n: isize, vm: &VirtualMachine) -> PyResult {
        use super::array::create_array_type_with_stg_info;
        if n < 0 {
            return Err(vm.new_value_error(format!("Array length must be >= 0, not {n}")));
        }
        // Pointer size
        let element_size = std::mem::size_of::<usize>();
        let total_size = element_size * (n as usize);
        let stg_info = super::util::StgInfo::new_array(
            total_size,
            element_size,
            n as usize,
            cls.as_object().to_owned(),
            element_size,
        );
        create_array_type_with_stg_info(stg_info, vm)
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
#[derive(Debug)]
pub struct PyCPointer {
    contents: PyRwLock<PyObjectRef>,
}

impl Constructor for PyCPointer {
    type Args = (crate::function::OptionalArg<PyObjectRef>,);

    fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        let args: Self::Args = args.bind(vm)?;
        // Get the initial contents value if provided
        let initial_contents = args.0.into_option().unwrap_or_else(|| vm.ctx.none());

        // Create a new PyCPointer instance with the provided value
        PyCPointer {
            contents: PyRwLock::new(initial_contents),
        }
        .into_ref_with_type(vm, cls)
        .map(Into::into)
    }

    fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
        unimplemented!("use slot_new")
    }
}

#[pyclass(flags(BASETYPE, IMMUTABLETYPE), with(Constructor))]
impl PyCPointer {
    // TODO: not correct
    #[pygetset]
    fn contents(&self) -> PyResult<PyObjectRef> {
        let contents = self.contents.read().clone();
        Ok(contents)
    }
    #[pygetset(setter)]
    fn set_contents(&self, contents: PyObjectRef, _vm: &VirtualMachine) -> PyResult<()> {
        // Validate that the contents is a CData instance if we have a _type_
        // For now, just store it
        *self.contents.write() = contents;
        Ok(())
    }

    #[pymethod]
    fn __init__(
        &self,
        value: crate::function::OptionalArg<PyObjectRef>,
        _vm: &VirtualMachine,
    ) -> PyResult<()> {
        // Pointer can be initialized with 0 or 1 argument
        // If 1 argument is provided, it should be a CData instance
        if let crate::function::OptionalArg::Present(val) = value {
            *self.contents.write() = val;
        }

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

    #[pyclassmethod]
    fn in_dll(
        cls: PyTypeRef,
        dll: PyObjectRef,
        name: crate::builtins::PyStrRef,
        vm: &VirtualMachine,
    ) -> PyResult {
        use libloading::Symbol;

        // Get the library handle from dll object
        let handle = if let Ok(int_handle) = dll.try_int(vm) {
            // dll is an integer handle
            int_handle
                .as_bigint()
                .to_usize()
                .ok_or_else(|| vm.new_value_error("Invalid library handle".to_owned()))?
        } else {
            // dll is a CDLL/PyDLL/WinDLL object with _handle attribute
            dll.get_attr("_handle", vm)?
                .try_int(vm)?
                .as_bigint()
                .to_usize()
                .ok_or_else(|| vm.new_value_error("Invalid library handle".to_owned()))?
        };

        // Get the library from cache
        let library_cache = crate::stdlib::ctypes::library::libcache().read();
        let library = library_cache
            .get_lib(handle)
            .ok_or_else(|| vm.new_attribute_error("Library not found".to_owned()))?;

        // Get symbol address from library
        let symbol_name = format!("{}\0", name.as_str());
        let inner_lib = library.lib.lock();

        let symbol_address = if let Some(lib) = &*inner_lib {
            unsafe {
                // Try to get the symbol from the library
                let symbol: Symbol<'_, *mut u8> = lib.get(symbol_name.as_bytes()).map_err(|e| {
                    vm.new_attribute_error(format!("{}: symbol '{}' not found", e, name.as_str()))
                })?;
                *symbol as usize
            }
        } else {
            return Err(vm.new_attribute_error("Library is closed".to_owned()));
        };

        // For pointer types, we return a pointer to the symbol address
        Ok(PyCPointer {
            contents: PyRwLock::new(vm.ctx.new_int(symbol_address).into()),
        }
        .into_ref_with_type(vm, cls)?
        .into())
    }
}

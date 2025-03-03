use crate::builtins::{PyStr, PyTupleRef, PyTypeRef};
use crate::convert::ToPyObject;
use crate::function::FuncArgs;
use crate::stdlib::ctypes::PyCData;
use crate::stdlib::ctypes::base::{PyCSimple, ffi_type_from_str};
use crate::types::{Callable, Constructor};
use crate::{Py, PyObjectRef, PyResult, VirtualMachine};
use crossbeam_utils::atomic::AtomicCell;
use libffi::middle::{Arg, Cif, CodePtr, Type};
use libloading::Symbol;
use num_traits::ToPrimitive;
use rustpython_common::lock::PyRwLock;
use std::fmt::Debug;

// https://github.com/python/cpython/blob/4f8bb3947cfbc20f970ff9d9531e1132a9e95396/Modules/_ctypes/callproc.c#L15

#[derive(Debug)]
pub struct Function {
    // TODO: no protection from use-after-free
    pointer: CodePtr,
    cif: Cif,
}

unsafe impl Send for Function {}
unsafe impl Sync for Function {}

type FP = unsafe extern "C" fn();

impl Function {
    pub unsafe fn load(
        library: &libloading::Library,
        function: &str,
        args: &[PyObjectRef],
        ret_type: &Option<PyTypeRef>,
        vm: &VirtualMachine,
    ) -> PyResult<Self> {
        // map each arg to a PyCSimple
        let args = args
            .iter()
            .map(|arg| {
                if let Some(data) = arg.downcast_ref::<PyCSimple>() {
                    Ok(ffi_type_from_str(&data._type_).unwrap())
                } else {
                    Err(vm.new_type_error("Expected a ctypes simple type".to_string()))
                }
            })
            .collect::<PyResult<Vec<Type>>>()?;
        let terminated = format!("{}\0", function);
        let pointer: Symbol<'_, FP> = unsafe {
            library
                .get(terminated.as_bytes())
                .map_err(|err| err.to_string())
                .map_err(|err| vm.new_value_error(err))?
        };
        let code_ptr = CodePtr(*pointer as *mut _);
        let return_type = match ret_type {
            // TODO: Fix this
            Some(_t) => {
                return Err(vm.new_not_implemented_error("Return type not implemented".to_string()));
            }
            None => Type::c_int(),
        };
        let cif = Cif::new(args, return_type);
        Ok(Function {
            cif,
            pointer: code_ptr,
        })
    }

    pub unsafe fn call(
        &self,
        args: Vec<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        let args = args
            .into_iter()
            .map(|arg| {
                if let Some(data) = arg.downcast_ref::<PyCSimple>() {
                    dbg!(&data);
                    todo!("HANDLE ARGUMENTS")
                } else {
                    Err(vm.new_type_error("Expected a ctypes simple type".to_string()))
                }
            })
            .collect::<PyResult<Vec<Arg>>>()?;
        // TODO: FIX return type
        let result: i32 = unsafe { self.cif.call(self.pointer, &args) };
        Ok(vm.ctx.new_int(result).into())
    }
}

#[pyclass(module = "_ctypes", name = "CFuncPtr", base = "PyCData")]
#[derive(PyPayload)]
pub struct PyCFuncPtr {
    pub name: PyRwLock<String>,
    pub _flags_: AtomicCell<u32>,
    // FIXME(arihant2math): This shouldn't be an option, setting the default as the none type should work
    //  This is a workaround for now and I'll fix it later
    pub _restype_: PyRwLock<Option<PyTypeRef>>,
    pub handler: PyObjectRef,
}

impl Debug for PyCFuncPtr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PyCFuncPtr")
            .field("name", &self.name)
            .finish()
    }
}

impl Constructor for PyCFuncPtr {
    type Args = (PyTupleRef, FuncArgs);

    fn py_new(_cls: PyTypeRef, (tuple, _args): Self::Args, vm: &VirtualMachine) -> PyResult {
        let name = tuple
            .first()
            .ok_or(vm.new_type_error("Expected a tuple with at least 2 elements".to_string()))?
            .downcast_ref::<PyStr>()
            .ok_or(vm.new_type_error("Expected a string".to_string()))?
            .to_string();
        let handler = tuple
            .into_iter()
            .nth(1)
            .ok_or(vm.new_type_error("Expected a tuple with at least 2 elements".to_string()))?
            .clone();
        Ok(Self {
            _flags_: AtomicCell::new(0),
            name: PyRwLock::new(name),
            _restype_: PyRwLock::new(None),
            handler,
        }
        .to_pyobject(vm))
    }
}

impl Callable for PyCFuncPtr {
    type Args = FuncArgs;
    fn call(zelf: &Py<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult {
        unsafe {
            let handle = zelf.handler.get_attr("_handle", vm)?;
            let handle = handle.try_int(vm)?.as_bigint().clone();
            let library_cache = crate::stdlib::ctypes::library::libcache().read();
            let library = library_cache
                .get_lib(
                    handle
                        .to_usize()
                        .ok_or(vm.new_value_error("Invalid handle".to_string()))?,
                )
                .ok_or_else(|| vm.new_value_error("Library not found".to_string()))?;
            let inner_lib = library.lib.lock();
            let name = zelf.name.read();
            let res_type = zelf._restype_.read();
            let func = Function::load(
                inner_lib.as_ref().unwrap(),
                &name,
                &args.args,
                &res_type,
                vm,
            )?;
            func.call(args.args, vm)
        }
    }
}

#[pyclass(flags(BASETYPE), with(Callable, Constructor))]
impl PyCFuncPtr {
    #[pygetset(magic)]
    fn name(&self) -> String {
        self.name.read().clone()
    }

    #[pygetset(setter, magic)]
    fn set_name(&self, name: String) {
        *self.name.write() = name;
    }
}

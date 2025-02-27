use crate::builtins::PyTypeRef;
use crate::stdlib::ctypes::PyCData;
use crate::types::{Callable, Constructor};
use crate::{AsObject, Py, PyObjectRef, PyRef, PyResult, VirtualMachine};
use crossbeam_utils::atomic::AtomicCell;
use rustpython_common::lock::{PyMutex, PyRwLock};
use std::ffi::c_void;
use std::fmt::Debug;
use std::sync::Arc;
use crate::class::StaticType;
use crate::stdlib::ctypes::base::PyCSimple;
use libffi::middle::{Arg, Cif, CodePtr, Type};
use libloading::Symbol;
// https://github.com/python/cpython/blob/4f8bb3947cfbc20f970ff9d9531e1132a9e95396/Modules/_ctypes/callproc.c#L15


#[derive(Debug)]
pub struct Function {
    // TODO: no protection from use-after-free
    pointer: CodePtr,
    cif: Cif
}

unsafe impl Send for Function {}
unsafe impl Sync for Function {}

type FP = unsafe extern "C" fn ();

impl Function {
    pub unsafe fn load(
        library: &libloading::Library,
        function: &str,
        args: &[PyObjectRef],
        ret_type: Option<PyTypeRef>,
        vm: &VirtualMachine,
    ) -> PyResult<Self> {
        // map each arg to a PyCSimple
        let args = args.into_iter().map(|arg| {
            if arg.is_subclass(PyCSimple::static_type().as_object(), vm).unwrap() {
                let arg_type = arg.get_attr("_type_", vm).unwrap().str(vm).unwrap().to_string();
                let value = arg.get_attr("value", vm).unwrap();
                match &*arg_type {
                    _ => todo!("HANDLE ARG TYPE")
                }
            } else {
                todo!("HANDLE ERROR")
            }
        }).collect::<Vec<Type>>();
        let terminated = format!("{}\0", function);
        let pointer: Symbol<FP> = library
            .get(terminated.as_bytes())
            .map_err(|err| err.to_string())
            .unwrap();
        let code_ptr = CodePtr(*pointer as *mut _);
        let return_type = match ret_type {
            Some(t) => todo!("HANDLE RETURN TYPE"),
            None => Type::c_int(),
        };
        let cif = Cif::new(args.into_iter(), return_type);
        Ok(Function {
            cif,
            pointer: code_ptr,
        })
    }

    pub unsafe fn call(&self, args: Vec<PyObjectRef>, vm: &VirtualMachine) -> PyObjectRef {
        let args: Vec<Arg> = vec![];
        let result = self.cif.call(self.pointer, &args);
        vm.ctx.none()
    }
}

#[pyclass(module = "_ctypes", name = "CFuncPtr", base = "PyCData")]
#[derive(PyPayload)]
pub struct PyCFuncPtr {
    pub _name_: String,
    pub _argtypes_: AtomicCell<Vec<PyObjectRef>>,
    pub _restype_: AtomicCell<PyObjectRef>,
    _handle: PyObjectRef,
    _f: PyRwLock<Function>,
}

impl Debug for PyCFuncPtr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PyCFuncPtr")
            .field("_name_", &self._name_)
            .finish()
    }
}

impl Constructor for PyCFuncPtr {
    type Args = ();

    fn py_new(cls: PyTypeRef, args: Self::Args, vm: &VirtualMachine) -> PyResult {
        todo!("PyCFuncPtr::py_new")
    }
}

impl Callable for PyCFuncPtr {
    type Args = Vec<PyObjectRef>;

    fn call(zelf: &Py<Self>, _args: Self::Args, vm: &VirtualMachine) -> PyResult {
        todo!()
    }
}

#[pyclass(flags(BASETYPE), with(Callable, Constructor))]
impl PyCFuncPtr {

}

use crate::builtins::{PyStr, PyTuple, PyTypeRef};
use crate::stdlib::ctypes::PyCData;
use crate::types::{Callable, Constructor};
use crate::{AsObject, Py, PyObjectRef, PyResult, VirtualMachine};
use crossbeam_utils::atomic::AtomicCell;
use std::fmt::Debug;
use crate::class::StaticType;
use crate::stdlib::ctypes::base::PyCSimple;
use libffi::middle::{Arg, Cif, CodePtr, Type};
use libloading::Symbol;
use num_traits::ToPrimitive;
use rustpython_common::lock::PyRwLock;
use crate::convert::ToPyObject;
use crate::function::FuncArgs;
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
        ret_type: &Option<PyTypeRef>,
        vm: &VirtualMachine,
    ) -> PyResult<Self> {
        // map each arg to a PyCSimple
        let args = args.into_iter().map(|arg| {
            if arg.is_subclass(PyCSimple::static_type().as_object(), vm).unwrap() {
                let arg_type = arg.get_attr("_type_", vm).unwrap().str(vm).unwrap().to_string();
                let _value = arg.get_attr("value", vm).unwrap();
                match &*arg_type {
                    _ => todo!("HANDLE ARG TYPE")
                }
            } else {
                todo!("HANDLE ERROR")
            }
        }).collect::<Vec<Type>>();
        let terminated = format!("{}\0", function);
        let pointer: Symbol<FP> = unsafe { library
            .get(terminated.as_bytes())
            .map_err(|err| err.to_string())
            .unwrap() };
        let code_ptr = CodePtr(*pointer as *mut _);
        let return_type = match ret_type {
            Some(_t) => todo!("HANDLE RETURN TYPE"),
            None => Type::c_int(),
        };
        let cif = Cif::new(args.into_iter(), return_type);
        Ok(Function {
            cif,
            pointer: code_ptr,
        })
    }

    pub unsafe fn call(&self, _args: Vec<PyObjectRef>, vm: &VirtualMachine) -> PyObjectRef {
        let args: Vec<Arg> = vec![];
        // TODO: FIX return type
        let result: i32 = unsafe { self.cif.call(self.pointer, &args) };
        vm.ctx.new_int(result).into()
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
    pub handler: PyObjectRef
}

impl Debug for PyCFuncPtr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PyCFuncPtr")
            .field("name", &self.name)
            .finish()
    }
}

impl Constructor for PyCFuncPtr {
    type Args = FuncArgs;

    fn py_new(_cls: PyTypeRef, args: Self::Args, vm: &VirtualMachine) -> PyResult {
        let tuple = args.args.first().unwrap();
        let tuple: &Py<PyTuple> = tuple.downcast_ref().unwrap();
        let name = tuple.first().unwrap().downcast_ref::<PyStr>().unwrap().to_string();
        let handler = tuple.into_iter().nth(1).unwrap().clone();
        Ok(Self {
            _flags_: AtomicCell::new(0),
            name: PyRwLock::new(name),
            _restype_: PyRwLock::new(None),
            handler
        }.to_pyobject(vm))
    }
}

impl Callable for PyCFuncPtr {
    type Args = FuncArgs;
    fn call(zelf: &Py<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult {
        unsafe {
            let handle = zelf.handler.get_attr("_handle", vm)?;
            let handle = handle.try_int(vm)?.as_bigint().clone();
            let library_cache = crate::stdlib::ctypes::library::libcache().read();
            let library = library_cache.get_lib(handle.to_usize().unwrap()).unwrap();
            let inner_lib = library.lib.lock();
            let name = zelf.name.read();
            let res_type = zelf._restype_.read();
            let func = Function::load(inner_lib.as_ref().unwrap(), &name, &args.args, &res_type, vm)?;
            Ok(func.call(args.args, vm))
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

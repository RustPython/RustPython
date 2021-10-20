pub(crate) use _contextvars::make_module;

#[pymodule]
mod _contextvars {
    use rustpython_vm::builtins::{PyStrRef, PyTypeRef};
    use rustpython_vm::function::OptionalArg;
    use rustpython_vm::{PyObjectRef, PyRef, PyResult, PyValue, VirtualMachine};

    #[pyattr]
    #[pyclass(name)]
    #[derive(Debug, PyValue)]
    struct Context {}

    #[pyimpl]
    impl Context {}

    #[pyattr]
    #[pyclass(name)]
    #[derive(Debug, PyValue)]
    struct ContextVar {
        #[allow(dead_code)] // TODO: RUSTPYTHON
        name: String,
        #[allow(dead_code)] // TODO: RUSTPYTHON
        default: Option<PyObjectRef>,
    }

    #[derive(FromArgs)]
    struct ContextVarOptions {
        #[pyarg(positional)]
        name: PyStrRef,
        #[pyarg(any, optional)]
        default: OptionalArg<PyObjectRef>,
    }

    #[pyimpl]
    impl ContextVar {
        #[pymethod(magic)]
        fn init(&self, _args: ContextVarOptions, _vm: &VirtualMachine) -> PyResult<()> {
            unimplemented!("ContextVar.__init__() is currently under construction")
        }

        #[pyproperty]
        fn name(&self) -> String {
            self.name.clone()
        }

        #[pymethod]
        fn get(
            &self,
            _default: OptionalArg<PyObjectRef>,
            _vm: &VirtualMachine,
        ) -> PyResult<PyObjectRef> {
            unimplemented!("ContextVar.get() is currently under construction")
        }

        #[pymethod]
        fn set(&self, _value: PyObjectRef, _vm: &VirtualMachine) -> PyResult<()> {
            unimplemented!("ContextVar.set() is currently under construction")
        }

        #[pymethod]
        fn reset(
            _zelf: PyRef<Self>,
            _token: PyRef<ContextToken>,
            _vm: &VirtualMachine,
        ) -> PyResult<()> {
            unimplemented!("ContextVar.reset() is currently under construction")
        }

        #[pyclassmethod(magic)]
        fn class_getitem(_cls: PyTypeRef, _key: PyStrRef, _vm: &VirtualMachine) -> PyResult<()> {
            unimplemented!("ContextVar.__class_getitem__() is currently under construction")
        }

        #[pymethod(magic)]
        fn repr(_zelf: PyRef<Self>, _vm: &VirtualMachine) -> String {
            unimplemented!("<ContextVar name={{}} default={{}} at {{}}")
            // format!(
            //     "<ContextVar name={} default={:?} at {:#x}>",
            //     zelf.name.as_str(),
            //     zelf.default.map_or("", |x| PyStr::from(*x).as_str()),
            //     zelf.get_id()
            // )
        }
    }

    #[pyattr]
    #[pyclass(name = "Token")]
    #[derive(Debug, PyValue)]
    struct ContextToken {}

    #[pyimpl]
    impl ContextToken {}

    #[pyfunction]
    fn copy_context() {}
}

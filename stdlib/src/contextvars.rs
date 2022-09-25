pub(crate) use _contextvars::make_module;

#[pymodule]
mod _contextvars {
    use crate::vm::{
        builtins::{PyFunction, PyStrRef, PyTypeRef},
        function::{ArgCallable, FuncArgs, OptionalArg},
        types::Initializer,
        PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    };

    #[pyattr]
    #[pyclass(name = "Context")]
    #[derive(Debug, PyPayload)]
    struct PyContext {} // not to confuse with vm::Context

    #[pyclass(with(Initializer))]
    impl PyContext {
        #[pymethod]
        fn run(
            &self,
            _callable: ArgCallable,
            _args: FuncArgs,
            _vm: &VirtualMachine,
        ) -> PyResult<PyFunction> {
            unimplemented!("Context.run is currently under construction")
        }

        #[pymethod]
        fn copy(&self, _vm: &VirtualMachine) -> PyResult<Self> {
            unimplemented!("Context.copy is currently under construction")
        }

        #[pymethod(magic)]
        fn getitem(&self, _var: PyObjectRef) -> PyResult<PyObjectRef> {
            unimplemented!("Context.__getitem__ is currently under construction")
        }

        #[pymethod(magic)]
        fn contains(&self, _var: PyObjectRef) -> PyResult<bool> {
            unimplemented!("Context.__contains__ is currently under construction")
        }

        #[pymethod(magic)]
        fn len(&self) -> usize {
            unimplemented!("Context.__len__ is currently under construction")
        }

        #[pymethod(magic)]
        fn iter(&self) -> PyResult {
            unimplemented!("Context.__iter__ is currently under construction")
        }

        #[pymethod]
        fn get(
            &self,
            _key: PyObjectRef,
            _default: OptionalArg<PyObjectRef>,
        ) -> PyResult<PyObjectRef> {
            unimplemented!("Context.get is currently under construction")
        }

        #[pymethod]
        fn keys(_zelf: PyRef<Self>, _vm: &VirtualMachine) -> Vec<PyObjectRef> {
            unimplemented!("Context.keys is currently under construction")
        }

        #[pymethod]
        fn values(_zelf: PyRef<Self>, _vm: &VirtualMachine) -> Vec<PyObjectRef> {
            unimplemented!("Context.values is currently under construction")
        }
    }

    impl Initializer for PyContext {
        type Args = FuncArgs;

        fn init(_obj: PyRef<Self>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<()> {
            unimplemented!("Context.__init__ is currently under construction")
        }
    }

    #[pyattr]
    #[pyclass(name)]
    #[derive(Debug, PyPayload)]
    struct ContextVar {
        #[allow(dead_code)] // TODO: RUSTPYTHON
        name: String,
        #[allow(dead_code)] // TODO: RUSTPYTHON
        default: Option<PyObjectRef>,
    }

    #[derive(FromArgs)]
    struct ContextVarOptions {
        #[pyarg(positional)]
        #[allow(dead_code)] // TODO: RUSTPYTHON
        name: PyStrRef,
        #[pyarg(any, optional)]
        #[allow(dead_code)] // TODO: RUSTPYTHON
        default: OptionalArg<PyObjectRef>,
    }

    #[pyclass(with(Initializer))]
    impl ContextVar {
        #[pygetset]
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

    impl Initializer for ContextVar {
        type Args = ContextVarOptions;

        fn init(_obj: PyRef<Self>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<()> {
            unimplemented!("ContextVar.__init__() is currently under construction")
        }
    }

    #[pyattr]
    #[pyclass(name = "Token")]
    #[derive(Debug, PyPayload)]
    struct ContextToken {}

    #[derive(FromArgs)]
    struct ContextTokenOptions {
        #[pyarg(positional)]
        #[allow(dead_code)] // TODO: RUSTPYTHON
        context: PyObjectRef,
        #[pyarg(positional)]
        #[allow(dead_code)] // TODO: RUSTPYTHON
        var: PyObjectRef,
        #[pyarg(positional)]
        #[allow(dead_code)] // TODO: RUSTPYTHON
        old_value: PyObjectRef,
    }

    #[pyclass(with(Initializer))]
    impl ContextToken {
        #[pygetset]
        fn var(&self, _vm: &VirtualMachine) -> PyObjectRef {
            unimplemented!("Token.var() is currently under construction")
        }

        #[pygetset]
        fn old_value(&self, _vm: &VirtualMachine) -> PyObjectRef {
            unimplemented!("Token.old_value() is currently under construction")
        }

        #[pymethod(magic)]
        fn repr(_zelf: PyRef<Self>, _vm: &VirtualMachine) -> String {
            unimplemented!("<Token {{}}var={{}} at {{}}>")
        }
    }

    impl Initializer for ContextToken {
        type Args = ContextTokenOptions;

        fn init(_obj: PyRef<Self>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<()> {
            unimplemented!("Token.__init__() is currently under construction")
        }
    }

    #[pyfunction]
    fn copy_context() {}
}

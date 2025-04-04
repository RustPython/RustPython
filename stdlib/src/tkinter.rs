// cspell:ignore createcommand

pub(crate) use self::_tkinter::make_module;

#[pymodule]
mod _tkinter {
    use crate::builtins::PyTypeRef;
    use rustpython_vm::function::{Either, FuncArgs};
    use rustpython_vm::{PyResult, VirtualMachine, function::OptionalArg};

    use crate::common::lock::PyRwLock;
    use std::sync::Arc;
    use tk::cmd::*;
    use tk::*;

    #[pyattr]
    const TK_VERSION: &str = "8.6";
    #[pyattr]
    const TCL_VERSION: &str = "8.6";
    #[pyattr]
    const READABLE: i32 = 2;
    #[pyattr]
    const WRITABLE: i32 = 4;
    #[pyattr]
    const EXCEPTION: i32 = 8;

    fn demo() -> tk::TkResult<()> {
        let tk = make_tk!()?;
        let root = tk.root();
        root.add_label(-text("constructs widgets and layout step by step"))?
            .pack(())?;
        let f = root.add_frame(())?.pack(())?;
        let _btn = f
            .add_button("btn" - text("quit") - command("destroy ."))?
            .pack(())?;
        Ok(main_loop())
    }

    #[pyattr(once, name = "TclError")]
    fn tcl_error(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.new_exception_type(
            "zlib",
            "TclError",
            Some(vec![vm.ctx.exceptions.exception_type.to_owned()]),
        )
    }

    #[pyfunction]
    fn create(args: FuncArgs, _vm: &VirtualMachine) -> PyResult<TkApp> {
        // TODO: handle arguments
        // TODO: this means creating 2 tk instances is not possible.
        let tk = Tk::new(()).unwrap();
        Ok(TkApp {
            tk: Arc::new(PyRwLock::new(tk)),
        })
    }

    #[pyattr]
    #[pyclass(name = "tkapp")]
    #[derive(PyPayload)]
    struct TkApp {
        tk: Arc<PyRwLock<tk::Tk<()>>>,
    }

    unsafe impl Send for TkApp {}

    unsafe impl Sync for TkApp {}

    impl std::fmt::Debug for TkApp {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("TkApp").finish()
        }
    }

    #[pyclass]
    impl TkApp {
        #[pymethod]
        fn getvar(&self, name: &str) -> PyResult<String> {
            let tk = self.tk.read().unwrap();
            Ok(tk.getvar(name).unwrap())
        }

        #[pymethod]
        fn createcommand(&self, name: String, callback: PyObjectRef) {}
    }
}

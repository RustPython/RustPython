pub(crate) use self::_tkinter::make_module;

#[pymodule]
mod _tkinter {
    use crate::vm::VirtualMachine;
    use crate::builtins::PyTypeRef;
    use tk::*;
    use tk::cmd::*;

    #[pyattr]
    const TK_VERSION: &str = "8.6";
    #[pyattr]
    const TCL_VERSION: &str = "8.6";

    fn demo() -> tk::TkResult<()> {
        let tk = make_tk!()?;
        let root = tk.root();
        root.add_label( -text("constructs widgets and layout step by step") )?
            .pack(())?;
        let f = root
            .add_frame(())?
            .pack(())?;
        let _btn = f
            .add_button( "btn" -text("quit") -command("destroy .") )?
            .pack(())?;
        Ok(main_loop())
    }

    #[pyfunction]
    fn tk_demo() {
        let _ = demo();
    }

    #[pyattr(once, name = "TclError")]
    fn tcl_error(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.new_exception_type(
            "zlib",
            "TclError",
            Some(vec![vm.ctx.exceptions.exception_type.to_owned()]),
        )
    }
}

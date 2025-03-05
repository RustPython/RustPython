pub(crate) use self::_tkinter::make_module;

#[pymodule]
mod _tkinter {
    use crate::vm::VirtualMachine;
    use crate::builtins::PyTypeRef;

    #[pyattr(once, name = "TclError")]
    fn tcl_error(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.new_exception_type(
            "zlib",
            "TclError",
            Some(vec![vm.ctx.exceptions.exception_type.to_owned()]),
        )
    }
}

pub(crate) use sysconfig::make_module;

#[pymodule(name = "_sysconfig")]
pub(crate) mod sysconfig {
    use crate::{VirtualMachine, builtins::PyDictRef, convert::ToPyObject};

    #[pyfunction]
    fn config_vars(vm: &VirtualMachine) -> PyDictRef {
        let vars = vm.ctx.new_dict();

        // FIXME: This is an entirely wrong implementation of EXT_SUFFIX.
        // EXT_SUFFIX must be a string starting with "." for pip compatibility
        // Using ".pyd" causes pip's _generic_abi() to fall back to _cpython_abis()
        vars.set_item("EXT_SUFFIX", ".pyd".to_pyobject(vm), vm)
            .unwrap();
        vars.set_item("SOABI", vm.ctx.none(), vm).unwrap();

        vars.set_item("Py_GIL_DISABLED", true.to_pyobject(vm), vm)
            .unwrap();
        vars.set_item("Py_DEBUG", false.to_pyobject(vm), vm)
            .unwrap();

        vars
    }
}

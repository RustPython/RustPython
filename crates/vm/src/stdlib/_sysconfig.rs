pub(crate) use _sysconfig::module_def;

#[pymodule]
pub(crate) mod _sysconfig {
    use crate::{VirtualMachine, builtins::PyDictRef, convert::ToPyObject};

    #[cfg(windows)]
    #[pyfunction]
    fn get_platform() -> Option<&'static str> {
        cfg_select! {
            target_arch = "x86_64" => Some("win-amd64"),
            target_arch = "aarch64" => Some("win-arm64"),
            target_arch = "x86" => Some("win32"),
            target_arch = "arm" => Some("win-arm32"),
            _ => None,
        }
    }

    #[pyfunction]
    fn config_vars(vm: &VirtualMachine) -> PyDictRef {
        let vars = vm.ctx.new_dict();

        // FIXME: This is an entirely wrong implementation of EXT_SUFFIX.
        // EXT_SUFFIX must be a string starting with "." for pip compatibility
        // Using ".pyd" causes pip's _generic_abi() to fall back to _cpython_abis()
        vars.set_item("EXT_SUFFIX", ".pyd".to_pyobject(vm), vm)
            .unwrap();
        vars.set_item("SOABI", vm.ctx.none(), vm).unwrap();

        vars.set_item("Py_GIL_DISABLED", (1).to_pyobject(vm), vm)
            .unwrap();
        vars.set_item("Py_DEBUG", (0).to_pyobject(vm), vm).unwrap();

        vars
    }
}

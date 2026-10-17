pub(crate) use _sysconfig::module_def;

#[pymodule]
pub(crate) mod _sysconfig {
    #[cfg(windows)]
    use crate::builtins::PyStrRef;
    use crate::{PyResult, VirtualMachine, builtins::PyDictRef, convert::ToPyObject};

    #[cfg(windows)]
    #[pyfunction]
    fn get_platform() -> Option<&'static str> {
        match std::env::consts::ARCH {
            "x86_64" => Some("win-amd64"),
            "aarch64" => Some("win-arm64"),
            "x86" => Some("win32"),
            "arm" => Some("win-arm32"),
            _ => None,
        }
    }

    #[pyfunction]
    fn config_vars(vm: &VirtualMachine) -> PyResult<PyDictRef> {
        let vars = vm.ctx.new_dict();

        #[cfg(windows)]
        {
            let source: PyStrRef = vm.sys_module.get_attr("_vpath", vm)?.try_into_value(vm)?;
            if !source.as_wtf8().is_empty() {
                vars.set_item("srcdir", source.into(), vm)?;
            }
        }

        // FIXME: This is an entirely wrong implementation of EXT_SUFFIX.
        // EXT_SUFFIX must be a string starting with "." for pip compatibility
        // Using ".pyd" causes pip's _generic_abi() to fall back to _cpython_abis()
        vars.set_item("EXT_SUFFIX", ".pyd".to_pyobject(vm), vm)?;
        vars.set_item("SOABI", vm.ctx.none(), vm)?;

        vars.set_item("Py_GIL_DISABLED", (1).to_pyobject(vm), vm)?;
        vars.set_item("Py_DEBUG", (0).to_pyobject(vm), vm)?;

        Ok(vars)
    }
}

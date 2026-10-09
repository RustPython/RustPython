pub(crate) use _missing_stdlib_info::module_def;

#[pymodule]
mod _missing_stdlib_info {
    use crate::{
        Py, PyResult, VirtualMachine,
        builtins::{PyDictRef, PyModule},
    };

    fn module_exec(vm: &VirtualMachine, module: &Py<PyModule>) -> PyResult<()> {
        module.set_attr("_MISSING_STDLIB_MODULE_MESSAGES", messages(vm)?, vm)
    }

    // Native counterpart of CPython's build-generated _missing_stdlib_info.py.
    fn messages(vm: &VirtualMachine) -> PyResult<PyDictRef> {
        let messages = vm.ctx.new_dict();
        if !cfg!(windows) {
            for name in [
                "_overlapped",
                "_testconsole",
                "_winapi",
                "_wmi",
                "msvcrt",
                "nt",
                "winreg",
                "winsound",
            ] {
                messages.set_item(
                    name,
                    vm.ctx
                        .new_str(format!(
                            "Unsupported platform for Windows-only standard library module '{name}'"
                        ))
                        .into(),
                    vm,
                )?;
            }
        }
        if !cfg!(target_os = "macos") {
            messages.set_item(
                "_scproxy",
                vm.ctx
                    .new_str("Unsupported platform for standard library module '_scproxy'")
                    .into(),
                vm,
            )?;
        }
        Ok(messages)
    }
}

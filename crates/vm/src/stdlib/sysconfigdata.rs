// spell-checker: words LDSHARED ARFLAGS CPPFLAGS CCSHARED BASECFLAGS BLDSHARED

pub(crate) use _sysconfigdata::module_def;

#[pymodule]
mod _sysconfigdata {
    use crate::stdlib::sys::{RUST_MULTIARCH, multiarch, sysconfigdata_name};
    use crate::{
        Py, PyResult, VirtualMachine,
        builtins::{PyDictRef, PyModule},
        convert::ToPyObject,
    };

    fn module_exec(vm: &VirtualMachine, module: &Py<PyModule>) -> PyResult<()> {
        // Set build_time_vars attribute
        let build_time_vars = build_time_vars(vm);
        module.set_attr("build_time_vars", build_time_vars, vm)?;

        // Ensure the module is registered under the platform-specific name
        // (import_builtin() already handles this, but double-check for safety)
        let sys_modules = vm.sys_module.get_attr("modules", vm)?;
        let sysconfigdata_name = sysconfigdata_name();
        sys_modules.set_item(sysconfigdata_name.as_str(), module.to_owned().into(), vm)?;

        Ok(())
    }

    #[pyattr]
    fn build_time_vars(vm: &VirtualMachine) -> PyDictRef {
        let vars = vm.ctx.new_dict();
        let multiarch = multiarch();
        macro_rules! sysvars {
            ($($key:literal => $value:expr),*$(,)?) => {{
                $(vars.set_item($key, $value.to_pyobject(vm), vm).unwrap();)*
            }};
        }
        sysvars! {
            // Extension module suffix in CPython-compatible format
            "EXT_SUFFIX" => format!(".rustpython313-{multiarch}.so"),
            "MULTIARCH" => multiarch.clone(),
            "RUST_MULTIARCH" => RUST_MULTIARCH,
            // enough for tests to stop expecting urandom() to fail after restricting file resources
            "HAVE_GETRANDOM" => 1,
            // RustPython has no GIL (like free-threaded Python)
            "Py_GIL_DISABLED" => 1,
            // Compiler configuration for native extension builds
            "CC" => "cc",
            "CXX" => "c++",
            "CFLAGS" => "",
            "CPPFLAGS" => "",
            "LDFLAGS" => "",
            "LDSHARED" => "cc -shared",
            "CCSHARED" => "",
            "SHLIB_SUFFIX" => ".so",
            "SO" => ".so",
            "AR" => "ar",
            "ARFLAGS" => "rcs",
            "OPT" => "",
            "BASECFLAGS" => "",
            "BLDSHARED" => "cc -shared",
        }
        include!(concat!(env!("OUT_DIR"), "/env_vars.rs"));
        vars
    }
}

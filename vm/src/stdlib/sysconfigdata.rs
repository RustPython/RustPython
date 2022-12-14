pub(crate) use _sysconfigdata::make_module;

#[pymodule]
pub(crate) mod _sysconfigdata {
    use crate::{builtins::PyDictRef, convert::ToPyObject, stdlib::sys::MULTIARCH, VirtualMachine};

    #[pyattr]
    fn build_time_vars(vm: &VirtualMachine) -> PyDictRef {
        let vars = vm.ctx.new_dict();
        macro_rules! sysvars {
            ($($key:literal => $value:expr),*$(,)?) => {{
                $(vars.set_item($key, $value.to_pyobject(vm), vm).unwrap();)*
            }};
        }
        sysvars! {
            // fake shared module extension
            "EXT_SUFFIX" => format!(".rustpython-{MULTIARCH}"),
            "MULTIARCH" => MULTIARCH,
            // enough for tests to stop expecting urandom() to fail after restricting file resources
            "HAVE_GETRANDOM" => 1,
        }
        include!(concat!(env!("OUT_DIR"), "/env_vars.rs"));
        vars
    }
}

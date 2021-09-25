use crate::sysmodule::MULTIARCH;
use crate::{IntoPyObject, ItemProtocol, PyObjectRef, VirtualMachine};

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let vars = vm.ctx.new_dict();
    macro_rules! sysvars {
        ($($key:literal => $value:expr),*$(,)?) => {{
            $(vars.set_item($key, $value.into_pyobject(vm), vm).unwrap();)*
        }};
    }
    sysvars! {
        // fake shared module extension
        "EXT_SUFFIX" => format!(".rustpython-{}", MULTIARCH),
        "MULTIARCH" => MULTIARCH,
        // enough for tests to stop expecting urandom() to fail after restricting file resources
        "HAVE_GETRANDOM" => 1,
    }
    include!(concat!(env!("OUT_DIR"), "/env_vars.rs"));

    py_module!(vm, "_sysconfigdata", {
        "build_time_vars" => vars,
    })
}

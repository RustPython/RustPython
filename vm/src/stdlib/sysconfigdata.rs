use crate::pyobject::{ItemProtocol, PyObjectRef};
use crate::VirtualMachine;

use crate::sysmodule::MULTIARCH;

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let vars = vm.ctx.new_dict();
    macro_rules! hashmap {
        ($($key:literal => $value:expr),*$(,)?) => {{
            $(vars.set_item($key, vm.ctx.new_str($value), vm).unwrap();)*
        }};
    }
    hashmap! {
        // fake shared module extension
        "EXT_SUFFIX" => format!(".rustpython-{}", MULTIARCH),
        "MULTIARCH" => MULTIARCH,
    }
    include!(concat!(env!("OUT_DIR"), "/env_vars.rs"));

    py_module!(vm, "_sysconfigdata", {
        "build_time_vars" => vars,
    })
}

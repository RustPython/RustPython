use crate::pyobject::{ItemProtocol, PyObjectRef};
use crate::VirtualMachine;

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let vars = vm.ctx.new_dict();
    macro_rules! hashmap {
        ($($key:literal => $value:literal),*) => {{
            $(vars.set_item($key, vm.ctx.new_str($value.to_owned()), vm).unwrap();)*
        }};
    }
    include!(concat!(env!("OUT_DIR"), "/env_vars.rs"));

    py_module!(vm, "_sysconfigdata", {
        "build_time_vars" => vars,
    })
}

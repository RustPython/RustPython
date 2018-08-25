mod json;
use std::collections::HashMap;

use super::pyobject::PyObjectRef;
use super::vm::VirtualMachine;

pub fn get_modules(vm: &VirtualMachine) -> HashMap<String, PyObjectRef> {
    let mut modules = HashMap::new();
    modules.insert("json".to_string(), json::mk_module(vm.context()));
    modules
}

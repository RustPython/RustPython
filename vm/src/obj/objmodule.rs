use crate::frame::ScopeRef;
use crate::pyobject::{
    DictProtocol, PyContext, PyFuncArgs, PyObjectPayload, PyObjectRef, PyResult, TypeProtocol,
};
use crate::vm::VirtualMachine;

fn module_dir(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(obj, Some(vm.ctx.module_type()))]);
    let scope = get_scope(obj);
    let keys = scope
        .locals
        .get_key_value_pairs()
        .iter()
        .map(|(k, _v)| k.clone())
        .collect();
    Ok(vm.ctx.new_list(keys))
}

pub fn init(context: &PyContext) {
    let module_type = &context.module_type;
    context.set_attr(&module_type, "__dir__", context.new_rustfunc(module_dir));
}

fn get_scope(obj: &PyObjectRef) -> &ScopeRef {
    if let PyObjectPayload::Module { ref scope, .. } = &obj.payload {
        &scope
    } else {
        panic!("Can't get scope from non-module.")
    }
}

/*! Python `super` class.

*/

use super::super::pyobject::{
    AttributeProtocol, PyContext, PyFuncArgs, PyObjectRef, PyResult, TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objtype;

pub fn init(context: &PyContext) {
    let ref super_type = context.super_type;
    super_type.set_attr("__init__", context.new_rustfunc(super_init));
}

fn super_init(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    trace!("super.__init__ {:?}", args.args);
    arg_check!(vm, args, required = [(_inst, None)]);

    // TODO: implement complex logic here....

    Ok(vm.get_none())
}

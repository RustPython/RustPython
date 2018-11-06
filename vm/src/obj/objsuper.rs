/*! Python `super` class.

See also:

https://github.com/python/cpython/blob/50b48572d9a90c5bb36e2bef6179548ea927a35a/Objects/typeobject.c#L7663

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
    arg_check!(
        vm,
        args,
        required = [(inst, None)],
        optional = [(py_type, None), (py_obj, None)]
    );

    // Get the type:
    let py_type = if let Some(ty) = py_type {
        ty.clone()
    } else {
        // TODO: implement complex logic here....
        unimplemented!("TODO: get frame and determine instance and class?");
        // vm.get_none()
    };

    // Check type argument:
    if !objtype::isinstance(&py_type, &vm.get_type()) {
        let type_name = objtype::get_type_name(&py_type.typ());
        return Err(vm.new_type_error(format!(
            "super() argument 1 must be type, not {}",
            type_name
        )));
    }

    // Get the bound object:
    let py_obj = if let Some(obj) = py_obj {
        obj.clone()
    } else {
        vm.get_none()
    };

    // Check obj type:
    if !(objtype::isinstance(&py_obj, &py_type) || objtype::issubclass(&py_obj, &py_type)) {
        return Err(vm.new_type_error(format!(
            "super(type, obj): obj must be an instance or subtype of type"
        )));
    }

    // TODO: how to store those types?
    inst.set_attr("type", py_type);
    inst.set_attr("obj", py_obj);

    Ok(vm.get_none())
}

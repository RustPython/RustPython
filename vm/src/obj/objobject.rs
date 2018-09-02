use super::super::objbool;
use super::super::pyobject::{
    AttributeProtocol, IdProtocol, PyContext, PyFuncArgs, PyObject, PyObjectKind, PyObjectRef,
    PyResult, TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objdict;
use super::objtype;

pub fn new_instance(vm: &mut VirtualMachine, mut args: PyFuncArgs) -> PyResult {
    // more or less __new__ operator
    let type_ref = args.shift();
    let dict = vm.new_dict();
    let obj = PyObject::new(PyObjectKind::Instance { dict: dict }, type_ref.clone());
    Ok(obj)
}

pub fn call(vm: &mut VirtualMachine, mut args: PyFuncArgs) -> PyResult {
    let instance = args.shift();
    let function = objtype::get_attribute(vm, instance, &String::from("__call__"))?;
    vm.invoke(function, args)
}

pub fn create_object(type_type: PyObjectRef, object_type: PyObjectRef, dict_type: PyObjectRef) {
    (*object_type.borrow_mut()).kind = PyObjectKind::Class {
        name: String::from("object"),
        dict: objdict::new(dict_type),
        mro: vec![],
    };
    (*object_type.borrow_mut()).typ = Some(type_type.clone());
}

fn object_eq(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.object())), (other, None)]
    );
    Ok(vm.ctx.new_bool(zelf.is(other)))
}

fn object_ne(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.object())), (other, None)]
    );
    let eq = vm.call_method(zelf.clone(), "__eq__", vec![other.clone()])?;
    objbool::not(vm, &eq)
}

fn object_str(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, Some(vm.ctx.object()))]);
    vm.call_method(zelf.clone(), "__repr__", vec![])
}

fn object_repr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(obj, Some(vm.ctx.object()))]);
    let type_name = objtype::get_type_name(&obj.typ());
    let address = obj.get_id();
    Ok(vm.new_str(format!("<{} object at 0x{:x}>", type_name, address)))
}

pub fn init(context: &PyContext) {
    let ref object = context.object;
    object.set_attr("__new__", context.new_rustfunc(new_instance));
    object.set_attr("__init__", context.new_rustfunc(object_init));
    object.set_attr("__eq__", context.new_rustfunc(object_eq));
    object.set_attr("__ne__", context.new_rustfunc(object_ne));
    object.set_attr("__dict__", context.new_member_descriptor(object_dict));
    object.set_attr("__str__", context.new_rustfunc(object_str));
    object.set_attr("__repr__", context.new_rustfunc(object_repr));
}

fn object_init(vm: &mut VirtualMachine, _args: PyFuncArgs) -> PyResult {
    Ok(vm.ctx.none())
}

fn object_dict(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    match args.args[0].borrow().kind {
        PyObjectKind::Class { ref dict, .. } => Ok(dict.clone()),
        PyObjectKind::Instance { ref dict, .. } => Ok(dict.clone()),
        _ => Err(vm.new_type_error("TypeError: no dictionary.".to_string())),
    }
}

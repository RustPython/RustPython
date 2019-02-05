use super::super::pyobject::{
    AttributeProtocol, IdProtocol, PyContext, PyFuncArgs, PyObject, PyObjectPayload, PyObjectRef,
    PyResult, TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objbool;
use super::objdict;
use super::objstr;
use super::objtype;

pub fn new_instance(vm: &mut VirtualMachine, mut args: PyFuncArgs) -> PyResult {
    // more or less __new__ operator
    let type_ref = args.shift();
    let dict = vm.new_dict();
    let obj = PyObject::new(PyObjectPayload::Instance { dict }, type_ref.clone());
    Ok(obj)
}

pub fn create_object(type_type: PyObjectRef, object_type: PyObjectRef, dict_type: PyObjectRef) {
    (*object_type.borrow_mut()).payload = PyObjectPayload::Class {
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
    let eq = vm.call_method(zelf, "__eq__", vec![other.clone()])?;
    objbool::not(vm, &eq)
}

fn object_hash(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(_zelf, Some(vm.ctx.object()))]);

    // For now default to non hashable
    Err(vm.new_type_error("unhashable type".to_string()))
}

// TODO: is object the right place for delattr?
fn object_delattr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [
            (zelf, Some(vm.ctx.object())),
            (attr, Some(vm.ctx.str_type()))
        ]
    );

    // Get dict:
    let dict = match zelf.borrow().payload {
        PyObjectPayload::Class { ref dict, .. } => dict.clone(),
        PyObjectPayload::Instance { ref dict, .. } => dict.clone(),
        _ => return Err(vm.new_type_error("TypeError: no dictionary.".to_string())),
    };

    // Delete attr from dict:
    vm.call_method(&dict, "__delitem__", vec![attr.clone()])
}

fn object_str(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, Some(vm.ctx.object()))]);
    vm.call_method(zelf, "__repr__", vec![])
}

fn object_repr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(obj, Some(vm.ctx.object()))]);
    let type_name = objtype::get_type_name(&obj.typ());
    let address = obj.get_id();
    Ok(vm.new_str(format!("<{} object at 0x{:x}>", type_name, address)))
}

pub fn init(context: &PyContext) {
    let ref object = context.object;
    context.set_attr(&object, "__new__", context.new_rustfunc(new_instance));
    context.set_attr(&object, "__init__", context.new_rustfunc(object_init));
    context.set_attr(&object, "__eq__", context.new_rustfunc(object_eq));
    context.set_attr(&object, "__ne__", context.new_rustfunc(object_ne));
    context.set_attr(&object, "__delattr__", context.new_rustfunc(object_delattr));
    context.set_attr(
        &object,
        "__dict__",
        context.new_member_descriptor(object_dict),
    );
    context.set_attr(&object, "__hash__", context.new_rustfunc(object_hash));
    context.set_attr(&object, "__str__", context.new_rustfunc(object_str));
    context.set_attr(&object, "__repr__", context.new_rustfunc(object_repr));
    context.set_attr(
        &object,
        "__getattribute__",
        context.new_rustfunc(object_getattribute),
    );
}

fn object_init(vm: &mut VirtualMachine, _args: PyFuncArgs) -> PyResult {
    Ok(vm.ctx.none())
}

fn object_dict(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    match args.args[0].borrow().payload {
        PyObjectPayload::Class { ref dict, .. } => Ok(dict.clone()),
        PyObjectPayload::Instance { ref dict, .. } => Ok(dict.clone()),
        _ => Err(vm.new_type_error("TypeError: no dictionary.".to_string())),
    }
}

fn object_getattribute(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [
            (obj, Some(vm.ctx.object())),
            (name_str, Some(vm.ctx.str_type()))
        ]
    );
    let name = objstr::get_value(&name_str);
    trace!("object.__getattribute__({:?}, {:?})", obj, name);
    let cls = obj.typ();

    if let Some(attr) = cls.get_attr(&name) {
        let attr_class = attr.typ();
        if attr_class.has_attr("__set__") {
            if let Some(descriptor) = attr_class.get_attr("__get__") {
                return vm.invoke(
                    descriptor,
                    PyFuncArgs {
                        args: vec![attr, obj.clone(), cls],
                        kwargs: vec![],
                    },
                );
            }
        }
    }

    if let Some(obj_attr) = obj.get_attr(&name) {
        Ok(obj_attr)
    } else if let Some(attr) = cls.get_attr(&name) {
        vm.call_get_descriptor(attr, obj.clone())
    } else {
        if let Some(getter) = cls.get_attr("__getattr__") {
            vm.invoke(
                getter,
                PyFuncArgs {
                    args: vec![cls, name_str.clone()],
                    kwargs: vec![],
                },
            )
        } else {
            let attribute_error = vm.context().exceptions.attribute_error.clone();
            Err(vm.new_exception(
                attribute_error,
                format!("{} has no attribute '{}'", obj.borrow(), name),
            ))
        }
    }
}

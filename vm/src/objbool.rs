use super::objtype;
use super::pyobject::{
    AttributeProtocol, PyContext, PyFuncArgs, PyObjectKind, PyObjectRef, PyResult, TypeProtocol,
};
use super::vm::VirtualMachine;

pub fn boolval(vm: &mut VirtualMachine, obj: PyObjectRef) -> Result<bool, PyObjectRef> {
    let result = match obj.borrow().kind {
        PyObjectKind::Boolean { value } => value,
        PyObjectKind::Integer { value } => value != 0,
        PyObjectKind::Float { value } => value != 0.0,
        PyObjectKind::List { ref elements } => !elements.is_empty(),
        PyObjectKind::Tuple { ref elements } => !elements.is_empty(),
        PyObjectKind::Dict { ref elements } => !elements.is_empty(),
        PyObjectKind::String { ref value } => !value.is_empty(),
        PyObjectKind::None { .. } => false,
        _ => {
            if let Ok(f) = objtype::get_attribute(vm, obj.clone(), &String::from("__bool__")) {
                match vm.invoke(f, PyFuncArgs::new()) {
                    Ok(result) => match result.borrow().kind {
                        PyObjectKind::Boolean { value } => value,
                        _ => return Err(vm.new_type_error(String::from("TypeError"))),
                    },
                    Err(err) => return Err(err),
                }
            } else {
                true
            }
        }
    };
    Ok(result)
}

pub fn init(context: &PyContext) {
    let ref bool_type = context.bool_type;
    bool_type.set_attr("__new__", context.new_rustfunc(bool_new));
    bool_type.set_attr("__str__", context.new_rustfunc(bool_str));
}

// Retrieve inner int value:
pub fn get_value(obj: &PyObjectRef) -> bool {
    if let PyObjectKind::Boolean { value } = &obj.borrow().kind {
        *value
    } else {
        panic!("Inner error getting inner boolean");
    }
}

fn bool_str(vm: &mut VirtualMachine, args: PyFuncArgs) -> Result<PyObjectRef, PyObjectRef> {
    arg_check!(vm, args, required = [(obj, Some(vm.ctx.bool_type()))]);
    let v = get_value(obj);
    let s = if v {
        "True".to_string()
    } else {
        "True".to_string()
    };
    Ok(vm.new_str(s))
}

fn bool_new(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(_zelf, Some(vm.ctx.type_type()))],
        optional = [(val, None)]
    );
    Ok(match val {
        Some(val) => {
            let bv = boolval(vm, val.clone())?;
            vm.new_bool(bv.clone())
        }
        None => vm.context().new_bool(false),
    })
}

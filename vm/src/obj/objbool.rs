use super::super::pyobject::{
    AttributeProtocol, PyContext, PyFuncArgs, PyObjectKind, PyObjectRef, PyResult, TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objtype;
use num_traits::Zero;

pub fn boolval(vm: &mut VirtualMachine, obj: PyObjectRef) -> Result<bool, PyObjectRef> {
    let result = match obj.borrow().kind {
        PyObjectKind::Integer { ref value } => !value.is_zero(),
        PyObjectKind::Float { value } => value != 0.0,
        PyObjectKind::List { ref elements } => !elements.is_empty(),
        PyObjectKind::Tuple { ref elements } => !elements.is_empty(),
        PyObjectKind::Dict { ref elements } => !elements.is_empty(),
        PyObjectKind::String { ref value } => !value.is_empty(),
        PyObjectKind::None { .. } => false,
        _ => {
            if let Ok(f) = vm.get_attribute(obj.clone(), &String::from("__bool__")) {
                let bool_res = vm.invoke(f, PyFuncArgs::default())?;
                let v = match bool_res.borrow().kind {
                    PyObjectKind::Integer { ref value } => !value.is_zero(),
                    _ => return Err(vm.new_type_error(String::from("TypeError"))),
                };
                v
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
    bool_type.set_attr("__repr__", context.new_rustfunc(bool_repr));
}

pub fn not(vm: &mut VirtualMachine, obj: &PyObjectRef) -> PyResult {
    if objtype::isinstance(obj, &vm.ctx.bool_type()) {
        let value = get_value(obj);
        Ok(vm.ctx.new_bool(!value))
    } else {
        Err(vm.new_type_error(format!("Can only invert a bool, on {:?}", obj)))
    }
}

// Retrieve inner int value:
pub fn get_value(obj: &PyObjectRef) -> bool {
    if let PyObjectKind::Integer { value } = &obj.borrow().kind {
        !value.is_zero()
    } else {
        panic!("Inner error getting inner boolean");
    }
}

fn bool_repr(vm: &mut VirtualMachine, args: PyFuncArgs) -> Result<PyObjectRef, PyObjectRef> {
    arg_check!(vm, args, required = [(obj, Some(vm.ctx.bool_type()))]);
    let v = get_value(obj);
    let s = if v {
        "True".to_string()
    } else {
        "False".to_string()
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

use super::objtype;
use super::pyobject::{
    AttributeProtocol, PyContext, PyFuncArgs, PyObjectKind, PyObjectRef, PyResult,
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
        _ => {
            let f = objtype::get_attribute(vm, obj.clone(), &String::from("__bool__"))?;
            match vm.invoke(f, PyFuncArgs::new()) {
                Ok(result) => match result.borrow().kind {
                    PyObjectKind::Boolean { value } => value,
                    _ => return Err(vm.new_type_error(String::from("TypeError"))),
                },
                Err(err) => return Err(err),
            }
        }
    };
    Ok(result)
}

pub fn init(context: &PyContext) {
    let ref bool_type = context.bool_type;
    bool_type.set_attr("__new__", context.new_rustfunc(bool_new));
}

fn bool_new(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    if args.args.len() == 1 {
        return Ok(vm.context().new_bool(false));
    }
    let ref value = boolval(vm, args.args[1].clone())?;
    Ok(vm.new_bool(value.clone()))
}

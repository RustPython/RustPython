use super::objdict::PyDict;
use super::objfloat::PyFloat;
use super::objstr::PyString;
use super::objtype;
use crate::pyobject::{
    IntoPyObject, PyContext, PyFuncArgs, PyObjectPayload, PyObjectRef, PyResult, TypeProtocol,
};
use crate::vm::VirtualMachine;
use num_traits::Zero;

impl IntoPyObject for bool {
    fn into_pyobject(self, ctx: &PyContext) -> PyResult {
        Ok(ctx.new_bool(self))
    }
}

pub fn boolval(vm: &mut VirtualMachine, obj: PyObjectRef) -> PyResult<bool> {
    if let Some(s) = obj.payload::<PyString>() {
        return Ok(!s.value.is_empty());
    }
    if let Some(value) = obj.payload::<PyFloat>() {
        return Ok(*value != PyFloat::from(0.0));
    }
    if let Some(dict) = obj.payload::<PyDict>() {
        return Ok(!dict.entries.borrow().is_empty());
    }
    let result = match obj.payload {
        PyObjectPayload::Integer { ref value } => !value.is_zero(),
        PyObjectPayload::Sequence { ref elements } => !elements.borrow().is_empty(),
        _ => {
            if let Ok(f) = vm.get_method(obj.clone(), "__bool__") {
                let bool_res = vm.invoke(f, PyFuncArgs::default())?;
                match bool_res.payload {
                    PyObjectPayload::Integer { ref value } => !value.is_zero(),
                    _ => return Err(vm.new_type_error(String::from("TypeError"))),
                }
            } else {
                true
            }
        }
    };
    Ok(result)
}

pub fn init(context: &PyContext) {
    let bool_doc = "bool(x) -> bool

Returns True when the argument x is true, False otherwise.
The builtins True and False are the only two instances of the class bool.
The class bool is a subclass of the class int, and cannot be subclassed.";

    let bool_type = &context.bool_type;
    context.set_attr(&bool_type, "__new__", context.new_rustfunc(bool_new));
    context.set_attr(&bool_type, "__repr__", context.new_rustfunc(bool_repr));
    context.set_attr(&bool_type, "__doc__", context.new_str(bool_doc.to_string()));
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
    if let PyObjectPayload::Integer { value } = &obj.payload {
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
            vm.new_bool(bv)
        }
        None => vm.context().new_bool(false),
    })
}

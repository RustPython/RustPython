use num_traits::Zero;

use crate::function::PyFuncArgs;
use crate::pyobject::{IntoPyObject, PyContext, PyObjectRef, PyResult, TryFromObject};
use crate::vm::VirtualMachine;

use super::objint::PyInt;
use super::objtype;

impl IntoPyObject for bool {
    fn into_pyobject(self, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.new_bool(self))
    }
}

impl TryFromObject for bool {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<bool> {
        boolval(vm, obj)
    }
}

pub fn boolval(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<bool> {
    Ok(if let Ok(f) = vm.get_method(obj.clone(), "__bool__") {
        let bool_res = vm.invoke(f, PyFuncArgs::default())?;
        match bool_res.payload::<PyInt>() {
            Some(i) => !i.as_bigint().is_zero(),
            None => return Err(vm.new_type_error(String::from("TypeError"))),
        }
    } else {
        true
    })
}

pub fn init(context: &PyContext) {
    let bool_doc = "bool(x) -> bool

Returns True when the argument x is true, False otherwise.
The builtins True and False are the only two instances of the class bool.
The class bool is a subclass of the class int, and cannot be subclassed.";

    let bool_type = &context.bool_type;
    extend_class!(context, bool_type, {
        "__new__" => context.new_rustfunc(bool_new),
        "__repr__" => context.new_rustfunc(bool_repr),
        "__or__" => context.new_rustfunc(bool_or),
        "__ror__" => context.new_rustfunc(bool_ror),
        "__and__" => context.new_rustfunc(bool_and),
        "__rand__" => context.new_rustfunc(bool_rand),
        "__xor__" => context.new_rustfunc(bool_xor),
        "__rxor__" => context.new_rustfunc(bool_rxor),
        "__doc__" => context.new_str(bool_doc.to_string())
    });
}

pub fn not(vm: &VirtualMachine, obj: &PyObjectRef) -> PyResult {
    if objtype::isinstance(obj, &vm.ctx.bool_type()) {
        let value = get_value(obj);
        Ok(vm.ctx.new_bool(!value))
    } else {
        Err(vm.new_type_error(format!("Can only invert a bool, on {:?}", obj)))
    }
}

// Retrieve inner int value:
pub fn get_value(obj: &PyObjectRef) -> bool {
    !obj.payload::<PyInt>().unwrap().as_bigint().is_zero()
}

fn bool_repr(vm: &VirtualMachine, args: PyFuncArgs) -> Result<PyObjectRef, PyObjectRef> {
    arg_check!(vm, args, required = [(obj, Some(vm.ctx.bool_type()))]);
    let v = get_value(obj);
    let s = if v {
        "True".to_string()
    } else {
        "False".to_string()
    };
    Ok(vm.new_str(s))
}

fn do_bool_or(vm: &VirtualMachine, lhs: &PyObjectRef, rhs: &PyObjectRef) -> PyResult {
    if objtype::isinstance(lhs, &vm.ctx.bool_type())
        && objtype::isinstance(rhs, &vm.ctx.bool_type())
    {
        let lhs = get_value(lhs);
        let rhs = get_value(rhs);
        (lhs || rhs).into_pyobject(vm)
    } else {
        Ok(lhs.payload::<PyInt>().unwrap().or(rhs.clone(), vm))
    }
}

fn bool_or(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(lhs, None), (rhs, None)]);
    do_bool_or(vm, lhs, rhs)
}

fn bool_ror(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(rhs, None), (lhs, None)]);
    do_bool_or(vm, lhs, rhs)
}

fn do_bool_and(vm: &VirtualMachine, lhs: &PyObjectRef, rhs: &PyObjectRef) -> PyResult {
    if objtype::isinstance(lhs, &vm.ctx.bool_type())
        && objtype::isinstance(rhs, &vm.ctx.bool_type())
    {
        let lhs = get_value(lhs);
        let rhs = get_value(rhs);
        (lhs && rhs).into_pyobject(vm)
    } else {
        Ok(lhs.payload::<PyInt>().unwrap().and(rhs.clone(), vm))
    }
}

fn bool_and(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(lhs, None), (rhs, None)]);
    do_bool_and(vm, lhs, rhs)
}

fn bool_rand(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(rhs, None), (lhs, None)]);
    do_bool_and(vm, lhs, rhs)
}

fn do_bool_xor(vm: &VirtualMachine, lhs: &PyObjectRef, rhs: &PyObjectRef) -> PyResult {
    if objtype::isinstance(lhs, &vm.ctx.bool_type())
        && objtype::isinstance(rhs, &vm.ctx.bool_type())
    {
        let lhs = get_value(lhs);
        let rhs = get_value(rhs);
        (lhs ^ rhs).into_pyobject(vm)
    } else {
        Ok(lhs.payload::<PyInt>().unwrap().xor(rhs.clone(), vm))
    }
}

fn bool_xor(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(lhs, None), (rhs, None)]);
    do_bool_xor(vm, lhs, rhs)
}

fn bool_rxor(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(rhs, None), (lhs, None)]);
    do_bool_xor(vm, lhs, rhs)
}

fn bool_new(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
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

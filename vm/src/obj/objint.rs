use super::super::pyobject::{
    AttributeProtocol, FromPyObjectRef, PyContext, PyFuncArgs, PyObjectKind, PyObjectRef, PyResult,
    TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objfloat;
use super::objtype;

fn str(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(int, Some(vm.ctx.int_type()))]);
    let v = get_value(int);
    Ok(vm.new_str(v.to_string()))
}

// __init__ (store value into objectkind)
fn int_init(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.int_type())), (arg, None)]
    );
    let val = if objtype::isinstance(arg.clone(), vm.ctx.int_type()) {
        get_value(arg)
    } else if objtype::isinstance(arg.clone(), vm.ctx.float_type()) {
        objfloat::get_value(arg) as i32
    } else {
        return Err(vm.new_type_error("Cannot construct int".to_string()));
    };
    set_value(zelf, val);
    Ok(vm.get_none())
}

// Retrieve inner int value:
pub fn get_value(obj: &PyObjectRef) -> i32 {
    if let PyObjectKind::Integer { value } = &obj.borrow().kind {
        *value
    } else {
        panic!("Inner error getting int {:?}", obj);
    }
}

fn set_value(obj: &PyObjectRef, value: i32) {
    obj.borrow_mut().kind = PyObjectKind::Integer { value };
}

impl FromPyObjectRef for i32 {
    fn from_pyobj(obj: &PyObjectRef) -> i32 {
        get_value(obj)
    }
}

fn int_eq(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.int_type())), (other, None)]
    );
    let result = if objtype::isinstance(other.clone(), vm.ctx.int_type()) {
        let zelf = i32::from_pyobj(zelf);
        let other = i32::from_pyobj(other);
        zelf == other
    } else if objtype::isinstance(other.clone(), vm.ctx.float_type()) {
        let zelf = i32::from_pyobj(zelf) as f64;
        let other = objfloat::get_value(other);
        zelf == other
    } else {
        false
    };
    Ok(vm.ctx.new_bool(result))
}

fn int_add(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(i, Some(vm.ctx.int_type())), (i2, None)]
    );
    let i = i32::from_pyobj(i);
    if objtype::isinstance(i2.clone(), vm.ctx.int_type()) {
        Ok(vm.ctx.new_int(i + get_value(i2)))
    } else if objtype::isinstance(i2.clone(), vm.ctx.float_type()) {
        Ok(vm.ctx.new_float(i as f64 + objfloat::get_value(i2)))
    } else {
        Err(vm.new_type_error(format!("Cannot add {:?} and {:?}", i, i2)))
    }
}

fn int_sub(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(i, Some(vm.ctx.int_type())), (i2, None)]
    );
    let i = i32::from_pyobj(i);
    if objtype::isinstance(i2.clone(), vm.ctx.int_type()) {
        Ok(vm.ctx.new_int(i - get_value(i2)))
    } else if objtype::isinstance(i2.clone(), vm.ctx.float_type()) {
        Ok(vm.ctx.new_float(i as f64 - objfloat::get_value(i2)))
    } else {
        Err(vm.new_type_error(format!("Cannot substract {:?} and {:?}", i, i2)))
    }
}

fn int_mul(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(i, Some(vm.ctx.int_type())), (i2, None)]
    );
    if objtype::isinstance(i2.clone(), vm.ctx.int_type()) {
        Ok(vm.ctx.new_int(get_value(i) * get_value(i2)))
    } else {
        Err(vm.new_type_error(format!("Cannot multiply {:?} and {:?}", i, i2)))
    }
}

fn int_truediv(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(i, Some(vm.ctx.int_type())), (i2, None)]
    );
    let v1 = get_value(i);
    if objtype::isinstance(i2.clone(), vm.ctx.int_type()) {
        Ok(vm.ctx.new_float(v1 as f64 / get_value(i2) as f64))
    } else if objtype::isinstance(i2.clone(), vm.ctx.float_type()) {
        Ok(vm.ctx.new_float(v1 as f64 / objfloat::get_value(i2)))
    } else {
        Err(vm.new_type_error(format!("Cannot multiply {:?} and {:?}", i, i2)))
    }
}

fn int_mod(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(i, Some(vm.ctx.int_type())), (i2, None)]
    );
    let v1 = get_value(i);
    if objtype::isinstance(i2.clone(), vm.ctx.int_type()) {
        Ok(vm.ctx.new_int(v1 % get_value(i2)))
    } else {
        Err(vm.new_type_error(format!("Cannot modulo {:?} and {:?}", i, i2)))
    }
}

fn int_pow(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(i, Some(vm.ctx.int_type())), (i2, None)]
    );
    let v1 = get_value(i);
    if objtype::isinstance(i2.clone(), vm.ctx.int_type()) {
        let v2 = get_value(i2);
        Ok(vm.ctx.new_int(v1.pow(v2 as u32)))
    } else if objtype::isinstance(i2.clone(), vm.ctx.float_type()) {
        let v2 = objfloat::get_value(i2);
        Ok(vm.ctx.new_float((v1 as f64).powf(v2)))
    } else {
        Err(vm.new_type_error(format!("Cannot modulo {:?} and {:?}", i, i2)))
    }
}

pub fn init(context: &PyContext) {
    let ref int_type = context.int_type;
    int_type.set_attr("__eq__", context.new_rustfunc(int_eq));
    int_type.set_attr("__add__", context.new_rustfunc(int_add));
    int_type.set_attr("__init__", context.new_rustfunc(int_init));
    int_type.set_attr("__mod__", context.new_rustfunc(int_mod));
    int_type.set_attr("__mul__", context.new_rustfunc(int_mul));
    int_type.set_attr("__pow__", context.new_rustfunc(int_pow));
    int_type.set_attr("__repr__", context.new_rustfunc(str));
    int_type.set_attr("__str__", context.new_rustfunc(str));
    int_type.set_attr("__sub__", context.new_rustfunc(int_sub));
    int_type.set_attr("__truediv__", context.new_rustfunc(int_truediv));
}

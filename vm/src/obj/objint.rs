use super::super::pyobject::{
    AttributeProtocol, FromPyObjectRef, PyContext, PyFuncArgs, PyObject, PyObjectKind, PyObjectRef,
    PyResult, TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objfloat;
use super::objtype;

fn str(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(int, Some(vm.ctx.int_type()))]);
    let v = get_value(int);
    Ok(vm.new_str(v.to_string()))
}

fn int_new(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    let ref cls = args.args[0];
    if !objtype::issubclass(cls.clone(), vm.ctx.int_type()) {
        return Err(vm.new_type_error(format!("{:?} is not a subtype of int", cls)));
    }
    let val = to_int(vm, &args.args[1].clone())?;
    Ok(PyObject::new(
        PyObjectKind::Integer { value: val },
        cls.clone(),
    ))
}

// Casting function:
pub fn to_int(vm: &mut VirtualMachine, obj: &PyObjectRef) -> Result<i32, PyObjectRef> {
    let val = if objtype::isinstance(obj.clone(), vm.ctx.int_type()) {
        get_value(obj)
    } else if objtype::isinstance(obj.clone(), vm.ctx.float_type()) {
        objfloat::get_value(obj) as i32
    } else {
        return Err(vm.new_type_error("Cannot construct int".to_string()));
    };
    Ok(val)
}

// Retrieve inner int value:
pub fn get_value(obj: &PyObjectRef) -> i32 {
    if let PyObjectKind::Integer { value } = &obj.borrow().kind {
        *value
    } else {
        panic!("Inner error getting int {:?}", obj);
    }
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

fn int_xor(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(i, Some(vm.ctx.int_type())), (i2, None)]
    );
    let v1 = get_value(i);
    if objtype::isinstance(i2.clone(), vm.ctx.int_type()) {
        let v2 = get_value(i2);
        Ok(vm.ctx.new_int(v1 ^ v2))
    } else {
        Err(vm.new_type_error(format!("Cannot xor {:?} and {:?}", i, i2)))
    }
}

fn int_or(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(i, Some(vm.ctx.int_type())), (i2, None)]
    );
    let v1 = get_value(i);
    if objtype::isinstance(i2.clone(), vm.ctx.int_type()) {
        let v2 = get_value(i2);
        Ok(vm.ctx.new_int(v1 | v2))
    } else {
        Err(vm.new_type_error(format!("Cannot or {:?} and {:?}", i, i2)))
    }
}

fn int_and(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(i, Some(vm.ctx.int_type())), (i2, None)]
    );
    let v1 = get_value(i);
    if objtype::isinstance(i2.clone(), vm.ctx.int_type()) {
        let v2 = get_value(i2);
        Ok(vm.ctx.new_int(v1 & v2))
    } else {
        Err(vm.new_type_error(format!("Cannot and {:?} and {:?}", i, i2)))
    }
}

pub fn init(context: &PyContext) {
    let ref int_type = context.int_type;
    int_type.set_attr("__eq__", context.new_rustfunc(int_eq));
    int_type.set_attr("__add__", context.new_rustfunc(int_add));
    int_type.set_attr("__and__", context.new_rustfunc(int_and));
    int_type.set_attr("__new__", context.new_rustfunc(int_new));
    int_type.set_attr("__mod__", context.new_rustfunc(int_mod));
    int_type.set_attr("__mul__", context.new_rustfunc(int_mul));
    int_type.set_attr("__or__", context.new_rustfunc(int_or));
    int_type.set_attr("__pow__", context.new_rustfunc(int_pow));
    int_type.set_attr("__repr__", context.new_rustfunc(str));
    int_type.set_attr("__str__", context.new_rustfunc(str));
    int_type.set_attr("__sub__", context.new_rustfunc(int_sub));
    int_type.set_attr("__truediv__", context.new_rustfunc(int_truediv));
    int_type.set_attr("__xor__", context.new_rustfunc(int_xor));
}

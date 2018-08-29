use super::objfloat;
use super::objtype;
use super::pyobject::{
    AttributeProtocol, PyContext, PyFuncArgs, PyObjectKind, PyObjectRef, PyResult, TypeProtocol,
};
use super::vm::VirtualMachine;

fn str(vm: &mut VirtualMachine, args: PyFuncArgs) -> Result<PyObjectRef, PyObjectRef> {
    arg_check!(vm, args, required = [(int, Some(vm.ctx.int_type()))]);
    let v = get_value(int.clone());
    Ok(vm.new_str(v.to_string()))
}

// Retrieve inner int value:
pub fn get_value(obj: PyObjectRef) -> i32 {
    if let PyObjectKind::Integer { value } = &obj.borrow().kind {
        *value
    } else {
        panic!("Inner error getting int");
    }
}

fn int_add(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(i, Some(vm.ctx.int_type())), (i2, None)]
    );
    if objtype::isinstance(i2.clone(), vm.ctx.int_type()) {
        Ok(vm.ctx.new_int(get_value(i.clone()) + get_value(i2.clone())))
    } else if objtype::isinstance(i2.clone(), vm.ctx.float_type()) {
        Ok(vm
            .ctx
            .new_float(get_value(i.clone()) as f64 + objfloat::get_value(i2.clone())))
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
    if objtype::isinstance(i2.clone(), vm.ctx.int_type()) {
        Ok(vm.ctx.new_int(get_value(i.clone()) - get_value(i2.clone())))
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
        Ok(vm.ctx.new_int(get_value(i.clone()) * get_value(i2.clone())))
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
    if objtype::isinstance(i2.clone(), vm.ctx.int_type()) {
        Ok(vm
            .ctx
            .new_float(get_value(i.clone()) as f64 / get_value(i2.clone()) as f64))
    } else if objtype::isinstance(i2.clone(), vm.ctx.float_type()) {
        Ok(vm
            .ctx
            .new_float(get_value(i.clone()) as f64 / objfloat::get_value(i2.clone())))
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
    if objtype::isinstance(i2.clone(), vm.ctx.int_type()) {
        Ok(vm.ctx.new_int(get_value(i.clone()) % get_value(i2.clone())))
    } else {
        Err(vm.new_type_error(format!("Cannot modulo {:?} and {:?}", i, i2)))
    }
}

pub fn init(context: &PyContext) {
    let ref int_type = context.int_type;
    int_type.set_attr("__add__", context.new_rustfunc(int_add));
    int_type.set_attr("__mod__", context.new_rustfunc(int_mod));
    int_type.set_attr("__mul__", context.new_rustfunc(int_mul));
    int_type.set_attr("__repr__", context.new_rustfunc(str));
    int_type.set_attr("__str__", context.new_rustfunc(str));
    int_type.set_attr("__sub__", context.new_rustfunc(int_sub));
    int_type.set_attr("__truediv__", context.new_rustfunc(int_truediv));
}

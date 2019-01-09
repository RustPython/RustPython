//! Implementation of the python bytearray object.

use super::super::pyobject::{
    PyContext, PyFuncArgs, PyObject, PyObjectKind, PyObjectRef, PyResult, TypeProtocol,
};

use super::objint;

use super::super::vm::VirtualMachine;
use super::objbytes::get_value;
use super::objtype;
use num_traits::ToPrimitive;
use num_bigint::{ToBigInt};

// Binary data support

/// Fill bytearray class methods dictionary.
pub fn init(context: &PyContext) {
    let ref bytearray_type = context.bytearray_type;
    context.set_attr(
        &bytearray_type,
        "__eq__",
        context.new_rustfunc(bytearray_eq),
    );
    context.set_attr(
        &bytearray_type,
        "__new__",
        context.new_rustfunc(bytearray_new),
    );
    context.set_attr(
        &bytearray_type,
        "__repr__",
        context.new_rustfunc(bytearray_repr),
    );
    context.set_attr(
        &bytearray_type,
        "__len__",
        context.new_rustfunc(bytesarray_len),
    );
}

fn bytearray_new(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(cls, None)],
        optional = [(val_option, None)]
    );
    if !objtype::issubclass(cls, &vm.ctx.bytearray_type()) {
        return Err(vm.new_type_error(format!("{:?} is not a subtype of bytearray", cls)));
    }

    // Create bytes data:
    let value = if let Some(ival) = val_option {
        let elements = vm.extract_elements(ival)?;
        let mut data_bytes = vec![];
        for elem in elements.iter() {
            let v = objint::to_int(vm, elem, 10)?;
            data_bytes.push(v.to_u8().unwrap());
        }
        data_bytes
    // return Err(vm.new_type_error("Cannot construct bytes".to_string()));
    } else {
        vec![]
    };

    Ok(PyObject::new(
        PyObjectKind::Bytes { value: value },
        cls.clone(),
    ))
}

fn bytesarray_len(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(a, Some(vm.ctx.bytearray_type()))]
    );

    let byte_vec = get_value(a).to_vec();
    let value = byte_vec.len().to_bigint();
    Ok(vm.ctx.new_int(value.unwrap()))
}


fn bytearray_eq(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(a, Some(vm.ctx.bytearray_type())), (b, None)]
    );

    let result = if objtype::isinstance(b, &vm.ctx.bytearray_type()) {
        get_value(a).to_vec() == get_value(b).to_vec()
    } else {
        false
    };
    Ok(vm.ctx.new_bool(result))
}

/*
fn bytearray_getitem(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(obj, Some(vm.ctx.bytearray_type())), (needle, None)]
    );
    let elements = get_elements(obj);
    get_item(vm, list, &, needle.clone())
}
*/
/*
fn set_value(obj: &PyObjectRef, value: Vec<u8>) {
    obj.borrow_mut().kind = PyObjectKind::Bytes { value };
}
*/

fn bytearray_repr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(obj, Some(vm.ctx.bytearray_type()))]);
    let value = get_value(obj);
    let data = String::from_utf8(value.to_vec()).unwrap();
    Ok(vm.new_str(format!("bytearray(b'{}')", data)))
}

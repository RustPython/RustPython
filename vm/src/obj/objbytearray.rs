//! Implementation of the python bytearray object.

use super::super::pyobject::{
    AttributeProtocol, PyContext, PyFuncArgs, PyObject, PyObjectKind, PyObjectRef, PyResult,
    TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objbytes::get_value;
use super::objint;
use super::objtype;
use num_traits::ToPrimitive;
// Binary data support

/// Fill bytearray class methods dictionary.
pub fn init(context: &PyContext) {
    let ref bytearray_type = context.bytearray_type;
    bytearray_type.set_attr("__eq__", context.new_rustfunc(bytearray_eq));
    bytearray_type.set_attr("__new__", context.new_rustfunc(bytearray_new));
    bytearray_type.set_attr("__repr__", context.new_rustfunc(bytearray_repr));
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
fn set_value(obj: &PyObjectRef, value: Vec<u8>) {
    obj.borrow_mut().kind = PyObjectKind::Bytes { value };
}
*/

fn bytearray_repr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(obj, Some(vm.ctx.bytearray_type()))]);
    let data = get_value(obj);
    let data: Vec<String> = data.iter().map(|b| format!("\\x{:02x}", b)).collect();
    let data = data.join("");
    Ok(vm.new_str(format!("bytearray(b'{}')", data)))
}

use super::super::pyobject::{
    PyContext, PyFuncArgs, PyObject, PyObjectPayload, PyObjectRef, PyResult, TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objint;
use super::objtype;
use num_traits::ToPrimitive;
use std::cell::Ref;
use std::hash::{Hash, Hasher};
use std::ops::Deref;

// Binary data support

// Fill bytes class methods:
pub fn init(context: &PyContext) {
    let bytes_type = &context.bytes_type;

    let bytes_doc =
        "bytes(iterable_of_ints) -> bytes\n\
         bytes(string, encoding[, errors]) -> bytes\n\
         bytes(bytes_or_buffer) -> immutable copy of bytes_or_buffer\n\
         bytes(int) -> bytes object of size given by the parameter initialized with null bytes\n\
         bytes() -> empty bytes object\n\nConstruct an immutable array of bytes from:\n  \
         - an iterable yielding integers in range(256)\n  \
         - a text string encoded using the specified encoding\n  \
         - any object implementing the buffer API.\n  \
         - an integer";

    context.set_attr(bytes_type, "__eq__", context.new_rustfunc(bytes_eq));
    context.set_attr(bytes_type, "__lt__", context.new_rustfunc(bytes_lt));
    context.set_attr(bytes_type, "__le__", context.new_rustfunc(bytes_le));
    context.set_attr(bytes_type, "__gt__", context.new_rustfunc(bytes_gt));
    context.set_attr(bytes_type, "__ge__", context.new_rustfunc(bytes_ge));
    context.set_attr(bytes_type, "__hash__", context.new_rustfunc(bytes_hash));
    context.set_attr(bytes_type, "__new__", context.new_rustfunc(bytes_new));
    context.set_attr(bytes_type, "__repr__", context.new_rustfunc(bytes_repr));
    context.set_attr(bytes_type, "__len__", context.new_rustfunc(bytes_len));
    context.set_attr(bytes_type, "__iter__", context.new_rustfunc(bytes_iter));
    context.set_attr(
        bytes_type,
        "__doc__",
        context.new_str(bytes_doc.to_string()),
    );
}

fn bytes_new(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(cls, None)],
        optional = [(val_option, None)]
    );
    if !objtype::issubclass(cls, &vm.ctx.bytes_type()) {
        return Err(vm.new_type_error(format!("{:?} is not a subtype of bytes", cls)));
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

    Ok(PyObject::new(PyObjectPayload::Bytes { value }, cls.clone()))
}

fn bytes_eq(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(a, Some(vm.ctx.bytes_type())), (b, None)]
    );

    let result = if objtype::isinstance(b, &vm.ctx.bytes_type()) {
        get_value(a).to_vec() == get_value(b).to_vec()
    } else {
        false
    };
    Ok(vm.ctx.new_bool(result))
}

fn bytes_ge(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(a, Some(vm.ctx.bytes_type())), (b, None)]
    );

    let result = if objtype::isinstance(b, &vm.ctx.bytes_type()) {
        get_value(a).to_vec() >= get_value(b).to_vec()
    } else {
        return Err(vm.new_type_error(format!(
            "Cannot compare {} and {} using '>'",
            a.borrow(),
            b.borrow()
        )));
    };
    Ok(vm.ctx.new_bool(result))
}

fn bytes_gt(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(a, Some(vm.ctx.bytes_type())), (b, None)]
    );

    let result = if objtype::isinstance(b, &vm.ctx.bytes_type()) {
        get_value(a).to_vec() > get_value(b).to_vec()
    } else {
        return Err(vm.new_type_error(format!(
            "Cannot compare {} and {} using '>='",
            a.borrow(),
            b.borrow()
        )));
    };
    Ok(vm.ctx.new_bool(result))
}

fn bytes_le(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(a, Some(vm.ctx.bytes_type())), (b, None)]
    );

    let result = if objtype::isinstance(b, &vm.ctx.bytes_type()) {
        get_value(a).to_vec() <= get_value(b).to_vec()
    } else {
        return Err(vm.new_type_error(format!(
            "Cannot compare {} and {} using '<'",
            a.borrow(),
            b.borrow()
        )));
    };
    Ok(vm.ctx.new_bool(result))
}

fn bytes_lt(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(a, Some(vm.ctx.bytes_type())), (b, None)]
    );

    let result = if objtype::isinstance(b, &vm.ctx.bytes_type()) {
        get_value(a).to_vec() < get_value(b).to_vec()
    } else {
        return Err(vm.new_type_error(format!(
            "Cannot compare {} and {} using '<='",
            a.borrow(),
            b.borrow()
        )));
    };
    Ok(vm.ctx.new_bool(result))
}

fn bytes_len(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(a, Some(vm.ctx.bytes_type()))]);

    let byte_vec = get_value(a).to_vec();
    Ok(vm.ctx.new_int(byte_vec.len()))
}

fn bytes_hash(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, Some(vm.ctx.bytes_type()))]);
    let data = get_value(zelf);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    data.hash(&mut hasher);
    let hash = hasher.finish();
    Ok(vm.ctx.new_int(hash))
}

pub fn get_value<'a>(obj: &'a PyObjectRef) -> impl Deref<Target = Vec<u8>> + 'a {
    Ref::map(obj.borrow(), |py_obj| {
        if let PyObjectPayload::Bytes { ref value } = py_obj.payload {
            value
        } else {
            panic!("Inner error getting int {:?}", obj);
        }
    })
}

fn bytes_repr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(obj, Some(vm.ctx.bytes_type()))]);
    let value = get_value(obj);
    let data = String::from_utf8(value.to_vec()).unwrap();
    Ok(vm.new_str(format!("b'{}'", data)))
}

fn bytes_iter(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(obj, Some(vm.ctx.bytes_type()))]);

    let iter_obj = PyObject::new(
        PyObjectPayload::Iterator {
            position: 0,
            iterated_obj: obj.clone(),
        },
        vm.ctx.iter_type(),
    );

    Ok(iter_obj)
}

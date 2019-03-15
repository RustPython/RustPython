//! Implementation of the python bytearray object.

use std::cell::RefCell;
use std::fmt::Write;
use std::ops::{Deref, DerefMut};

use crate::pyobject::{
    PyContext, PyFuncArgs, PyObject, PyObjectRef, PyResult, PyValue, TypeProtocol,
};

use super::objint;

use super::objtype;
use crate::vm::VirtualMachine;
use num_traits::ToPrimitive;

#[derive(Debug)]
pub struct PyByteArray {
    // TODO: shouldn't be public
    pub value: RefCell<Vec<u8>>,
}

impl PyByteArray {
    pub fn new(data: Vec<u8>) -> Self {
        PyByteArray {
            value: RefCell::new(data),
        }
    }
}

impl PyValue for PyByteArray {
    fn class(vm: &mut VirtualMachine) -> PyObjectRef {
        vm.ctx.bytearray_type()
    }
}

pub fn get_value<'a>(obj: &'a PyObjectRef) -> impl Deref<Target = Vec<u8>> + 'a {
    obj.payload::<PyByteArray>().unwrap().value.borrow()
}

pub fn get_mut_value<'a>(obj: &'a PyObjectRef) -> impl DerefMut<Target = Vec<u8>> + 'a {
    obj.payload::<PyByteArray>().unwrap().value.borrow_mut()
}

// Binary data support

/// Fill bytearray class methods dictionary.
pub fn init(context: &PyContext) {
    let bytearray_type = &context.bytearray_type;

    let bytearray_doc =
        "bytearray(iterable_of_ints) -> bytearray\n\
         bytearray(string, encoding[, errors]) -> bytearray\n\
         bytearray(bytes_or_buffer) -> mutable copy of bytes_or_buffer\n\
         bytearray(int) -> bytes array of size given by the parameter initialized with null bytes\n\
         bytearray() -> empty bytes array\n\n\
         Construct a mutable bytearray object from:\n  \
         - an iterable yielding integers in range(256)\n  \
         - a text string encoded using the specified encoding\n  \
         - a bytes or a buffer object\n  \
         - any object implementing the buffer API.\n  \
         - an integer";

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
        crate::VM_REPR,
        context.new_rustfunc(bytearray_repr),
    );
    context.set_attr(
        &bytearray_type,
        "__len__",
        context.new_rustfunc(bytesarray_len),
    );
    context.set_attr(
        &bytearray_type,
        "__doc__",
        context.new_str(bytearray_doc.to_string()),
    );
    context.set_attr(
        &bytearray_type,
        "isalnum",
        context.new_rustfunc(bytearray_isalnum),
    );
    context.set_attr(
        &bytearray_type,
        "isalpha",
        context.new_rustfunc(bytearray_isalpha),
    );
    context.set_attr(
        &bytearray_type,
        "isascii",
        context.new_rustfunc(bytearray_isascii),
    );
    context.set_attr(
        &bytearray_type,
        "isdigit",
        context.new_rustfunc(bytearray_isdigit),
    );
    context.set_attr(
        &bytearray_type,
        "islower",
        context.new_rustfunc(bytearray_islower),
    );
    context.set_attr(
        &bytearray_type,
        "isspace",
        context.new_rustfunc(bytearray_isspace),
    );
    context.set_attr(
        &bytearray_type,
        "isupper",
        context.new_rustfunc(bytearray_isupper),
    );
    context.set_attr(
        &bytearray_type,
        "istitle",
        context.new_rustfunc(bytearray_istitle),
    );
    context.set_attr(
        &bytearray_type,
        "clear",
        context.new_rustfunc(bytearray_clear),
    );
    context.set_attr(&bytearray_type, "pop", context.new_rustfunc(bytearray_pop));
    context.set_attr(
        &bytearray_type,
        "lower",
        context.new_rustfunc(bytearray_lower),
    );
    context.set_attr(
        &bytearray_type,
        "upper",
        context.new_rustfunc(bytearray_upper),
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
            if let Some(i) = v.to_u8() {
                data_bytes.push(i);
            } else {
                return Err(vm.new_value_error("byte must be in range(0, 256)".to_string()));
            }
        }
        data_bytes
    // return Err(vm.new_type_error("Cannot construct bytes".to_string()));
    } else {
        vec![]
    };
    Ok(PyObject::new(PyByteArray::new(value), cls.clone()))
}

fn bytesarray_len(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(a, Some(vm.ctx.bytearray_type()))]);

    let byte_vec = get_value(a).to_vec();
    Ok(vm.ctx.new_int(byte_vec.len()))
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

fn bytearray_isalnum(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, Some(vm.ctx.bytearray_type()))]);
    let bytes = get_value(zelf);
    Ok(vm.new_bool(!bytes.is_empty() && bytes.iter().all(|x| char::from(*x).is_alphanumeric())))
}

fn bytearray_isalpha(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, Some(vm.ctx.bytearray_type()))]);
    let bytes = get_value(zelf);
    Ok(vm.new_bool(!bytes.is_empty() && bytes.iter().all(|x| char::from(*x).is_alphabetic())))
}

fn bytearray_isascii(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, Some(vm.ctx.bytearray_type()))]);
    let bytes = get_value(zelf);
    Ok(vm.new_bool(!bytes.is_empty() && bytes.iter().all(|x| char::from(*x).is_ascii())))
}

fn bytearray_isdigit(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, Some(vm.ctx.bytearray_type()))]);
    let bytes = get_value(zelf);
    Ok(vm.new_bool(!bytes.is_empty() && bytes.iter().all(|x| char::from(*x).is_digit(10))))
}

fn bytearray_islower(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, Some(vm.ctx.bytearray_type()))]);
    let bytes = get_value(zelf);
    Ok(vm.new_bool(
        !bytes.is_empty()
            && bytes
                .iter()
                .filter(|x| !char::from(**x).is_whitespace())
                .all(|x| char::from(*x).is_lowercase()),
    ))
}

fn bytearray_isspace(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, Some(vm.ctx.bytearray_type()))]);
    let bytes = get_value(zelf);
    Ok(vm.new_bool(!bytes.is_empty() && bytes.iter().all(|x| char::from(*x).is_whitespace())))
}

fn bytearray_isupper(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, Some(vm.ctx.bytearray_type()))]);
    let bytes = get_value(zelf);
    Ok(vm.new_bool(
        !bytes.is_empty()
            && bytes
                .iter()
                .filter(|x| !char::from(**x).is_whitespace())
                .all(|x| char::from(*x).is_uppercase()),
    ))
}

fn bytearray_istitle(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, Some(vm.ctx.bytearray_type()))]);
    let bytes = get_value(zelf);

    if bytes.is_empty() {
        Ok(vm.new_bool(false))
    } else {
        let mut iter = bytes.iter().peekable();
        let mut prev_cased = false;

        while let Some(c) = iter.next() {
            let current = char::from(*c);
            let next = if let Some(k) = iter.peek() {
                char::from(**k)
            } else if current.is_uppercase() {
                return Ok(vm.new_bool(!prev_cased));
            } else {
                return Ok(vm.new_bool(prev_cased));
            };

            if (is_cased(current) && next.is_uppercase() && !prev_cased)
                || (!is_cased(current) && next.is_lowercase())
            {
                return Ok(vm.new_bool(false));
            }

            prev_cased = is_cased(current);
        }

        Ok(vm.new_bool(true))
    }
}

// helper function for istitle
fn is_cased(c: char) -> bool {
    c.to_uppercase().next().unwrap() != c || c.to_lowercase().next().unwrap() != c
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
    obj.borrow_mut().kind = PyObjectPayload::Bytes { value };
}
*/

/// Return a lowercase hex representation of a bytearray
fn bytearray_to_hex(bytearray: &[u8]) -> String {
    bytearray.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "\\x{:02x}", b);
        s
    })
}

fn bytearray_repr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(obj, Some(vm.ctx.bytearray_type()))]);
    let value = get_value(obj);
    let data =
        String::from_utf8(value.to_vec()).unwrap_or_else(|_| bytearray_to_hex(&value.to_vec()));
    Ok(vm.new_str(format!("bytearray(b'{}')", data)))
}

fn bytearray_clear(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, Some(vm.ctx.bytearray_type()))]);
    get_mut_value(zelf).clear();
    Ok(vm.get_none())
}

fn bytearray_pop(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(obj, Some(vm.ctx.bytearray_type()))]);
    let mut value = get_mut_value(obj);

    if let Some(i) = value.pop() {
        Ok(vm.ctx.new_int(i))
    } else {
        Err(vm.new_index_error("pop from empty bytearray".to_string()))
    }
}

fn bytearray_lower(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(obj, Some(vm.ctx.bytearray_type()))]);
    let value = get_value(obj).to_vec().to_ascii_lowercase();
    Ok(vm.ctx.new_bytearray(value))
}

fn bytearray_upper(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(obj, Some(vm.ctx.bytearray_type()))]);
    let value = get_value(obj).to_vec().to_ascii_uppercase();
    Ok(vm.ctx.new_bytearray(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytearray_to_hex_formatting() {
        assert_eq!(&bytearray_to_hex(&[11u8, 222u8]), "\\x0b\\xde");
    }
}

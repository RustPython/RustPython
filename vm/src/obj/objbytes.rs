use std::cell::Cell;
use std::hash::{Hash, Hasher};
use std::ops::Deref;

use num_traits::ToPrimitive;

use crate::function::{OptionalArg, PyFuncArgs};
use crate::pyobject::{
    PyContext, PyIteratorValue, PyObjectRef, PyRef, PyResult, PyValue, TypeProtocol,
};
use crate::vm::VirtualMachine;

use super::objint;
use super::objtype::{self, PyClassRef};

#[derive(Debug)]
pub struct PyBytes {
    value: Vec<u8>,
}
type PyBytesRef = PyRef<PyBytes>;

impl PyBytes {
    pub fn new(data: Vec<u8>) -> Self {
        PyBytes { value: data }
    }
}

impl Deref for PyBytes {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.value
    }
}

impl PyValue for PyBytes {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.bytes_type()
    }
}

// Binary data support

// Fill bytes class methods:
pub fn init(context: &PyContext) {
    let bytes_type = context.bytes_type.as_object();

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

    extend_class!(context, bytes_type, {
        "__eq__" => context.new_rustfunc(bytes_eq),
        "__lt__" => context.new_rustfunc(bytes_lt),
        "__le__" => context.new_rustfunc(bytes_le),
        "__gt__" => context.new_rustfunc(bytes_gt),
        "__ge__" => context.new_rustfunc(bytes_ge),
        "__hash__" => context.new_rustfunc(bytes_hash),
        "__new__" => context.new_rustfunc(bytes_new),
        "__repr__" => context.new_rustfunc(bytes_repr),
        "__len__" => context.new_rustfunc(bytes_len),
        "__iter__" => context.new_rustfunc(bytes_iter),
        "__doc__" => context.new_str(bytes_doc.to_string()),
        "__add__" => context.new_rustfunc(PyBytesRef::add),
        "__contains__" => context.new_rustfunc(PyBytesRef::contains),
    });
}

fn bytes_new(
    cls: PyClassRef,
    val_option: OptionalArg<PyObjectRef>,
    vm: &VirtualMachine,
) -> PyResult<PyBytesRef> {
    // Create bytes data:
    let value = if let OptionalArg::Present(ival) = val_option {
        let elements = vm.extract_elements(&ival)?;
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

    PyBytes::new(value).into_ref_with_type(vm, cls)
}

fn bytes_eq(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
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

fn bytes_ge(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(a, Some(vm.ctx.bytes_type())), (b, None)]
    );

    let result = if objtype::isinstance(b, &vm.ctx.bytes_type()) {
        get_value(a).to_vec() >= get_value(b).to_vec()
    } else {
        return Err(vm.new_type_error(format!("Cannot compare {} and {} using '>'", a, b)));
    };
    Ok(vm.ctx.new_bool(result))
}

fn bytes_gt(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(a, Some(vm.ctx.bytes_type())), (b, None)]
    );

    let result = if objtype::isinstance(b, &vm.ctx.bytes_type()) {
        get_value(a).to_vec() > get_value(b).to_vec()
    } else {
        return Err(vm.new_type_error(format!("Cannot compare {} and {} using '>='", a, b)));
    };
    Ok(vm.ctx.new_bool(result))
}

fn bytes_le(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(a, Some(vm.ctx.bytes_type())), (b, None)]
    );

    let result = if objtype::isinstance(b, &vm.ctx.bytes_type()) {
        get_value(a).to_vec() <= get_value(b).to_vec()
    } else {
        return Err(vm.new_type_error(format!("Cannot compare {} and {} using '<'", a, b)));
    };
    Ok(vm.ctx.new_bool(result))
}

fn bytes_lt(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(a, Some(vm.ctx.bytes_type())), (b, None)]
    );

    let result = if objtype::isinstance(b, &vm.ctx.bytes_type()) {
        get_value(a).to_vec() < get_value(b).to_vec()
    } else {
        return Err(vm.new_type_error(format!("Cannot compare {} and {} using '<='", a, b)));
    };
    Ok(vm.ctx.new_bool(result))
}

fn bytes_len(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(a, Some(vm.ctx.bytes_type()))]);

    let byte_vec = get_value(a).to_vec();
    Ok(vm.ctx.new_int(byte_vec.len()))
}

fn bytes_hash(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, Some(vm.ctx.bytes_type()))]);
    let data = get_value(zelf);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    data.hash(&mut hasher);
    let hash = hasher.finish();
    Ok(vm.ctx.new_int(hash))
}

pub fn get_value<'a>(obj: &'a PyObjectRef) -> impl Deref<Target = Vec<u8>> + 'a {
    &obj.payload::<PyBytes>().unwrap().value
}

fn bytes_repr(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(obj, Some(vm.ctx.bytes_type()))]);
    let value = get_value(obj);
    let data = String::from_utf8(value.to_vec()).unwrap();
    Ok(vm.new_str(format!("b'{}'", data)))
}

fn bytes_iter(obj: PyBytesRef, _vm: &VirtualMachine) -> PyIteratorValue {
    PyIteratorValue {
        position: Cell::new(0),
        iterated_obj: obj.into_object(),
    }
}

fn compare_slice(a: &[u8], b: &[u8]) -> bool {
    for (i, j) in a.iter().zip(b.iter()) {
        if i != j {
            return false;
        }
    }
    return true;
}

fn compare_vec(a: &Vec<u8>, b: &Vec<u8>) -> bool {
    let a_len = a.len();
    let b_len = b.len();
    for (n, i) in a.iter().enumerate() {
        if n + b_len <= a_len && *i == b[0] {
            if compare_slice(&a[n..n + b_len], b) {
                return true;
            }
        }
    }
    false
}

impl PyBytesRef {
    fn add(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if objtype::isinstance(&other, &vm.ctx.bytes_type()) {
            let rhs = get_value(&other);
            let elements: Vec<u8> = self.value.iter().chain(rhs.iter()).cloned().collect();
            Ok(vm.ctx.new_bytes(elements))
        } else {
            Err(vm.new_not_implemented_error("".to_string()))
        }
    }

    fn contains(self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        if objtype::isinstance(&needle, &vm.ctx.bytes_type()) {
            if compare_vec(&self.value, &get_value(&needle)) {
                return Ok(true);
            } else {
                return Ok(false);
            }
        } else if objtype::isinstance(&needle, &vm.ctx.int_type()) {
            let c = self
                .value
                .contains(&objint::get_value(&needle).to_u8().unwrap());
            if c == true {
                return Ok(true);
            } else {
                return Ok(false);
            }
        } else {
            Err(vm.new_type_error(format!("Cannot add {:?} and {:?}", self, needle)))
        }
    }
}

use std::cell::Cell;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::ops::Deref;

use num_traits::ToPrimitive;

use crate::function::OptionalArg;
use crate::pyobject::{PyContext, PyIteratorValue, PyObjectRef, PyRef, PyResult, PyValue};
use crate::vm::VirtualMachine;

use super::objint;
use super::objsequence::PySliceableSequence;
use super::objtype;
use super::objtype::PyClassRef;

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
        "__new__" => context.new_rustfunc(bytes_new),
        "__eq__" => context.new_rustfunc(PyBytesRef::eq),
        "__lt__" => context.new_rustfunc(PyBytesRef::lt),
        "__le__" => context.new_rustfunc(PyBytesRef::le),
        "__gt__" => context.new_rustfunc(PyBytesRef::gt),
        "__ge__" => context.new_rustfunc(PyBytesRef::ge),
        "__hash__" => context.new_rustfunc(PyBytesRef::hash),
        "__repr__" => context.new_rustfunc(PyBytesRef::repr),
        "__len__" => context.new_rustfunc(PyBytesRef::len),
        "__iter__" => context.new_rustfunc(PyBytesRef::iter),
        "__doc__" => context.new_str(bytes_doc.to_string()),
        "__add__" => context.new_rustfunc(PyBytesRef::add),
        "__contains__" => context.new_rustfunc(PyBytesRef::contains),
        "__getitem__" => context.new_rustfunc(PyBytesRef::getitem),
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

impl PyBytesRef {
    fn eq(self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if let Ok(other) = other.downcast::<PyBytes>() {
            vm.ctx.new_bool(self.value == other.value)
        } else {
            vm.ctx.not_implemented()
        }
    }

    fn ge(self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if let Ok(other) = other.downcast::<PyBytes>() {
            vm.ctx.new_bool(self.value >= other.value)
        } else {
            vm.ctx.not_implemented()
        }
    }

    fn gt(self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if let Ok(other) = other.downcast::<PyBytes>() {
            vm.ctx.new_bool(self.value > other.value)
        } else {
            vm.ctx.not_implemented()
        }
    }

    fn le(self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if let Ok(other) = other.downcast::<PyBytes>() {
            vm.ctx.new_bool(self.value <= other.value)
        } else {
            vm.ctx.not_implemented()
        }
    }

    fn lt(self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if let Ok(other) = other.downcast::<PyBytes>() {
            vm.ctx.new_bool(self.value < other.value)
        } else {
            vm.ctx.not_implemented()
        }
    }

    fn len(self, _vm: &VirtualMachine) -> usize {
        self.value.len()
    }

    fn hash(self, _vm: &VirtualMachine) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.value.hash(&mut hasher);
        hasher.finish()
    }

    fn repr(self, _vm: &VirtualMachine) -> String {
        // TODO: don't just unwrap
        // let data = String::from_utf8(self.value.clone()).unwrap();
        // format!("b'{}'", data)

        let data = format_bytes(&self.value);
        format!("b'{}'", data)
    }

    fn iter(obj: PyBytesRef, _vm: &VirtualMachine) -> PyIteratorValue {
        PyIteratorValue {
            position: Cell::new(0),
            iterated_obj: obj.into_object(),
        }
    }

    fn add(self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if let Ok(other) = other.downcast::<PyBytes>() {
            let rhs = &other.value;
            let elements: Vec<u8> = self.value.iter().chain(rhs.iter()).cloned().collect();
            vm.ctx.new_bytes(elements)
        } else {
            vm.ctx.not_implemented()
        }
    }

    fn contains(self, needle: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        // no new style since objint is not.
        if objtype::isinstance(&needle, &vm.ctx.bytes_type()) {
            let result = vec_contains(&self.value, &get_value(&needle));
            vm.ctx.new_bool(result)
        } else if objtype::isinstance(&needle, &vm.ctx.int_type()) {
            let result = self
                .value
                .contains(&objint::get_value(&needle).to_u8().unwrap());
            vm.ctx.new_bool(result)
        } else {
            vm.new_type_error(format!("Cannot add {:?} and {:?}", self, needle))
        }
    }

    fn getitem(self, needle: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if objtype::isinstance(&needle, &vm.ctx.int_type()) {
            match objint::get_value(&needle).to_i32() {
                Some(pos) => {
                    if let Some(idx) = self.value.get_pos(pos) {
                        vm.ctx.new_int(self.value[idx])
                    } else {
                        vm.new_index_error("index out of range".to_string())
                    }
                }
                None => {
                    vm.new_index_error("cannot fit 'int' into an index-sized integer".to_string())
                }
            }
        } else if objtype::isinstance(&needle, &vm.ctx.slice_type()) {
            vm.ctx
                .new_bytes((self.value.get_slice_items(&vm, &needle)).unwrap())
        } else {
            vm.new_type_error(format!(
                "byte indices must be integers or slices, not {}",
                needle
            ))
        }
    }
}

pub fn get_value<'a>(obj: &'a PyObjectRef) -> impl Deref<Target = Vec<u8>> + 'a {
    &obj.payload::<PyBytes>().unwrap().value
}

/// return true if b is a subset of a.
fn vec_contains(a: &Vec<u8>, b: &Vec<u8>) -> bool {
    let a_len = a.len();
    let b_len = b.len();
    for (n, i) in a.iter().enumerate() {
        if n + b_len <= a_len && *i == b[0] {
            if &a[n..n + b_len] == b.as_slice() {
                return true;
            }
        }
    }
    false
}

fn format_bytes(chars: &Vec<u8>) -> String {
    let mut res = String::new();
    for i in chars {
        res.push_str(BYTES_REPR[*i as usize])
    }
    res
}

const BYTES_REPR: &'static [&'static str] = &[
    "\\x00", "\\x01", "\\x02", "\\x03", "\\x04", "\\x05", "\\x06", "\\x07", "\\x08", "\\t", "\\n",
    "\\x0b", "\\x0c", "\\r", "\\x0e", "\\x0f", "\\x10", "\\x11", "\\x12", "\\x13", "\\x14",
    "\\x15", "\\x16", "\\x17", "\\x18", "\\x19", "\\x1a", "\\x1b", "\\x1c", "\\x1d", "\\x1e",
    "\\x1f", " ", "!", "\"", "#", "$", "%", "&", "'", "(", ")", "*", "+", ",", "-", ".", "/", "0",
    "1", "2", "3", "4", "5", "6", "7", "8", "9", ":", ";", "<", "=", ">", "?", "@", "A", "B", "C",
    "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R", "S", "T", "U", "V",
    "W", "X", "Y", "Z", "[", "\\", "]", "^", "_", "`", "a", "b", "c", "d", "e", "f", "g", "h", "i",
    "j", "k", "l", "m", "n", "o", "p", "q", "r", "s", "t", "u", "v", "w", "x", "y", "z", "{", "|",
    "}", "~", "\\x7f", "\\x80", "\\x81", "\\x82", "\\x83", "\\x84", "\\x85", "\\x86", "\\x87",
    "\\x88", "\\x89", "\\x8a", "\\x8b", "\\x8c", "\\x8d", "\\x8e", "\\x8f", "\\x90", "\\x91",
    "\\x92", "\\x93", "\\x94", "\\x95", "\\x96", "\\x97", "\\x98", "\\x99", "\\x9a", "\\x9b",
    "\\x9c", "\\x9d", "\\x9e", "\\x9f", "\\xa0", "\\xa1", "\\xa2", "\\xa3", "\\xa4", "\\xa5",
    "\\xa6", "\\xa7", "\\xa8", "\\xa9", "\\xaa", "\\xab", "\\xac", "\\xad", "\\xae", "\\xaf",
    "\\xb0", "\\xb1", "\\xb2", "\\xb3", "\\xb4", "\\xb5", "\\xb6", "\\xb7", "\\xb8", "\\xb9",
    "\\xba", "\\xbb", "\\xbc", "\\xbd", "\\xbe", "\\xbf", "\\xc0", "\\xc1", "\\xc2", "\\xc3",
    "\\xc4", "\\xc5", "\\xc6", "\\xc7", "\\xc8", "\\xc9", "\\xca", "\\xcb", "\\xcc", "\\xcd",
    "\\xce", "\\xcf", "\\xd0", "\\xd1", "\\xd2", "\\xd3", "\\xd4", "\\xd5", "\\xd6", "\\xd7",
    "\\xd8", "\\xd9", "\\xda", "\\xdb", "\\xdc", "\\xdd", "\\xde", "\\xdf", "\\xe0", "\\xe1",
    "\\xe2", "\\xe3", "\\xe4", "\\xe5", "\\xe6", "\\xe7", "\\xe8", "\\xe9", "\\xea", "\\xeb",
    "\\xec", "\\xed", "\\xee", "\\xef", "\\xf0", "\\xf1", "\\xf2", "\\xf3", "\\xf4", "\\xf5",
    "\\xf6", "\\xf7", "\\xf8", "\\xf9", "\\xfa", "\\xfb", "\\xfc", "\\xfd", "\\xfe", "\\xff",
];

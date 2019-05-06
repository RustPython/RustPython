//! Implementation of the python bytearray object.

use std::cell::{Cell, RefCell};
use std::fmt::Write;
use std::ops::{Deref, DerefMut};

use num_traits::ToPrimitive;

use crate::function::OptionalArg;
use crate::pyobject::{
    PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue, TypeProtocol,
};
use crate::vm::VirtualMachine;

use super::objbyteinner::ByteOr;
use super::objbytes::PyBytes;
use super::objint;
use super::objint::PyInt;
use super::objiter;
use super::objstr;
use super::objtype::PyClassRef;

#[derive(Debug)]
pub struct PyByteArray {
    // TODO: shouldn't be public
    pub value: RefCell<Vec<u8>>,
}
type PyByteArrayRef = PyRef<PyByteArray>;

impl PyByteArray {
    pub fn new(data: Vec<u8>) -> Self {
        PyByteArray {
            value: RefCell::new(data),
        }
    }
}

impl PyValue for PyByteArray {
    fn class(vm: &VirtualMachine) -> PyClassRef {
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

    extend_class!(context, bytearray_type, {
        "__doc__" => context.new_str(bytearray_doc.to_string()),
        "__new__" => context.new_rustfunc(bytearray_new),
        "__eq__" => context.new_rustfunc(PyByteArrayRef::eq),
        "__len__" => context.new_rustfunc(PyByteArrayRef::len),
        "__repr__" => context.new_rustfunc(PyByteArrayRef::repr),
        "__iter__" => context.new_rustfunc(PyByteArrayRef::iter),
        "clear" => context.new_rustfunc(PyByteArrayRef::clear),
        "isalnum" => context.new_rustfunc(PyByteArrayRef::isalnum),
        "isalpha" => context.new_rustfunc(PyByteArrayRef::isalpha),
        "isascii" => context.new_rustfunc(PyByteArrayRef::isascii),
        "isdigit" => context.new_rustfunc(PyByteArrayRef::isdigit),
        "islower" => context.new_rustfunc(PyByteArrayRef::islower),
        "isspace" => context.new_rustfunc(PyByteArrayRef::isspace),
        "istitle" =>context.new_rustfunc(PyByteArrayRef::istitle),
        "isupper" => context.new_rustfunc(PyByteArrayRef::isupper),
        "lower" => context.new_rustfunc(PyByteArrayRef::lower),
        "append" => context.new_rustfunc(PyByteArrayRef::append),
        "pop" => context.new_rustfunc(PyByteArrayRef::pop),
        "upper" => context.new_rustfunc(PyByteArrayRef::upper),
        "count" => context.new_rustfunc(PyByteArrayRef::count),
    });

    PyByteArrayIterator::extend_class(context, &context.bytearrayiterator_type);
}

fn bytearray_new(
    cls: PyClassRef,
    val_option: OptionalArg<PyObjectRef>,
    vm: &VirtualMachine,
) -> PyResult<PyByteArrayRef> {
    // Create bytes data:
    let value = if let OptionalArg::Present(ival) = val_option {
        let elements = vm.extract_elements(&ival)?;
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
    PyByteArray::new(value).into_ref_with_type(vm, cls.clone())
}

impl PyByteArrayRef {
    fn len(self, _vm: &VirtualMachine) -> usize {
        self.value.borrow().len()
    }

    fn eq(self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if let Ok(other) = other.downcast::<PyByteArray>() {
            vm.ctx
                .new_bool(self.value.borrow().as_slice() == other.value.borrow().as_slice())
        } else {
            vm.ctx.not_implemented()
        }
    }

    fn isalnum(self, _vm: &VirtualMachine) -> bool {
        let bytes = self.value.borrow();
        !bytes.is_empty() && bytes.iter().all(|x| char::from(*x).is_alphanumeric())
    }

    fn isalpha(self, _vm: &VirtualMachine) -> bool {
        let bytes = self.value.borrow();
        !bytes.is_empty() && bytes.iter().all(|x| char::from(*x).is_alphabetic())
    }

    fn isascii(self, _vm: &VirtualMachine) -> bool {
        let bytes = self.value.borrow();
        !bytes.is_empty() && bytes.iter().all(|x| char::from(*x).is_ascii())
    }

    fn isdigit(self, _vm: &VirtualMachine) -> bool {
        let bytes = self.value.borrow();
        !bytes.is_empty() && bytes.iter().all(|x| char::from(*x).is_digit(10))
    }

    fn islower(self, _vm: &VirtualMachine) -> bool {
        let bytes = self.value.borrow();
        !bytes.is_empty()
            && bytes
                .iter()
                .filter(|x| !char::from(**x).is_whitespace())
                .all(|x| char::from(*x).is_lowercase())
    }

    fn isspace(self, _vm: &VirtualMachine) -> bool {
        let bytes = self.value.borrow();
        !bytes.is_empty() && bytes.iter().all(|x| char::from(*x).is_whitespace())
    }

    fn isupper(self, _vm: &VirtualMachine) -> bool {
        let bytes = self.value.borrow();
        !bytes.is_empty()
            && bytes
                .iter()
                .filter(|x| !char::from(**x).is_whitespace())
                .all(|x| char::from(*x).is_uppercase())
    }

    fn istitle(self, _vm: &VirtualMachine) -> bool {
        let bytes = self.value.borrow();
        if bytes.is_empty() {
            return false;
        }

        let mut iter = bytes.iter().peekable();
        let mut prev_cased = false;

        while let Some(c) = iter.next() {
            let current = char::from(*c);
            let next = if let Some(k) = iter.peek() {
                char::from(**k)
            } else if current.is_uppercase() {
                return !prev_cased;
            } else {
                return prev_cased;
            };

            if (is_cased(current) && next.is_uppercase() && !prev_cased)
                || (!is_cased(current) && next.is_lowercase())
            {
                return false;
            }

            prev_cased = is_cased(current);
        }

        true
    }

    fn repr(self, _vm: &VirtualMachine) -> String {
        let data = vec_to_string(self.value.borrow().clone());
        format!("bytearray(b'{}')", data)
    }

    fn clear(self, _vm: &VirtualMachine) {
        self.value.borrow_mut().clear();
    }

    fn append(self, x: u8, _vm: &VirtualMachine) {
        self.value.borrow_mut().push(x);
    }

    fn pop(self, vm: &VirtualMachine) -> PyResult<u8> {
        let mut bytes = self.value.borrow_mut();
        bytes
            .pop()
            .ok_or_else(|| vm.new_index_error("pop from empty bytearray".to_string()))
    }

    fn lower(self, _vm: &VirtualMachine) -> PyByteArray {
        let bytes = self.value.borrow().clone().to_ascii_lowercase();
        PyByteArray {
            value: RefCell::new(bytes),
        }
    }

    fn upper(self, _vm: &VirtualMachine) -> PyByteArray {
        let bytes = self.value.borrow().clone().to_ascii_uppercase();
        PyByteArray {
            value: RefCell::new(bytes),
        }
    }

    fn iter(self, _vm: &VirtualMachine) -> PyByteArrayIterator {
        PyByteArrayIterator {
            position: Cell::new(0),
            bytearray: self,
        }
    }

    fn count(
        self,
        sub: PyObjectRef,
        start: OptionalArg<isize>,
        end: OptionalArg<isize>,
        vm: &VirtualMachine,
    ) -> PyResult<usize> {
        let sub_string = match sub.class().name.as_ref() {
            "bytearray" => sub
                .downcast::<PyByteArray>()
                .unwrap()
                .value
                .borrow()
                .clone(),
            "bytes" => sub.downcast::<PyBytes>().unwrap().get_value().to_vec(),
            "int" => vec![sub.downcast::<PyInt>()?.as_bigint().byte_or(vm)?],
            _ => {
                return Err(vm.new_type_error(format!(
                    "a bytes-like object is required, not {}",
                    sub.class()
                )))
            }
        };

        if let Some((start, end)) = objstr::adjust_indices(start, end, self.value.borrow().len()) {
            let data = vec_to_string(self.value.borrow().clone());
            Ok(data[start..end].matches(&vec_to_string(sub_string)).count())
        } else {
            Ok(0)
        }
    }
}

// helper function for istitle
fn is_cased(c: char) -> bool {
    c.to_uppercase().next().unwrap() != c || c.to_lowercase().next().unwrap() != c
}

// helper function for repr and count
fn vec_to_string(b: Vec<u8>) -> String {
    String::from_utf8(b.clone()).unwrap_or_else(|_| to_hex(&b))
}

/*
fn getitem(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
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
fn to_hex(bytearray: &[u8]) -> String {
    bytearray.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "\\x{:02x}", b);
        s
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytearray_to_hex_formatting() {
        assert_eq!(&to_hex(&[11u8, 222u8]), "\\x0b\\xde");
    }
}

#[pyclass]
#[derive(Debug)]
pub struct PyByteArrayIterator {
    position: Cell<usize>,
    bytearray: PyByteArrayRef,
}

impl PyValue for PyByteArrayIterator {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.bytearrayiterator_type()
    }
}

#[pyimpl]
impl PyByteArrayIterator {
    #[pymethod(name = "__next__")]
    fn next(&self, vm: &VirtualMachine) -> PyResult<u8> {
        if self.position.get() < self.bytearray.value.borrow().len() {
            let ret = self.bytearray.value.borrow()[self.position.get()];
            self.position.set(self.position.get() + 1);
            Ok(ret)
        } else {
            Err(objiter::new_stop_iteration(vm))
        }
    }

    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyRef<Self> {
        zelf
    }
}

use std::cell::Cell;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::ops::Deref;

use num_traits::ToPrimitive;

use crate::function::OptionalArg;
use crate::pyobject::{PyContext, PyObjectRef, PyRef, PyResult, PyValue};
use crate::vm::VirtualMachine;

use super::objint;
use super::objiter;
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
        "__doc__" => context.new_str(bytes_doc.to_string())
    });

    let bytesiterator_type = &context.bytesiterator_type;
    extend_class!(context, bytesiterator_type, {
        "__next__" => context.new_rustfunc(PyBytesIteratorRef::next),
        "__iter__" => context.new_rustfunc(PyBytesIteratorRef::iter),
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
        let data = String::from_utf8(self.value.clone()).unwrap();
        format!("b'{}'", data)
    }

    fn iter(self, _vm: &VirtualMachine) -> PyBytesIterator {
        PyBytesIterator {
            position: Cell::new(0),
            bytes: self,
        }
    }
}

pub fn get_value<'a>(obj: &'a PyObjectRef) -> impl Deref<Target = Vec<u8>> + 'a {
    &obj.payload::<PyBytes>().unwrap().value
}

#[derive(Debug)]
pub struct PyBytesIterator {
    position: Cell<usize>,
    bytes: PyBytesRef,
}

impl PyValue for PyBytesIterator {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.bytesiterator_type()
    }
}

type PyBytesIteratorRef = PyRef<PyBytesIterator>;

impl PyBytesIteratorRef {
    fn next(self, vm: &VirtualMachine) -> PyResult<u8> {
        if self.position.get() < self.bytes.value.len() {
            let ret = self.bytes[self.position.get()];
            self.position.set(self.position.get() + 1);
            Ok(ret)
        } else {
            Err(objiter::new_stop_iteration(vm))
        }
    }

    fn iter(self, _vm: &VirtualMachine) -> Self {
        self
    }
}

use crate::vm::VirtualMachine;
use std::ops::Deref;

use crate::function::OptionalArg;
use crate::pyobject::{PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue};

use super::objbyteinner::PyByteInner;
use super::objtype::PyClassRef;
/// "bytes(iterable_of_ints) -> bytes\n\
/// bytes(string, encoding[, errors]) -> bytes\n\
/// bytes(bytes_or_buffer) -> immutable copy of bytes_or_buffer\n\
/// bytes(int) -> bytes object of size given by the parameter initialized with null bytes\n\
/// bytes() -> empty bytes object\n\nConstruct an immutable array of bytes from:\n  \
/// - an iterable yielding integers in range(256)\n  \
/// - a text string encoded using the specified encoding\n  \
/// - any object implementing the buffer API.\n  \
/// - an integer";
#[pyclass(name = "bytes", __inside_vm)]
#[derive(Clone, Debug)]
pub struct PyBytes {
    inner: PyByteInner,
}
type PyBytesRef = PyRef<PyBytes>;

impl PyBytes {
    pub fn new(elements: Vec<u8>) -> Self {
        PyBytes {
            inner: PyByteInner { elements },
        }
    }
}

impl Deref for PyBytes {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.inner.elements
    }
}

impl PyValue for PyBytes {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.bytes_type()
    }
}

// Binary data support

// Fill bytes class methods:

pub fn get_value<'a>(obj: &'a PyObjectRef) -> impl Deref<Target = Vec<u8>> + 'a {
    &obj.payload::<PyBytes>().unwrap().inner.elements
}

// pub fn init(context: &PyContext) {
//    let bytes_doc =
pub fn init(ctx: &PyContext) {
    PyBytesRef::extend_class(ctx, &ctx.bytes_type);
}
//extend_class!(context, &context.bytes_type, {
//"__new__" => context.new_rustfunc(bytes_new),
/*        "__eq__" => context.new_rustfunc(PyBytesRef::eq),
"__lt__" => context.new_rustfunc(PyBytesRef::lt),
"__le__" => context.new_rustfunc(PyBytesRef::le),
"__gt__" => context.new_rustfunc(PyBytesRef::gt),
"__ge__" => context.new_rustfunc(PyBytesRef::ge),
"__hash__" => context.new_rustfunc(PyBytesRef::hash),
"__repr__" => context.new_rustfunc(PyBytesRef::repr),
"__len__" => context.new_rustfunc(PyBytesRef::len),
"__iter__" => context.new_rustfunc(PyBytesRef::iter),
"__doc__" => context.new_str(bytes_doc.to_string())*/
// });

/*    let bytesiterator_type = &context.bytesiterator_type;
extend_class!(context, bytesiterator_type, {
    "__next__" => context.new_rustfunc(PyBytesIteratorRef::next),
    "__iter__" => context.new_rustfunc(PyBytesIteratorRef::iter),
});*/
//}

#[pyimpl(__inside_vm)]
impl PyBytesRef {
    #[pymethod(name = "__new__")]
    fn bytes_new(
        cls: PyClassRef,
        val_option: OptionalArg<PyObjectRef>,
        enc_option: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyBytesRef> {
        PyBytes {
            inner: PyByteInner::new(val_option, enc_option, vm)?,
        }
        .into_ref_with_type(vm, cls)
    }

    #[pymethod(name = "__repr__")]
    fn repr(self, vm: &VirtualMachine) -> PyResult {
        Ok(vm.new_str(format!("b'{}'", self.inner.repr()?)))
    }

    #[pymethod(name = "__len__")]
    fn len(self, _vm: &VirtualMachine) -> usize {
        self.inner.len()
    }

    #[pymethod(name = "__eq__")]
    fn eq(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        match_class!(other,
        bytes @ PyBytes => self.inner.eq(&bytes.inner, vm),
        _  => Ok(vm.ctx.not_implemented()))
    }

    #[pymethod(name = "__ge__")]
    fn ge(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        match_class!(other,
        bytes @ PyBytes => self.inner.ge(&bytes.inner, vm),
        _  => Ok(vm.ctx.not_implemented()))
    }
    #[pymethod(name = "__le__")]
    fn le(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        match_class!(other,
        bytes @ PyBytes => self.inner.le(&bytes.inner, vm),
        _  => Ok(vm.ctx.not_implemented()))
    }
    #[pymethod(name = "__gt__")]
    fn gt(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        match_class!(other,
        bytes @ PyBytes => self.inner.gt(&bytes.inner, vm),
        _  => Ok(vm.ctx.not_implemented()))
    }
    #[pymethod(name = "__lt__")]
    fn lt(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        match_class!(other,
        bytes @ PyBytes => self.inner.lt(&bytes.inner, vm),
        _  => Ok(vm.ctx.not_implemented()))
    }
}

//     fn hash(self, _vm: &VirtualMachine) -> u64 {
//         let mut hasher = DefaultHasher::new();
//         self.value.hash(&mut hasher);
//         hasher.finish()
//     }

//     fn iter(self, _vm: &VirtualMachine) -> PyBytesIterator {
//         PyBytesIterator {
//             position: Cell::new(0),
//             bytes: self,
//         }
//     }
// }

// #[derive(Debug)]
// pub struct PyBytesIterator {
//     position: Cell<usize>,
//     bytes: PyBytesRef,
// }

// impl PyValue for PyBytesIterator {
//     fn class(vm: &VirtualMachine) -> PyClassRef {
//         vm.ctx.bytesiterator_type()
//     }
// }

// type PyBytesIteratorRef = PyRef<PyBytesIterator>;

// impl PyBytesIteratorRef {
//     fn next(self, vm: &VirtualMachine) -> PyResult<u8> {
//         if self.position.get() < self.bytes.value.len() {
//             let ret = self.bytes[self.position.get()];
//             self.position.set(self.position.get() + 1);
//             Ok(ret)
//         } else {
//             Err(objiter::new_stop_iteration(vm))
//         }
//     }

//     fn iter(self, _vm: &VirtualMachine) -> Self {
//         self
//     }
// }

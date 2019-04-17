use crate::obj::objint::PyInt;
use crate::obj::objstr::PyString;
use crate::vm::VirtualMachine;
use core::cell::Cell;
use std::ops::Deref;

use crate::function::OptionalArg;
use crate::pyobject::{PyClassImpl, PyContext, PyIterable, PyObjectRef, PyRef, PyResult, PyValue};

use super::objbyteinner::PyByteInner;
use super::objiter;
use super::objslice::PySlice;
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

    pub fn get_value(&self) -> &[u8] {
        &self.inner.elements
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

pub fn get_value<'a>(obj: &'a PyObjectRef) -> impl Deref<Target = Vec<u8>> + 'a {
    &obj.payload::<PyBytes>().unwrap().inner.elements
}

pub fn init(context: &PyContext) {
    PyBytesRef::extend_class(context, &context.bytes_type);
    let bytes_type = &context.bytes_type;
    extend_class!(context, bytes_type, {
"fromhex" => context.new_rustfunc(PyBytesRef::fromhex),
});
    let bytesiterator_type = &context.bytesiterator_type;
    extend_class!(context, bytesiterator_type, {
"__next__" => context.new_rustfunc(PyBytesIteratorRef::next),
"__iter__" => context.new_rustfunc(PyBytesIteratorRef::iter),
});
}

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
    #[pymethod(name = "__hash__")]
    fn hash(self, _vm: &VirtualMachine) -> usize {
        self.inner.hash()
    }

    #[pymethod(name = "__iter__")]
    fn iter(self, _vm: &VirtualMachine) -> PyBytesIterator {
        PyBytesIterator {
            position: Cell::new(0),
            bytes: self,
        }
    }

    #[pymethod(name = "__add__")]
    fn add(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        match_class!(other,
        bytes @ PyBytes => Ok(vm.ctx.new_bytes(self.inner.add(&bytes.inner, vm))),
        _  => Ok(vm.ctx.not_implemented()))
    }

    #[pymethod(name = "__contains__")]
    fn contains(self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        match_class!(needle,
        bytes @ PyBytes => self.inner.contains_bytes(&bytes.inner, vm),
        int @ PyInt => self.inner.contains_int(&int, vm),
        obj  => Err(vm.new_type_error(format!("a bytes-like object is required, not {}", obj))))
    }

    #[pymethod(name = "__getitem__")]
    fn getitem(self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        match_class!(needle,
        int @ PyInt => self.inner.getitem_int(&int, vm),
        slice @ PySlice => self.inner.getitem_slice(slice.as_object(), vm),
        obj  => Err(vm.new_type_error(format!("byte indices must be integers or slices, not {}", obj))))
    }

    #[pymethod(name = "isalnum")]
    fn isalnum(self, vm: &VirtualMachine) -> PyResult {
        self.inner.isalnum(vm)
    }

    #[pymethod(name = "isalpha")]
    fn isalpha(self, vm: &VirtualMachine) -> PyResult {
        self.inner.isalpha(vm)
    }

    #[pymethod(name = "isascii")]
    fn isascii(self, vm: &VirtualMachine) -> PyResult {
        self.inner.isascii(vm)
    }

    #[pymethod(name = "isdigit")]
    fn isdigit(self, vm: &VirtualMachine) -> PyResult {
        self.inner.isdigit(vm)
    }

    #[pymethod(name = "islower")]
    fn islower(self, vm: &VirtualMachine) -> PyResult {
        self.inner.islower(vm)
    }

    #[pymethod(name = "isspace")]
    fn isspace(self, vm: &VirtualMachine) -> PyResult {
        self.inner.isspace(vm)
    }

    #[pymethod(name = "isupper")]
    fn isupper(self, vm: &VirtualMachine) -> PyResult {
        self.inner.isupper(vm)
    }

    #[pymethod(name = "istitle")]
    fn istitle(self, vm: &VirtualMachine) -> PyResult {
        self.inner.istitle(vm)
    }

    #[pymethod(name = "lower")]
    fn lower(self, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.new_bytes(self.inner.lower(vm)))
    }

    #[pymethod(name = "upper")]
    fn upper(self, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.new_bytes(self.inner.upper(vm)))
    }

    #[pymethod(name = "capitalize")]
    fn capitalize(self, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.new_bytes(self.inner.capitalize(vm)))
    }

    #[pymethod(name = "hex")]
    fn hex(self, vm: &VirtualMachine) -> PyResult {
        self.inner.hex(vm)
    }

    // #[pymethod(name = "fromhex")]
    fn fromhex(string: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        match_class!(string,
        s @ PyString => {
        match PyByteInner::fromhex(s.to_string(), vm) {
        Ok(x) => Ok(vm.ctx.new_bytes(x)),
        Err(y) => Err(y)}},
        obj => Err(vm.new_type_error(format!("fromhex() argument must be str, not {}", obj )))
        )
    }

    #[pymethod(name = "center")]
    fn center(
        self,
        width: PyObjectRef,
        fillbyte: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        Ok(vm.ctx.new_bytes(self.inner.center(width, fillbyte, vm)?))
    }

    #[pymethod(name = "count")]
    fn count(
        self,
        sub: PyObjectRef,
        start: OptionalArg<PyObjectRef>,
        end: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<usize> {
        self.inner.count(sub, start, end, vm)
    }

    #[pymethod(name = "join")]
    fn join(self, iter: PyIterable, vm: &VirtualMachine) -> PyResult {
        self.inner.join(iter, vm)
    }
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
        if self.position.get() < self.bytes.inner.len() {
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

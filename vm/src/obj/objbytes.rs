use crate::obj::objint::PyInt;
use crate::obj::objlist;
use crate::obj::objlist::PyList;
use crate::obj::objstr::PyString;
use crate::obj::objtuple::PyTuple;
use crate::obj::objtype;
use std::cell::Cell;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::ops::Deref;

use num_traits::ToPrimitive;

use crate::function::OptionalArg;
use crate::pyobject::{
    IntoPyObject, PyClassImpl, PyContext, PyIterable, PyObjectRef, PyRef, PyResult, PyValue,
    TryFromObject, TypeProtocol,
};
use crate::vm::VirtualMachine;

use super::objbyteinner::PyByteInner;
use super::objint;
use super::objiter;
use super::objstr;
use super::objtype::PyClassRef;
use std::clone::Clone;

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
#[derive(Debug)]
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
        // First handle bytes(string, encoding[, errors])
        if let OptionalArg::Present(enc) = enc_option {
            if let OptionalArg::Present(eval) = val_option {
                if let Ok(input) = eval.downcast::<PyString>() {
                    if let Ok(encoding) = enc.clone().downcast::<PyString>() {
                        if encoding.value.to_lowercase() == "utf8".to_string()
                            || encoding.value.to_lowercase() == "utf-8".to_string()
                        // TODO: different encoding
                        {
                            return PyBytes::new(input.value.as_bytes().to_vec())
                                .into_ref_with_type(vm, cls.clone());
                        } else {
                            return Err(
                                vm.new_value_error(format!("unknown encoding: {}", encoding.value)), //should be lookup error
                            );
                        }
                    } else {
                        return Err(vm.new_type_error(format!(
                            "bytes() argument 2 must be str, not {}",
                            enc.class().name
                        )));
                    }
                } else {
                    return Err(vm.new_type_error("encoding without a string argument".to_string()));
                }
            } else {
                return Err(vm.new_type_error("encoding without a string argument".to_string()));
            }
        // On ly one argument
        } else {
            let value = if let OptionalArg::Present(ival) = val_option {
                match_class!(ival.clone(),
                    i @ PyInt => {
                            let size = objint::get_value(&i.into_object()).to_usize().unwrap();
                            let mut res: Vec<u8> = Vec::with_capacity(size);
                            for _ in 0..size {
                                res.push(0)
                            }
                            Ok(res)},
                    _l @ PyString=> {return Err(vm.new_type_error(format!(
                        "string argument without an encoding"
                    )));},
                    obj => {
                        let elements = vm.extract_elements(&obj).or_else(|_| {return Err(vm.new_type_error(format!(
                        "cannot convert {} object to bytes", obj.class().name)));});

                        let mut data_bytes = vec![];
                        for elem in elements.unwrap(){
                            let v = objint::to_int(vm, &elem, 10)?;
                            if let Some(i) = v.to_u8() {
                                data_bytes.push(i);
                            } else {
                                return Err(vm.new_value_error("bytes must be in range(0, 256)".to_string()));
                                }

                            }
                        Ok(data_bytes)
                        }
                )
            } else {
                Ok(vec![])
            };
            match value {
                Ok(val) => PyBytes::new(val).into_ref_with_type(vm, cls.clone()),
                Err(err) => Err(err),
            }
        }
    }

    #[pymethod(name = "__repr__")]
    fn repr(self, _vm: &VirtualMachine) -> String {
        // TODO: don't just unwrap
        let data = self.inner.elements.clone();
        format!("b'{:?}'", data)
    }

    #[pymethod(name = "__len__")]
    fn len(self, _vm: &VirtualMachine) -> usize {
        self.inner.elements.len()
    }
}
/*
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


    fn hash(self, _vm: &VirtualMachine) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.value.hash(&mut hasher);
        hasher.finish()
    }


    fn iter(self, _vm: &VirtualMachine) -> PyBytesIterator {
        PyBytesIterator {
            position: Cell::new(0),
            bytes: self,
        }
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
}*/

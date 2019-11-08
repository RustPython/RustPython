use crate::function::OptionalArg;
use crate::obj::objbytes::PyBytesRef;
use crate::obj::objstr::PyStringRef;
use crate::obj::objtype::PyClassRef;
use crate::obj::{objbool, objiter};
use crate::pyobject::{
    IntoPyObject, PyClassImpl, PyIterable, PyObjectRef, PyRef, PyResult, PyValue, TryFromObject,
};
use crate::VirtualMachine;

use std::cell::{Cell, RefCell};
use std::fmt;

struct ArrayTypeSpecifierError {
    _priv: (),
}

impl fmt::Display for ArrayTypeSpecifierError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "bad typecode (must be b, B, u, h, H, i, I, l, L, q, Q, f or d)"
        )
    }
}

macro_rules! def_array_enum {
    ($(($n:ident, $t:ident, $c:literal)),*$(,)?) => {
        #[derive(Debug)]
        enum ArrayContentType {
            $($n(Vec<$t>),)*
        }

        #[allow(clippy::naive_bytecount, clippy::float_cmp)]
        impl ArrayContentType {
            fn from_char(c: char) -> Result<Self, ArrayTypeSpecifierError> {
                match c {
                    $($c => Ok(ArrayContentType::$n(Vec::new())),)*
                    _ => Err(ArrayTypeSpecifierError { _priv: () }),
                }
            }

            fn typecode(&self) -> char {
                match self {
                    $(ArrayContentType::$n(_) => $c,)*
                }
            }

            fn itemsize(&self) -> usize {
                match self {
                    $(ArrayContentType::$n(_) => std::mem::size_of::<$t>(),)*
                }
            }

            fn addr(&self) -> usize {
                match self {
                    $(ArrayContentType::$n(v) => v.as_ptr() as usize,)*
                }
            }

            fn len(&self) -> usize {
                match self {
                    $(ArrayContentType::$n(v) => v.len(),)*
                }
            }

            fn push(&mut self, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
                match self {
                    $(ArrayContentType::$n(v) => {
                        let val = $t::try_from_object(vm, obj)?;
                        v.push(val);
                    })*
                }
                Ok(())
            }

            fn pop(&mut self, i: usize, vm: &VirtualMachine) -> PyResult {
                match self {
                    $(ArrayContentType::$n(v) => {
                        v.remove(i).into_pyobject(vm)
                    })*
                }
            }

            fn insert(&mut self, i: usize, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
                match self {
                    $(ArrayContentType::$n(v) => {
                        let val = $t::try_from_object(vm, obj)?;
                        v.insert(i, val);
                    })*
                }
                Ok(())
            }

            fn count(&self, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
                match self {
                    $(ArrayContentType::$n(v) => {
                        let val = $t::try_from_object(vm, obj)?;
                        Ok(v.iter().filter(|&&a| a == val).count())
                    })*
                }
            }

            fn frombytes(&mut self, b: &[u8]) {
                match self {
                    $(ArrayContentType::$n(v) => {
                        // safe because every configuration of bytes for the types we
                        // support are valid
                        let ptr = b.as_ptr() as *const $t;
                        let ptr_len = b.len() / std::mem::size_of::<$t>();
                        let slice = unsafe { std::slice::from_raw_parts(ptr, ptr_len) };
                        v.extend_from_slice(slice);
                    })*
                }
            }

            fn tobytes(&self) -> Vec<u8> {
                match self {
                    $(ArrayContentType::$n(v) => {
                        // safe because we're just reading memory as bytes
                        let ptr = v.as_ptr() as *const u8;
                        let ptr_len = v.len() * std::mem::size_of::<$t>();
                        let slice = unsafe { std::slice::from_raw_parts(ptr, ptr_len) };
                        slice.to_vec()
                    })*
                }
            }

            fn index(&self, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<Option<usize>> {
                match self {
                    $(ArrayContentType::$n(v) => {
                        let val = $t::try_from_object(vm, x)?;
                        Ok(v.iter().position(|&a| a == val))
                    })*
                }
            }

            fn reverse(&mut self) {
                match self {
                    $(ArrayContentType::$n(v) => v.reverse(),)*
                }
            }

            fn getitem(&self, i: usize, vm: &VirtualMachine) -> Option<PyResult> {
                match self {
                    $(ArrayContentType::$n(v) => v.get(i).map(|x| x.into_pyobject(vm)),)*
                }
            }

            fn iter<'a>(&'a self, vm: &'a VirtualMachine) -> impl Iterator<Item = PyResult> + 'a {
                let mut i = 0;
                std::iter::from_fn(move || {
                    let ret = self.getitem(i, vm);
                    i += 1;
                    ret
                })
            }
        }
    };
}

def_array_enum!(
    (SignedByte, i8, 'b'),
    (UnsignedByte, u8, 'B'),
    // TODO: support unicode char
    (SignedShort, i16, 'h'),
    (UnsignedShort, u16, 'H'),
    (SignedInt, i32, 'i'),
    (UnsignedInt, u32, 'I'),
    (SignedLong, i64, 'l'),
    (UnsignedLong, u64, 'L'),
    (SignedLongLong, i64, 'q'),
    (UnsignedLongLong, u64, 'Q'),
    (Float, f32, 'f'),
    (Double, f64, 'd'),
);

#[pyclass]
#[derive(Debug)]
pub struct PyArray {
    array: RefCell<ArrayContentType>,
}
pub type PyArrayRef = PyRef<PyArray>;

impl PyValue for PyArray {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("array", "array")
    }
}

#[pyimpl]
impl PyArray {
    #[pyslot(new)]
    fn tp_new(
        cls: PyClassRef,
        spec: PyStringRef,
        init: OptionalArg<PyIterable>,
        vm: &VirtualMachine,
    ) -> PyResult<PyArrayRef> {
        let spec = match spec.as_str().len() {
            1 => spec.as_str().chars().next().unwrap(),
            _ => {
                return Err(vm.new_type_error(
                    "array() argument 1 must be a unicode character, not str".to_owned(),
                ))
            }
        };
        let array =
            ArrayContentType::from_char(spec).map_err(|err| vm.new_value_error(err.to_string()))?;
        let zelf = PyArray {
            array: RefCell::new(array),
        };
        if let OptionalArg::Present(init) = init {
            zelf.extend(init, vm)?;
        }
        zelf.into_ref_with_type(vm, cls)
    }

    #[pyproperty]
    fn typecode(&self, _vm: &VirtualMachine) -> String {
        self.array.borrow().typecode().to_string()
    }

    #[pyproperty]
    fn itemsize(&self, _vm: &VirtualMachine) -> usize {
        self.array.borrow().itemsize()
    }

    #[pymethod]
    fn append(&self, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.array.borrow_mut().push(x, vm)
    }

    #[pymethod]
    fn buffer_info(&self, _vm: &VirtualMachine) -> (usize, usize) {
        let array = self.array.borrow();
        (array.addr(), array.len())
    }

    #[pymethod]
    fn count(&self, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
        self.array.borrow().count(x, vm)
    }

    fn idx(&self, i: isize, vm: &VirtualMachine) -> PyResult<usize> {
        let len = self.array.borrow().len();
        if len == 0 {
            return Err(vm.new_index_error("pop from empty array".to_owned()));
        }
        let i = if i.is_negative() {
            len - i.abs() as usize
        } else {
            i as usize
        };
        if i > len - 1 {
            return Err(vm.new_index_error("pop index out of range".to_owned()));
        }
        Ok(i)
    }

    #[pymethod]
    fn extend(&self, iter: PyIterable, vm: &VirtualMachine) -> PyResult<()> {
        let mut array = self.array.borrow_mut();
        for elem in iter.iter(vm)? {
            array.push(elem?, vm)?;
        }
        Ok(())
    }

    #[pymethod]
    fn frombytes(&self, b: PyBytesRef, vm: &VirtualMachine) -> PyResult<()> {
        let b = b.get_value();
        let itemsize = self.array.borrow().itemsize();
        if b.len() % itemsize != 0 {
            return Err(vm.new_value_error("bytes length not a multiple of item size".to_owned()));
        }
        if b.len() / itemsize > 0 {
            self.array.borrow_mut().frombytes(&b);
        }
        Ok(())
    }

    #[pymethod]
    fn index(&self, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
        self.array
            .borrow()
            .index(x, vm)?
            .ok_or_else(|| vm.new_value_error("x not in array".to_owned()))
    }

    #[pymethod]
    fn insert(&self, i: isize, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let i = self.idx(i, vm)?;
        self.array.borrow_mut().insert(i, x, vm)
    }

    #[pymethod]
    fn pop(&self, i: OptionalArg<isize>, vm: &VirtualMachine) -> PyResult {
        let i = self.idx(i.unwrap_or(-1), vm)?;
        self.array.borrow_mut().pop(i, vm)
    }

    #[pymethod]
    fn tobytes(&self, _vm: &VirtualMachine) -> Vec<u8> {
        self.array.borrow().tobytes()
    }

    #[pymethod]
    fn tolist(&self, vm: &VirtualMachine) -> PyResult {
        let array = self.array.borrow();
        let mut v = Vec::with_capacity(array.len());
        for obj in array.iter(vm) {
            v.push(obj?);
        }
        Ok(vm.ctx.new_list(v))
    }

    #[pymethod]
    fn reverse(&self, _vm: &VirtualMachine) {
        self.array.borrow_mut().reverse()
    }

    #[pymethod(name = "__getitem__")]
    fn getitem(&self, i: isize, vm: &VirtualMachine) -> PyResult {
        let i = self.idx(i, vm)?;
        self.array
            .borrow()
            .getitem(i, vm)
            .unwrap_or_else(|| Err(vm.new_index_error("array index out of range".to_owned())))
    }

    #[pymethod(name = "__eq__")]
    fn eq(lhs: PyObjectRef, rhs: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let lhs = class_or_notimplemented!(vm, Self, lhs);
        let rhs = class_or_notimplemented!(vm, Self, rhs);
        let lhs = lhs.array.borrow();
        let rhs = rhs.array.borrow();
        if lhs.len() != rhs.len() {
            Ok(vm.new_bool(false))
        } else {
            for (a, b) in lhs.iter(vm).zip(rhs.iter(vm)) {
                let ne = objbool::boolval(vm, vm._ne(a?, b?)?)?;
                if ne {
                    return Ok(vm.new_bool(false));
                }
            }
            Ok(vm.new_bool(true))
        }
    }

    #[pymethod(name = "__len__")]
    fn len(&self, _vm: &VirtualMachine) -> usize {
        self.array.borrow().len()
    }

    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyArrayIter {
        PyArrayIter {
            position: Cell::new(0),
            array: zelf,
        }
    }
}

#[pyclass]
#[derive(Debug)]
pub struct PyArrayIter {
    position: Cell<usize>,
    array: PyArrayRef,
}

impl PyValue for PyArrayIter {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("array", "arrayiterator")
    }
}

#[pyimpl]
impl PyArrayIter {
    #[pymethod(name = "__next__")]
    fn next(&self, vm: &VirtualMachine) -> PyResult {
        if self.position.get() < self.array.array.borrow().len() {
            let ret = self
                .array
                .array
                .borrow()
                .getitem(self.position.get(), vm)
                .unwrap()?;
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

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    py_module!(vm, "array", {
        "array" => PyArray::make_class(&vm.ctx),
        "arrayiterator" => PyArrayIter::make_class(&vm.ctx),
    })
}

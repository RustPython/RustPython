use crate::function::OptionalArg;
use crate::obj::objbytes::PyBytesRef;
use crate::obj::objstr::PyStringRef;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{
    IntoPyObject, PyClassImpl, PyIterable, PyObjectRef, PyRef, PyResult, PyValue, TryFromObject,
};
use crate::VirtualMachine;

use std::cell::RefCell;

macro_rules! def_array_enum {
    ($(($n:ident, $t:ident, $c:literal)),*$(,)?) => {
        #[derive(Debug)]
        enum ArrayContentType {
            $($n(Vec<$t>),)*
        }

        impl ArrayContentType {
            fn from_char(c: char) -> Option<Self> {
                match c {
                    $($c => Some(ArrayContentType::$n(Vec::new())),)*
                    _ => None,
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
        }
    };
}

def_array_enum!(
    (SignedByte, i8, 'b'),
    (UnsignedByte, u8, 'B'),
    (SignedShort, i16, 'h'),
    (UnsignedShort, u16, 'H'),
    (SignedInt, i16, 'i'),
    (UnsignedInt, u16, 'I'),
    (SignedLong, i32, 'l'),
    (UnsignedLong, u32, 'L'),
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
    #[pymethod(name = "__new__")]
    fn new(
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
        let array = ArrayContentType::from_char(spec).ok_or_else(|| {
            vm.new_value_error(
                "bad typecode (must be b, B, u, h, H, i, I, l, L, q, Q, f or d)".to_owned(),
            )
        })?;
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
    fn reverse(&self, _vm: &VirtualMachine) {
        self.array.borrow_mut().reverse()
    }
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    py_module!(vm, "array", {
        "array" => PyArray::make_class(&vm.ctx),
    })
}

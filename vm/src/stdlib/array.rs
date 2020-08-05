use crate::function::OptionalArg;
use crate::obj::objbytes::PyBytesRef;
use crate::obj::objslice::PySliceRef;
use crate::obj::objstr::PyStringRef;
use crate::obj::objtype::PyClassRef;
use crate::obj::{objbool, objiter};
use crate::pyobject::{
    BorrowValue, Either, IntoPyObject, PyClassImpl, PyIterable, PyObjectRef, PyRef, PyResult,
    PyValue, TryFromObject,
};
use crate::VirtualMachine;

use crate::common::cell::{
    PyMappedRwLockReadGuard, PyMappedRwLockWriteGuard, PyRwLock, PyRwLockReadGuard,
    PyRwLockWriteGuard,
};
use std::fmt;

use crossbeam_utils::atomic::AtomicCell;

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

            fn pop(&mut self, i: usize, vm: &VirtualMachine) -> PyObjectRef {
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
                        Ok(<Option<$t>>::try_from_object(vm, obj)?.map_or(0, |val| {
                            v.iter().filter(|&&a| a == val).count()
                        }))
                    })*
                }
            }

            fn remove(&mut self, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<()>{
                match self {
                    $(ArrayContentType::$n(v) => {
                        let pos = <Option<$t>>::try_from_object(vm, obj)?.map_or(None, |val| {
                            v.iter().position(|&a| a == val)
                        });

                        match pos {
                            Some(x) => {
                                v.remove(x);
                            },
                            None => return Err(vm.new_value_error("array.remove(x): x not in array".to_owned()))
                        }
                    })*
                }
                Ok(())
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

            fn get_bytes(&self) -> &[u8] {
                match self {
                    $(ArrayContentType::$n(v) => {
                        // safe because we're just reading memory as bytes
                        let ptr = v.as_ptr() as *const u8;
                        let ptr_len = v.len() * std::mem::size_of::<$t>();
                        unsafe { std::slice::from_raw_parts(ptr, ptr_len) }
                    })*
                }
            }

            fn get_bytes_mut(&mut self) -> &mut [u8] {
                match self {
                    $(ArrayContentType::$n(v) => {
                        // safe because we're just reading memory as bytes
                        let ptr = v.as_ptr() as *mut u8;
                        let ptr_len = v.len() * std::mem::size_of::<$t>();
                        unsafe { std::slice::from_raw_parts_mut(ptr, ptr_len) }
                    })*
                }
            }

            fn index(&self, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<Option<usize>> {
                match self {
                    $(ArrayContentType::$n(v) => {
                        Ok(<Option<$t>>::try_from_object(vm, x)?.map_or(None, |val| {
                            v.iter().position(|&a| a == val)
                        }))
                    })*
                }
            }

            fn reverse(&mut self) {
                match self {
                    $(ArrayContentType::$n(v) => v.reverse(),)*
                }
            }

            fn idx(&self, i: isize, msg: &str, vm: &VirtualMachine) -> PyResult<usize> {
                let len = self.len();
                let i = if i.is_negative() {
                    if i.abs() as usize > len {
                        return Err(vm.new_index_error(format!("{} index out of range", msg)));
                    } else {
                        len - i.abs() as usize
                    }
                } else {
                    i as usize
                };
                if i > len - 1 {
                    return Err(vm.new_index_error(format!("{} index out of range", msg)));
                }
                Ok(i)
            }

            fn getitem_by_idx(&self, i: usize, vm: &VirtualMachine) -> Option<PyObjectRef> {
                match self {
                    $(ArrayContentType::$n(v) => v.get(i).map(|x| x.into_pyobject(vm)),)*
                }
            }

            fn getitem(&self, needle: Either<isize, PySliceRef>, vm: &VirtualMachine) -> PyResult {
                match needle {
                    Either::A(i) => {
                        self.idx(i, "array", vm).map(|i| {
                            self.getitem_by_idx(i, vm).unwrap()
                        })
                    }
                    Either::B(_slice) => Err(vm.new_not_implemented_error("array slice is not implemented".to_owned())),
                }
            }

            fn setitem(&mut self, needle: Either<isize, PySliceRef>, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
                match needle {
                    Either::A(i) => {
                        let i = self.idx(i, "array assignment", vm)?;
                        match self {
                            $(ArrayContentType::$n(v) => { v[i] = TryFromObject::try_from_object(vm, value)? },)*
                        }
                        Ok(())
                    }
                    Either::B(_slice) => Err(vm.new_not_implemented_error("array slice is not implemented".to_owned())),
                }
            }

            fn iter<'a>(&'a self, vm: &'a VirtualMachine) -> impl Iterator<Item = PyObjectRef> + 'a {
                let mut i = 0;
                std::iter::from_fn(move || {
                    let ret = self.getitem_by_idx(i, vm);
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

#[pyclass(name = "array")]
#[derive(Debug)]
pub struct PyArray {
    array: PyRwLock<ArrayContentType>,
}

pub type PyArrayRef = PyRef<PyArray>;

impl PyValue for PyArray {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("array", "array")
    }
}

#[pyimpl(flags(BASETYPE))]
impl PyArray {
    fn borrow_value(&self) -> PyRwLockReadGuard<'_, ArrayContentType> {
        self.array.read()
    }

    fn borrow_value_mut(&self) -> PyRwLockWriteGuard<'_, ArrayContentType> {
        self.array.write()
    }

    #[pyslot]
    fn tp_new(
        cls: PyClassRef,
        spec: PyStringRef,
        init: OptionalArg<PyIterable>,
        vm: &VirtualMachine,
    ) -> PyResult<PyArrayRef> {
        let spec = match spec.borrow_value().len() {
            1 => spec.borrow_value().chars().next().unwrap(),
            _ => {
                return Err(vm.new_type_error(
                    "array() argument 1 must be a unicode character, not str".to_owned(),
                ))
            }
        };
        let array =
            ArrayContentType::from_char(spec).map_err(|err| vm.new_value_error(err.to_string()))?;
        let zelf = PyArray {
            array: PyRwLock::new(array),
        };
        if let OptionalArg::Present(init) = init {
            zelf.extend(init, vm)?;
        }
        zelf.into_ref_with_type(vm, cls)
    }

    #[pyproperty]
    fn typecode(&self) -> String {
        self.borrow_value().typecode().to_string()
    }

    #[pyproperty]
    fn itemsize(&self) -> usize {
        self.borrow_value().itemsize()
    }

    #[pymethod]
    fn append(&self, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.borrow_value_mut().push(x, vm)
    }

    #[pymethod]
    fn buffer_info(&self) -> (usize, usize) {
        let array = self.borrow_value();
        (array.addr(), array.len())
    }

    #[pymethod]
    fn count(&self, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
        self.borrow_value().count(x, vm)
    }

    #[pymethod]
    fn remove(&self, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.borrow_value_mut().remove(x, vm)
    }

    #[pymethod]
    fn extend(&self, iter: PyIterable, vm: &VirtualMachine) -> PyResult<()> {
        let mut array = self.borrow_value_mut();
        for elem in iter.iter(vm)? {
            array.push(elem?, vm)?;
        }
        Ok(())
    }

    #[pymethod]
    fn frombytes(&self, b: PyBytesRef, vm: &VirtualMachine) -> PyResult<()> {
        let b = b.borrow_value();
        let itemsize = self.borrow_value().itemsize();
        if b.len() % itemsize != 0 {
            return Err(vm.new_value_error("bytes length not a multiple of item size".to_owned()));
        }
        if b.len() / itemsize > 0 {
            self.borrow_value_mut().frombytes(&b);
        }
        Ok(())
    }

    #[pymethod]
    fn index(&self, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
        self.borrow_value()
            .index(x, vm)?
            .ok_or_else(|| vm.new_value_error("x not in array".to_owned()))
    }

    #[pymethod]
    fn insert(&self, i: isize, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let len = self.len();
        let i = if i.is_negative() {
            let i = i.abs() as usize;
            if i > len {
                0
            } else {
                len - i
            }
        } else {
            let i = i as usize;
            if i > len {
                len
            } else {
                i
            }
        };
        self.borrow_value_mut().insert(i, x, vm)
    }

    #[pymethod]
    fn pop(&self, i: OptionalArg<isize>, vm: &VirtualMachine) -> PyResult {
        if self.len() == 0 {
            Err(vm.new_index_error("pop from empty array".to_owned()))
        } else {
            let i = self.borrow_value().idx(i.unwrap_or(-1), "pop", vm)?;
            Ok(self.borrow_value_mut().pop(i, vm))
        }
    }

    #[pymethod]
    pub(crate) fn tobytes(&self) -> Vec<u8> {
        self.borrow_value().get_bytes().to_vec()
    }

    pub(crate) fn get_bytes(&self) -> PyMappedRwLockReadGuard<'_, [u8]> {
        PyRwLockReadGuard::map(self.borrow_value(), |a| a.get_bytes())
    }

    pub(crate) fn get_bytes_mut(&self) -> PyMappedRwLockWriteGuard<'_, [u8]> {
        PyRwLockWriteGuard::map(self.borrow_value_mut(), |a| a.get_bytes_mut())
    }

    #[pymethod]
    fn tolist(&self, vm: &VirtualMachine) -> PyResult {
        let array = self.borrow_value();
        let mut v = Vec::with_capacity(array.len());
        for obj in array.iter(vm) {
            v.push(obj);
        }
        Ok(vm.ctx.new_list(v))
    }

    #[pymethod]
    fn reverse(&self) {
        self.borrow_value_mut().reverse()
    }

    #[pymethod(magic)]
    fn getitem(&self, needle: Either<isize, PySliceRef>, vm: &VirtualMachine) -> PyResult {
        self.borrow_value().getitem(needle, vm)
    }

    #[pymethod(magic)]
    fn setitem(
        &self,
        needle: Either<isize, PySliceRef>,
        obj: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        self.borrow_value_mut().setitem(needle, obj, vm)
    }

    #[pymethod(name = "__eq__")]
    fn eq(lhs: PyObjectRef, rhs: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let lhs = class_or_notimplemented!(vm, Self, lhs);
        let rhs = class_or_notimplemented!(vm, Self, rhs);
        let lhs = lhs.borrow_value();
        let rhs = rhs.borrow_value();
        if lhs.len() != rhs.len() {
            Ok(vm.ctx.new_bool(false))
        } else {
            for (a, b) in lhs.iter(vm).zip(rhs.iter(vm)) {
                let ne = objbool::boolval(vm, vm._ne(a, b)?)?;
                if ne {
                    return Ok(vm.ctx.new_bool(false));
                }
            }
            Ok(vm.ctx.new_bool(true))
        }
    }

    #[pymethod(name = "__lt__")]
    fn lt(lhs: PyObjectRef, rhs: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let lhs = class_or_notimplemented!(vm, Self, lhs);
        let rhs = class_or_notimplemented!(vm, Self, rhs);
        let lhs = lhs.borrow_value();
        let rhs = rhs.borrow_value();

        for (a, b) in lhs.iter(vm).zip(rhs.iter(vm)) {
            let lt = objbool::boolval(vm, vm._lt(a, b)?)?;

            if lt {
                return Ok(vm.ctx.new_bool(true));
            }
        }

        Ok(vm.ctx.new_bool(lhs.len() < rhs.len()))
    }

    #[pymethod(name = "__le__")]
    fn le(lhs: PyObjectRef, rhs: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let lhs = class_or_notimplemented!(vm, Self, lhs);
        let rhs = class_or_notimplemented!(vm, Self, rhs);
        let lhs = lhs.borrow_value();
        let rhs = rhs.borrow_value();

        for (a, b) in lhs.iter(vm).zip(rhs.iter(vm)) {
            let le = objbool::boolval(vm, vm._le(a, b)?)?;

            if le {
                return Ok(vm.ctx.new_bool(true));
            }
        }

        Ok(vm.ctx.new_bool(lhs.len() <= rhs.len()))
    }

    #[pymethod(name = "__gt__")]
    fn gt(lhs: PyObjectRef, rhs: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let lhs = class_or_notimplemented!(vm, Self, lhs);
        let rhs = class_or_notimplemented!(vm, Self, rhs);
        let lhs = lhs.borrow_value();
        let rhs = rhs.borrow_value();

        for (a, b) in lhs.iter(vm).zip(rhs.iter(vm)) {
            let gt = objbool::boolval(vm, vm._gt(a, b)?)?;

            if gt {
                return Ok(vm.ctx.new_bool(true));
            }
        }

        Ok(vm.ctx.new_bool(lhs.len() > rhs.len()))
    }

    #[pymethod(name = "__ge__")]
    fn ge(lhs: PyObjectRef, rhs: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let lhs = class_or_notimplemented!(vm, Self, lhs);
        let rhs = class_or_notimplemented!(vm, Self, rhs);
        let lhs = lhs.borrow_value();
        let rhs = rhs.borrow_value();

        for (a, b) in lhs.iter(vm).zip(rhs.iter(vm)) {
            let ge = objbool::boolval(vm, vm._ge(a, b)?)?;

            if ge {
                return Ok(vm.ctx.new_bool(true));
            }
        }

        Ok(vm.ctx.new_bool(lhs.len() >= rhs.len()))
    }

    #[pymethod(name = "__len__")]
    pub(crate) fn len(&self) -> usize {
        self.borrow_value().len()
    }

    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyRef<Self>) -> PyArrayIter {
        PyArrayIter {
            position: AtomicCell::new(0),
            array: zelf,
        }
    }
}

#[pyclass]
#[derive(Debug)]
pub struct PyArrayIter {
    position: AtomicCell<usize>,
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
        let pos = self.position.fetch_add(1);
        if let Some(item) = self.array.borrow_value().getitem_by_idx(pos, vm) {
            Ok(item)
        } else {
            Err(objiter::new_stop_iteration(vm))
        }
    }

    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyRef<Self>) -> PyRef<Self> {
        zelf
    }
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    py_module!(vm, "array", {
        "array" => PyArray::make_class(&vm.ctx),
        "arrayiterator" => PyArrayIter::make_class(&vm.ctx),
    })
}

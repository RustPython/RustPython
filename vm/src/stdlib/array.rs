use crate::common::cell::{
    PyMappedRwLockReadGuard, PyMappedRwLockWriteGuard, PyRwLock, PyRwLockReadGuard,
    PyRwLockWriteGuard,
};
use crate::function::OptionalArg;
use crate::obj::objbytes::PyBytesRef;
use crate::obj::objfloat::try_float;
use crate::obj::objiter;
use crate::obj::objsequence::{PySliceableSequence, PySliceableSequenceMut};
use crate::obj::objslice::PySliceRef;
use crate::obj::objstr::PyStringRef;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{
    BorrowValue, Either, IdProtocol, IntoPyObject, PyArithmaticValue, PyClassImpl,
    PyComparisonValue, PyIterable, PyObjectRef, PyRef, PyResult, PyValue, TryFromObject,
    TypeProtocol,
};
use crate::VirtualMachine;
use crossbeam_utils::atomic::AtomicCell;
use itertools::Itertools;
use std::cmp::Ordering;
use std::fmt;
use PyArithmaticValue::Implemented;

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
        #[derive(Debug, Clone)]
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
                        let val = $t::try_into_from_object(vm, obj)?;
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
                        let val = $t::try_into_from_object(vm, obj)?;
                        v.insert(i, val);
                    })*
                }
                Ok(())
            }

            fn count(&self, obj: PyObjectRef, vm: &VirtualMachine) -> usize {
                match self {
                    $(ArrayContentType::$n(v) => {
                        if let Ok(val) = $t::try_into_from_object(vm, obj) {
                            v.iter().filter(|&&a| a == val).count()
                        } else {
                            0
                        }
                    })*
                }
            }

            fn remove(&mut self, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<()>{
                match self {
                    $(ArrayContentType::$n(v) => {
                        if let Ok(val) = $t::try_into_from_object(vm, obj) {
                            if let Some(pos) = v.iter().position(|&a| a == val) {
                                v.remove(pos);
                                return Ok(());
                            }
                        }
                        Err(vm.new_value_error("array.remove(x): x not in array".to_owned()))
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

            fn index(&self, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
                match self {
                    $(ArrayContentType::$n(v) => {
                        if let Ok(val) = $t::try_into_from_object(vm, obj) {
                            if let Some(pos) = v.iter().position(|&a| a == val) {
                                return Ok(pos);
                            }
                        }
                        Err(vm.new_value_error("array.index(x): x not in array".to_owned()))
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

            fn getitem_by_slice(&self, slice: PySliceRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
                match self {
                    $(ArrayContentType::$n(v) => {
                        let elements = v.get_slice_items(vm, &slice)?;
                        let sliced = ArrayContentType::$n(elements);
                        let obj = PyArray {
                            array: PyRwLock::new(sliced)
                        }
                        .into_object(vm);
                        Ok(obj)
                    })*
                }
            }

            fn getitem(&self, needle: Either<isize, PySliceRef>, vm: &VirtualMachine) -> PyResult {
                match needle {
                    Either::A(i) => {
                        self.idx(i, "array", vm).map(|i| {
                            self.getitem_by_idx(i, vm).unwrap()
                        })
                    }
                    Either::B(slice) => self.getitem_by_slice(slice, vm),
                }
            }

            fn setitem_by_slice(&mut self, slice: PySliceRef, items: &ArrayContentType, vm: &VirtualMachine) -> PyResult<()> {
                match self {
                    $(ArrayContentType::$n(elements) => if let ArrayContentType::$n(items) = items {
                        elements.set_slice_items(vm, &slice, items)
                    } else {
                        Err(vm.new_type_error("bad argument type for built-in operation".to_owned()))
                    },)*
                }
            }

            fn setitem_by_idx(&mut self, i: isize, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
                let i = self.idx(i, "array assignment", vm)?;
                match self {
                    $(ArrayContentType::$n(v) => { v[i] = $t::try_into_from_object(vm, value)? },)*
                }
                Ok(())
            }

            fn delitem_by_idx(&mut self, i: isize, vm: &VirtualMachine) -> PyResult<()> {
                let i = self.idx(i, "array assignment", vm)?;
                match self {
                    $(ArrayContentType::$n(v) => { v.remove(i); },)*
                }
                Ok(())
            }

            fn delitem_by_slice(&mut self, slice: PySliceRef, vm: &VirtualMachine) -> PyResult<()> {

                match self {
                    $(ArrayContentType::$n(elements) => {
                        elements.delete_slice(vm, &slice)
                    })*
                }
            }

            fn add(&self, other: &ArrayContentType, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
                match self {
                    $(ArrayContentType::$n(v) => if let ArrayContentType::$n(other) = other {
                        let elements = v.iter().chain(other.iter()).cloned().collect();
                        let sliced = ArrayContentType::$n(elements);
                        let obj = PyArray {
                            array: PyRwLock::new(sliced)
                        }
                        .into_object(vm);
                        Ok(obj)
                    } else {
                        Err(vm.new_type_error("bad argument type for built-in operation".to_owned()))
                    },)*
                }
            }

            fn iadd(&mut self, other: ArrayContentType, vm: &VirtualMachine) -> PyResult<()> {
                match self {
                    $(ArrayContentType::$n(v) => if let ArrayContentType::$n(mut other) = other {
                        v.append(&mut other);
                        Ok(())
                    } else {
                        Err(vm.new_type_error("can only extend with array of same kind".to_owned()))
                    },)*
                }
            }

            fn mul(&self, counter: isize, vm: &VirtualMachine) -> PyObjectRef {
                let counter = if counter < 0 { 0 } else { counter as usize };
                match self {
                    $(ArrayContentType::$n(v) => {
                        let elements = v.iter().cycle().take(v.len() * counter).cloned().collect();
                        let sliced = ArrayContentType::$n(elements);
                        PyArray {
                            array: PyRwLock::new(sliced)
                        }
                        .into_object(vm)
                    })*
                }
            }

            fn imul(&mut self, counter: isize) {
                let counter = if counter < 0 { 0 } else { counter as usize };
                match self {
                    $(ArrayContentType::$n(v) => {
                        let mut elements = v.iter().cycle().take(v.len() * counter).cloned().collect();
                        std::mem::swap(v, &mut elements);
                    })*
                }
            }

            fn repr(&self, _vm: &VirtualMachine) -> PyResult<String> {
                // we don't need ReprGuard here
                let s = match self {
                    $(ArrayContentType::$n(v) => {
                        if v.is_empty() {
                            format!("array('{}')", $c)
                        } else {
                            format!("array('{}', [{}])", $c, v.iter().format(", "))
                        }
                    })*
                };
                Ok(s)
            }

            fn iter<'a>(&'a self, vm: &'a VirtualMachine) -> impl Iterator<Item = PyObjectRef> + 'a {
                let mut i = 0;
                std::iter::from_fn(move || {
                    let ret = self.getitem_by_idx(i, vm);
                    i += 1;
                    ret
                })
            }

            fn cmp(&self, other: &ArrayContentType) -> Result<Option<Ordering>, ()> {
                match self {
                    $(ArrayContentType::$n(v) => {
                        if let ArrayContentType::$n(other) = other {
                            Ok(PartialOrd::partial_cmp(v, other))
                        } else {
                            Err(())
                        }
                    })*
                }
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

trait ArrayElement: Sized {
    fn try_into_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self>;
}

macro_rules! adapt_try_into_from_object {
    ($(($t:ty, $f:path),)*) => {$(
        impl ArrayElement for $t {
            fn try_into_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
                $f(vm, obj)
            }
        }
    )*};
}

adapt_try_into_from_object!(
    (i8, i8::try_from_object),
    (u8, u8::try_from_object),
    (i16, i16::try_from_object),
    (u16, u16::try_from_object),
    (i32, i32::try_from_object),
    (u32, u32::try_from_object),
    (i64, i64::try_from_object),
    (u64, u64::try_from_object),
    (f32, f32_try_into_from_object),
    (f64, f64_try_into_from_object),
);

fn f32_try_into_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<f32> {
    try_float(&obj, vm)?
        .map(|x| x as f32)
        .ok_or_else(|| vm.new_type_error(format!("must be real number, not {}", obj.class().name)))
}

fn f64_try_into_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<f64> {
    try_float(&obj, vm)?
        .ok_or_else(|| vm.new_type_error(format!("must be real number, not {}", obj.class().name)))
}

#[pyclass(module = "array", name = "array")]
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
        let spec = spec.borrow_value().chars().exactly_one().map_err(|_| {
            vm.new_type_error("array() argument 1 must be a unicode character, not str".to_owned())
        })?;
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
    fn count(&self, x: PyObjectRef, vm: &VirtualMachine) -> usize {
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
        self.borrow_value().index(x, vm)
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
        zelf: PyRef<Self>,
        needle: Either<isize, PySliceRef>,
        obj: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        match needle {
            Either::A(i) => zelf.borrow_value_mut().setitem_by_idx(i, obj, vm),
            Either::B(slice) => {
                let cloned;
                let guard;
                let items = if zelf.is(&obj) {
                    cloned = zelf.borrow_value().clone();
                    &cloned
                } else {
                    match obj.payload::<PyArray>() {
                        Some(array) => {
                            guard = array.borrow_value();
                            &*guard
                        }
                        None => {
                            return Err(vm.new_type_error(format!(
                                "can only assign array (not \"{}\") to array slice",
                                obj.class().name
                            )));
                        }
                    }
                };
                zelf.borrow_value_mut().setitem_by_slice(slice, items, vm)
            }
        }
    }

    #[pymethod(name = "__delitem__")]
    fn delitem(&self, needle: Either<isize, PySliceRef>, vm: &VirtualMachine) -> PyResult<()> {
        match needle {
            Either::A(i) => self.borrow_value_mut().delitem_by_idx(i, vm),
            Either::B(slice) => self.borrow_value_mut().delitem_by_slice(slice, vm),
        }
    }

    #[pymethod(name = "__add__")]
    fn add(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        if let Some(other) = other.payload::<PyArray>() {
            self.borrow_value().add(&*other.borrow_value(), vm)
        } else {
            Err(vm.new_type_error(format!(
                "can only append array (not \"{}\") to array",
                other.class().name
            )))
        }
    }

    #[pymethod(name = "__iadd__")]
    fn iadd(zelf: PyRef<Self>, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        if let Some(other) = other.payload::<PyArray>() {
            let other = other.borrow_value().clone();
            let result = zelf.borrow_value_mut().iadd(other, vm);
            result.map(|_| zelf.into_object())
        } else {
            Err(vm.new_type_error(format!(
                "can only extend array with array (not \"{}\")",
                other.class().name
            )))
        }
    }

    #[pymethod(name = "__mul__")]
    fn mul(&self, counter: isize, vm: &VirtualMachine) -> PyObjectRef {
        self.borrow_value().mul(counter, vm)
    }

    #[pymethod(name = "__rmul__")]
    fn rmul(&self, counter: isize, vm: &VirtualMachine) -> PyObjectRef {
        self.mul(counter, &vm)
    }

    #[pymethod(name = "__imul__")]
    fn imul(zelf: PyRef<Self>, counter: isize) -> PyRef<Self> {
        zelf.borrow_value_mut().imul(counter);
        zelf
    }

    #[pymethod(name = "__repr__")]
    fn repr(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<String> {
        zelf.borrow_value().repr(vm)
    }

    fn cmp<L, O>(
        &self,
        other: PyArrayRef,
        len_cmp: L,
        obj_cmp: O,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue>
    where
        L: Fn(usize, usize) -> bool,
        O: Fn(PyObjectRef, PyObjectRef) -> PyResult<Option<bool>>,
    {
        let array_a = self.borrow_value();
        let array_b = other.borrow_value();
        let iter = Iterator::zip(array_a.iter(vm), array_b.iter(vm));
        for (a, b) in iter {
            if let Some(v) = obj_cmp(a, b)? {
                return Ok(Implemented(v));
            }
        }
        Ok(Implemented(len_cmp(self.len(), other.len())))
    }

    #[pymethod(name = "__eq__")]
    fn eq(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyComparisonValue> {
        // we cannot use zelf.is(other) for shortcut because if we contenting a
        // float value NaN we always return False even they are the same object.
        let other = class_or_notimplemented!(vm, Self, other);
        if self.len() != other.len() {
            return Ok(Implemented(false));
        }
        let array_a = self.borrow_value();
        let array_b = other.borrow_value();

        // fast path for same ArrayContentType type
        if let Ok(ord) = array_a.cmp(&*array_b) {
            let r = match ord {
                Some(Ordering::Equal) => true,
                _ => false,
            };
            return Ok(Implemented(r));
        }

        let iter = Iterator::zip(array_a.iter(vm), array_b.iter(vm));
        for (a, b) in iter {
            if !vm.bool_eq(a, b)? {
                return Ok(Implemented(false));
            }
        }
        Ok(Implemented(true))
    }

    #[pymethod(name = "__ne__")]
    fn ne(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyComparisonValue> {
        Ok(self.eq(other, vm)?.map(|v| !v))
    }

    #[pymethod(name = "__lt__")]
    fn lt(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyComparisonValue> {
        let other = class_or_notimplemented!(vm, Self, other);
        // fast path for same ArrayContentType type
        if let Ok(ord) = self.borrow_value().cmp(&*other.borrow_value()) {
            let r = match ord {
                Some(Ordering::Less) => true,
                _ => false,
            };
            return Ok(Implemented(r));
        }
        self.cmp(other, |a, b| a < b, |a, b| vm.bool_seq_lt(a, b), vm)
    }

    #[pymethod(name = "__le__")]
    fn le(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyComparisonValue> {
        let other = class_or_notimplemented!(vm, Self, other);
        // fast path for same ArrayContentType type
        if let Ok(ord) = self.borrow_value().cmp(&*other.borrow_value()) {
            let r = match ord {
                Some(Ordering::Less) | Some(Ordering::Equal) => true,
                _ => false,
            };
            return Ok(Implemented(r));
        }
        self.cmp(other, |a, b| a <= b, |a, b| vm.bool_seq_lt(a, b), vm)
    }

    #[pymethod(name = "__gt__")]
    fn gt(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyComparisonValue> {
        let other = class_or_notimplemented!(vm, Self, other);
        // fast path for same ArrayContentType type
        if let Ok(ord) = self.borrow_value().cmp(&*other.borrow_value()) {
            let r = match ord {
                Some(Ordering::Greater) => true,
                _ => false,
            };
            return Ok(Implemented(r));
        }
        self.cmp(other, |a, b| a > b, |a, b| vm.bool_seq_gt(a, b), vm)
    }

    #[pymethod(name = "__ge__")]
    fn ge(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyComparisonValue> {
        let other = class_or_notimplemented!(vm, Self, other);
        // fast path for same ArrayContentType type
        if let Ok(ord) = self.borrow_value().cmp(&*other.borrow_value()) {
            let r = match ord {
                Some(Ordering::Greater) | Some(Ordering::Equal) => true,
                _ => false,
            };
            return Ok(Implemented(r));
        }
        self.cmp(other, |a, b| a >= b, |a, b| vm.bool_seq_gt(a, b), vm)
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

#[pyclass(module = "array", name = "array_iterator")]
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

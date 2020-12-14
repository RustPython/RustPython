use crate::builtins::bytes::PyBytesRef;
use crate::builtins::float::IntoPyFloat;
use crate::builtins::list::PyList;
use crate::builtins::memory::{Buffer, BufferOptions, ResizeGuard};
use crate::builtins::pystr::PyStrRef;
use crate::builtins::pytype::PyTypeRef;
use crate::builtins::slice::PySliceRef;
use crate::common::borrow::{BorrowedValue, BorrowedValueMut};
use crate::common::lock::{
    PyMappedRwLockReadGuard, PyMappedRwLockWriteGuard, PyRwLock, PyRwLockReadGuard,
    PyRwLockWriteGuard,
};
use crate::function::OptionalArg;
use crate::pyobject::{
    BorrowValue, Either, IdProtocol, IntoPyObject, PyClassImpl, PyComparisonValue, PyIterable,
    PyObjectRef, PyRef, PyResult, PyValue, StaticType, TryFromObject, TypeProtocol,
};
use crate::sliceable::{saturate_index, PySliceableSequence, PySliceableSequenceMut};
use crate::slots::{BufferProtocol, Comparable, Iterable, PyComparisonOp, PyIter};
use crate::VirtualMachine;
use crossbeam_utils::atomic::AtomicCell;
use itertools::Itertools;
use std::cmp::Ordering;
use std::{fmt, os::raw};

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
    ($(($n:ident, $t:ty, $c:literal, $scode:literal)),*$(,)?) => {
        #[derive(Debug, Clone)]
        pub(crate) enum ArrayContentType {
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

            fn typecode_str(&self) -> &'static str {
                match self {
                    $(ArrayContentType::$n(_) => $scode,)*
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
                        let val = <$t>::try_into_from_object(vm, obj)?;
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
                        let val = <$t>::try_into_from_object(vm, obj)?;
                        v.insert(i, val);
                    })*
                }
                Ok(())
            }

            fn count(&self, obj: PyObjectRef, vm: &VirtualMachine) -> usize {
                match self {
                    $(ArrayContentType::$n(v) => {
                        if let Ok(val) = <$t>::try_into_from_object(vm, obj) {
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
                        if let Ok(val) = <$t>::try_into_from_object(vm, obj) {
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

            fn fromlist(&mut self, list: &PyList, vm: &VirtualMachine) -> PyResult<()> {
                match self {
                    $(ArrayContentType::$n(v) => {
                        // convert list before modify self
                        let mut list: Vec<$t> = list
                            .borrow_value()
                            .iter()
                            .cloned()
                            .map(|value| <$t>::try_into_from_object(vm, value))
                            .try_collect()?;
                        v.append(&mut list);
                        Ok(())
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
                        if let Ok(val) = <$t>::try_into_from_object(vm, obj) {
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
                        let array: PyArray = ArrayContentType::$n(elements).into();
                        Ok(array.into_object(vm))
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

            fn setitem_by_slice_no_resize(&mut self, slice: PySliceRef, items: &ArrayContentType, vm: &VirtualMachine) -> PyResult<()> {
                match self {
                    $(ArrayContentType::$n(elements) => if let ArrayContentType::$n(items) = items {
                        elements.set_slice_items_no_resize(vm, &slice, items)
                    } else {
                        Err(vm.new_type_error("bad argument type for built-in operation".to_owned()))
                    },)*
                }
            }

            fn setitem_by_idx(&mut self, i: isize, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
                let i = self.idx(i, "array assignment", vm)?;
                match self {
                    $(ArrayContentType::$n(v) => { v[i] = <$t>::try_into_from_object(vm, value)? },)*
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

            fn add(&self, other: &ArrayContentType, vm: &VirtualMachine) -> PyResult<Self> {
                match self {
                    $(ArrayContentType::$n(v) => if let ArrayContentType::$n(other) = other {
                        let elements = v.iter().chain(other.iter()).cloned().collect();
                        Ok(ArrayContentType::$n(elements))
                    } else {
                        Err(vm.new_type_error("bad argument type for built-in operation".to_owned()))
                    },)*
                }
            }

            fn iadd(&mut self, other: &ArrayContentType, vm: &VirtualMachine) -> PyResult<()> {
                match self {
                    $(ArrayContentType::$n(v) => if let ArrayContentType::$n(other) = other {
                        v.extend(other);
                        Ok(())
                    } else {
                        Err(vm.new_type_error("can only extend with array of same kind".to_owned()))
                    },)*
                }
            }

            fn mul(&self, counter: isize) -> Self {
                let counter = if counter < 0 { 0 } else { counter as usize };
                match self {
                    $(ArrayContentType::$n(v) => {
                        let elements = v.repeat(counter);
                        ArrayContentType::$n(elements)
                    })*
                }
            }

            fn clear(&mut self) {
                match self {
                    $(ArrayContentType::$n(v) => v.clear(),)*
                }
            }

            fn imul(&mut self, counter: isize) {
                if counter <= 0 {
                    self.clear();
                } else if counter != 1 {
                    let counter = counter as usize;
                    match self {
                        $(ArrayContentType::$n(v) => {
                            let old = v.clone();
                            v.reserve((counter - 1) * old.len());
                            for _ in 1..counter {
                                v.extend(&old);
                            }
                        })*
                    }
                }
            }

            fn byteswap(&mut self) {
                match self {
                    $(ArrayContentType::$n(v) => {
                        for element in v.iter_mut() {
                            let x = element.byteswap();
                            *element = x;
                        }
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
    (SignedByte, i8, 'b', "b"),
    (UnsignedByte, u8, 'B', "B"),
    // TODO: support unicode char
    (SignedShort, raw::c_short, 'h', "h"),
    (UnsignedShort, raw::c_ushort, 'H', "H"),
    (SignedInt, raw::c_int, 'i', "i"),
    (UnsignedInt, raw::c_uint, 'I', "I"),
    (SignedLong, raw::c_long, 'l', "l"),
    (UnsignedLong, raw::c_ulong, 'L', "L"),
    (SignedLongLong, raw::c_longlong, 'q', "q"),
    (UnsignedLongLong, raw::c_ulonglong, 'Q', "Q"),
    (Float, f32, 'f', "f"),
    (Double, f64, 'd', "d"),
);

trait ArrayElement: Sized {
    fn try_into_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self>;
    fn byteswap(self) -> Self;
}

macro_rules! impl_array_element {
    ($(($t:ty, $f_into:path, $f_swap:path),)*) => {$(
        impl ArrayElement for $t {
            fn try_into_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
                $f_into(vm, obj)
            }
            fn byteswap(self) -> Self {
                $f_swap(self)
            }
        }
    )*};
}

impl_array_element!(
    (i8, i8::try_from_object, i8::swap_bytes),
    (u8, u8::try_from_object, u8::swap_bytes),
    (i16, i16::try_from_object, i16::swap_bytes),
    (u16, u16::try_from_object, u16::swap_bytes),
    (i32, i32::try_from_object, i32::swap_bytes),
    (u32, u32::try_from_object, u32::swap_bytes),
    (i64, i64::try_from_object, i64::swap_bytes),
    (u64, u64::try_from_object, u64::swap_bytes),
    (f32, f32_try_into_from_object, f32_swap_bytes),
    (f64, f64_try_into_from_object, f64_swap_bytes),
);

fn f32_swap_bytes(x: f32) -> f32 {
    f32::from_bits(x.to_bits().swap_bytes())
}

fn f64_swap_bytes(x: f64) -> f64 {
    f64::from_bits(x.to_bits().swap_bytes())
}

fn f32_try_into_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<f32> {
    IntoPyFloat::try_from_object(vm, obj).map(|x| x.to_f64() as f32)
}

fn f64_try_into_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<f64> {
    IntoPyFloat::try_from_object(vm, obj).map(|x| x.to_f64())
}

#[pyclass(module = "array", name = "array")]
#[derive(Debug)]
pub struct PyArray {
    array: PyRwLock<ArrayContentType>,
    exports: AtomicCell<usize>,
}

pub type PyArrayRef = PyRef<PyArray>;

impl PyValue for PyArray {
    fn class(_vm: &VirtualMachine) -> &PyTypeRef {
        Self::static_type()
    }
}

impl From<ArrayContentType> for PyArray {
    fn from(array: ArrayContentType) -> Self {
        PyArray {
            array: PyRwLock::new(array),
            exports: AtomicCell::new(0),
        }
    }
}

#[pyimpl(flags(BASETYPE), with(Comparable, BufferProtocol, Iterable))]
impl PyArray {
    fn borrow_value(&self) -> PyRwLockReadGuard<'_, ArrayContentType> {
        self.array.read()
    }

    fn borrow_value_mut(&self) -> PyRwLockWriteGuard<'_, ArrayContentType> {
        self.array.write()
    }

    #[pyslot]
    fn tp_new(
        cls: PyTypeRef,
        spec: PyStrRef,
        init: OptionalArg<PyIterable>,
        vm: &VirtualMachine,
    ) -> PyResult<PyArrayRef> {
        let spec = spec.borrow_value().chars().exactly_one().map_err(|_| {
            vm.new_type_error("array() argument 1 must be a unicode character, not str".to_owned())
        })?;
        let mut array =
            ArrayContentType::from_char(spec).map_err(|err| vm.new_value_error(err.to_string()))?;
        // TODO: support buffer protocol
        if let OptionalArg::Present(init) = init {
            for obj in init.iter(vm)? {
                array.push(obj?, vm)?;
            }
        }
        Self::from(array).into_ref_with_type(vm, cls)
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
    fn append(zelf: PyRef<Self>, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        zelf.try_resizable(vm)?.push(x, vm)
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
    fn remove(zelf: PyRef<Self>, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        zelf.try_resizable(vm)?.remove(x, vm)
    }

    #[pymethod]
    fn extend(zelf: PyRef<Self>, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let mut w = zelf.try_resizable(vm)?;
        if zelf.is(&obj) {
            w.imul(2);
            Ok(())
        } else if let Some(array) = obj.payload::<PyArray>() {
            w.iadd(&*array.borrow_value(), vm)
        } else {
            let iter = PyIterable::try_from_object(vm, obj)?;
            // zelf.extend_from_iterable(iter, vm)
            for obj in iter.iter(vm)? {
                w.push(obj?, vm)?;
            }
            Ok(())
        }
    }

    #[pymethod]
    fn frombytes(zelf: PyRef<Self>, b: PyBytesRef, vm: &VirtualMachine) -> PyResult<()> {
        let b = b.borrow_value();
        let itemsize = zelf.borrow_value().itemsize();
        if b.len() % itemsize != 0 {
            return Err(vm.new_value_error("bytes length not a multiple of item size".to_owned()));
        }
        if b.len() / itemsize > 0 {
            zelf.try_resizable(vm)?.frombytes(&b);
        }
        Ok(())
    }

    #[pymethod]
    fn byteswap(&self) {
        self.borrow_value_mut().byteswap();
    }

    #[pymethod]
    fn index(&self, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
        self.borrow_value().index(x, vm)
    }

    #[pymethod]
    fn insert(zelf: PyRef<Self>, i: isize, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let mut w = zelf.try_resizable(vm)?;
        let i = saturate_index(i, w.len());
        w.insert(i, x, vm)
    }

    #[pymethod]
    fn pop(zelf: PyRef<Self>, i: OptionalArg<isize>, vm: &VirtualMachine) -> PyResult {
        let mut w = zelf.try_resizable(vm)?;
        if w.len() == 0 {
            Err(vm.new_index_error("pop from empty array".to_owned()))
        } else {
            let i = w.idx(i.unwrap_or(-1), "pop", vm)?;
            Ok(w.pop(i, vm))
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
    fn fromlist(zelf: PyRef<Self>, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        if let Some(list) = obj.payload::<PyList>() {
            zelf.try_resizable(vm)?.fromlist(list, vm)
        } else {
            Err(vm.new_type_error("arg must be list".to_owned()))
        }
    }

    #[pymethod]
    fn reverse(&self) {
        self.borrow_value_mut().reverse()
    }

    #[pymethod(magic)]
    fn copy(&self) -> PyArray {
        self.array.read().clone().into()
    }

    #[pymethod(magic)]
    fn deepcopy(&self, _memo: PyObjectRef) -> PyArray {
        self.copy()
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
                if let Ok(mut w) = zelf.try_resizable(vm) {
                    w.setitem_by_slice(slice, items, vm)
                } else {
                    zelf.borrow_value_mut()
                        .setitem_by_slice_no_resize(slice, items, vm)
                }
            }
        }
    }

    #[pymethod(name = "__delitem__")]
    fn delitem(
        zelf: PyRef<Self>,
        needle: Either<isize, PySliceRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        match needle {
            Either::A(i) => zelf.try_resizable(vm)?.delitem_by_idx(i, vm),
            Either::B(slice) => zelf.try_resizable(vm)?.delitem_by_slice(slice, vm),
        }
    }

    #[pymethod(name = "__add__")]
    fn add(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        if let Some(other) = other.payload::<PyArray>() {
            self.borrow_value()
                .add(&*other.borrow_value(), vm)
                .map(|array| PyArray::from(array).into_ref(vm))
        } else {
            Err(vm.new_type_error(format!(
                "can only append array (not \"{}\") to array",
                other.class().name
            )))
        }
    }

    #[pymethod(name = "__iadd__")]
    fn iadd(zelf: PyRef<Self>, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        if zelf.is(&other) {
            zelf.try_resizable(vm)?.imul(2);
            Ok(zelf)
        } else if let Some(other) = other.payload::<PyArray>() {
            let result = zelf.try_resizable(vm)?.iadd(&*other.borrow_value(), vm);
            result.map(|_| zelf)
        } else {
            Err(vm.new_type_error(format!(
                "can only extend array with array (not \"{}\")",
                other.class().name
            )))
        }
    }

    #[pymethod(name = "__mul__")]
    fn mul(&self, counter: isize, vm: &VirtualMachine) -> PyRef<Self> {
        PyArray::from(self.borrow_value().mul(counter)).into_ref(vm)
    }

    #[pymethod(name = "__rmul__")]
    fn rmul(&self, counter: isize, vm: &VirtualMachine) -> PyRef<Self> {
        self.mul(counter, &vm)
    }

    #[pymethod(name = "__imul__")]
    fn imul(zelf: PyRef<Self>, counter: isize, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        zelf.try_resizable(vm)?.imul(counter);
        Ok(zelf)
    }

    #[pymethod(name = "__repr__")]
    fn repr(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<String> {
        zelf.borrow_value().repr(vm)
    }

    #[pymethod(name = "__len__")]
    pub(crate) fn len(&self) -> usize {
        self.borrow_value().len()
    }

    fn array_eq(&self, other: &Self, vm: &VirtualMachine) -> PyResult<bool> {
        // we cannot use zelf.is(other) for shortcut because if we contenting a
        // float value NaN we always return False even they are the same object.
        if self.len() != other.len() {
            return Ok(false);
        }
        let array_a = self.borrow_value();
        let array_b = other.borrow_value();

        // fast path for same ArrayContentType type
        if let Ok(ord) = array_a.cmp(&*array_b) {
            return Ok(ord == Some(Ordering::Equal));
        }

        let iter = Iterator::zip(array_a.iter(vm), array_b.iter(vm));

        for (a, b) in iter {
            if !vm.bool_eq(&a, &b)? {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

impl Comparable for PyArray {
    fn cmp(
        zelf: &PyRef<Self>,
        other: &PyObjectRef,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        // TODO: deduplicate this logic with sequence::cmp in sequence.rs. Maybe make it generic?

        // we cannot use zelf.is(other) for shortcut because if we contenting a
        // float value NaN we always return False even they are the same object.
        let other = class_or_notimplemented!(Self, other);

        if let PyComparisonValue::Implemented(x) =
            op.eq_only(|| Ok(zelf.array_eq(&other, vm)?.into()))?
        {
            return Ok(x.into());
        }

        let array_a = zelf.borrow_value();
        let array_b = other.borrow_value();

        let res = match array_a.cmp(&*array_b) {
            // fast path for same ArrayContentType type
            Ok(partial_ord) => partial_ord.map_or(false, |ord| op.eval_ord(ord)),
            Err(()) => {
                let iter = Iterator::zip(array_a.iter(vm), array_b.iter(vm));

                for (a, b) in iter {
                    let ret = match op {
                        PyComparisonOp::Lt | PyComparisonOp::Le => vm.bool_seq_lt(&a, &b)?,
                        PyComparisonOp::Gt | PyComparisonOp::Ge => vm.bool_seq_gt(&a, &b)?,
                        _ => unreachable!(),
                    };
                    if let Some(v) = ret {
                        return Ok(PyComparisonValue::Implemented(v));
                    }
                }

                // fallback:
                op.eval_ord(array_a.len().cmp(&array_b.len()))
            }
        };

        Ok(res.into())
    }
}

impl BufferProtocol for PyArray {
    fn get_buffer(zelf: &PyRef<Self>, _vm: &VirtualMachine) -> PyResult<Box<dyn Buffer>> {
        zelf.exports.fetch_add(1);
        let array = zelf.borrow_value();
        let buf = ArrayBuffer {
            array: zelf.clone(),
            options: BufferOptions {
                readonly: false,
                len: array.len(),
                itemsize: array.itemsize(),
                format: array.typecode_str().into(),
                ..Default::default()
            },
        };
        Ok(Box::new(buf))
    }
}

#[derive(Debug)]
struct ArrayBuffer {
    array: PyArrayRef,
    options: BufferOptions,
}

impl Buffer for ArrayBuffer {
    fn obj_bytes(&self) -> BorrowedValue<[u8]> {
        self.array.get_bytes().into()
    }

    fn obj_bytes_mut(&self) -> BorrowedValueMut<[u8]> {
        self.array.get_bytes_mut().into()
    }

    fn release(&self) {
        self.array.exports.fetch_sub(1);
    }

    fn get_options(&self) -> &BufferOptions {
        &self.options
    }
}

impl Iterable for PyArray {
    fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        Ok(PyArrayIter {
            position: AtomicCell::new(0),
            array: zelf,
        }
        .into_object(vm))
    }
}

impl<'a> ResizeGuard<'a> for PyArray {
    type Resizable = PyRwLockWriteGuard<'a, ArrayContentType>;

    fn try_resizable(&'a self, vm: &VirtualMachine) -> PyResult<Self::Resizable> {
        let w = self.borrow_value_mut();
        if self.exports.load() == 0 {
            Ok(w)
        } else {
            Err(vm
                .new_buffer_error("Existing exports of data: object cannot be re-sized".to_owned()))
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
    fn class(_vm: &VirtualMachine) -> &PyTypeRef {
        Self::static_type()
    }
}

#[pyimpl(with(PyIter))]
impl PyArrayIter {}

impl PyIter for PyArrayIter {
    fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        let pos = zelf.position.fetch_add(1);
        if let Some(item) = zelf.array.borrow_value().getitem_by_idx(pos, vm) {
            Ok(item)
        } else {
            Err(vm.new_stop_iteration())
        }
    }
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    py_module!(vm, "array", {
        "array" => PyArray::make_class(&vm.ctx),
        "arrayiterator" => PyArrayIter::make_class(&vm.ctx),
    })
}

//! Implementation of the python bytearray object.
use bstr::ByteSlice;
use crossbeam_utils::atomic::AtomicCell;
use std::convert::TryFrom;
use std::mem::size_of;
use std::str::FromStr;
use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

use super::objbyteinner::{
    ByteInnerFindOptions, ByteInnerNewOptions, ByteInnerPaddingOptions, ByteInnerSplitOptions,
    ByteInnerTranslateOptions, ByteOr, PyByteInner,
};
use super::objint::PyIntRef;
use super::objiter;
use super::objsequence::SequenceIndex;
use super::objstr::{PyString, PyStringRef};
use super::objtype::PyClassRef;
use super::pystr::{self, PyCommonString};
use crate::cformat::CFormatString;
use crate::function::{OptionalArg, OptionalOption};
use crate::obj::objstr::do_cformat_string;
use crate::pyobject::{
    Either, PyClassImpl, PyComparisonValue, PyContext, PyIterable, PyObjectRef, PyRef, PyResult,
    PyValue, TryFromObject, TypeProtocol,
};
use crate::vm::VirtualMachine;

/// "bytearray(iterable_of_ints) -> bytearray\n\
///  bytearray(string, encoding[, errors]) -> bytearray\n\
///  bytearray(bytes_or_buffer) -> mutable copy of bytes_or_buffer\n\
///  bytearray(int) -> bytes array of size given by the parameter initialized with null bytes\n\
///  bytearray() -> empty bytes array\n\n\
///  Construct a mutable bytearray object from:\n  \
///  - an iterable yielding integers in range(256)\n  \
///  - a text string encoded using the specified encoding\n  \
///  - a bytes or a buffer object\n  \
///  - any object implementing the buffer API.\n  \
///  - an integer";
#[pyclass(name = "bytearray")]
#[derive(Debug)]
pub struct PyByteArray {
    inner: RwLock<PyByteInner>,
}

pub type PyByteArrayRef = PyRef<PyByteArray>;

impl PyByteArray {
    pub fn new(data: Vec<u8>) -> Self {
        PyByteArray {
            inner: RwLock::new(PyByteInner { elements: data }),
        }
    }

    fn from_inner(inner: PyByteInner) -> Self {
        PyByteArray {
            inner: RwLock::new(inner),
        }
    }

    pub fn borrow_value(&self) -> RwLockReadGuard<'_, PyByteInner> {
        self.inner.read().unwrap()
    }

    pub fn borrow_value_mut(&self) -> RwLockWriteGuard<'_, PyByteInner> {
        self.inner.write().unwrap()
    }
}

impl From<Vec<u8>> for PyByteArray {
    fn from(elements: Vec<u8>) -> Self {
        Self::new(elements)
    }
}

impl PyValue for PyByteArray {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.bytearray_type()
    }
}

/// Fill bytearray class methods dictionary.
pub(crate) fn init(context: &PyContext) {
    PyByteArray::extend_class(context, &context.types.bytearray_type);
    let bytearray_type = &context.types.bytearray_type;
    extend_class!(context, bytearray_type, {
        "maketrans" => context.new_method(PyByteInner::maketrans),
    });

    PyByteArrayIterator::extend_class(context, &context.types.bytearrayiterator_type);
}

#[pyimpl(flags(BASETYPE))]
impl PyByteArray {
    #[pyslot]
    fn tp_new(
        cls: PyClassRef,
        options: ByteInnerNewOptions,
        vm: &VirtualMachine,
    ) -> PyResult<PyByteArrayRef> {
        PyByteArray::from_inner(options.get_value(vm)?).into_ref_with_type(vm, cls)
    }

    #[pymethod(name = "__repr__")]
    fn repr(&self) -> PyResult<String> {
        Ok(format!("bytearray(b'{}')", self.borrow_value().repr()?))
    }

    #[pymethod(name = "__len__")]
    fn len(&self) -> usize {
        self.borrow_value().len()
    }

    #[pymethod(name = "__sizeof__")]
    fn sizeof(&self) -> usize {
        size_of::<Self>() + self.borrow_value().len() * size_of::<u8>()
    }

    #[pymethod(name = "__eq__")]
    fn eq(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyComparisonValue {
        self.borrow_value().eq(other, vm)
    }

    #[pymethod(name = "__ge__")]
    fn ge(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyComparisonValue {
        self.borrow_value().ge(other, vm)
    }

    #[pymethod(name = "__le__")]
    fn le(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyComparisonValue {
        self.borrow_value().le(other, vm)
    }

    #[pymethod(name = "__gt__")]
    fn gt(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyComparisonValue {
        self.borrow_value().gt(other, vm)
    }

    #[pymethod(name = "__lt__")]
    fn lt(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyComparisonValue {
        self.borrow_value().lt(other, vm)
    }

    #[pymethod(name = "__hash__")]
    fn hash(&self, vm: &VirtualMachine) -> PyResult<()> {
        Err(vm.new_type_error("unhashable type: bytearray".to_owned()))
    }

    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyRef<Self>) -> PyByteArrayIterator {
        PyByteArrayIterator {
            position: AtomicCell::new(0),
            bytearray: zelf,
        }
    }

    #[pymethod(name = "__add__")]
    fn add(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if let Ok(other) = PyByteInner::try_from_object(vm, other) {
            Ok(vm.ctx.new_bytearray(self.borrow_value().add(other)))
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    #[pymethod(name = "__contains__")]
    fn contains(
        &self,
        needle: Either<PyByteInner, PyIntRef>,
        vm: &VirtualMachine,
    ) -> PyResult<bool> {
        self.borrow_value().contains(needle, vm)
    }

    #[pymethod(name = "__getitem__")]
    fn getitem(&self, needle: SequenceIndex, vm: &VirtualMachine) -> PyResult {
        self.borrow_value().getitem(needle, vm)
    }

    #[pymethod(name = "__setitem__")]
    fn setitem(&self, needle: SequenceIndex, value: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.borrow_value_mut().setitem(needle, value, vm)
    }

    #[pymethod(name = "__delitem__")]
    fn delitem(&self, needle: SequenceIndex, vm: &VirtualMachine) -> PyResult<()> {
        self.borrow_value_mut().delitem(needle, vm)
    }

    #[pymethod(name = "isalnum")]
    fn isalnum(&self) -> bool {
        self.borrow_value().isalnum()
    }

    #[pymethod(name = "isalpha")]
    fn isalpha(&self) -> bool {
        self.borrow_value().isalpha()
    }

    #[pymethod(name = "isascii")]
    fn isascii(&self) -> bool {
        self.borrow_value().isascii()
    }

    #[pymethod(name = "isdigit")]
    fn isdigit(&self) -> bool {
        self.borrow_value().isdigit()
    }

    #[pymethod(name = "islower")]
    fn islower(&self) -> bool {
        self.borrow_value().islower()
    }

    #[pymethod(name = "isspace")]
    fn isspace(&self) -> bool {
        self.borrow_value().isspace()
    }

    #[pymethod(name = "isupper")]
    fn isupper(&self) -> bool {
        self.borrow_value().isupper()
    }

    #[pymethod(name = "istitle")]
    fn istitle(&self) -> bool {
        self.borrow_value().istitle()
    }

    #[pymethod(name = "lower")]
    fn lower(&self) -> PyByteArray {
        self.borrow_value().lower().into()
    }

    #[pymethod(name = "upper")]
    fn upper(&self) -> PyByteArray {
        self.borrow_value().upper().into()
    }

    #[pymethod(name = "capitalize")]
    fn capitalize(&self) -> PyByteArray {
        self.borrow_value().capitalize().into()
    }

    #[pymethod(name = "swapcase")]
    fn swapcase(&self) -> PyByteArray {
        self.borrow_value().swapcase().into()
    }

    #[pymethod(name = "hex")]
    fn hex(&self) -> String {
        self.borrow_value().hex()
    }

    #[pymethod]
    fn fromhex(string: PyStringRef, vm: &VirtualMachine) -> PyResult<PyByteArray> {
        Ok(PyByteInner::fromhex(string.as_str(), vm)?.into())
    }

    #[pymethod(name = "center")]
    fn center(
        &self,
        options: ByteInnerPaddingOptions,
        vm: &VirtualMachine,
    ) -> PyResult<PyByteArray> {
        Ok(self.borrow_value().center(options, vm)?.into())
    }

    #[pymethod(name = "ljust")]
    fn ljust(
        &self,
        options: ByteInnerPaddingOptions,
        vm: &VirtualMachine,
    ) -> PyResult<PyByteArray> {
        Ok(self.borrow_value().ljust(options, vm)?.into())
    }

    #[pymethod(name = "rjust")]
    fn rjust(
        &self,
        options: ByteInnerPaddingOptions,
        vm: &VirtualMachine,
    ) -> PyResult<PyByteArray> {
        Ok(self.borrow_value().rjust(options, vm)?.into())
    }

    #[pymethod(name = "count")]
    fn count(&self, options: ByteInnerFindOptions, vm: &VirtualMachine) -> PyResult<usize> {
        self.borrow_value().count(options, vm)
    }

    #[pymethod(name = "join")]
    fn join(&self, iter: PyIterable<PyByteInner>, vm: &VirtualMachine) -> PyResult<PyByteArray> {
        Ok(self.borrow_value().join(iter, vm)?.into())
    }

    #[pymethod(name = "endswith")]
    fn endswith(&self, options: pystr::StartsEndsWithArgs, vm: &VirtualMachine) -> PyResult<bool> {
        self.borrow_value().elements[..].py_startsendswith(
            options,
            "endswith",
            "bytes",
            |s, x: &PyByteInner| s.ends_with(&x.elements[..]),
            vm,
        )
    }

    #[pymethod(name = "startswith")]
    fn startswith(
        &self,
        options: pystr::StartsEndsWithArgs,
        vm: &VirtualMachine,
    ) -> PyResult<bool> {
        self.borrow_value().elements[..].py_startsendswith(
            options,
            "startswith",
            "bytes",
            |s, x: &PyByteInner| s.starts_with(&x.elements[..]),
            vm,
        )
    }

    #[pymethod(name = "find")]
    fn find(&self, options: ByteInnerFindOptions, vm: &VirtualMachine) -> PyResult<isize> {
        let index = self.borrow_value().find(options, |h, n| h.find(n), vm)?;
        Ok(index.map_or(-1, |v| v as isize))
    }

    #[pymethod(name = "index")]
    fn index(&self, options: ByteInnerFindOptions, vm: &VirtualMachine) -> PyResult<usize> {
        let index = self.borrow_value().find(options, |h, n| h.find(n), vm)?;
        index.ok_or_else(|| vm.new_value_error("substring not found".to_owned()))
    }

    #[pymethod(name = "rfind")]
    fn rfind(&self, options: ByteInnerFindOptions, vm: &VirtualMachine) -> PyResult<isize> {
        let index = self.borrow_value().find(options, |h, n| h.rfind(n), vm)?;
        Ok(index.map_or(-1, |v| v as isize))
    }

    #[pymethod(name = "rindex")]
    fn rindex(&self, options: ByteInnerFindOptions, vm: &VirtualMachine) -> PyResult<usize> {
        let index = self.borrow_value().find(options, |h, n| h.rfind(n), vm)?;
        index.ok_or_else(|| vm.new_value_error("substring not found".to_owned()))
    }

    #[pymethod(name = "remove")]
    fn remove(&self, x: PyIntRef, vm: &VirtualMachine) -> PyResult<()> {
        let x = x.as_bigint().byte_or(vm)?;

        let bytes = &mut self.borrow_value_mut().elements;
        let pos = bytes
            .iter()
            .position(|b| *b == x)
            .ok_or_else(|| vm.new_value_error("value not found in bytearray".to_owned()))?;

        bytes.remove(pos);

        Ok(())
    }

    #[pymethod(name = "translate")]
    fn translate(
        &self,
        options: ByteInnerTranslateOptions,
        vm: &VirtualMachine,
    ) -> PyResult<PyByteArray> {
        Ok(self.borrow_value().translate(options, vm)?.into())
    }

    #[pymethod(name = "strip")]
    fn strip(&self, chars: OptionalOption<PyByteInner>) -> PyByteArray {
        self.borrow_value().strip(chars).into()
    }

    #[pymethod(name = "lstrip")]
    fn lstrip(&self, chars: OptionalOption<PyByteInner>) -> PyByteArray {
        self.borrow_value().lstrip(chars).into()
    }

    #[pymethod(name = "rstrip")]
    fn rstrip(&self, chars: OptionalOption<PyByteInner>) -> PyByteArray {
        self.borrow_value().rstrip(chars).into()
    }

    /// removeprefix($self, prefix, /)
    ///
    ///
    /// Return a bytearray object with the given prefix string removed if present.
    ///
    /// If the bytearray starts with the prefix string, return string[len(prefix):]
    /// Otherwise, return a copy of the original bytearray.
    #[pymethod(name = "removeprefix")]
    fn removeprefix(&self, prefix: PyByteInner) -> PyByteArray {
        self.borrow_value().removeprefix(prefix).into()
    }

    /// removesuffix(self, prefix, /)
    ///
    ///
    /// Return a bytearray object with the given suffix string removed if present.
    ///
    /// If the bytearray ends with the suffix string, return string[:len(suffix)]
    /// Otherwise, return a copy of the original bytearray.
    #[pymethod(name = "removesuffix")]
    fn removesuffix(&self, suffix: PyByteInner) -> PyByteArray {
        self.borrow_value().removesuffix(suffix).to_vec().into()
    }

    #[pymethod(name = "split")]
    fn split(&self, options: ByteInnerSplitOptions, vm: &VirtualMachine) -> PyResult {
        self.borrow_value()
            .split(options, |s, vm| vm.ctx.new_bytearray(s.to_vec()), vm)
    }

    #[pymethod(name = "rsplit")]
    fn rsplit(&self, options: ByteInnerSplitOptions, vm: &VirtualMachine) -> PyResult {
        self.borrow_value()
            .rsplit(options, |s, vm| vm.ctx.new_bytearray(s.to_vec()), vm)
    }

    #[pymethod(name = "partition")]
    fn partition(&self, sep: PyByteInner, vm: &VirtualMachine) -> PyResult {
        // sep ALWAYS converted to  bytearray even it's bytes or memoryview
        // so its ok to accept PyByteInner
        let value = self.borrow_value();
        let (front, has_mid, back) = value.partition(&sep, vm)?;
        Ok(vm.ctx.new_tuple(vec![
            vm.ctx.new_bytearray(front.to_vec()),
            vm.ctx
                .new_bytearray(if has_mid { sep.elements } else { Vec::new() }),
            vm.ctx.new_bytearray(back.to_vec()),
        ]))
    }

    #[pymethod(name = "rpartition")]
    fn rpartition(&self, sep: PyByteInner, vm: &VirtualMachine) -> PyResult {
        let value = self.borrow_value();
        let (front, has_mid, back) = value.rpartition(&sep, vm)?;
        Ok(vm.ctx.new_tuple(vec![
            vm.ctx.new_bytearray(front.to_vec()),
            vm.ctx
                .new_bytearray(if has_mid { sep.elements } else { Vec::new() }),
            vm.ctx.new_bytearray(back.to_vec()),
        ]))
    }

    #[pymethod(name = "expandtabs")]
    fn expandtabs(&self, options: pystr::ExpandTabsArgs) -> PyByteArray {
        self.borrow_value().expandtabs(options).into()
    }

    #[pymethod(name = "splitlines")]
    fn splitlines(&self, options: pystr::SplitLinesArgs, vm: &VirtualMachine) -> PyResult {
        let as_bytes = self
            .borrow_value()
            .splitlines(options)
            .iter()
            .map(|x| vm.ctx.new_bytearray(x.to_vec()))
            .collect::<Vec<PyObjectRef>>();
        Ok(vm.ctx.new_list(as_bytes))
    }

    #[pymethod(name = "zfill")]
    fn zfill(&self, width: isize) -> PyByteArray {
        self.borrow_value().zfill(width).into()
    }

    #[pymethod(name = "replace")]
    fn replace(
        &self,
        old: PyByteInner,
        new: PyByteInner,
        count: OptionalArg<isize>,
        vm: &VirtualMachine,
    ) -> PyResult<PyByteArray> {
        Ok(self.borrow_value().replace(old, new, count, vm)?.into())
    }

    #[pymethod(name = "clear")]
    fn clear(&self) {
        self.borrow_value_mut().elements.clear();
    }

    #[pymethod(name = "copy")]
    fn copy(&self) -> PyByteArray {
        self.borrow_value().elements.clone().into()
    }

    #[pymethod(name = "append")]
    fn append(&self, x: PyIntRef, vm: &VirtualMachine) -> PyResult<()> {
        self.borrow_value_mut()
            .elements
            .push(x.as_bigint().byte_or(vm)?);
        Ok(())
    }

    #[pymethod(name = "extend")]
    fn extend(&self, iterable_of_ints: PyIterable, vm: &VirtualMachine) -> PyResult<()> {
        for x in iterable_of_ints.iter(vm)? {
            let x = x?;
            let x = PyIntRef::try_from_object(vm, x)?;
            let x = x.as_bigint().byte_or(vm)?;
            self.borrow_value_mut().elements.push(x);
        }

        Ok(())
    }

    #[pymethod(name = "insert")]
    fn insert(&self, mut index: isize, x: PyIntRef, vm: &VirtualMachine) -> PyResult<()> {
        let bytes = &mut self.borrow_value_mut().elements;
        let len = isize::try_from(bytes.len())
            .map_err(|_e| vm.new_overflow_error("bytearray too big".to_owned()))?;

        let x = x.as_bigint().byte_or(vm)?;

        if index >= len {
            bytes.push(x);
            return Ok(());
        }

        if index < 0 {
            index += len;
            index = index.max(0);
        }

        let index = usize::try_from(index)
            .map_err(|_e| vm.new_overflow_error("overflow in index calculation".to_owned()))?;

        bytes.insert(index, x);

        Ok(())
    }

    #[pymethod(name = "pop")]
    fn pop(&self, vm: &VirtualMachine) -> PyResult<u8> {
        self.borrow_value_mut()
            .elements
            .pop()
            .ok_or_else(|| vm.new_index_error("pop from empty bytearray".to_owned()))
    }

    #[pymethod(name = "title")]
    fn title(&self) -> PyByteArray {
        self.borrow_value().title().into()
    }

    #[pymethod(name = "__mul__")]
    fn repeat(&self, n: isize) -> PyByteArray {
        self.borrow_value().repeat(n).into()
    }

    #[pymethod(name = "__rmul__")]
    fn rmul(&self, n: isize) -> PyByteArray {
        self.repeat(n)
    }

    #[pymethod(name = "__imul__")]
    fn irepeat(&self, n: isize) {
        self.borrow_value_mut().irepeat(n)
    }

    fn do_cformat(
        &self,
        vm: &VirtualMachine,
        format_string: CFormatString,
        values_obj: PyObjectRef,
    ) -> PyResult<PyByteArray> {
        let final_string = do_cformat_string(vm, format_string, values_obj)?;
        Ok(final_string.as_str().as_bytes().to_owned().into())
    }

    #[pymethod(name = "__mod__")]
    fn modulo(&self, values: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyByteArray> {
        let format_string =
            CFormatString::from_str(std::str::from_utf8(&self.borrow_value().elements).unwrap())
                .map_err(|err| vm.new_value_error(err.to_string()))?;
        self.do_cformat(vm, format_string, values.clone())
    }

    #[pymethod(name = "__rmod__")]
    fn rmod(&self, _values: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.not_implemented()
    }

    #[pymethod(name = "reverse")]
    fn reverse(&self) -> PyResult<()> {
        self.borrow_value_mut().elements.reverse();
        Ok(())
    }

    #[pymethod]
    fn decode(
        zelf: PyRef<Self>,
        encoding: OptionalArg<PyStringRef>,
        errors: OptionalArg<PyStringRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyStringRef> {
        let encoding = encoding.into_option();
        vm.decode(zelf.into_object(), encoding.clone(), errors.into_option())?
            .downcast::<PyString>()
            .map_err(|obj| {
                vm.new_type_error(format!(
                    "'{}' decoder returned '{}' instead of 'str'; use codecs.encode() to \
                     encode arbitrary types",
                    encoding.as_ref().map_or("utf-8", |s| s.as_str()),
                    obj.class().name,
                ))
            })
    }
}

// fn set_value(obj: &PyObjectRef, value: Vec<u8>) {
//     obj.borrow_mut().kind = PyObjectPayload::Bytes { value };
// }

#[pyclass]
#[derive(Debug)]
pub struct PyByteArrayIterator {
    position: AtomicCell<usize>,
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
        let pos = self.position.fetch_add(1);
        if let Some(&ret) = self.bytearray.borrow_value().elements.get(pos) {
            Ok(ret)
        } else {
            Err(objiter::new_stop_iteration(vm))
        }
    }

    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyRef<Self>) -> PyRef<Self> {
        zelf
    }
}

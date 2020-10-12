//! Implementation of the python bytearray object.
use super::bytes::{PyBytes, PyBytesRef};
use super::int::PyIntRef;
use super::iter;
use super::memory::{Buffer, BufferOptions};
use super::pystr::PyStrRef;
use super::pytype::PyTypeRef;
use super::tuple::PyTupleRef;
use crate::anystr::{self, AnyStr};
use crate::bytesinner::{
    bytes_decode, ByteInnerFindOptions, ByteInnerNewOptions, ByteInnerPaddingOptions,
    ByteInnerSplitOptions, ByteInnerTranslateOptions, DecodeArgs, PyBytesInner,
};
use crate::byteslike::PyBytesLike;
use crate::common::borrow::{BorrowedValue, BorrowedValueMut};
use crate::common::lock::{
    PyRwLock, PyRwLockReadGuard, PyRwLockUpgradableReadGuard, PyRwLockWriteGuard,
};
use crate::function::{OptionalArg, OptionalOption};
use crate::pyobject::{
    BorrowValue, Either, IdProtocol, IntoPyObject, PyClassImpl, PyComparisonValue, PyContext,
    PyIterable, PyObjectRef, PyRef, PyResult, PyValue,
};
use crate::sliceable::SequenceIndex;
use crate::slots::{BufferProtocol, Comparable, Hashable, PyComparisonOp, Unhashable};
use crate::vm::VirtualMachine;
use bstr::ByteSlice;
use crossbeam_utils::atomic::AtomicCell;
use std::mem::size_of;

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
#[pyclass(module = false, name = "bytearray")]
#[derive(Debug)]
pub struct PyByteArray {
    inner: PyRwLock<PyBytesInner>,
    exports: AtomicCell<usize>,
    buffer_options: PyRwLock<Option<Box<BufferOptions>>>,
}

pub type PyByteArrayRef = PyRef<PyByteArray>;

impl<'a> BorrowValue<'a> for PyByteArray {
    type Borrowed = PyRwLockReadGuard<'a, PyBytesInner>;

    fn borrow_value(&'a self) -> Self::Borrowed {
        self.inner.read()
    }
}

impl PyByteArray {
    fn from_inner(inner: PyBytesInner) -> Self {
        PyByteArray {
            inner: PyRwLock::new(inner),
            exports: AtomicCell::new(0),
            buffer_options: PyRwLock::new(None),
        }
    }

    pub fn borrow_value_mut(&self) -> PyRwLockWriteGuard<'_, PyBytesInner> {
        self.inner.write()
    }
}

impl From<PyBytesInner> for PyByteArray {
    fn from(inner: PyBytesInner) -> Self {
        Self::from_inner(inner)
    }
}

impl From<Vec<u8>> for PyByteArray {
    fn from(elements: Vec<u8>) -> Self {
        Self::from(PyBytesInner { elements })
    }
}

impl PyValue for PyByteArray {
    fn class(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.types.bytearray_type.clone()
    }
}

/// Fill bytearray class methods dictionary.
pub(crate) fn init(context: &PyContext) {
    PyByteArray::extend_class(context, &context.types.bytearray_type);
    let bytearray_type = &context.types.bytearray_type;
    extend_class!(context, bytearray_type, {
        "maketrans" => context.new_method(PyBytesInner::maketrans),
    });

    PyByteArrayIterator::extend_class(context, &context.types.bytearray_iterator_type);
}

#[pyimpl(flags(BASETYPE), with(Hashable, Comparable, BufferProtocol))]
impl PyByteArray {
    #[pyslot]
    fn tp_new(
        cls: PyTypeRef,
        options: ByteInnerNewOptions,
        vm: &VirtualMachine,
    ) -> PyResult<PyByteArrayRef> {
        PyByteArray::from_inner(options.get_value(vm)?).into_ref_with_type(vm, cls)
    }

    #[pymethod(name = "__repr__")]
    fn repr(&self) -> String {
        format!("bytearray(b'{}')", self.borrow_value().repr())
    }

    #[pymethod(name = "__len__")]
    fn len(&self) -> usize {
        self.borrow_value().len()
    }

    #[pymethod(name = "__sizeof__")]
    fn sizeof(&self) -> usize {
        size_of::<Self>() + self.borrow_value().len() * size_of::<u8>()
    }

    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyRef<Self>) -> PyByteArrayIterator {
        PyByteArrayIterator {
            position: AtomicCell::new(0),
            bytearray: zelf,
        }
    }

    #[pymethod(name = "__add__")]
    fn add(&self, other: PyBytesLike, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx
            .new_bytearray(self.borrow_value().add(&*other.borrow_value()))
    }

    #[pymethod(name = "__iadd__")]
    fn iadd(zelf: PyRef<Self>, other: PyBytesLike, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        zelf.try_resizable(vm)?;
        zelf.borrow_value_mut().iadd(&*other.borrow_value());
        Ok(zelf)
    }

    #[pymethod(name = "__contains__")]
    fn contains(
        &self,
        needle: Either<PyBytesInner, PyIntRef>,
        vm: &VirtualMachine,
    ) -> PyResult<bool> {
        self.borrow_value().contains(needle, vm)
    }

    #[pymethod(name = "__getitem__")]
    fn getitem(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.borrow_value().getitem("bytearray", needle, vm)
    }

    #[pymethod(name = "__setitem__")]
    fn setitem(
        zelf: PyRef<Self>,
        needle: SequenceIndex,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        match needle {
            SequenceIndex::Int(int) => zelf.borrow_value_mut().setindex(int, value, vm),
            SequenceIndex::Slice(slice) => match (zelf.is(&value), zelf.is_resizable()) {
                (true, true) => zelf.borrow_value_mut().setslice_from_self(slice, vm),
                (true, false) => zelf
                    .borrow_value_mut()
                    .setslice_from_self_no_resize(slice, vm),
                (false, true) => zelf.borrow_value_mut().setslice(slice, value, vm),
                (false, false) => zelf.borrow_value_mut().setslice_no_resize(slice, value, vm),
            },
        }
    }

    #[pymethod(name = "__delitem__")]
    fn delitem(zelf: PyRef<Self>, needle: SequenceIndex, vm: &VirtualMachine) -> PyResult<()> {
        zelf.try_resizable(vm)?;
        zelf.borrow_value_mut().delitem(needle, vm)
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
    fn lower(&self) -> Self {
        self.borrow_value().lower().into()
    }

    #[pymethod(name = "upper")]
    fn upper(&self) -> Self {
        self.borrow_value().upper().into()
    }

    #[pymethod(name = "capitalize")]
    fn capitalize(&self) -> Self {
        self.borrow_value().capitalize().into()
    }

    #[pymethod(name = "swapcase")]
    fn swapcase(&self) -> Self {
        self.borrow_value().swapcase().into()
    }

    #[pymethod(name = "hex")]
    fn hex(
        &self,
        sep: OptionalArg<Either<PyStrRef, PyBytesRef>>,
        bytes_per_sep: OptionalArg<isize>,
        vm: &VirtualMachine,
    ) -> PyResult<String> {
        self.borrow_value().hex(sep, bytes_per_sep, vm)
    }

    #[pymethod]
    fn fromhex(string: PyStrRef, vm: &VirtualMachine) -> PyResult<PyByteArray> {
        Ok(PyBytesInner::fromhex(string.borrow_value(), vm)?.into())
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
    fn join(&self, iter: PyIterable<PyBytesInner>, vm: &VirtualMachine) -> PyResult<PyByteArray> {
        Ok(self.borrow_value().join(iter, vm)?.into())
    }

    #[pymethod(name = "endswith")]
    fn endswith(&self, options: anystr::StartsEndsWithArgs, vm: &VirtualMachine) -> PyResult<bool> {
        self.borrow_value().elements[..].py_startsendswith(
            options,
            "endswith",
            "bytes",
            |s, x: &PyBytesInner| s.ends_with(&x.elements[..]),
            vm,
        )
    }

    #[pymethod(name = "startswith")]
    fn startswith(
        &self,
        options: anystr::StartsEndsWithArgs,
        vm: &VirtualMachine,
    ) -> PyResult<bool> {
        self.borrow_value().elements[..].py_startsendswith(
            options,
            "startswith",
            "bytes",
            |s, x: &PyBytesInner| s.starts_with(&x.elements[..]),
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
    fn remove(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.borrow_value_mut().remove(value, vm)
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
    fn strip(&self, chars: OptionalOption<PyBytesInner>) -> Self {
        self.borrow_value().strip(chars).into()
    }

    #[pymethod(name = "lstrip")]
    fn lstrip(&self, chars: OptionalOption<PyBytesInner>) -> Self {
        self.borrow_value().lstrip(chars).into()
    }

    #[pymethod(name = "rstrip")]
    fn rstrip(&self, chars: OptionalOption<PyBytesInner>) -> Self {
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
    fn removeprefix(&self, prefix: PyBytesInner) -> Self {
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
    fn removesuffix(&self, suffix: PyBytesInner) -> Self {
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
    fn partition(&self, sep: PyBytesInner, vm: &VirtualMachine) -> PyResult {
        // sep ALWAYS converted to  bytearray even it's bytes or memoryview
        // so its ok to accept PyBytesInner
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
    fn rpartition(&self, sep: PyBytesInner, vm: &VirtualMachine) -> PyResult {
        let value = self.borrow_value();
        let (back, has_mid, front) = value.rpartition(&sep, vm)?;
        Ok(vm.ctx.new_tuple(vec![
            vm.ctx.new_bytearray(front.to_vec()),
            vm.ctx
                .new_bytearray(if has_mid { sep.elements } else { Vec::new() }),
            vm.ctx.new_bytearray(back.to_vec()),
        ]))
    }

    #[pymethod(name = "expandtabs")]
    fn expandtabs(&self, options: anystr::ExpandTabsArgs) -> Self {
        self.borrow_value().expandtabs(options).into()
    }

    #[pymethod(name = "splitlines")]
    fn splitlines(&self, options: anystr::SplitLinesArgs, vm: &VirtualMachine) -> PyResult {
        let lines = self
            .borrow_value()
            .splitlines(options, |x| vm.ctx.new_bytearray(x.to_vec()));
        Ok(vm.ctx.new_list(lines))
    }

    #[pymethod(name = "zfill")]
    fn zfill(&self, width: isize) -> Self {
        self.borrow_value().zfill(width).into()
    }

    #[pymethod(name = "replace")]
    fn replace(
        &self,
        old: PyBytesInner,
        new: PyBytesInner,
        count: OptionalArg<isize>,
        vm: &VirtualMachine,
    ) -> PyResult<PyByteArray> {
        Ok(self.borrow_value().replace(old, new, count, vm)?.into())
    }

    #[pymethod(name = "clear")]
    fn clear(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<()> {
        zelf.try_resizable(vm)?;
        zelf.borrow_value_mut().elements.clear();
        Ok(())
    }

    #[pymethod(name = "copy")]
    fn copy(&self) -> Self {
        self.borrow_value().elements.clone().into()
    }

    #[pymethod(name = "append")]
    fn append(zelf: PyRef<Self>, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        zelf.try_resizable(vm)?;
        zelf.borrow_value_mut().append(value, vm)
    }

    #[pymethod(name = "extend")]
    fn extend(zelf: PyRef<Self>, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        zelf.try_resizable(vm)?;
        if zelf.is(&value) {
            zelf.borrow_value_mut().irepeat(2);
            Ok(())
        } else {
            zelf.borrow_value_mut().extend(value, vm)
        }
    }

    #[pymethod(name = "insert")]
    fn insert(
        zelf: PyRef<Self>,
        index: isize,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        zelf.try_resizable(vm)?;
        zelf.borrow_value_mut().insert(index, value, vm)
    }

    #[pymethod(name = "pop")]
    fn pop(zelf: PyRef<Self>, index: OptionalArg<isize>, vm: &VirtualMachine) -> PyResult<u8> {
        zelf.try_resizable(vm)?;
        let index = index.unwrap_or(-1);
        zelf.borrow_value_mut().pop(index, vm)
    }

    #[pymethod(name = "title")]
    fn title(&self) -> Self {
        self.borrow_value().title().into()
    }

    #[pymethod(name = "__mul__")]
    #[pymethod(name = "__rmul__")]
    fn mul(&self, n: isize) -> Self {
        self.borrow_value().repeat(n).into()
    }

    #[pymethod(name = "__imul__")]
    fn imul(zelf: PyRef<Self>, n: isize, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        zelf.try_resizable(vm)?;
        zelf.borrow_value_mut().irepeat(n);
        Ok(zelf)
    }

    #[pymethod(name = "__mod__")]
    fn modulo(&self, values: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyByteArray> {
        let formatted = self.borrow_value().cformat(values, vm)?;
        Ok(formatted.into_bytes().into())
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
    fn decode(zelf: PyRef<Self>, args: DecodeArgs, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        bytes_decode(zelf.into_object(), args, vm)
    }

    #[pymethod(magic)]
    fn reduce_ex(zelf: PyRef<Self>, _proto: usize, vm: &VirtualMachine) -> (PyTypeRef, PyTupleRef) {
        Self::reduce(zelf, vm)
    }

    #[pymethod(magic)]
    fn reduce(zelf: PyRef<Self>, vm: &VirtualMachine) -> (PyTypeRef, PyTupleRef) {
        let bytes = PyBytes::from(zelf.borrow_value().elements.clone()).into_pyobject(vm);
        (
            Self::class(vm),
            PyTupleRef::with_elements(vec![bytes], &vm.ctx),
        )
    }
}

impl Comparable for PyByteArray {
    fn cmp(
        zelf: &PyRef<Self>,
        other: &PyObjectRef,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        if let Some(res) = op.identical_optimization(&zelf, &other) {
            return Ok(res.into());
        }
        Ok(zelf.borrow_value().cmp(other, op, vm))
    }
}

impl BufferProtocol for PyByteArray {
    fn get_buffer(zelf: &PyRef<Self>, _vm: &VirtualMachine) -> PyResult<Box<dyn Buffer>> {
        zelf.exports.fetch_add(1);
        Ok(Box::new(zelf.clone()))
    }
}

impl Buffer for PyByteArrayRef {
    fn obj_bytes(&self) -> BorrowedValue<[u8]> {
        PyRwLockReadGuard::map(self.borrow_value(), |x| x.elements.as_slice()).into()
    }

    fn obj_bytes_mut(&self) -> BorrowedValueMut<[u8]> {
        PyRwLockWriteGuard::map(self.borrow_value_mut(), |x| x.elements.as_mut_slice()).into()
    }

    fn release(&self) {
        let mut w = self.buffer_options.write();
        if self.exports.fetch_sub(1) == 1 {
            *w = None;
        }
    }

    fn is_resizable(&self) -> bool {
        self.exports.load() == 0
    }

    fn get_options(&self) -> BorrowedValue<BufferOptions> {
        let guard = self.buffer_options.upgradable_read();
        let guard = if guard.is_none() {
            let mut w = PyRwLockUpgradableReadGuard::upgrade(guard);
            *w = Some(Box::new(BufferOptions {
                readonly: false,
                len: self.len(),
                ..Default::default()
            }));
            PyRwLockWriteGuard::downgrade(w)
        } else {
            PyRwLockUpgradableReadGuard::downgrade(guard)
        };
        PyRwLockReadGuard::map(guard, |x| x.as_ref().unwrap().as_ref()).into()
    }
}

impl Unhashable for PyByteArray {}

// fn set_value(obj: &PyObjectRef, value: Vec<u8>) {
//     obj.borrow_mut().kind = PyObjectPayload::Bytes { value };
// }

#[pyclass(module = false, name = "bytearray_iterator")]
#[derive(Debug)]
pub struct PyByteArrayIterator {
    position: AtomicCell<usize>,
    bytearray: PyByteArrayRef,
}

impl PyValue for PyByteArrayIterator {
    fn class(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.types.bytearray_iterator_type.clone()
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
            Err(iter::new_stop_iteration(vm))
        }
    }

    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyRef<Self>) -> PyRef<Self> {
        zelf
    }
}

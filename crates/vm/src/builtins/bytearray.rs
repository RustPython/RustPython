//! Implementation of the python bytearray object.
use super::{
    PositionIterInternal, PyBytes, PyDictRef, PyGenericAlias, PyStrRef, PyTuple, PyTupleRef,
    PyType, PyTypeRef, iter::builtins_iter,
};
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject,
    VirtualMachine,
    anystr::{self, AnyStr},
    atomic_func,
    byte::{bytes_from_object, value_from_object},
    bytes_inner::{
        ByteInnerFindOptions, ByteInnerHexOptions, ByteInnerNewOptions, ByteInnerPaddingOptions,
        ByteInnerSplitOptions, ByteInnerSub, ByteInnerTranslateOptions, DecodeArgs, PyBytesInner,
        bytes_decode,
    },
    class::PyClassImpl,
    common::{
        atomic::{AtomicUsize, Ordering},
        lock::{
            PyMappedRwLockReadGuard, PyMappedRwLockWriteGuard, PyMutex, PyRwLock,
            PyRwLockReadGuard, PyRwLockWriteGuard,
        },
    },
    convert::{ToPyObject, ToPyResult},
    function::{
        ArgBytesLike, ArgIterable, ArgSize, FuncArgs, OptionalArg, OptionalOption,
        PyComparisonValue, check_meth_o, check_no_kwargs, check_noargs, check_positional,
    },
    protocol::{
        BufferDescriptor, BufferFlags, BufferMethods, BufferResizeGuard, PyBuffer, PyIterReturn,
        PyMappingMethods, PyNumberMethods, PySequenceMethods,
    },
    sliceable::{SequenceIndex, SliceableSequenceMutOp, SliceableSequenceOp},
    types::{
        AsBuffer, AsMapping, AsNumber, AsSequence, Callable, Comparable, Constructor,
        DefaultConstructor, Initializer, IterNext, Iterable, PyComparisonOp, Representable,
        SelfIter,
    },
};
use bstr::ByteSlice;
use core::mem::size_of;

#[pyclass(module = false, name = "bytearray", unhashable = true)]
#[derive(Debug, Default)]
pub struct PyByteArray {
    inner: PyRwLock<PyBytesInner>,
    exports: AtomicUsize,
}

pub(crate) type PyByteArrayRef = PyRef<PyByteArray>;

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

impl PyPayload for PyByteArray {
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.bytearray_type
    }
}

/// Fill bytearray class methods dictionary.
pub(crate) fn init(context: &'static Context) {
    PyByteArray::extend_class(context, context.types.bytearray_type);
    PyByteArrayIterator::extend_class(context, context.types.bytearray_iterator_type);
}

impl PyByteArray {
    #[deprecated(note = "use PyByteArray::from(...).into_ref() instead")]
    pub fn new_ref(data: Vec<u8>, ctx: &Context) -> PyRef<Self> {
        Self::from(data).into_ref(ctx)
    }

    const fn from_inner(inner: PyBytesInner) -> Self {
        Self {
            inner: PyRwLock::new(inner),
            exports: AtomicUsize::new(0),
        }
    }

    pub fn borrow_buf(&self) -> PyMappedRwLockReadGuard<'_, [u8]> {
        PyRwLockReadGuard::map(self.inner.read(), |inner| &*inner.elements)
    }

    pub fn borrow_buf_mut(&self) -> PyMappedRwLockWriteGuard<'_, Vec<u8>> {
        PyRwLockWriteGuard::map(self.inner.write(), |inner| &mut inner.elements)
    }

    fn repeat(&self, value: isize, vm: &VirtualMachine) -> PyResult<Self> {
        self.inner().mul(value, vm).map(|x| x.into())
    }

    fn _setitem_by_index(&self, i: isize, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let value = value_from_object(vm, &value)?;
        self.borrow_buf_mut().setitem_by_index(vm, i, value)
    }

    fn _setitem(
        zelf: &Py<Self>,
        needle: &PyObject,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        match SequenceIndex::try_from_borrowed_object(vm, needle, "bytearray")? {
            SequenceIndex::Int(i) => zelf._setitem_by_index(i, value, vm),
            SequenceIndex::Slice(slice) => {
                let items = if zelf.is(&value) {
                    zelf.borrow_buf().to_vec()
                } else {
                    bytes_from_object(vm, &value)?
                };
                if let Some(mut w) = zelf.try_resizable_opt() {
                    w.elements.setitem_by_slice(vm, slice, &items)
                } else {
                    zelf.borrow_buf_mut()
                        .setitem_by_slice_no_resize(vm, slice, &items)
                }
            }
        }
    }

    fn _getitem(&self, needle: &PyObject, vm: &VirtualMachine) -> PyResult {
        match SequenceIndex::try_from_borrowed_object(vm, needle, "bytearray")? {
            SequenceIndex::Int(i) => self
                .borrow_buf()
                .getitem_by_index(vm, i)
                .map(|x| vm.ctx.new_int(x).into()),
            SequenceIndex::Slice(slice) => self
                .borrow_buf()
                .getitem_by_slice(vm, slice)
                .map(|x| vm.ctx.new_bytearray(x).into()),
        }
    }

    pub fn _delitem(&self, needle: &PyObject, vm: &VirtualMachine) -> PyResult<()> {
        match SequenceIndex::try_from_borrowed_object(vm, needle, "bytearray")? {
            SequenceIndex::Int(i) => self.try_resizable(vm)?.elements.delitem_by_index(vm, i),
            SequenceIndex::Slice(slice) => {
                // TODO: delete 0 elements don't need resizable
                self.try_resizable(vm)?.elements.delitem_by_slice(vm, slice)
            }
        }
    }

    fn irepeat(zelf: &Py<Self>, n: isize, vm: &VirtualMachine) -> PyResult<()> {
        if n == 1 {
            return Ok(());
        }
        let mut w = match zelf.try_resizable(vm) {
            Ok(w) => w,
            Err(err) => {
                return if zelf.borrow_buf().is_empty() {
                    // We can multiple an empty vector by any integer
                    Ok(())
                } else {
                    Err(err)
                };
            }
        };

        w.imul(n, vm)
    }
}

#[pyclass(
    flags(BASETYPE, _MATCH_SELF),
    with(
        Py,
        PyRef,
        Constructor,
        Initializer,
        Comparable,
        AsBuffer,
        AsMapping,
        AsSequence,
        AsNumber,
        Iterable,
        Representable
    )
)]
impl PyByteArray {
    #[cfg(debug_assertions)]
    #[pygetset]
    fn exports(&self) -> usize {
        self.exports.load(Ordering::Relaxed)
    }

    #[inline]
    fn inner(&self) -> PyRwLockReadGuard<'_, PyBytesInner> {
        self.inner.read()
    }
    #[inline]
    fn inner_mut(&self) -> PyRwLockWriteGuard<'_, PyBytesInner> {
        self.inner.write()
    }

    #[pymethod]
    fn __alloc__(&self) -> usize {
        self.inner().capacity()
    }

    fn __len__(&self) -> usize {
        self.borrow_buf().len()
    }

    #[pymethod]
    fn __sizeof__(&self) -> usize {
        size_of::<Self>() + self.borrow_buf().len() * size_of::<u8>()
    }

    #[pyslot]
    fn slot_str(zelf: &PyObject, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        let zelf = zelf.downcast_ref::<Self>().expect("expected bytearray");
        PyBytesInner::warn_on_str("str() on a bytearray instance", vm)?;
        let class_name = zelf.class().name();
        let repr = zelf.inner().repr_with_name(&class_name, vm)?;
        Ok(vm.ctx.new_str(repr))
    }

    fn __add__(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<Self> {
        // bytearray_concat: "can't concat %.100s to %.100s"
        let class_name = other.class().slot_name().to_string();
        let other = <ArgBytesLike as TryFromObject>::try_from_object(vm, other)
            .map_err(|_| vm.new_type_error(format!("can't concat {class_name} to bytearray")))?;
        Ok(self.inner().add(&other.borrow_buf()).into())
    }

    fn __contains__(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        let needle = ByteInnerSub::from_contains_arg(needle, vm)?;
        self.inner().contains(needle, vm)
    }

    fn __iadd__(
        zelf: PyRef<Self>,
        other: ArgBytesLike,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<Self>> {
        zelf.try_resizable(vm)?
            .elements
            .extend(&*other.borrow_buf());
        Ok(zelf)
    }

    fn __getitem__(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self._getitem(&needle, vm)
    }

    pub fn __delitem__(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self._delitem(&needle, vm)
    }

    #[pystaticmethod]
    fn maketrans(from: PyBytesInner, to: PyBytesInner, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        PyBytesInner::maketrans(from, to, vm)
    }

    #[pymethod]
    fn isalnum(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<bool> {
        check_noargs(vm, "bytearray.isalnum", &func_args)?;
        Ok(self.inner().isalnum())
    }

    #[pymethod]
    fn isalpha(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<bool> {
        check_noargs(vm, "bytearray.isalpha", &func_args)?;
        Ok(self.inner().isalpha())
    }

    #[pymethod]
    fn isascii(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<bool> {
        check_noargs(vm, "bytearray.isascii", &func_args)?;
        Ok(self.inner().isascii())
    }

    #[pymethod]
    fn isdigit(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<bool> {
        check_noargs(vm, "bytearray.isdigit", &func_args)?;
        Ok(self.inner().isdigit())
    }

    #[pymethod]
    fn islower(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<bool> {
        check_noargs(vm, "bytearray.islower", &func_args)?;
        Ok(self.inner().islower())
    }

    #[pymethod]
    fn isspace(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<bool> {
        check_noargs(vm, "bytearray.isspace", &func_args)?;
        Ok(self.inner().isspace())
    }

    #[pymethod]
    fn isupper(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<bool> {
        check_noargs(vm, "bytearray.isupper", &func_args)?;
        Ok(self.inner().isupper())
    }

    #[pymethod]
    fn istitle(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<bool> {
        check_noargs(vm, "bytearray.istitle", &func_args)?;
        Ok(self.inner().istitle())
    }

    #[pymethod]
    fn lower(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<Self> {
        check_noargs(vm, "bytearray.lower", &func_args)?;
        Ok(self.inner().lower().into())
    }

    #[pymethod]
    fn upper(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<Self> {
        check_noargs(vm, "bytearray.upper", &func_args)?;
        Ok(self.inner().upper().into())
    }

    #[pymethod]
    fn capitalize(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<Self> {
        check_noargs(vm, "bytearray.capitalize", &func_args)?;
        Ok(self.inner().capitalize().into())
    }

    #[pymethod]
    fn swapcase(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<Self> {
        check_noargs(vm, "bytearray.swapcase", &func_args)?;
        Ok(self.inner().swapcase().into())
    }

    #[pymethod]
    fn hex(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<String> {
        // clinic signature: max 2 optional arguments
        if func_args.args.len() > 2 {
            return Err(vm.new_type_error(format!(
                "hex() takes at most 2 arguments ({} given)",
                func_args.args.len()
            )));
        }
        let options: ByteInnerHexOptions = func_args.bind(vm)?;
        // Measuring the separator runs Python, so it happens before the buffer
        // is borrowed.
        let (sep, bytes_per_sep) = options.resolve(vm)?;
        Ok(self.inner().hex(sep, bytes_per_sep))
    }

    #[pyclassmethod]
    fn fromhex(cls: PyTypeRef, string: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let bytes = PyBytesInner::fromhex_object(string, vm)?;
        let bytes = vm.ctx.new_bytes(bytes);
        let args = vec![bytes.into()].into();
        PyType::call(&cls, args, vm)
    }

    #[pymethod]
    fn center(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<Self> {
        check_no_kwargs(vm, "bytearray.center", &func_args)?;
        check_positional(vm, "center", func_args.args.len(), 1, 2)?;
        let options: ByteInnerPaddingOptions = func_args.bind(vm)?;
        Ok(self.inner().center(options, vm)?.into())
    }

    #[pymethod]
    fn ljust(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<Self> {
        check_no_kwargs(vm, "bytearray.ljust", &func_args)?;
        check_positional(vm, "ljust", func_args.args.len(), 1, 2)?;
        let options: ByteInnerPaddingOptions = func_args.bind(vm)?;
        Ok(self.inner().ljust(options, vm)?.into())
    }

    #[pymethod]
    fn rjust(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<Self> {
        check_no_kwargs(vm, "bytearray.rjust", &func_args)?;
        check_positional(vm, "rjust", func_args.args.len(), 1, 2)?;
        let options: ByteInnerPaddingOptions = func_args.bind(vm)?;
        Ok(self.inner().rjust(options, vm)?.into())
    }

    #[pymethod]
    fn count(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<usize> {
        check_no_kwargs(vm, "bytearray.count", &func_args)?;
        check_positional(vm, "count", func_args.args.len(), 1, 3)?;
        let options: ByteInnerFindOptions = func_args.bind(vm)?;
        self.inner().count(options, vm)
    }

    #[pymethod]
    fn join(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<Self> {
        check_meth_o(vm, "bytearray.join", &func_args)?;
        let (iter,): (ArgIterable<PyBytesInner>,) = func_args.bind(vm)?;
        // Driving the iterable runs Python, which can reach this bytearray,
        // so the separator is taken by value rather than left borrowed.
        let separator = self.inner().clone();
        Ok(separator.join(iter, vm)?.into())
    }

    #[pymethod]
    fn endswith(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<bool> {
        check_no_kwargs(vm, "bytearray.endswith", &func_args)?;
        check_positional(vm, "endswith", func_args.args.len(), 1, 3)?;
        let options: anystr::StartsEndsWithArgs = func_args.bind(vm)?;
        let borrowed = self.borrow_buf();
        let (affix, substr) =
            match options.prepare(&*borrowed, borrowed.len(), |s, r| s.get_bytes(r)) {
                Some(x) => x,
                None => return Ok(false),
            };
        substr.py_starts_ends_with(
            &affix,
            "endswith",
            "bytes",
            |s, x: PyBytesInner| s.ends_with(x.as_bytes()),
            vm,
        )
    }

    #[pymethod]
    fn startswith(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<bool> {
        check_no_kwargs(vm, "bytearray.startswith", &func_args)?;
        check_positional(vm, "startswith", func_args.args.len(), 1, 3)?;
        let options: anystr::StartsEndsWithArgs = func_args.bind(vm)?;
        let borrowed = self.borrow_buf();
        let (affix, substr) =
            match options.prepare(&*borrowed, borrowed.len(), |s, r| s.get_bytes(r)) {
                Some(x) => x,
                None => return Ok(false),
            };
        substr.py_starts_ends_with(
            &affix,
            "startswith",
            "bytes",
            |s, x: PyBytesInner| s.starts_with(x.as_bytes()),
            vm,
        )
    }

    #[pymethod]
    fn find(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<isize> {
        check_no_kwargs(vm, "bytearray.find", &func_args)?;
        check_positional(vm, "find", func_args.args.len(), 1, 3)?;
        let options: ByteInnerFindOptions = func_args.bind(vm)?;
        let index = self.inner().find(options, |h, n| h.find(n), vm)?;
        Ok(index.map_or(-1, |v| v as isize))
    }

    #[pymethod]
    fn index(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<usize> {
        check_no_kwargs(vm, "bytearray.index", &func_args)?;
        check_positional(vm, "index", func_args.args.len(), 1, 3)?;
        let options: ByteInnerFindOptions = func_args.bind(vm)?;
        let index = self.inner().find(options, |h, n| h.find(n), vm)?;
        index.ok_or_else(|| vm.new_value_error("substring not found"))
    }

    #[pymethod]
    fn rfind(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<isize> {
        check_no_kwargs(vm, "bytearray.rfind", &func_args)?;
        check_positional(vm, "rfind", func_args.args.len(), 1, 3)?;
        let options: ByteInnerFindOptions = func_args.bind(vm)?;
        let index = self.inner().find(options, |h, n| h.rfind(n), vm)?;
        Ok(index.map_or(-1, |v| v as isize))
    }

    #[pymethod]
    fn rindex(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<usize> {
        check_no_kwargs(vm, "bytearray.rindex", &func_args)?;
        check_positional(vm, "rindex", func_args.args.len(), 1, 3)?;
        let options: ByteInnerFindOptions = func_args.bind(vm)?;
        let index = self.inner().find(options, |h, n| h.rfind(n), vm)?;
        index.ok_or_else(|| vm.new_value_error("substring not found"))
    }

    #[pymethod]
    fn translate(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<Self> {
        // clinic signature: table is positional-only, delete is optional
        if func_args.args.is_empty() {
            return Err(
                vm.new_type_error("translate() takes at least 1 positional argument (0 given)")
            );
        }
        if func_args.args.len() > 2 {
            return Err(vm.new_type_error(format!(
                "translate() takes at most 2 arguments ({} given)",
                func_args.args.len()
            )));
        }
        let options: ByteInnerTranslateOptions = func_args.bind(vm)?;
        Ok(self.inner().translate(options, vm)?.into())
    }

    #[pymethod]
    fn strip(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<Self> {
        check_no_kwargs(vm, "bytearray.strip", &func_args)?;
        check_positional(vm, "strip", func_args.args.len(), 0, 1)?;
        let chars: OptionalOption<PyBytesInner> = func_args.bind(vm)?;
        Ok(self.inner().strip(chars).into())
    }

    #[pymethod]
    fn removeprefix(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<Self> {
        check_meth_o(vm, "bytearray.removeprefix", &func_args)?;
        let (prefix,): (PyBytesInner,) = func_args.bind(vm)?;
        Ok(self.inner().removeprefix(prefix).into())
    }

    #[pymethod]
    fn removesuffix(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<Self> {
        check_meth_o(vm, "bytearray.removesuffix", &func_args)?;
        let (suffix,): (PyBytesInner,) = func_args.bind(vm)?;
        Ok(self.inner().removesuffix(suffix).to_vec().into())
    }

    #[pymethod]
    fn split(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
        // clinic signature: max 2 optional arguments
        if func_args.args.len() > 2 {
            return Err(vm.new_type_error(format!(
                "split() takes at most 2 arguments ({} given)",
                func_args.args.len()
            )));
        }
        let options: ByteInnerSplitOptions = func_args.bind(vm)?;
        self.inner()
            .split(options, |s, vm| vm.ctx.new_bytearray(s.to_vec()).into(), vm)
    }

    #[pymethod]
    fn rsplit(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
        // clinic signature: max 2 optional arguments
        if func_args.args.len() > 2 {
            return Err(vm.new_type_error(format!(
                "rsplit() takes at most 2 arguments ({} given)",
                func_args.args.len()
            )));
        }
        let options: ByteInnerSplitOptions = func_args.bind(vm)?;
        self.inner()
            .rsplit(options, |s, vm| vm.ctx.new_bytearray(s.to_vec()).into(), vm)
    }

    #[pymethod]
    fn partition(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
        check_meth_o(vm, "bytearray.partition", &func_args)?;
        let (sep,): (PyBytesInner,) = func_args.bind(vm)?;
        // sep ALWAYS converted to  bytearray even it's bytes or memoryview
        // so its ok to accept PyBytesInner
        let value = self.inner();
        let (front, has_mid, back) = value.partition(&sep, vm)?;
        Ok(vm.new_tuple((
            vm.ctx.new_bytearray(front.to_vec()),
            vm.ctx
                .new_bytearray(if has_mid { sep.elements } else { Vec::new() }),
            vm.ctx.new_bytearray(back.to_vec()),
        )))
    }

    #[pymethod]
    fn rpartition(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
        check_meth_o(vm, "bytearray.rpartition", &func_args)?;
        let (sep,): (PyBytesInner,) = func_args.bind(vm)?;
        let value = self.inner();
        let (back, has_mid, front) = value.rpartition(&sep, vm)?;
        Ok(vm.new_tuple((
            vm.ctx.new_bytearray(front.to_vec()),
            vm.ctx
                .new_bytearray(if has_mid { sep.elements } else { Vec::new() }),
            vm.ctx.new_bytearray(back.to_vec()),
        )))
    }

    #[pymethod]
    fn expandtabs(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<Self> {
        // clinic signature: max 1 optional argument
        if func_args.args.len() > 1 {
            return Err(vm.new_type_error(format!(
                "expandtabs() takes at most 1 argument ({} given)",
                func_args.args.len()
            )));
        }
        let options: anystr::ExpandTabsArgs = func_args.bind(vm)?;
        Ok(self.inner().expandtabs(options).into())
    }

    #[pymethod]
    fn splitlines(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
        // clinic signature: max 1 optional argument
        if func_args.args.len() > 1 {
            return Err(vm.new_type_error(format!(
                "splitlines() takes at most 1 argument ({} given)",
                func_args.args.len()
            )));
        }
        let options: anystr::SplitLinesArgs = func_args.bind(vm)?;
        Ok(self
            .inner()
            .splitlines(options, |x| vm.ctx.new_bytearray(x.to_vec()).into()))
    }

    #[pymethod]
    fn zfill(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<Self> {
        check_meth_o(vm, "bytearray.zfill", &func_args)?;
        let (width,): (PyObjectRef,) = func_args.bind(vm)?;
        let width = crate::builtins::to_c_ssize_t(&width, vm)?;
        Ok(self.inner().zfill(width, vm)?.into())
    }

    #[pymethod]
    fn replace(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<Self> {
        check_no_kwargs(vm, "bytearray.replace", &func_args)?;
        check_positional(vm, "replace", func_args.args.len(), 2, 3)?;
        let (old, new, count): (PyBytesInner, PyBytesInner, OptionalArg<isize>) =
            func_args.bind(vm)?;
        Ok(self.inner().replace(old, new, count, vm)?.into())
    }

    #[pymethod]
    fn copy(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<Self> {
        check_noargs(vm, "bytearray.copy", &func_args)?;
        Ok(self.borrow_buf().to_vec().into())
    }

    #[pymethod]
    fn title(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<Self> {
        check_noargs(vm, "bytearray.title", &func_args)?;
        Ok(self.inner().title().into())
    }

    fn __mul__(&self, value: ArgSize, vm: &VirtualMachine) -> PyResult<Self> {
        self.repeat(value.into(), vm)
    }

    fn __imul__(zelf: PyRef<Self>, value: ArgSize, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        Self::irepeat(&zelf, value.into(), vm)?;
        Ok(zelf)
    }

    fn __mod__(&self, values: PyObjectRef, vm: &VirtualMachine) -> PyResult<Self> {
        // Formatting calls the values' conversion methods, which can reach
        // this bytearray, so the format is taken by value.
        let format = self.inner().clone();
        let formatted = format.cformat(values, vm)?;
        Ok(formatted.into())
    }

    #[pymethod]
    fn reverse(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
        check_noargs(vm, "bytearray.reverse", &func_args)?;
        self.borrow_buf_mut().reverse();
        Ok(())
    }

    #[pymethod]
    pub fn resize(&self, size: isize, vm: &VirtualMachine) -> PyResult<()> {
        if size < 0 {
            return Err(vm.new_value_error("bytearray.resize(): new size must be >= 0"));
        }
        self.try_resizable(vm)?.elements.resize(size as usize, 0);
        Ok(())
    }

    // TODO: Uncomment when Python adds __class_getitem__ to bytearray
    // #[pyclassmethod]
    fn __class_getitem__(
        cls: PyTypeRef,
        args: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyGenericAlias> {
        PyGenericAlias::from_args(cls, args, vm)
    }
}

#[pyclass]
impl Py<PyByteArray> {
    fn __setitem__(
        &self,
        needle: PyObjectRef,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        PyByteArray::_setitem(self, &needle, value, vm)
    }

    #[pymethod]
    fn pop(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<u8> {
        check_no_kwargs(vm, "bytearray.pop", &func_args)?;
        check_positional(vm, "pop", func_args.args.len(), 0, 1)?;
        let index: OptionalArg<isize> = func_args.bind(vm)?;
        let elements = &mut self.try_resizable(vm)?.elements;
        let index = elements
            .wrap_index(index.unwrap_or(-1))
            .ok_or_else(|| vm.new_index_error("index out of range"))?;
        Ok(elements.remove(index))
    }

    #[pymethod]
    fn insert(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
        check_no_kwargs(vm, "bytearray.insert", &func_args)?;
        check_positional(vm, "insert", func_args.args.len(), 2, 2)?;
        let (index, object): (isize, PyObjectRef) = func_args.bind(vm)?;
        let value = value_from_object(vm, &object)?;
        let elements = &mut self.try_resizable(vm)?.elements;
        let index = elements.saturate_index(index);
        elements.insert(index, value);
        Ok(())
    }

    #[pymethod]
    fn append(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
        check_meth_o(vm, "bytearray.append", &func_args)?;
        let (object,): (PyObjectRef,) = func_args.bind(vm)?;
        let value = value_from_object(vm, &object)?;
        self.try_resizable(vm)?.elements.push(value);
        Ok(())
    }

    #[pymethod]
    fn remove(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
        check_meth_o(vm, "bytearray.remove", &func_args)?;
        let (object,): (PyObjectRef,) = func_args.bind(vm)?;
        let value = value_from_object(vm, &object)?;
        let elements = &mut self.try_resizable(vm)?.elements;
        let index = elements
            .find_byte(value)
            .ok_or_else(|| vm.new_value_error("value not found in bytearray"))?;
        elements.remove(index);
        Ok(())
    }

    #[pymethod]
    fn extend(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
        check_meth_o(vm, "bytearray.extend", &func_args)?;
        let (object,): (PyObjectRef,) = func_args.bind(vm)?;
        if self.is(&object) {
            return PyByteArray::irepeat(self, 2, vm);
        }
        // bytearray_setslice keeps the export alive across the resize, so a value
        // looking at this bytearray is what stops it from growing.
        let buffer = object
            .check_buffer()
            .then(|| {
                PyBuffer::from_object(vm, &object, BufferFlags::SIMPLE).map_err(|_| {
                    // What an exporter refuses to hand out leaves the value simply
                    // not usable here, whatever the exporter's own complaint was.
                    vm.new_type_error(format!(
                        "can't set bytearray slice from {}",
                        object.class().name()
                    ))
                })
            })
            .transpose()?;
        let items = match &buffer {
            Some(buffer) => buffer
                .as_contiguous()
                .ok_or_else(|| {
                    vm.new_buffer_error("non-contiguous buffer is not a bytes-like object")
                })?
                .to_vec(),
            None => bytes_from_object(vm, &object)?,
        };
        self.try_resizable(vm)?.elements.extend(items);
        Ok(())
    }

    #[pymethod]
    fn clear(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
        check_noargs(vm, "bytearray.clear", &func_args)?;
        self.try_resizable(vm)?.elements.clear();
        Ok(())
    }

    #[pymethod]
    fn __reduce_ex__(
        &self,
        _proto: usize,
        vm: &VirtualMachine,
    ) -> (PyTypeRef, PyTupleRef, Option<PyDictRef>) {
        self.__reduce__(vm)
    }

    #[pymethod]
    fn __reduce__(&self, vm: &VirtualMachine) -> (PyTypeRef, PyTupleRef, Option<PyDictRef>) {
        let bytes = PyBytes::from(self.borrow_buf().to_vec()).to_pyobject(vm);
        (
            self.class().to_owned(),
            PyTuple::new_ref(vec![bytes], &vm.ctx),
            self.as_object().dict(),
        )
    }
}

#[pyclass]
impl PyRef<PyByteArray> {
    #[pymethod]
    fn lstrip(self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<Self> {
        check_no_kwargs(vm, "bytearray.lstrip", &func_args)?;
        check_positional(vm, "lstrip", func_args.args.len(), 0, 1)?;
        let chars: OptionalOption<PyBytesInner> = func_args.bind(vm)?;
        let inner = self.inner();
        let stripped = inner.lstrip(chars);
        let elements = &inner.elements;
        if stripped == elements {
            drop(inner);
            Ok(self)
        } else {
            Ok(vm.ctx.new_pyref(PyByteArray::from(stripped.to_vec())))
        }
    }

    #[pymethod]
    fn rstrip(self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<Self> {
        check_no_kwargs(vm, "bytearray.rstrip", &func_args)?;
        check_positional(vm, "rstrip", func_args.args.len(), 0, 1)?;
        let chars: OptionalOption<PyBytesInner> = func_args.bind(vm)?;
        let inner = self.inner();
        let stripped = inner.rstrip(chars);
        let elements = &inner.elements;
        if stripped == elements {
            drop(inner);
            Ok(self)
        } else {
            Ok(vm.ctx.new_pyref(PyByteArray::from(stripped.to_vec())))
        }
    }

    #[pymethod]
    fn decode(self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        // clinic signature: max 2 optional arguments
        if func_args.args.len() > 2 {
            return Err(vm.new_type_error(format!(
                "decode() takes at most 2 arguments ({} given)",
                func_args.args.len()
            )));
        }
        let args: DecodeArgs = func_args.bind(vm)?;
        bytes_decode(self.into(), args, vm)
    }
}

impl DefaultConstructor for PyByteArray {}

impl Initializer for PyByteArray {
    type Args = ByteInnerNewOptions;

    fn slot_init(zelf: PyObjectRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
        if args.args.len() > 3 {
            return Err(vm.new_type_error(format!(
                "bytearray() takes at most 3 arguments ({} given)",
                args.args.len()
            )));
        }
        ByteInnerNewOptions::check_encoding_errors(&args, "bytearray", vm)?;
        let zelf: PyRef<Self> = zelf.try_into_value(vm)?;
        let options: Self::Args = args.bind(vm)?;
        Self::init(zelf, options, vm)
    }

    fn init(zelf: PyRef<Self>, options: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
        // First unpack bytearray and *then* get a lock to set it.
        let mut inner = options.get_bytearray_inner("bytearray", vm)?;
        core::mem::swap(&mut *zelf.inner_mut(), &mut inner);
        Ok(())
    }
}

impl Comparable for PyByteArray {
    fn cmp(
        zelf: &Py<Self>,
        other: &PyObject,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        if let Some(res) = op.identical_optimization(zelf, other) {
            return Ok(res.into());
        }
        Ok(zelf.inner().cmp(other, op, vm))
    }
}

static BUFFER_METHODS: BufferMethods = BufferMethods {
    obj_bytes: |buffer| buffer.obj_as::<PyByteArray>().borrow_buf().into(),
    obj_bytes_mut: |buffer| {
        PyMappedRwLockWriteGuard::map(buffer.obj_as::<PyByteArray>().borrow_buf_mut(), |x| {
            x.as_mut_slice()
        })
        .into()
    },
    release: |buffer| {
        buffer
            .obj_as::<PyByteArray>()
            .exports
            .fetch_sub(1, Ordering::Release);
    },
    retain: |buffer| {
        buffer
            .obj_as::<PyByteArray>()
            .exports
            .fetch_add(1, Ordering::Release);
    },
};

impl AsBuffer for PyByteArray {
    const RELEASE_BUFFER: bool = true;

    fn slot_as_buffer(
        zelf: &PyObject,
        flags: BufferFlags,
        vm: &VirtualMachine,
    ) -> PyResult<PyBuffer> {
        let zelf = zelf
            .downcast_ref::<Self>()
            .ok_or_else(|| vm.new_type_error("unexpected payload for as_buffer"))?;
        flags.fill_info_check(false, vm)?;
        Self::as_buffer(zelf, vm)
    }

    fn as_buffer(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<PyBuffer> {
        Ok(PyBuffer::new(
            zelf.to_owned().into(),
            BufferDescriptor::simple(zelf.__len__(), false),
            &BUFFER_METHODS,
        ))
    }
}

impl BufferResizeGuard for PyByteArray {
    type Resizable<'a> = PyRwLockWriteGuard<'a, PyBytesInner>;

    fn try_resizable_opt(&self) -> Option<Self::Resizable<'_>> {
        // An export is a borrow someone else still holds, so it is answered
        // before the lock rather than by waiting on it.
        (self.exports.load(Ordering::SeqCst) == 0).then(|| self.inner.write())
    }
}

impl AsMapping for PyByteArray {
    fn as_mapping() -> &'static PyMappingMethods {
        static AS_MAPPING: PyMappingMethods = PyMappingMethods {
            length: atomic_func!(|mapping, _vm| Ok(
                PyByteArray::mapping_downcast(mapping).__len__()
            )),
            subscript: atomic_func!(|mapping, needle, vm| {
                PyByteArray::mapping_downcast(mapping).__getitem__(needle.to_owned(), vm)
            }),
            ass_subscript: atomic_func!(|mapping, needle, value, vm| {
                let zelf = PyByteArray::mapping_downcast(mapping);
                if let Some(value) = value {
                    zelf.__setitem__(needle.to_owned(), value, vm)
                } else {
                    zelf.__delitem__(needle.to_owned(), vm)
                }
            }),
        };
        &AS_MAPPING
    }
}

impl AsSequence for PyByteArray {
    fn as_sequence() -> &'static PySequenceMethods {
        static AS_SEQUENCE: PySequenceMethods = PySequenceMethods {
            length: atomic_func!(|seq, _vm| Ok(PyByteArray::sequence_downcast(seq).__len__())),
            concat: atomic_func!(|seq, other, vm| {
                PyByteArray::sequence_downcast(seq)
                    .inner()
                    .concat(other, vm)
                    .map_err(|_| {
                        // bytearray_concat: "can't concat %.100s to %.100s"
                        vm.new_type_error(format!(
                            "can't concat {} to bytearray",
                            other.class().slot_name()
                        ))
                    })
                    .map(|x| PyByteArray::from(x).into_pyobject(vm))
            }),
            repeat: atomic_func!(|seq, n, vm| {
                PyByteArray::sequence_downcast(seq)
                    .repeat(n, vm)
                    .map(|x| x.into_pyobject(vm))
            }),
            item: atomic_func!(|seq, i, vm| {
                PyByteArray::sequence_downcast(seq)
                    .borrow_buf()
                    .getitem_by_index(vm, i)
                    .map(|x| vm.ctx.new_bytes(vec![x]).into())
            }),
            ass_item: atomic_func!(|seq, i, value, vm| {
                let zelf = PyByteArray::sequence_downcast(seq);
                if let Some(value) = value {
                    zelf._setitem_by_index(i, value, vm)
                } else {
                    zelf.borrow_buf_mut().delitem_by_index(vm, i)
                }
            }),
            contains: atomic_func!(|seq, other, vm| {
                PyByteArray::sequence_downcast(seq).__contains__(other.to_owned(), vm)
            }),
            inplace_concat: atomic_func!(|seq, other, vm| {
                let class_name = other.class().slot_name().to_string();
                let other = ArgBytesLike::try_from_object(vm, other.to_owned()).map_err(|_| {
                    vm.new_type_error(format!("can't concat {class_name} to bytearray"))
                })?;
                let zelf = PyByteArray::sequence_downcast(seq).to_owned();
                PyByteArray::__iadd__(zelf, other, vm).map(|x| x.into())
            }),
            inplace_repeat: atomic_func!(|seq, n, vm| {
                let zelf = PyByteArray::sequence_downcast(seq).to_owned();
                PyByteArray::irepeat(&zelf, n, vm)?;
                Ok(zelf.into())
            }),
        };
        &AS_SEQUENCE
    }
}

impl AsNumber for PyByteArray {
    fn as_number() -> &'static PyNumberMethods {
        static AS_NUMBER: PyNumberMethods = PyNumberMethods {
            remainder: Some(|a, b, vm| {
                if let Some(a) = a.downcast_ref::<PyByteArray>() {
                    a.__mod__(b.to_owned(), vm).to_pyresult(vm)
                } else {
                    Ok(vm.ctx.not_implemented())
                }
            }),
            ..PyNumberMethods::NOT_IMPLEMENTED
        };
        &AS_NUMBER
    }
}

impl Iterable for PyByteArray {
    fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        Ok(PyByteArrayIterator {
            internal: PyMutex::new(PositionIterInternal::new(zelf, 0)),
        }
        .into_pyobject(vm))
    }
}

impl Representable for PyByteArray {
    #[inline]
    fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
        let class = zelf.class();
        let class_name = class.name();
        zelf.inner().repr_with_name(&class_name, vm)
    }
}

#[pyclass(module = false, name = "bytearray_iterator")]
#[derive(Debug)]
pub(crate) struct PyByteArrayIterator {
    internal: PyMutex<PositionIterInternal<PyByteArrayRef>>,
}

impl PyPayload for PyByteArrayIterator {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.bytearray_iterator_type
    }
}

#[pyclass(flags(DISALLOW_INSTANTIATION), with(IterNext, Iterable))]
impl PyByteArrayIterator {
    #[pymethod]
    fn __length_hint__(&self) -> usize {
        self.internal.lock().length_hint(|obj| obj.__len__())
    }
    #[pymethod]
    fn __reduce__(&self, vm: &VirtualMachine) -> PyTupleRef {
        let func = builtins_iter(vm);
        self.internal.lock().reduce(
            func,
            |x| x.clone().into(),
            |vm| vm.ctx.empty_tuple.clone().into(),
            vm,
        )
    }

    #[pymethod]
    fn __setstate__(&self, state: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.internal
            .lock()
            .set_state(state, |obj, pos| pos.min(obj.__len__()), vm)
    }
}

impl SelfIter for PyByteArrayIterator {}
impl IterNext for PyByteArrayIterator {
    fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        zelf.internal.lock().next(|bytearray, pos| {
            let buf = bytearray.borrow_buf();
            Ok(PyIterReturn::from_result(
                buf.get(pos).map(|&x| vm.new_pyobj(x)).ok_or(None),
            ))
        })
    }
}

//! Implementation of the python bytearray object.
use super::bytes::{PyBytes, PyBytesRef};
use super::dict::PyDictRef;
use super::int::PyIntRef;
use super::pystr::PyStrRef;
use super::pytype::PyTypeRef;
use super::tuple::PyTupleRef;
use crate::anystr::{self, AnyStr};
use crate::buffer::{BufferOptions, PyBuffer, ResizeGuard};
use crate::bytesinner::{
    bytes_decode, bytes_from_object, value_from_object, ByteInnerFindOptions, ByteInnerNewOptions,
    ByteInnerPaddingOptions, ByteInnerSplitOptions, ByteInnerTranslateOptions, DecodeArgs,
    PyBytesInner,
};
use crate::byteslike::ArgBytesLike;
use crate::common::borrow::{BorrowedValue, BorrowedValueMut};
use crate::common::lock::{
    PyMappedRwLockReadGuard, PyMappedRwLockWriteGuard, PyRwLock, PyRwLockReadGuard,
    PyRwLockWriteGuard,
};
use crate::function::{FuncArgs, OptionalArg, OptionalOption};
use crate::sliceable::{PySliceableSequence, PySliceableSequenceMut, SequenceIndex};
use crate::slots::{
    AsBuffer, Callable, Comparable, Hashable, Iterable, PyComparisonOp, PyIter, Unhashable,
};
use crate::utils::Either;
use crate::vm::VirtualMachine;
use crate::{
    IdProtocol, IntoPyObject, PyClassDef, PyClassImpl, PyComparisonValue, PyContext, PyIterable,
    PyObjectRef, PyRef, PyResult, PyValue, TypeProtocol,
};
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
#[derive(Debug, Default)]
pub struct PyByteArray {
    inner: PyRwLock<PyBytesInner>,
    exports: AtomicCell<usize>,
}

pub type PyByteArrayRef = PyRef<PyByteArray>;

impl PyByteArray {
    fn from_inner(inner: PyBytesInner) -> Self {
        PyByteArray {
            inner: PyRwLock::new(inner),
            exports: AtomicCell::new(0),
        }
    }

    pub fn borrow_buf(&self) -> PyMappedRwLockReadGuard<'_, [u8]> {
        PyRwLockReadGuard::map(self.inner.read(), |inner| &*inner.elements)
    }

    pub fn borrow_buf_mut(&self) -> PyMappedRwLockWriteGuard<'_, Vec<u8>> {
        PyRwLockWriteGuard::map(self.inner.write(), |inner| &mut inner.elements)
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
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.bytearray_type
    }
}

/// Fill bytearray class methods dictionary.
pub(crate) fn init(context: &PyContext) {
    PyByteArray::extend_class(context, &context.types.bytearray_type);
    let bytearray_type = &context.types.bytearray_type;
    extend_class!(context, bytearray_type, {
        "maketrans" => context.new_method("maketrans", bytearray_type.clone(), PyBytesInner::maketrans),
    });

    PyByteArrayIterator::extend_class(context, &context.types.bytearray_iterator_type);
}

#[pyimpl(flags(BASETYPE), with(Hashable, Comparable, AsBuffer, Iterable))]
impl PyByteArray {
    #[pyslot]
    fn tp_new(cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        PyByteArray::default().into_pyresult_with_type(vm, cls)
    }

    /// Initialize self.  See help(type(self)) for accurate signature.
    #[pymethod(magic)]
    fn init(&self, options: ByteInnerNewOptions, vm: &VirtualMachine) -> PyResult<()> {
        // First unpack bytearray and *then* get a lock to set it.
        let mut inner = options.get_bytearray_inner(vm)?;
        std::mem::swap(&mut *self.inner_mut(), &mut inner);
        Ok(())
    }

    #[inline]
    fn inner(&self) -> PyRwLockReadGuard<'_, PyBytesInner> {
        self.inner.read()
    }
    #[inline]
    fn inner_mut(&self) -> PyRwLockWriteGuard<'_, PyBytesInner> {
        self.inner.write()
    }

    /// Return repr(self).
    #[pymethod(magic)]
    fn repr(&self) -> String {
        self.inner().repr("bytearray(", ")")
    }

    /// B.__alloc__() -> int
    /// 
    /// Return the number of bytes actually allocated.
    #[pymethod(magic)]
    fn alloc(&self) -> usize {
        self.inner().capacity()
    }

    /// Return len(self).
    #[pymethod(magic)]
    fn len(&self) -> usize {
        self.borrow_buf().len()
    }

    /// Returns the size of the bytearray object in memory, in bytes.
    #[pymethod(magic)]
    fn sizeof(&self) -> usize {
        size_of::<Self>() + self.borrow_buf().len() * size_of::<u8>()
    }

    /// Return self+value.
    #[pymethod(magic)]
    fn add(&self, other: ArgBytesLike, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_bytearray(self.inner().add(&*other.borrow_buf()))
    }

    /// Return key in self.
    #[pymethod(magic)]
    fn contains(
        &self,
        needle: Either<PyBytesInner, PyIntRef>,
        vm: &VirtualMachine,
    ) -> PyResult<bool> {
        self.inner().contains(needle, vm)
    }

    /// Set self[key] to value.
    #[pymethod(magic)]
    fn setitem(
        zelf: PyRef<Self>,
        needle: PyObjectRef,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        match SequenceIndex::try_from_object_for(vm, needle, Self::NAME)? {
            SequenceIndex::Int(i) => {
                let value = value_from_object(vm, &value)?;
                let mut elements = zelf.borrow_buf_mut();
                if let Some(i) = elements.wrap_index(i) {
                    elements[i] = value;
                    Ok(())
                } else {
                    Err(vm.new_index_error("index out of range".to_owned()))
                }
            }
            SequenceIndex::Slice(slice) => {
                let items = if zelf.is(&value) {
                    zelf.borrow_buf().to_vec()
                } else {
                    bytes_from_object(vm, &value)?
                };
                if let Ok(mut w) = zelf.try_resizable(vm) {
                    w.elements.set_slice_items(vm, &slice, items.as_slice())
                } else {
                    zelf.borrow_buf_mut()
                        .set_slice_items_no_resize(vm, &slice, items.as_slice())
                }
            }
        }
    }

    /// Implement self+=value.
    #[pymethod(magic)]
    fn iadd(zelf: PyRef<Self>, other: ArgBytesLike, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        zelf.try_resizable(vm)?
            .elements
            .extend(&*other.borrow_buf());
        Ok(zelf)
    }

    /// Return self[key].
    #[pymethod(magic)]
    fn getitem(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.inner().getitem(Self::NAME, needle, vm)
    }

    /// Delete self[key].
    #[pymethod(magic)]
    pub fn delitem(&self, needle: SequenceIndex, vm: &VirtualMachine) -> PyResult<()> {
        let elements = &mut self.try_resizable(vm)?.elements;
        match needle {
            SequenceIndex::Int(int) => {
                if let Some(idx) = elements.wrap_index(int) {
                    elements.remove(idx);
                    Ok(())
                } else {
                    Err(vm.new_index_error("index out of range".to_owned()))
                }
            }
            SequenceIndex::Slice(slice) => elements.delete_slice(vm, &slice),
        }
    }

    /// Remove and return a single item from B.
    /// 
    ///   index
    ///     The index from where to remove the item.
    ///     -1 (the default value) means remove the last item.
    /// 
    /// If no index argument is given, will pop the last item.
    #[pymethod]
    fn pop(zelf: PyRef<Self>, index: OptionalArg<isize>, vm: &VirtualMachine) -> PyResult<u8> {
        let elements = &mut zelf.try_resizable(vm)?.elements;
        let index = elements
            .wrap_index(index.unwrap_or(-1))
            .ok_or_else(|| vm.new_index_error("index out of range".to_owned()))?;
        Ok(elements.remove(index))
    }

    /// Insert a single item into the bytearray before the given index.
    /// 
    ///   index
    ///     The index where the value is to be inserted.
    ///   item
    ///     The item to be inserted.
    #[pymethod]
    fn insert(
        zelf: PyRef<Self>,
        index: isize,
        object: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let value = value_from_object(vm, &object)?;
        let elements = &mut zelf.try_resizable(vm)?.elements;
        let index = elements.saturate_index(index);
        elements.insert(index, value);
        Ok(())
    }

    /// Append a single item to the end of the bytearray.
    /// 
    ///   item
    ///     The item to be appended.
    #[pymethod]
    fn append(zelf: PyRef<Self>, object: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let value = value_from_object(vm, &object)?;
        zelf.try_resizable(vm)?.elements.push(value);
        Ok(())
    }

    /// Remove the first occurrence of a value in the bytearray.
    /// 
    ///   value
    ///     The value to remove.
    #[pymethod]
    fn remove(zelf: PyRef<Self>, object: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let value = value_from_object(vm, &object)?;
        let elements = &mut zelf.try_resizable(vm)?.elements;
        if let Some(index) = elements.find_byte(value) {
            elements.remove(index);
            Ok(())
        } else {
            Err(vm.new_value_error("value not found in bytearray".to_owned()))
        }
    }

    /// Append all the items from the iterator or sequence to the end of the bytearray.
    /// 
    ///   iterable_of_ints
    ///     The iterable of items to append.
    #[pymethod]
    fn extend(zelf: PyRef<Self>, object: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        if zelf.is(&object) {
            Self::irepeat(&zelf, 2, vm)
        } else {
            let items = bytes_from_object(vm, &object)?;
            zelf.try_resizable(vm)?.elements.extend(items);
            Ok(())
        }
    }

    fn irepeat(zelf: &PyRef<Self>, n: usize, vm: &VirtualMachine) -> PyResult<()> {
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
        let elements = &mut w.elements;

        if n == 0 {
            elements.clear();
        } else if n != 1 {
            let old = elements.clone();

            elements.reserve((n - 1) * old.len());
            for _ in 1..n {
                elements.extend(&old);
            }
        }
        Ok(())
    }

    /// B.isalnum() -> bool
    /// 
    /// Return True if all characters in B are alphanumeric
    /// and there is at least one character in B, False otherwise.
    #[pymethod]
    fn isalnum(&self) -> bool {
        self.inner().isalnum()
    }

    /// B.isalpha() -> bool
    /// 
    /// Return True if all characters in B are alphabetic
    /// and there is at least one character in B, False otherwise.
    #[pymethod]
    fn isalpha(&self) -> bool {
        self.inner().isalpha()
    }

    /// B.isascii() -> bool
    /// 
    /// Return True if B is empty or all characters in B are ASCII,
    /// False otherwise.
    #[pymethod]
    fn isascii(&self) -> bool {
        self.inner().isascii()
    }

    /// B.isdigit() -> bool
    /// 
    /// Return True if all characters in B are digits
    /// and there is at least one character in B, False otherwise.
    #[pymethod]
    fn isdigit(&self) -> bool {
        self.inner().isdigit()
    }

    /// B.islower() -> bool
    /// 
    /// Return True if all cased characters in B are lowercase and there is
    /// at least one cased character in B, False otherwise.
    #[pymethod]
    fn islower(&self) -> bool {
        self.inner().islower()
    }

    /// B.isspace() -> bool
    /// 
    /// Return True if all characters in B are whitespace
    /// and there is at least one character in B, False otherwise.
    #[pymethod]
    fn isspace(&self) -> bool {
        self.inner().isspace()
    }

    /// B.isupper() -> bool
    /// 
    /// Return True if all cased characters in B are uppercase and there is
    /// at least one cased character in B, False otherwise.
    #[pymethod]
    fn isupper(&self) -> bool {
        self.inner().isupper()
    }

    /// B.istitle() -> bool
    /// 
    /// Return True if B is a titlecased string and there is at least one
    /// character in B, i.e. uppercase characters may only follow uncased
    /// \ncharacters and lowercase characters only cased ones. Return False
    /// otherwise.
    #[pymethod]
    fn istitle(&self) -> bool {
        self.inner().istitle()
    }

    /// B.lower() -> copy of B
    /// 
    /// Return a copy of B with all ASCII characters converted to lowercase.
    #[pymethod]
    fn lower(&self) -> Self {
        self.inner().lower().into()
    }

    /// B.upper() -> copy of B
    /// 
    /// Return a copy of B with all ASCII characters converted to uppercase.
    #[pymethod]
    fn upper(&self) -> Self {
        self.inner().upper().into()
    }

    /// B.capitalize() -> copy of B
    /// 
    /// Return a copy of B with only its first character capitalized (ASCII)
    /// and the rest lower-cased.
    #[pymethod]
    fn capitalize(&self) -> Self {
        self.inner().capitalize().into()
    }

    /// B.swapcase() -> copy of B
    /// 
    /// Return a copy of B with uppercase ASCII characters converted
    /// to lowercase ASCII and vice versa.
    #[pymethod]
    fn swapcase(&self) -> Self {
        self.inner().swapcase().into()
    }

    /// Create a str of hexadecimal numbers from a bytearray object.
    /// 
    ///   sep
    ///     An optional single character or byte to separate hex bytes.
    ///   bytes_per_sep
    ///     How many bytes between separators.  Positive values count from the
    ///     right, negative values count from the left.
    /// 
    /// Example:
    /// >>> value = bytearray([0xb9, 0x01, 0xef])
    /// >>> value.hex()
    /// 'b901ef'
    /// >>> value.hex(':')
    /// 'b9:01:ef'
    /// >>> value.hex(':', 2)
    /// 'b9:01ef'
    /// >>> value.hex(':', -2)
    /// 'b901:ef
    #[pymethod]
    fn hex(
        &self,
        sep: OptionalArg<Either<PyStrRef, PyBytesRef>>,
        bytes_per_sep: OptionalArg<isize>,
        vm: &VirtualMachine,
    ) -> PyResult<String> {
        self.inner().hex(sep, bytes_per_sep, vm)
    }

    /// Create a bytearray object from a string of hexadecimal numbers.
    /// 
    /// Spaces between two numbers are accepted.
    /// Example: bytearray.fromhex('B9 01EF') -> bytearray(b'\\\\xb9\\\\x01\\\\xef')
    #[pyclassmethod]
    fn fromhex(cls: PyTypeRef, string: PyStrRef, vm: &VirtualMachine) -> PyResult {
        let bytes = PyBytesInner::fromhex(string.as_str(), vm)?;
        let bytes = vm.ctx.new_bytes(bytes);
        Callable::call(&cls, vec![bytes].into(), vm)
    }

    /// Return a centered string of length width.
    /// 
    /// Padding is done using the specified fill character.
    #[pymethod]
    fn center(
        &self,
        options: ByteInnerPaddingOptions,
        vm: &VirtualMachine,
    ) -> PyResult<PyByteArray> {
        Ok(self.inner().center(options, vm)?.into())
    }

    /// Return a left-justified string of length width.
    /// 
    /// Padding is done using the specified fill character.
    #[pymethod]
    fn ljust(
        &self,
        options: ByteInnerPaddingOptions,
        vm: &VirtualMachine,
    ) -> PyResult<PyByteArray> {
        Ok(self.inner().ljust(options, vm)?.into())
    }

    /// Return a right-justified string of length width.
    /// 
    /// Padding is done using the specified fill character.
    #[pymethod]
    fn rjust(
        &self,
        options: ByteInnerPaddingOptions,
        vm: &VirtualMachine,
    ) -> PyResult<PyByteArray> {
        Ok(self.inner().rjust(options, vm)?.into())
    }

    /// B.count(sub[, start[, end]]) -> int
    /// 
    /// Return the number of non-overlapping occurrences of subsection sub in
    /// bytes B[start:end].  Optional arguments start and end are interpreted
    /// as in slice notation.
    #[pymethod]
    fn count(&self, options: ByteInnerFindOptions, vm: &VirtualMachine) -> PyResult<usize> {
        self.inner().count(options, vm)
    }

    /// Concatenate any number of bytes/bytearray objects.
    /// 
    /// The bytearray whose method is called is inserted in between each pair.
    /// 
    /// The result is returned as a new bytearray object.
    #[pymethod]
    fn join(&self, iter: PyIterable<PyBytesInner>, vm: &VirtualMachine) -> PyResult<PyByteArray> {
        Ok(self.inner().join(iter, vm)?.into())
    }

    /// B.endswith(suffix[, start[, end]]) -> bool
    /// 
    /// Return True if B ends with the specified suffix, False otherwise.
    /// With optional start, test B beginning at that position.
    /// With optional end, stop comparing B at that position.
    /// suffix can also be a tuple of bytes to try.
    #[pymethod]
    fn endswith(&self, options: anystr::StartsEndsWithArgs, vm: &VirtualMachine) -> PyResult<bool> {
        self.borrow_buf().py_startsendswith(
            options,
            "endswith",
            "bytes",
            |s, x: &PyBytesInner| s.ends_with(&x.elements[..]),
            vm,
        )
    }

    /// B.startswith(prefix[, start[, end]]) -> bool
    /// 
    /// Return True if B starts with the specified prefix, False otherwise.
    /// With optional start, test B beginning at that position.
    /// With optional end, stop comparing B at that position.
    /// prefix can also be a tuple of bytes to try.
    #[pymethod]
    fn startswith(
        &self,
        options: anystr::StartsEndsWithArgs,
        vm: &VirtualMachine,
    ) -> PyResult<bool> {
        self.borrow_buf().py_startsendswith(
            options,
            "startswith",
            "bytes",
            |s, x: &PyBytesInner| s.starts_with(&x.elements[..]),
            vm,
        )
    }

    /// B.find(sub[, start[, end]]) -> int
    /// 
    /// Return the lowest index in B where subsection sub is found,
    /// such that sub is contained within B[start,end].  Optional
    /// arguments start and end are interpreted as in slice notation.
    /// 
    /// Return -1 on failure.
    #[pymethod]
    fn find(&self, options: ByteInnerFindOptions, vm: &VirtualMachine) -> PyResult<isize> {
        let index = self.inner().find(options, |h, n| h.find(n), vm)?;
        Ok(index.map_or(-1, |v| v as isize))
    }

    /// B.index(sub[, start[, end]]) -> int
    /// 
    /// Return the lowest index in B where subsection sub is found,
    /// such that sub is contained within B[start,end].  Optional
    /// arguments start and end are interpreted as in slice notation.
    /// 
    /// Raises ValueError when the subsection is not found.
    #[pymethod]
    fn index(&self, options: ByteInnerFindOptions, vm: &VirtualMachine) -> PyResult<usize> {
        let index = self.inner().find(options, |h, n| h.find(n), vm)?;
        index.ok_or_else(|| vm.new_value_error("substring not found".to_owned()))
    }

    /// B.rfind(sub[, start[, end]]) -> int
    /// 
    /// Return the highest index in B where subsection sub is found,
    /// such that sub is contained within B[start,end].  Optional
    /// arguments start and end are interpreted as in slice notation.
    /// 
    /// Return -1 on failure.
    #[pymethod]
    fn rfind(&self, options: ByteInnerFindOptions, vm: &VirtualMachine) -> PyResult<isize> {
        let index = self.inner().find(options, |h, n| h.rfind(n), vm)?;
        Ok(index.map_or(-1, |v| v as isize))
    }

    /// B.rindex(sub[, start[, end]]) -> int
    /// 
    /// Return the highest index in B where subsection sub is found,
    /// such that sub is contained within B[start,end].  Optional
    /// arguments start and end are interpreted as in slice notation.
    /// 
    /// Raise ValueError when the subsection is not found.
    #[pymethod]
    fn rindex(&self, options: ByteInnerFindOptions, vm: &VirtualMachine) -> PyResult<usize> {
        let index = self.inner().find(options, |h, n| h.rfind(n), vm)?;
        index.ok_or_else(|| vm.new_value_error("substring not found".to_owned()))
    }

    /// Return a copy with each character mapped by the given translation table.
    /// 
    ///   table
    ///     Translation table, which must be a bytes object of length 256.
    /// 
    /// All characters occurring in the optional argument delete are removed.
    /// The remaining characters are mapped through the given translation table.
    #[pymethod]
    fn translate(
        &self,
        options: ByteInnerTranslateOptions,
        vm: &VirtualMachine,
    ) -> PyResult<PyByteArray> {
        Ok(self.inner().translate(options, vm)?.into())
    }

    /// Strip leading and trailing bytes contained in the argument.
    /// 
    /// If the argument is omitted or None, strip leading and trailing ASCII whitespace.
    #[pymethod]
    fn strip(&self, chars: OptionalOption<PyBytesInner>) -> Self {
        self.inner().strip(chars).into()
    }

    /// Strip leading bytes contained in the argument.
    /// 
    /// If the argument is omitted or None, strip leading ASCII whitespace.
    #[pymethod]
    fn lstrip(&self, chars: OptionalOption<PyBytesInner>) -> Self {
        self.inner().lstrip(chars).into()
    }

    /// Strip trailing bytes contained in the argument.
    /// 
    /// If the argument is omitted or None, strip trailing ASCII whitespace.
    #[pymethod]
    fn rstrip(&self, chars: OptionalOption<PyBytesInner>) -> Self {
        self.inner().rstrip(chars).into()
    }

    /// removeprefix($self, prefix, /)
    ///
    ///
    /// Return a bytearray object with the given prefix string removed if present.
    ///
    /// If the bytearray starts with the prefix string, return string[len(prefix):]
    /// Otherwise, return a copy of the original bytearray.
    #[pymethod]
    fn removeprefix(&self, prefix: PyBytesInner) -> Self {
        self.inner().removeprefix(prefix).into()
    }

    /// removesuffix(self, prefix, /)
    ///
    ///
    /// Return a bytearray object with the given suffix string removed if present.
    ///
    /// If the bytearray ends with the suffix string, return string[:len(suffix)]
    /// Otherwise, return a copy of the original bytearray.
    #[pymethod]
    fn removesuffix(&self, suffix: PyBytesInner) -> Self {
        self.inner().removesuffix(suffix).to_vec().into()
    }

    /// Return a list of the sections in the bytearray, using sep as the delimiter.
    /// 
    ///   sep
    ///     The delimiter according which to split the bytearray.
    ///     None (the default value) means split on ASCII whitespace characters
    ///     (space, tab, return, newline, formfeed, vertical tab).
    ///   maxsplit
    ///     Maximum number of splits to do.
    ///     -1 (the default value) means no limit.
    #[pymethod]
    fn split(&self, options: ByteInnerSplitOptions, vm: &VirtualMachine) -> PyResult {
        self.inner()
            .split(options, |s, vm| vm.ctx.new_bytearray(s.to_vec()), vm)
    }

    /// Return a list of the sections in the bytearray, using sep as the delimiter.
    /// 
    ///   sep
    ///     The delimiter according which to split the bytearray.
    ///     None (the default value) means split on ASCII whitespace characters
    ///     (space, tab, return, newline, formfeed, vertical tab).
    ///   maxsplit
    ///     Maximum number of splits to do.
    ///     -1 (the default value) means no limit.
    /// 
    /// Splitting is done starting at the end of the bytearray and working to the front.
    #[pymethod]
    fn rsplit(&self, options: ByteInnerSplitOptions, vm: &VirtualMachine) -> PyResult {
        self.inner()
            .rsplit(options, |s, vm| vm.ctx.new_bytearray(s.to_vec()), vm)
    }

    /// Partition the bytearray into three parts using the given separator.
    /// 
    /// This will search for the separator sep in the bytearray. If the separator is
    /// found, returns a 3-tuple containing the part before the separator, the
    /// separator itself, and the part after it as new bytearray objects.
    /// 
    /// If the separator is not found, returns a 3-tuple containing the copy of the
    /// original bytearray object and two empty bytearray objects.
    #[pymethod]
    fn partition(&self, sep: PyBytesInner, vm: &VirtualMachine) -> PyResult {
        // sep ALWAYS converted to  bytearray even it's bytes or memoryview
        // so its ok to accept PyBytesInner
        let value = self.inner();
        let (front, has_mid, back) = value.partition(&sep, vm)?;
        Ok(vm.ctx.new_tuple(vec![
            vm.ctx.new_bytearray(front.to_vec()),
            vm.ctx
                .new_bytearray(if has_mid { sep.elements } else { Vec::new() }),
            vm.ctx.new_bytearray(back.to_vec()),
        ]))
    }

    /// Partition the bytearray into three parts using the given separator.
    /// 
    /// This will search for the separator sep in the bytearray, starting at the end.
    /// If the separator is found, returns a 3-tuple containing the part before the
    /// 
    /// separator, the separator itself, and the part after it as new bytearray
    /// objects.
    /// 
    /// If the separator is not found, returns a 3-tuple containing two empty bytearray
    /// objects and the copy of the original bytearray object.
    #[pymethod]
    fn rpartition(&self, sep: PyBytesInner, vm: &VirtualMachine) -> PyResult {
        let value = self.inner();
        let (back, has_mid, front) = value.rpartition(&sep, vm)?;
        Ok(vm.ctx.new_tuple(vec![
            vm.ctx.new_bytearray(front.to_vec()),
            vm.ctx
                .new_bytearray(if has_mid { sep.elements } else { Vec::new() }),
            vm.ctx.new_bytearray(back.to_vec()),
        ]))
    }

    /// Return a copy where all tab characters are expanded using spaces.
    /// 
    /// If tabsize is not given, a tab size of 8 characters is assumed.
    #[pymethod]
    fn expandtabs(&self, options: anystr::ExpandTabsArgs) -> Self {
        self.inner().expandtabs(options).into()
    }

    /// Return a list of the lines in the bytearray, breaking at line boundaries.
    /// 
    /// Line breaks are not included in the resulting list unless keepends is given and
    /// true.
    #[pymethod]
    fn splitlines(&self, options: anystr::SplitLinesArgs, vm: &VirtualMachine) -> PyObjectRef {
        let lines = self
            .inner()
            .splitlines(options, |x| vm.ctx.new_bytearray(x.to_vec()));
        vm.ctx.new_list(lines)
    }

    /// Pad a numeric string with zeros on the left, to fill a field of the given width.
    /// 
    /// The original string is never truncated.
    #[pymethod]
    fn zfill(&self, width: isize) -> Self {
        self.inner().zfill(width).into()
    }

    /// Return a copy with all occurrences of substring old replaced by new.
    /// 
    ///   count
    ///     Maximum number of occurrences to replace.
    ///     -1 (the default value) means replace all occurrences.
    /// 
    /// If the optional argument count is given, only the first count occurrences are
    /// replaced.
    #[pymethod]
    fn replace(
        &self,
        old: PyBytesInner,
        new: PyBytesInner,
        count: OptionalArg<isize>,
        vm: &VirtualMachine,
    ) -> PyResult<PyByteArray> {
        Ok(self.inner().replace(old, new, count, vm)?.into())
    }

    /// Remove all items from the bytearray.
    #[pymethod]
    fn clear(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<()> {
        zelf.try_resizable(vm)?.elements.clear();
        Ok(())
    }

    /// Return a copy of B.
    #[pymethod]
    fn copy(&self) -> Self {
        self.borrow_buf().to_vec().into()
    }

    /// B.title() -> copy of B
    /// 
    /// Return a titlecased version of B, i.e. ASCII words start with uppercase
    /// characters, all remaining cased characters have lowercase.
    #[pymethod]
    fn title(&self) -> Self {
        self.inner().title().into()
    }

    #[pymethod(name = "__rmul__")]
    #[pymethod(magic)]
    fn mul(&self, value: isize, vm: &VirtualMachine) -> PyResult<Self> {
        vm.check_repeat_or_memory_error(self.len(), value)
            .map(|value| self.inner().repeat(value).into())
    }

    /// Implement self*=value.
    #[pymethod(magic)]
    fn imul(zelf: PyRef<Self>, value: isize, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        vm.check_repeat_or_memory_error(zelf.len(), value)
            .and_then(|value| Self::irepeat(&zelf, value, vm).map(|_| zelf))
    }

    /// Return self%value.
    #[pymethod(name = "__mod__")]
    fn mod_(&self, values: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyByteArray> {
        let formatted = self.inner().cformat(values, vm)?;
        Ok(formatted.into())
    }

    /// Return value%self.
    #[pymethod(magic)]
    fn rmod(&self, _values: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.not_implemented()
    }

    /// Reverse the order of the values in B in place.
    #[pymethod]
    fn reverse(&self) {
        self.borrow_buf_mut().reverse();
    }

    /// Decode the bytearray using the codec registered for encoding.
    /// 
    ///   encoding
    ///     The encoding with which to decode the bytearray.
    ///   errors
    ///     The error handling scheme to use for the handling of decoding errors.
    ///     The default is 'strict' meaning that decoding errors raise a
    ///     UnicodeDecodeError. Other possible values are 'ignore' and 'replace'
    ///     as well as any other name registered with codecs.register_error that
    ///     can handle UnicodeDecodeErrors.
    #[pymethod]
    fn decode(zelf: PyRef<Self>, args: DecodeArgs, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        bytes_decode(zelf.into_object(), args, vm)
    }

    /// Return state information for pickling.
    #[pymethod(magic)]
    fn reduce_ex(
        zelf: PyRef<Self>,
        _proto: usize,
        vm: &VirtualMachine,
    ) -> (PyTypeRef, PyTupleRef, Option<PyDictRef>) {
        Self::reduce(zelf, vm)
    }

    /// Return state information for pickling.
    #[pymethod(magic)]
    fn reduce(
        zelf: PyRef<Self>,
        vm: &VirtualMachine,
    ) -> (PyTypeRef, PyTupleRef, Option<PyDictRef>) {
        let bytes = PyBytes::from(zelf.borrow_buf().to_vec()).into_pyobject(vm);
        (
            zelf.as_object().clone_class(),
            PyTupleRef::with_elements(vec![bytes], &vm.ctx),
            zelf.as_object().dict(),
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
        Ok(zelf.inner().cmp(other, op, vm))
    }
}

impl AsBuffer for PyByteArray {
    fn get_buffer(zelf: &PyRef<Self>, _vm: &VirtualMachine) -> PyResult<Box<dyn PyBuffer>> {
        zelf.exports.fetch_add(1);
        let buf = ByteArrayBuffer {
            bytearray: zelf.clone(),
            options: BufferOptions {
                readonly: false,
                len: zelf.len(),
                ..Default::default()
            },
        };
        Ok(Box::new(buf))
    }
}

#[derive(Debug)]
struct ByteArrayBuffer {
    bytearray: PyByteArrayRef,
    options: BufferOptions,
}

impl PyBuffer for ByteArrayBuffer {
    fn obj_bytes(&self) -> BorrowedValue<[u8]> {
        self.bytearray.borrow_buf().into()
    }

    fn obj_bytes_mut(&self) -> BorrowedValueMut<[u8]> {
        PyRwLockWriteGuard::map(self.bytearray.inner_mut(), |inner| &mut *inner.elements).into()
    }

    fn release(&self) {
        self.bytearray.exports.fetch_sub(1);
    }

    fn get_options(&self) -> &BufferOptions {
        &self.options
    }
}

impl<'a> ResizeGuard<'a> for PyByteArray {
    type Resizable = PyRwLockWriteGuard<'a, PyBytesInner>;

    fn try_resizable(&'a self, vm: &VirtualMachine) -> PyResult<Self::Resizable> {
        let w = self.inner.upgradable_read();
        if self.exports.load() == 0 {
            Ok(parking_lot::lock_api::RwLockUpgradableReadGuard::upgrade(w))
        } else {
            Err(vm
                .new_buffer_error("Existing exports of data: object cannot be re-sized".to_owned()))
        }
    }
}

impl Unhashable for PyByteArray {}

impl Iterable for PyByteArray {
    fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        Ok(PyByteArrayIterator {
            position: AtomicCell::new(0),
            bytearray: zelf,
        }
        .into_object(vm))
    }
}

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
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.bytearray_iterator_type
    }
}

#[pyimpl(with(PyIter))]
impl PyByteArrayIterator {}
impl PyIter for PyByteArrayIterator {
    fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        let pos = zelf.position.fetch_add(1);
        if let Some(&ret) = zelf.bytearray.borrow_buf().get(pos) {
            Ok(ret.into_pyobject(vm))
        } else {
            Err(vm.new_stop_iteration())
        }
    }
}

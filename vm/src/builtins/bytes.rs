use super::dict::PyDictRef;
use super::int::PyIntRef;
use super::pystr::PyStrRef;
use super::pytype::PyTypeRef;
use crate::anystr::{self, AnyStr};
use crate::buffer::{BufferOptions, PyBuffer};
use crate::builtins::tuple::PyTupleRef;
use crate::bytesinner::{
    bytes_decode, ByteInnerFindOptions, ByteInnerNewOptions, ByteInnerPaddingOptions,
    ByteInnerSplitOptions, ByteInnerTranslateOptions, DecodeArgs, PyBytesInner,
};
use crate::byteslike::ArgBytesLike;
use crate::common::hash::PyHash;
use crate::function::{OptionalArg, OptionalOption};
use crate::slots::{
    AsBuffer, Callable, Comparable, Hashable, Iterable, PyComparisonOp, PyIter, SlotConstructor,
};
use crate::utils::Either;
use crate::vm::VirtualMachine;
use crate::{
    IdProtocol, IntoPyObject, IntoPyResult, PyClassImpl, PyComparisonValue, PyContext, PyIterable,
    PyObjectRef, PyRef, PyResult, PyValue, TryFromBorrowedObject, TypeProtocol,
};
use bstr::ByteSlice;
use crossbeam_utils::atomic::AtomicCell;
use rustpython_common::borrow::{BorrowedValue, BorrowedValueMut};
use std::mem::size_of;
use std::ops::Deref;

/// "bytes(iterable_of_ints) -> bytes\n\
/// bytes(string, encoding[, errors]) -> bytes\n\
/// bytes(bytes_or_buffer) -> immutable copy of bytes_or_buffer\n\
/// bytes(int) -> bytes object of size given by the parameter initialized with null bytes\n\
/// bytes() -> empty bytes object\n\nConstruct an immutable array of bytes from:\n  \
/// - an iterable yielding integers in range(256)\n  \
/// - a text string encoded using the specified encoding\n  \
/// - any object implementing the buffer API.\n  \
/// - an integer";
#[pyclass(module = false, name = "bytes")]
#[derive(Clone, Debug)]
pub struct PyBytes {
    inner: PyBytesInner,
}

pub type PyBytesRef = PyRef<PyBytes>;

impl From<Vec<u8>> for PyBytes {
    fn from(elements: Vec<u8>) -> Self {
        Self {
            inner: PyBytesInner { elements },
        }
    }
}

impl From<PyBytesInner> for PyBytes {
    fn from(inner: PyBytesInner) -> Self {
        Self { inner }
    }
}

impl IntoPyObject for Vec<u8> {
    fn into_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_bytes(self)
    }
}

impl Deref for PyBytes {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.inner.elements
    }
}

impl AsRef<[u8]> for PyBytes {
    fn as_ref(&self) -> &[u8] {
        &self.inner.elements
    }
}
impl AsRef<[u8]> for PyBytesRef {
    fn as_ref(&self) -> &[u8] {
        &self.inner.elements
    }
}

impl PyValue for PyBytes {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.bytes_type
    }
}

pub(crate) fn init(context: &PyContext) {
    PyBytes::extend_class(context, &context.types.bytes_type);
    let bytes_type = &context.types.bytes_type;
    extend_class!(context, bytes_type, {
        "maketrans" => context.new_method("maketrans", bytes_type.clone(), PyBytesInner::maketrans),
    });
    PyBytesIterator::extend_class(context, &context.types.bytes_iterator_type);
}

impl SlotConstructor for PyBytes {
    type Args = ByteInnerNewOptions;

    fn py_new(cls: PyTypeRef, options: Self::Args, vm: &VirtualMachine) -> PyResult {
        options.get_bytes(cls, vm).into_pyresult(vm)
    }
}

#[pyimpl(
    flags(BASETYPE),
    with(Hashable, Comparable, AsBuffer, Iterable, SlotConstructor)
)]
impl PyBytes {
    /// Return repr(self).
    #[pymethod(magic)]
    pub(crate) fn repr(&self) -> String {
        self.inner.repr("", "")
    }

    /// Return len(self).
    #[pymethod(magic)]
    #[inline]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        &self.inner.elements
    }

    #[pymethod(magic)]
    fn bytes(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyRef<Self> {
        if zelf.is(&vm.ctx.types.bytes_type) {
            zelf
        } else {
            PyBytes::from(zelf.inner.clone()).into_ref(vm)
        }
    }

    /// Size of object in memory, in bytes.
    #[pymethod(magic)]
    fn sizeof(&self) -> usize {
        size_of::<Self>() + self.inner.elements.len() * size_of::<u8>()
    }

    /// Return self+value.
    #[pymethod(magic)]
    fn add(&self, other: ArgBytesLike, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_bytes(self.inner.add(&*other.borrow_buf()))
    }

    /// Return key in self.
    #[pymethod(magic)]
    fn contains(
        &self,
        needle: Either<PyBytesInner, PyIntRef>,
        vm: &VirtualMachine,
    ) -> PyResult<bool> {
        self.inner.contains(needle, vm)
    }

    /// Return self[key].
    #[pymethod(magic)]
    fn getitem(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.inner.getitem("byte", needle, vm) // byte != Self::NAME
    }

    /// B.isalnum() -> bool
    /// 
    /// Return True if all characters in B are alphanumeric
    /// and there is at least one character in B, False otherwise.
    #[pymethod]
    fn isalnum(&self) -> bool {
        self.inner.isalnum()
    }

    /// B.isalpha() -> bool
    /// 
    /// Return True if all characters in B are alphabetic
    /// and there is at least one character in B, False otherwise.
    #[pymethod]
    fn isalpha(&self) -> bool {
        self.inner.isalpha()
    }

    /// B.isascii() -> bool
    /// 
    /// Return True if B is empty or all characters in B are ASCII,
    /// False otherwise.
    #[pymethod]
    fn isascii(&self) -> bool {
        self.inner.isascii()
    }

    /// B.isdigit() -> bool
    /// 
    /// Return True if all characters in B are digits
    /// and there is at least one character in B, False otherwise.
    #[pymethod]
    fn isdigit(&self) -> bool {
        self.inner.isdigit()
    }

    /// B.islower() -> bool
    /// 
    /// Return True if all cased characters in B are lowercase and there is
    /// at least one cased character in B, False otherwise.
    #[pymethod]
    fn islower(&self) -> bool {
        self.inner.islower()
    }

    /// B.isspace() -> bool
    /// 
    /// Return True if all characters in B are whitespace
    /// and there is at least one character in B, False otherwise.
    #[pymethod]
    fn isspace(&self) -> bool {
        self.inner.isspace()
    }

    /// B.isupper() -> bool
    /// 
    /// Return True if all cased characters in B are uppercase and there is
    /// at least one cased character in B, False otherwise.
    #[pymethod]
    fn isupper(&self) -> bool {
        self.inner.isupper()
    }

    /// B.istitle() -> bool
    /// 
    /// Return True if B is a titlecased string and there is at least one
    /// character in B, i.e. uppercase characters may only follow uncased
    /// characters and lowercase characters only cased ones. Return False
    /// otherwise.
    #[pymethod]
    fn istitle(&self) -> bool {
        self.inner.istitle()
    }

    /// B.lower() -> copy of B
    /// 
    /// Return a copy of B with all ASCII characters converted to lowercase.
    #[pymethod]
    fn lower(&self) -> Self {
        self.inner.lower().into()
    }

    /// B.upper() -> copy of B
    /// 
    /// Return a copy of B with all ASCII characters converted to uppercase.
    #[pymethod]
    fn upper(&self) -> Self {
        self.inner.upper().into()
    }

    /// B.capitalize() -> copy of B
    /// 
    /// Return a copy of B with only its first character capitalized (ASCII)
    /// and the rest lower-cased.
    #[pymethod]
    fn capitalize(&self) -> Self {
        self.inner.capitalize().into()
    }

    /// B.swapcase() -> copy of B
    /// 
    /// Return a copy of B with uppercase ASCII characters converted
    /// to lowercase ASCII and vice versa.
    #[pymethod]
    fn swapcase(&self) -> Self {
        self.inner.swapcase().into()
    }

    /// Create a str of hexadecimal numbers from a bytes object.
    /// 
    /// sep
    ///   An optional single character or byte to separate hex bytes.
    /// bytes_per_sep
    ///   How many bytes between separators.  Positive values count from the
    ///   right, negative values count from the left.
    /// 
    /// Example:
    /// >>> value = b'\xb9\x01\xef'
    /// >>> value.hex()
    /// 'b901ef'
    /// >>> value.hex(':')
    /// 'b9:01:ef'
    /// >>> value.hex(':', 2)
    /// 'b9:01ef'
    /// >>> value.hex(':', -2)
    /// 'b901:ef'
    #[pymethod]
    pub(crate) fn hex(
        &self,
        sep: OptionalArg<Either<PyStrRef, PyBytesRef>>,
        bytes_per_sep: OptionalArg<isize>,
        vm: &VirtualMachine,
    ) -> PyResult<String> {
        self.inner.hex(sep, bytes_per_sep, vm)
    }

    /// Create a bytes object from a string of hexadecimal numbers.
    /// 
    /// Spaces between two numbers are accepted.
    /// Example: bytes.fromhex('B9 01EF') -> b'\\xb9\\x01\\xef'.
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
    fn center(&self, options: ByteInnerPaddingOptions, vm: &VirtualMachine) -> PyResult<PyBytes> {
        Ok(self.inner.center(options, vm)?.into())
    }

    /// Return a left-justified string of length width.
    /// 
    /// Padding is done using the specified fill character.
    #[pymethod]
    fn ljust(&self, options: ByteInnerPaddingOptions, vm: &VirtualMachine) -> PyResult<PyBytes> {
        Ok(self.inner.ljust(options, vm)?.into())
    }

    /// Return a right-justified string of length width.
    /// 
    /// Padding is done using the specified fill character.
    #[pymethod]
    fn rjust(&self, options: ByteInnerPaddingOptions, vm: &VirtualMachine) -> PyResult<PyBytes> {
        Ok(self.inner.rjust(options, vm)?.into())
    }

    /// B.count(sub[, start[, end]]) -> int
    /// 
    /// Return the number of non-overlapping occurrences of subsection sub in
    /// bytes B[start:end].  Optional arguments start and end are interpreted
    /// as in slice notation.
    #[pymethod]
    fn count(&self, options: ByteInnerFindOptions, vm: &VirtualMachine) -> PyResult<usize> {
        self.inner.count(options, vm)
    }

    /// Concatenate any number of bytes objects.
    /// 
    /// The bytes whose method is called is inserted in between each pair.
    /// 
    /// The result is returned as a new bytes object.
    /// 
    /// Example: b'.'.join([b'ab', b'pq', b'rs']) -> b'ab.pq.rs'.
    #[pymethod]
    fn join(&self, iter: PyIterable<PyBytesInner>, vm: &VirtualMachine) -> PyResult<PyBytes> {
        Ok(self.inner.join(iter, vm)?.into())
    }

    /// B.endswith(suffix[, start[, end]]) -> bool
    /// 
    /// Return True if B ends with the specified suffix, False otherwise.
    /// With optional start, test B beginning at that position.
    /// With optional end, stop comparing B at that position.
    /// suffix can also be a tuple of bytes to try.
    #[pymethod]
    fn endswith(&self, options: anystr::StartsEndsWithArgs, vm: &VirtualMachine) -> PyResult<bool> {
        self.inner.elements[..].py_startsendswith(
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
        self.inner.elements[..].py_startsendswith(
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
        let index = self.inner.find(options, |h, n| h.find(n), vm)?;
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
        let index = self.inner.find(options, |h, n| h.find(n), vm)?;
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
        let index = self.inner.find(options, |h, n| h.rfind(n), vm)?;
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
        let index = self.inner.find(options, |h, n| h.rfind(n), vm)?;
        index.ok_or_else(|| vm.new_value_error("substring not found".to_owned()))
    }

    /// Return a copy with each character mapped by the given translation table.
    /// 
    /// table
    ///   Translation table, which must be a bytes object of length 256.
    /// 
    /// All characters occurring in the optional argument delete are removed.
    /// The remaining characters are mapped through the given translation table.
    #[pymethod]
    fn translate(
        &self,
        options: ByteInnerTranslateOptions,
        vm: &VirtualMachine,
    ) -> PyResult<PyBytes> {
        Ok(self.inner.translate(options, vm)?.into())
    }

    /// Strip leading and trailing bytes contained in the argument.
    /// 
    /// If the argument is omitted or None, strip leading and trailing ASCII whitespace.
    #[pymethod]
    fn strip(&self, chars: OptionalOption<PyBytesInner>) -> Self {
        self.inner.strip(chars).into()
    }

    /// Strip leading bytes contained in the argument.
    /// 
    /// If the argument is omitted or None, strip leading  ASCII whitespace.
    #[pymethod]
    fn lstrip(&self, chars: OptionalOption<PyBytesInner>) -> Self {
        self.inner.lstrip(chars).into()
    }

    /// Strip trailing bytes contained in the argument.
    /// 
    /// If the argument is omitted or None, strip trailing ASCII whitespace.
    #[pymethod]
    fn rstrip(&self, chars: OptionalOption<PyBytesInner>) -> Self {
        self.inner.rstrip(chars).into()
    }

    /// removeprefix($self, prefix, /)
    ///
    ///
    /// Return a bytes object with the given prefix string removed if present.
    ///
    /// If the bytes starts with the prefix string, return string[len(prefix):]
    /// Otherwise, return a copy of the original bytes.
    #[pymethod]
    fn removeprefix(&self, prefix: PyBytesInner) -> Self {
        self.inner.removeprefix(prefix).into()
    }

    /// removesuffix(self, prefix, /)
    ///
    ///
    /// Return a bytes object with the given suffix string removed if present.
    ///
    /// If the bytes ends with the suffix string, return string[:len(suffix)]
    /// Otherwise, return a copy of the original bytes.
    #[pymethod]
    fn removesuffix(&self, suffix: PyBytesInner) -> Self {
        self.inner.removesuffix(suffix).into()
    }

    /// Return a list of the sections in the bytes, using sep as the delimiter.
    /// 
    /// sep
    ///   The delimiter according which to split the bytes.
    ///   None (the default value) means split on ASCII whitespace characters
    ///   (space, tab, return, newline, formfeed, vertical tab).
    /// maxsplit
    ///   Maximum number of splits to do.
    ///   -1 (the default value) means no limit.
    #[pymethod]
    fn split(&self, options: ByteInnerSplitOptions, vm: &VirtualMachine) -> PyResult {
        self.inner
            .split(options, |s, vm| vm.ctx.new_bytes(s.to_vec()), vm)
    }

    /// Return a list of the sections in the bytes, using sep as the delimiter.
    /// 
    /// sep
    ///   The delimiter according which to split the bytes.
    ///   None (the default value) means split on ASCII whitespace characters
    ///   (space, tab, return, newline, formfeed, vertical tab).
    /// maxsplit
    ///   Maximum number of splits to do.
    ///   -1 (the default value) means no limit.
    /// 
    /// Splitting is done starting at the end of the bytes and working to the front.
    #[pymethod]
    fn rsplit(&self, options: ByteInnerSplitOptions, vm: &VirtualMachine) -> PyResult {
        self.inner
            .rsplit(options, |s, vm| vm.ctx.new_bytes(s.to_vec()), vm)
    }

    /// Partition the bytes into three parts using the given separator.
    /// 
    /// This will search for the separator sep in the bytes. If the separator is found,
    /// returns a 3-tuple containing the part before the separator, the separator
    /// itself, and the part after it.
    /// 
    /// If the separator is not found, returns a 3-tuple containing the original bytes
    /// object and two empty bytes objects.
    #[pymethod]
    fn partition(&self, sep: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let sub = PyBytesInner::try_from_borrowed_object(vm, &sep)?;
        let (front, has_mid, back) = self.inner.partition(&sub, vm)?;
        Ok(vm.ctx.new_tuple(vec![
            vm.ctx.new_bytes(front),
            if has_mid {
                sep
            } else {
                vm.ctx.new_bytes(Vec::new())
            },
            vm.ctx.new_bytes(back),
        ]))
    }

    /// Partition the bytes into three parts using the given separator.
    /// 
    /// This will search for the separator sep in the bytes, starting at the end. If
    /// the separator is found, returns a 3-tuple containing the part before the
    /// separator, the separator itself, and the part after it.
    /// 
    /// If the separator is not found, returns a 3-tuple containing two empty bytes
    /// objects and the original bytes object.
    #[pymethod]
    fn rpartition(&self, sep: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let sub = PyBytesInner::try_from_borrowed_object(vm, &sep)?;
        let (back, has_mid, front) = self.inner.rpartition(&sub, vm)?;
        Ok(vm.ctx.new_tuple(vec![
            vm.ctx.new_bytes(front),
            if has_mid {
                sep
            } else {
                vm.ctx.new_bytes(Vec::new())
            },
            vm.ctx.new_bytes(back),
        ]))
    }

    /// Return a copy where all tab characters are expanded using spaces.
    /// 
    /// If tabsize is not given, a tab size of 8 characters is assumed.
    #[pymethod]
    fn expandtabs(&self, options: anystr::ExpandTabsArgs) -> Self {
        self.inner.expandtabs(options).into()
    }

    /// Return a list of the lines in the bytes, breaking at line boundaries.
    /// 
    /// Line breaks are not included in the resulting list unless keepends is given and
    /// true.
    #[pymethod]
    fn splitlines(&self, options: anystr::SplitLinesArgs, vm: &VirtualMachine) -> PyObjectRef {
        let lines = self
            .inner
            .splitlines(options, |x| vm.ctx.new_bytes(x.to_vec()));
        vm.ctx.new_list(lines)
    }

    /// Pad a numeric string with zeros on the left, to fill a field of the given width.
    /// 
    /// The original string is never truncated.
    #[pymethod]
    fn zfill(&self, width: isize) -> Self {
        self.inner.zfill(width).into()
    }

    /// Return a copy with all occurrences of substring old replaced by new.
    /// 
    /// count
    ///   Maximum number of occurrences to replace.
    ///   -1 (the default value) means replace all occurrences.
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
    ) -> PyResult<PyBytes> {
        Ok(self.inner.replace(old, new, count, vm)?.into())
    }

    /// B.title() -> copy of B
    /// 
    /// Return a titlecased version of B, i.e. ASCII words start with uppercase
    /// characters, all remaining cased characters have lowercase.
    #[pymethod]
    fn title(&self) -> Self {
        self.inner.title().into()
    }

    /// Return value*self.
    #[pymethod(name = "__rmul__")]
    #[pymethod(magic)]
    fn mul(zelf: PyRef<Self>, value: isize, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        if value == 1 && zelf.class().is(&vm.ctx.types.bytes_type) {
            // Special case: when some `bytes` is multiplied by `1`,
            // nothing really happens, we need to return an object itself
            // with the same `id()` to be compatible with CPython.
            // This only works for `bytes` itself, not its subclasses.
            return Ok(zelf);
        }
        // todo: map err to overflow.
        vm.check_repeat_or_memory_error(zelf.inner.len(), value)
            .map(|value| {
                let bytes: PyBytes = zelf.inner.repeat(value).into();
                bytes.into_ref(vm)
            })
            // see issue 45044 on b.p.o.
            .map_err(|_| vm.new_overflow_error("repeated bytes are too long".to_owned()))
    }

    /// Return self%value.
    #[pymethod(name = "__mod__")]
    fn mod_(&self, values: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyBytes> {
        let formatted = self.inner.cformat(values, vm)?;
        Ok(formatted.into())
    }

    /// Return value%self.
    #[pymethod(magic)]
    fn rmod(&self, _values: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.not_implemented()
    }

    /// Return a string decoded from the given bytes.
    /// Default encoding is 'utf-8'.
    /// Default errors is 'strict', meaning that encoding errors raise a UnicodeError.
    /// Other possible values are 'ignore', 'replace'
    /// For a list of possible encodings,
    /// see https://docs.python.org/3/library/codecs.html#standard-encodings
    /// currently, only 'utf-8' and 'ascii' emplemented
    #[pymethod]
    fn decode(zelf: PyRef<Self>, args: DecodeArgs, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        bytes_decode(zelf.into_object(), args, vm)
    }

    /// 
    #[pymethod(magic)]
    fn getnewargs(&self, vm: &VirtualMachine) -> PyTupleRef {
        let param: Vec<PyObjectRef> = self
            .inner
            .elements
            .iter()
            .map(|x| x.into_pyobject(vm))
            .collect();
        PyTupleRef::with_elements(param, &vm.ctx)
    }

    /// Helper for pickle.
    #[pymethod(magic)]
    fn reduce_ex(
        zelf: PyRef<Self>,
        _proto: usize,
        vm: &VirtualMachine,
    ) -> (PyTypeRef, PyTupleRef, Option<PyDictRef>) {
        Self::reduce(zelf, vm)
    }

    /// Helper for pickle.
    #[pymethod(magic)]
    fn reduce(
        zelf: PyRef<Self>,
        vm: &VirtualMachine,
    ) -> (PyTypeRef, PyTupleRef, Option<PyDictRef>) {
        let bytes = PyBytes::from(zelf.inner.elements.clone()).into_pyobject(vm);
        (
            zelf.as_object().clone_class(),
            PyTupleRef::with_elements(vec![bytes], &vm.ctx),
            zelf.as_object().dict(),
        )
    }
}

impl AsBuffer for PyBytes {
    fn get_buffer(zelf: &PyRef<Self>, _vm: &VirtualMachine) -> PyResult<Box<dyn PyBuffer>> {
        let buf = BytesBuffer {
            bytes: zelf.clone(),
            options: BufferOptions {
                len: zelf.len(),
                ..Default::default()
            },
        };
        Ok(Box::new(buf))
    }
}

#[derive(Debug)]
struct BytesBuffer {
    bytes: PyBytesRef,
    options: BufferOptions,
}

impl PyBuffer for BytesBuffer {
    fn obj_bytes(&self) -> BorrowedValue<[u8]> {
        self.bytes.as_bytes().into()
    }

    fn obj_bytes_mut(&self) -> BorrowedValueMut<[u8]> {
        unreachable!("bytes is not mutable")
    }

    fn release(&self) {}

    fn get_options(&self) -> &BufferOptions {
        &self.options
    }
}

impl Hashable for PyBytes {
    fn hash(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyHash> {
        Ok(zelf.inner.hash(vm))
    }
}

impl Comparable for PyBytes {
    fn cmp(
        zelf: &PyRef<Self>,
        other: &PyObjectRef,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        Ok(if let Some(res) = op.identical_optimization(zelf, other) {
            res.into()
        } else if other.isinstance(&vm.ctx.types.memoryview_type)
            && op != PyComparisonOp::Eq
            && op != PyComparisonOp::Ne
        {
            return Err(vm.new_type_error(format!(
                "'{}' not supported between instances of '{}' and '{}'",
                op.operator_token(),
                zelf.class().name(),
                other.class().name()
            )));
        } else {
            zelf.inner.cmp(other, op, vm)
        })
    }
}

impl Iterable for PyBytes {
    fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        Ok(PyBytesIterator {
            position: AtomicCell::new(0),
            bytes: zelf,
        }
        .into_object(vm))
    }
}

#[pyclass(module = false, name = "bytes_iterator")]
#[derive(Debug)]
pub struct PyBytesIterator {
    position: AtomicCell<usize>,
    bytes: PyBytesRef,
}

impl PyValue for PyBytesIterator {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.bytes_iterator_type
    }
}

#[pyimpl(with(PyIter))]
impl PyBytesIterator {}
impl PyIter for PyBytesIterator {
    fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        let pos = zelf.position.fetch_add(1);
        if let Some(&ret) = zelf.bytes.as_bytes().get(pos) {
            Ok(vm.ctx.new_int(ret))
        } else {
            Err(vm.new_stop_iteration())
        }
    }
}

impl TryFromBorrowedObject for PyBytes {
    fn try_from_borrowed_object(vm: &VirtualMachine, obj: &PyObjectRef) -> PyResult<Self> {
        PyBytesInner::try_from_borrowed_object(vm, obj).map(|x| x.into())
    }
}
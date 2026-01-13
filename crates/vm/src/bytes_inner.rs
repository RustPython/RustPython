// spell-checker:ignore unchunked
use crate::{
    AsObject, PyObject, PyObjectRef, PyResult, TryFromBorrowedObject, VirtualMachine,
    anystr::{self, AnyStr, AnyStrContainer, AnyStrWrapper},
    builtins::{
        PyBaseExceptionRef, PyByteArray, PyBytes, PyBytesRef, PyInt, PyIntRef, PyStr, PyStrRef,
        pystr,
    },
    byte::bytes_from_object,
    cformat::cformat_bytes,
    common::hash,
    function::{ArgIterable, Either, OptionalArg, OptionalOption, PyComparisonValue},
    literal::escape::Escape,
    protocol::PyBuffer,
    sequence::{SequenceExt, SequenceMutExt},
    types::PyComparisonOp,
};
use bstr::ByteSlice;
use itertools::Itertools;
use malachite_bigint::BigInt;
use num_traits::ToPrimitive;

const STRING_WITHOUT_ENCODING: &str = "string argument without an encoding";
const ENCODING_WITHOUT_STRING: &str = "encoding without a string argument";

#[derive(Debug, Default, Clone)]
pub struct PyBytesInner {
    pub(super) elements: Vec<u8>,
}

impl From<Vec<u8>> for PyBytesInner {
    fn from(elements: Vec<u8>) -> Self {
        Self { elements }
    }
}

impl<'a> TryFromBorrowedObject<'a> for PyBytesInner {
    fn try_from_borrowed_object(vm: &VirtualMachine, obj: &'a PyObject) -> PyResult<Self> {
        bytes_from_object(vm, obj).map(Self::from)
    }
}

#[derive(FromArgs)]
pub struct ByteInnerNewOptions {
    #[pyarg(any, optional)]
    pub source: OptionalArg<PyObjectRef>,
    #[pyarg(any, optional)]
    pub encoding: OptionalArg<PyStrRef>,
    #[pyarg(any, optional)]
    pub errors: OptionalArg<PyStrRef>,
}

impl ByteInnerNewOptions {
    fn get_value_from_string(
        s: PyStrRef,
        encoding: PyStrRef,
        errors: OptionalArg<PyStrRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyBytesInner> {
        let bytes = pystr::encode_string(s, Some(encoding), errors.into_option(), vm)?;
        Ok(bytes.as_bytes().to_vec().into())
    }

    fn get_value_from_source(source: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyBytesInner> {
        bytes_from_object(vm, &source).map(|x| x.into())
    }

    fn get_value_from_size(size: PyIntRef, vm: &VirtualMachine) -> PyResult<PyBytesInner> {
        let size = size
            .as_bigint()
            .to_isize()
            .ok_or_else(|| vm.new_overflow_error("cannot fit 'int' into an index-sized integer"))?;
        let size = if size < 0 {
            return Err(vm.new_value_error("negative count"));
        } else {
            size as usize
        };
        Ok(vec![0; size].into())
    }

    fn handle_object_fallback(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyBytesInner> {
        match_class!(match obj {
            i @ PyInt => {
                Self::get_value_from_size(i, vm)
            }
            _s @ PyStr => Err(vm.new_type_error(STRING_WITHOUT_ENCODING.to_owned())),
            obj => {
                Self::get_value_from_source(obj, vm)
            }
        })
    }

    pub fn get_bytearray_inner(self, vm: &VirtualMachine) -> PyResult<PyBytesInner> {
        match (self.source, self.encoding, self.errors) {
            (OptionalArg::Present(obj), OptionalArg::Missing, OptionalArg::Missing) => {
                // Try __index__ first to handle int-like objects that might raise custom exceptions
                if let Some(index_result) = obj.try_index_opt(vm) {
                    match index_result {
                        Ok(index) => Self::get_value_from_size(index, vm),
                        Err(e) => {
                            // Only propagate non-TypeError exceptions
                            // TypeError means the object doesn't support __index__, so fall back
                            if e.fast_isinstance(vm.ctx.exceptions.type_error) {
                                // Fall back to treating as buffer-like object
                                Self::handle_object_fallback(obj, vm)
                            } else {
                                // Propagate other exceptions (e.g., ZeroDivisionError)
                                Err(e)
                            }
                        }
                    }
                } else {
                    Self::handle_object_fallback(obj, vm)
                }
            }
            (OptionalArg::Present(obj), OptionalArg::Present(encoding), errors) => {
                if let Ok(s) = obj.downcast::<PyStr>() {
                    Self::get_value_from_string(s, encoding, errors, vm)
                } else {
                    Err(vm.new_type_error(ENCODING_WITHOUT_STRING.to_owned()))
                }
            }
            (OptionalArg::Missing, OptionalArg::Missing, OptionalArg::Missing) => {
                Ok(PyBytesInner::default())
            }
            (OptionalArg::Missing, OptionalArg::Present(_), _) => {
                Err(vm.new_type_error(ENCODING_WITHOUT_STRING.to_owned()))
            }
            (OptionalArg::Missing, _, OptionalArg::Present(_)) => {
                Err(vm.new_type_error("errors without a string argument"))
            }
            (OptionalArg::Present(_), OptionalArg::Missing, OptionalArg::Present(_)) => {
                Err(vm.new_type_error(STRING_WITHOUT_ENCODING.to_owned()))
            }
        }
    }
}

#[derive(FromArgs)]
pub struct ByteInnerFindOptions {
    #[pyarg(positional)]
    sub: Either<PyBytesInner, PyIntRef>,
    #[pyarg(positional, default)]
    start: Option<PyIntRef>,
    #[pyarg(positional, default)]
    end: Option<PyIntRef>,
}

impl ByteInnerFindOptions {
    pub fn get_value(
        self,
        len: usize,
        vm: &VirtualMachine,
    ) -> PyResult<(Vec<u8>, core::ops::Range<usize>)> {
        let sub = match self.sub {
            Either::A(v) => v.elements.to_vec(),
            Either::B(int) => vec![int.as_bigint().byte_or(vm)?],
        };
        let range = anystr::adjust_indices(self.start, self.end, len);
        Ok((sub, range))
    }
}

#[derive(FromArgs)]
pub struct ByteInnerPaddingOptions {
    #[pyarg(positional)]
    width: isize,
    #[pyarg(positional, optional)]
    fillchar: OptionalArg<PyObjectRef>,
}

impl ByteInnerPaddingOptions {
    fn get_value(self, fn_name: &str, vm: &VirtualMachine) -> PyResult<(isize, u8)> {
        let fillchar = if let OptionalArg::Present(v) = self.fillchar {
            try_as_bytes(v.clone(), |bytes| bytes.iter().copied().exactly_one().ok())
                .flatten()
                .ok_or_else(|| {
                    vm.new_type_error(format!(
                        "{}() argument 2 must be a byte string of length 1, not {}",
                        fn_name,
                        v.class().name()
                    ))
                })?
        } else {
            b' ' // default is space
        };

        Ok((self.width, fillchar))
    }
}

#[derive(FromArgs)]
pub struct ByteInnerTranslateOptions {
    #[pyarg(positional)]
    table: Option<PyObjectRef>,
    #[pyarg(any, optional)]
    delete: OptionalArg<PyObjectRef>,
}

impl ByteInnerTranslateOptions {
    pub fn get_value(self, vm: &VirtualMachine) -> PyResult<(Vec<u8>, Vec<u8>)> {
        let table = self.table.map_or_else(
            || Ok((0..=u8::MAX).collect::<Vec<u8>>()),
            |v| {
                let bytes = v
                    .try_into_value::<PyBytesInner>(vm)
                    .ok()
                    .filter(|v| v.elements.len() == 256)
                    .ok_or_else(|| {
                        vm.new_value_error("translation table must be 256 characters long")
                    })?;
                Ok(bytes.elements.to_vec())
            },
        )?;

        let delete = match self.delete {
            OptionalArg::Present(byte) => {
                let byte: PyBytesInner = byte.try_into_value(vm)?;
                byte.elements
            }
            _ => vec![],
        };

        Ok((table, delete))
    }
}

pub type ByteInnerSplitOptions = anystr::SplitArgs<PyBytesInner>;

impl PyBytesInner {
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        &self.elements
    }

    fn new_repr_overflow_error(vm: &VirtualMachine) -> PyBaseExceptionRef {
        vm.new_overflow_error("bytes object is too large to make repr")
    }

    pub fn repr_with_name(&self, class_name: &str, vm: &VirtualMachine) -> PyResult<String> {
        const DECORATION_LEN: isize = 2 + 3; // 2 for (), 3 for b"" => bytearray(b"")
        let escape = crate::literal::escape::AsciiEscape::new_repr(&self.elements);
        let len = escape
            .layout()
            .len
            .and_then(|len| (len as isize).checked_add(DECORATION_LEN + class_name.len() as isize))
            .ok_or_else(|| Self::new_repr_overflow_error(vm))? as usize;
        let mut buf = String::with_capacity(len);
        buf.push_str(class_name);
        buf.push('(');
        escape.bytes_repr().write(&mut buf).unwrap();
        buf.push(')');
        debug_assert_eq!(buf.len(), len);
        Ok(buf)
    }

    pub fn repr_bytes(&self, vm: &VirtualMachine) -> PyResult<String> {
        let escape = crate::literal::escape::AsciiEscape::new_repr(&self.elements);
        let len = 3 + escape
            .layout()
            .len
            .ok_or_else(|| Self::new_repr_overflow_error(vm))?;
        let mut buf = String::with_capacity(len);
        escape.bytes_repr().write(&mut buf).unwrap();
        debug_assert_eq!(buf.len(), len);
        Ok(buf)
    }

    #[inline]
    pub const fn len(&self) -> usize {
        self.elements.len()
    }

    #[inline]
    pub const fn capacity(&self) -> usize {
        self.elements.capacity()
    }

    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    pub fn cmp(
        &self,
        other: &PyObject,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyComparisonValue {
        // TODO: bytes can compare with any object implemented buffer protocol
        // but not memoryview, and not equal if compare with unicode str(PyStr)
        PyComparisonValue::from_option(
            other
                .try_bytes_like(vm, |other| op.eval_ord(self.elements.as_slice().cmp(other)))
                .ok(),
        )
    }

    pub fn hash(&self, vm: &VirtualMachine) -> hash::PyHash {
        vm.state.hash_secret.hash_bytes(&self.elements)
    }

    pub fn add(&self, other: &[u8]) -> Vec<u8> {
        self.elements.py_add(other)
    }

    pub fn contains(&self, needle: Either<Self, PyIntRef>, vm: &VirtualMachine) -> PyResult<bool> {
        Ok(match needle {
            Either::A(byte) => self.elements.contains_str(byte.elements.as_slice()),
            Either::B(int) => self.elements.contains(&int.as_bigint().byte_or(vm)?),
        })
    }

    pub fn isalnum(&self) -> bool {
        !self.elements.is_empty()
            && self
                .elements
                .iter()
                .all(|x| char::from(*x).is_alphanumeric())
    }

    pub fn isalpha(&self) -> bool {
        !self.elements.is_empty() && self.elements.iter().all(|x| char::from(*x).is_alphabetic())
    }

    pub fn isascii(&self) -> bool {
        self.elements.iter().all(|x| char::from(*x).is_ascii())
    }

    pub fn isdigit(&self) -> bool {
        !self.elements.is_empty()
            && self
                .elements
                .iter()
                .all(|x| char::from(*x).is_ascii_digit())
    }

    pub fn islower(&self) -> bool {
        self.elements.py_islower()
    }

    pub fn isupper(&self) -> bool {
        self.elements.py_isupper()
    }

    pub fn isspace(&self) -> bool {
        !self.elements.is_empty()
            && self
                .elements
                .iter()
                .all(|x| char::from(*x).is_ascii_whitespace())
    }

    pub fn istitle(&self) -> bool {
        if self.elements.is_empty() {
            return false;
        }

        let mut iter = self.elements.iter().peekable();
        let mut prev_cased = false;

        while let Some(c) = iter.next() {
            let current = char::from(*c);
            let next = if let Some(k) = iter.peek() {
                char::from(**k)
            } else if current.is_uppercase() {
                return !prev_cased;
            } else {
                return prev_cased;
            };

            let is_cased = current.to_uppercase().next().unwrap() != current
                || current.to_lowercase().next().unwrap() != current;
            if (is_cased && next.is_uppercase() && !prev_cased)
                || (!is_cased && next.is_lowercase())
            {
                return false;
            }

            prev_cased = is_cased;
        }

        true
    }

    pub fn lower(&self) -> Vec<u8> {
        self.elements.to_ascii_lowercase()
    }

    pub fn upper(&self) -> Vec<u8> {
        self.elements.to_ascii_uppercase()
    }

    pub fn capitalize(&self) -> Vec<u8> {
        let mut new: Vec<u8> = Vec::with_capacity(self.elements.len());
        if let Some((first, second)) = self.elements.split_first() {
            new.push(first.to_ascii_uppercase());
            second.iter().for_each(|x| new.push(x.to_ascii_lowercase()));
        }
        new
    }

    pub fn swapcase(&self) -> Vec<u8> {
        let mut new: Vec<u8> = Vec::with_capacity(self.elements.len());
        for w in &self.elements {
            match w {
                b'A'..=b'Z' => new.push(w.to_ascii_lowercase()),
                b'a'..=b'z' => new.push(w.to_ascii_uppercase()),
                x => new.push(*x),
            }
        }
        new
    }

    pub fn hex(
        &self,
        sep: OptionalArg<Either<PyStrRef, PyBytesRef>>,
        bytes_per_sep: OptionalArg<isize>,
        vm: &VirtualMachine,
    ) -> PyResult<String> {
        bytes_to_hex(self.elements.as_slice(), sep, bytes_per_sep, vm)
    }

    pub fn fromhex(bytes: &[u8], vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        let mut iter = bytes.iter().enumerate();
        let mut result: Vec<u8> = Vec::with_capacity(bytes.len() / 2);
        // None means odd number of hex digits, Some(i) means invalid char at position i
        let invalid_char: Option<usize> = loop {
            let (i, &b) = match iter.next() {
                Some(val) => val,
                None => {
                    return Ok(result);
                }
            };

            if is_py_ascii_whitespace(b) {
                continue;
            }

            let top = match b {
                b'0'..=b'9' => b - b'0',
                b'a'..=b'f' => 10 + b - b'a',
                b'A'..=b'F' => 10 + b - b'A',
                _ => break Some(i),
            };

            let (i, b) = match iter.next() {
                Some(val) => val,
                None => break None, // odd number of hex digits
            };

            let bot = match b {
                b'0'..=b'9' => b - b'0',
                b'a'..=b'f' => 10 + b - b'a',
                b'A'..=b'F' => 10 + b - b'A',
                _ => break Some(i),
            };

            result.push((top << 4) + bot);
        };

        match invalid_char {
            None => Err(vm.new_value_error(
                "fromhex() arg must contain an even number of hexadecimal digits".to_owned(),
            )),
            Some(i) => Err(vm.new_value_error(format!(
                "non-hexadecimal number found in fromhex() arg at position {i}"
            ))),
        }
    }

    /// Parse hex string from str or bytes-like object
    pub fn fromhex_object(string: PyObjectRef, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        if let Some(s) = string.downcast_ref::<PyStr>() {
            Self::fromhex(s.as_bytes(), vm)
        } else if let Ok(buffer) = PyBuffer::try_from_borrowed_object(vm, &string) {
            let borrowed = buffer.as_contiguous().ok_or_else(|| {
                vm.new_buffer_error("fromhex() requires a contiguous buffer".to_owned())
            })?;
            Self::fromhex(&borrowed, vm)
        } else {
            Err(vm.new_type_error(format!(
                "fromhex() argument must be str or bytes-like, not {}",
                string.class().name()
            )))
        }
    }

    #[inline]
    fn _pad(
        &self,
        options: ByteInnerPaddingOptions,
        pad: fn(&[u8], usize, u8, usize) -> Vec<u8>,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<u8>> {
        let (width, fillchar) = options.get_value("center", vm)?;
        let len = self.len();
        Ok(if len as isize >= width {
            Vec::from(&self.elements[..])
        } else {
            pad(&self.elements, width as usize, fillchar, len)
        })
    }

    pub fn center(
        &self,
        options: ByteInnerPaddingOptions,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<u8>> {
        self._pad(options, AnyStr::py_center, vm)
    }

    pub fn ljust(
        &self,
        options: ByteInnerPaddingOptions,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<u8>> {
        self._pad(options, AnyStr::py_ljust, vm)
    }

    pub fn rjust(
        &self,
        options: ByteInnerPaddingOptions,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<u8>> {
        self._pad(options, AnyStr::py_rjust, vm)
    }

    pub fn count(&self, options: ByteInnerFindOptions, vm: &VirtualMachine) -> PyResult<usize> {
        let (needle, range) = options.get_value(self.elements.len(), vm)?;
        Ok(self
            .elements
            .py_count(needle.as_slice(), range, |h, n| h.find_iter(n).count()))
    }

    pub fn join(&self, iterable: ArgIterable<Self>, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        let iter = iterable.iter(vm)?;
        self.elements.py_join(iter)
    }

    #[inline]
    pub fn find<F>(
        &self,
        options: ByteInnerFindOptions,
        find: F,
        vm: &VirtualMachine,
    ) -> PyResult<Option<usize>>
    where
        F: Fn(&[u8], &[u8]) -> Option<usize>,
    {
        let (needle, range) = options.get_value(self.elements.len(), vm)?;
        Ok(self.elements.py_find(&needle, range, find))
    }

    pub fn maketrans(from: Self, to: Self, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        if from.len() != to.len() {
            return Err(vm.new_value_error("the two maketrans arguments must have equal length"));
        }
        let mut res = vec![];

        for i in 0..=u8::MAX {
            res.push(if let Some(position) = from.elements.find_byte(i) {
                to.elements[position]
            } else {
                i
            });
        }

        Ok(res)
    }

    pub fn translate(
        &self,
        options: ByteInnerTranslateOptions,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<u8>> {
        let (table, delete) = options.get_value(vm)?;

        let mut res = if delete.is_empty() {
            Vec::with_capacity(self.elements.len())
        } else {
            Vec::new()
        };

        for i in &self.elements {
            if !delete.contains(i) {
                res.push(table[*i as usize]);
            }
        }

        Ok(res)
    }

    pub fn strip(&self, chars: OptionalOption<Self>) -> Vec<u8> {
        self.elements
            .py_strip(
                chars,
                |s, chars| s.trim_with(|c| chars.contains(&(c as u8))),
                |s| s.trim(),
            )
            .to_vec()
    }

    pub fn lstrip(&self, chars: OptionalOption<Self>) -> &[u8] {
        self.elements.py_strip(
            chars,
            |s, chars| s.trim_start_with(|c| chars.contains(&(c as u8))),
            |s| s.trim_start(),
        )
    }

    pub fn rstrip(&self, chars: OptionalOption<Self>) -> &[u8] {
        self.elements.py_strip(
            chars,
            |s, chars| s.trim_end_with(|c| chars.contains(&(c as u8))),
            |s| s.trim_end(),
        )
    }

    // new in Python 3.9
    pub fn removeprefix(&self, prefix: Self) -> Vec<u8> {
        self.elements
            .py_removeprefix(&prefix.elements, prefix.elements.len(), |s, p| {
                s.starts_with(p)
            })
            .to_vec()
    }

    // new in Python 3.9
    pub fn removesuffix(&self, suffix: Self) -> Vec<u8> {
        self.elements
            .py_removesuffix(&suffix.elements, suffix.elements.len(), |s, p| {
                s.ends_with(p)
            })
            .to_vec()
    }

    pub fn split<F>(
        &self,
        options: ByteInnerSplitOptions,
        convert: F,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<PyObjectRef>>
    where
        F: Fn(&[u8], &VirtualMachine) -> PyObjectRef,
    {
        let elements = self.elements.py_split(
            options,
            vm,
            || convert(&self.elements, vm),
            |v, s, vm| v.split_str(s).map(|v| convert(v, vm)).collect(),
            |v, s, n, vm| v.splitn_str(n, s).map(|v| convert(v, vm)).collect(),
            |v, n, vm| v.py_split_whitespace(n, |v| convert(v, vm)),
        )?;
        Ok(elements)
    }

    pub fn rsplit<F>(
        &self,
        options: ByteInnerSplitOptions,
        convert: F,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<PyObjectRef>>
    where
        F: Fn(&[u8], &VirtualMachine) -> PyObjectRef,
    {
        let mut elements = self.elements.py_split(
            options,
            vm,
            || convert(&self.elements, vm),
            |v, s, vm| v.rsplit_str(s).map(|v| convert(v, vm)).collect(),
            |v, s, n, vm| v.rsplitn_str(n, s).map(|v| convert(v, vm)).collect(),
            |v, n, vm| v.py_rsplit_whitespace(n, |v| convert(v, vm)),
        )?;
        elements.reverse();
        Ok(elements)
    }

    pub fn partition(&self, sub: &Self, vm: &VirtualMachine) -> PyResult<(Vec<u8>, bool, Vec<u8>)> {
        self.elements.py_partition(
            &sub.elements,
            || self.elements.splitn_str(2, &sub.elements),
            vm,
        )
    }

    pub fn rpartition(
        &self,
        sub: &Self,
        vm: &VirtualMachine,
    ) -> PyResult<(Vec<u8>, bool, Vec<u8>)> {
        self.elements.py_partition(
            &sub.elements,
            || self.elements.rsplitn_str(2, &sub.elements),
            vm,
        )
    }

    pub fn expandtabs(&self, options: anystr::ExpandTabsArgs) -> Vec<u8> {
        let tabsize = options.tabsize();
        let mut counter: usize = 0;
        let mut res = vec![];

        if tabsize == 0 {
            return self
                .elements
                .iter()
                .copied()
                .filter(|x| *x != b'\t')
                .collect();
        }

        for i in &self.elements {
            if *i == b'\t' {
                let len = tabsize - counter % tabsize;
                res.extend_from_slice(&vec![b' '; len]);
                counter += len;
            } else {
                res.push(*i);
                if *i == b'\r' || *i == b'\n' {
                    counter = 0;
                } else {
                    counter += 1;
                }
            }
        }

        res
    }

    pub fn splitlines<FW, W>(&self, options: anystr::SplitLinesArgs, into_wrapper: FW) -> Vec<W>
    where
        FW: Fn(&[u8]) -> W,
    {
        self.elements.py_bytes_splitlines(options, into_wrapper)
    }

    pub fn zfill(&self, width: isize) -> Vec<u8> {
        self.elements.py_zfill(width)
    }

    // len(self)>=1, from="", len(to)>=1, max_count>=1
    fn replace_interleave(&self, to: Self, max_count: Option<usize>) -> Vec<u8> {
        let place_count = self.elements.len() + 1;
        let count = max_count.map_or(place_count, |v| core::cmp::min(v, place_count)) - 1;
        let capacity = self.elements.len() + count * to.len();
        let mut result = Vec::with_capacity(capacity);
        let to_slice = to.elements.as_slice();
        result.extend_from_slice(to_slice);
        for c in &self.elements[..count] {
            result.push(*c);
            result.extend_from_slice(to_slice);
        }
        result.extend_from_slice(&self.elements[count..]);
        result
    }

    fn replace_delete(&self, from: Self, max_count: Option<usize>) -> Vec<u8> {
        let count = count_substring(
            self.elements.as_slice(),
            from.elements.as_slice(),
            max_count,
        );
        if count == 0 {
            // no matches
            return self.elements.clone();
        }

        let result_len = self.len() - (count * from.len());
        debug_assert!(self.len() >= count * from.len());

        let mut result = Vec::with_capacity(result_len);
        let mut last_end = 0;
        let mut count = count;
        for offset in self.elements.find_iter(&from.elements) {
            result.extend_from_slice(&self.elements[last_end..offset]);
            last_end = offset + from.len();
            count -= 1;
            if count == 0 {
                break;
            }
        }
        result.extend_from_slice(&self.elements[last_end..]);
        result
    }

    pub fn replace_in_place(&self, from: Self, to: Self, max_count: Option<usize>) -> Vec<u8> {
        let len = from.len();
        let mut iter = self.elements.find_iter(&from.elements);

        let mut new = if let Some(offset) = iter.next() {
            let mut new = self.elements.clone();
            new[offset..offset + len].clone_from_slice(to.elements.as_slice());
            if max_count == Some(1) {
                return new;
            } else {
                new
            }
        } else {
            return self.elements.clone();
        };

        let mut count = max_count.unwrap_or(usize::MAX) - 1;
        for offset in iter {
            new[offset..offset + len].clone_from_slice(to.elements.as_slice());
            count -= 1;
            if count == 0 {
                break;
            }
        }
        new
    }

    fn replace_general(
        &self,
        from: Self,
        to: Self,
        max_count: Option<usize>,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<u8>> {
        let count = count_substring(
            self.elements.as_slice(),
            from.elements.as_slice(),
            max_count,
        );
        if count == 0 {
            // no matches, return unchanged
            return Ok(self.elements.clone());
        }

        // Check for overflow
        //    result_len = self_len + count * (to_len-from_len)
        debug_assert!(count > 0);
        if to.len() as isize - from.len() as isize
            > (isize::MAX - self.elements.len() as isize) / count as isize
        {
            return Err(vm.new_overflow_error("replace bytes is too long"));
        }
        let result_len = (self.elements.len() as isize
            + count as isize * (to.len() as isize - from.len() as isize))
            as usize;

        let mut result = Vec::with_capacity(result_len);
        let mut last_end = 0;
        let mut count = count;
        for offset in self.elements.find_iter(&from.elements) {
            result.extend_from_slice(&self.elements[last_end..offset]);
            result.extend_from_slice(to.elements.as_slice());
            last_end = offset + from.len();
            count -= 1;
            if count == 0 {
                break;
            }
        }
        result.extend_from_slice(&self.elements[last_end..]);
        Ok(result)
    }

    pub fn replace(
        &self,
        from: Self,
        to: Self,
        max_count: OptionalArg<isize>,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<u8>> {
        // stringlib_replace in CPython
        let max_count = match max_count {
            OptionalArg::Present(max_count) if max_count >= 0 => {
                if max_count == 0 || (self.elements.is_empty() && !from.is_empty()) {
                    // nothing to do; return the original bytes
                    return Ok(self.elements.clone());
                } else if self.elements.is_empty() && from.is_empty() {
                    return Ok(to.elements);
                }
                Some(max_count as usize)
            }
            _ => None,
        };

        // Handle zero-length special cases
        if from.elements.is_empty() {
            if to.elements.is_empty() {
                // nothing to do; return the original bytes
                return Ok(self.elements.clone());
            }
            // insert the 'to' bytes everywhere.
            //     >>> b"Python".replace(b"", b".")
            //     b'.P.y.t.h.o.n.'
            return Ok(self.replace_interleave(to, max_count));
        }

        // Except for b"".replace(b"", b"A") == b"A" there is no way beyond this
        // point for an empty self bytes to generate a non-empty bytes
        // Special case so the remaining code always gets a non-empty bytes
        if self.elements.is_empty() {
            return Ok(self.elements.clone());
        }

        if to.elements.is_empty() {
            // delete all occurrences of 'from' bytes
            Ok(self.replace_delete(from, max_count))
        } else if from.len() == to.len() {
            // Handle special case where both bytes have the same length
            Ok(self.replace_in_place(from, to, max_count))
        } else {
            // Otherwise use the more generic algorithms
            self.replace_general(from, to, max_count, vm)
        }
    }

    pub fn title(&self) -> Vec<u8> {
        let mut res = vec![];
        let mut spaced = true;

        for i in &self.elements {
            match i {
                b'A'..=b'Z' | b'a'..=b'z' => {
                    if spaced {
                        res.push(i.to_ascii_uppercase());
                        spaced = false
                    } else {
                        res.push(i.to_ascii_lowercase());
                    }
                }
                _ => {
                    res.push(*i);
                    spaced = true
                }
            }
        }

        res
    }

    pub fn cformat(&self, values: PyObjectRef, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        cformat_bytes(vm, self.elements.as_slice(), values)
    }

    pub fn mul(&self, n: isize, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        self.elements.mul(vm, n)
    }

    pub fn imul(&mut self, n: isize, vm: &VirtualMachine) -> PyResult<()> {
        self.elements.imul(vm, n)
    }

    pub fn concat(&self, other: &PyObject, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        let buffer = PyBuffer::try_from_borrowed_object(vm, other)?;
        let borrowed = buffer.as_contiguous();
        if let Some(other) = borrowed {
            let mut v = Vec::with_capacity(self.elements.len() + other.len());
            v.extend_from_slice(&self.elements);
            v.extend_from_slice(&other);
            Ok(v)
        } else {
            let mut v = self.elements.clone();
            buffer.append_to(&mut v);
            Ok(v)
        }
    }
}

pub fn try_as_bytes<F, R>(obj: PyObjectRef, f: F) -> Option<R>
where
    F: Fn(&[u8]) -> R,
{
    match_class!(match obj {
        i @ PyBytes => Some(f(i.as_bytes())),
        j @ PyByteArray => Some(f(&j.borrow_buf())),
        _ => None,
    })
}

#[inline]
fn count_substring(haystack: &[u8], needle: &[u8], max_count: Option<usize>) -> usize {
    let substrings = haystack.find_iter(needle);
    if let Some(max_count) = max_count {
        core::cmp::min(substrings.take(max_count).count(), max_count)
    } else {
        substrings.count()
    }
}

pub trait ByteOr: ToPrimitive {
    fn byte_or(&self, vm: &VirtualMachine) -> PyResult<u8> {
        match self.to_u8() {
            Some(value) => Ok(value),
            None => Err(vm.new_value_error("byte must be in range(0, 256)")),
        }
    }
}

impl ByteOr for BigInt {}

impl AnyStrWrapper<[u8]> for PyBytesInner {
    fn as_ref(&self) -> Option<&[u8]> {
        Some(&self.elements)
    }

    fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }
}

impl AnyStrContainer<[u8]> for Vec<u8> {
    fn new() -> Self {
        Self::new()
    }

    fn with_capacity(capacity: usize) -> Self {
        Self::with_capacity(capacity)
    }

    fn push_str(&mut self, other: &[u8]) {
        self.extend(other)
    }
}

const ASCII_WHITESPACES: [u8; 6] = [0x20, 0x09, 0x0a, 0x0c, 0x0d, 0x0b];

impl anystr::AnyChar for u8 {
    fn is_lowercase(self) -> bool {
        self.is_ascii_lowercase()
    }

    fn is_uppercase(self) -> bool {
        self.is_ascii_uppercase()
    }

    fn bytes_len(self) -> usize {
        1
    }
}

impl AnyStr for [u8] {
    type Char = u8;
    type Container = Vec<u8>;

    fn to_container(&self) -> Self::Container {
        self.to_vec()
    }

    fn as_bytes(&self) -> &[u8] {
        self
    }

    fn elements(&self) -> impl Iterator<Item = u8> {
        self.iter().copied()
    }

    fn get_bytes(&self, range: core::ops::Range<usize>) -> &Self {
        &self[range]
    }

    fn get_chars(&self, range: core::ops::Range<usize>) -> &Self {
        &self[range]
    }

    fn is_empty(&self) -> bool {
        Self::is_empty(self)
    }

    fn bytes_len(&self) -> usize {
        Self::len(self)
    }

    fn py_split_whitespace<F>(&self, maxsplit: isize, convert: F) -> Vec<PyObjectRef>
    where
        F: Fn(&Self) -> PyObjectRef,
    {
        let mut splits = Vec::new();
        let mut count = maxsplit;
        let mut haystack = self;
        while let Some(offset) = haystack.find_byteset(ASCII_WHITESPACES) {
            if offset != 0 {
                if count == 0 {
                    break;
                }
                splits.push(convert(&haystack[..offset]));
                count -= 1;
            }
            haystack = &haystack[offset + 1..];
        }
        if !haystack.is_empty() {
            splits.push(convert(haystack));
        }
        splits
    }

    fn py_rsplit_whitespace<F>(&self, maxsplit: isize, convert: F) -> Vec<PyObjectRef>
    where
        F: Fn(&Self) -> PyObjectRef,
    {
        let mut splits = Vec::new();
        let mut count = maxsplit;
        let mut haystack = self;
        while let Some(offset) = haystack.rfind_byteset(ASCII_WHITESPACES) {
            if offset + 1 != haystack.len() {
                if count == 0 {
                    break;
                }
                splits.push(convert(&haystack[offset + 1..]));
                count -= 1;
            }
            haystack = &haystack[..offset];
        }
        if !haystack.is_empty() {
            splits.push(convert(haystack));
        }
        splits
    }
}

#[derive(FromArgs)]
pub struct DecodeArgs {
    #[pyarg(any, default)]
    encoding: Option<PyStrRef>,
    #[pyarg(any, default)]
    errors: Option<PyStrRef>,
}

pub fn bytes_decode(
    zelf: PyObjectRef,
    args: DecodeArgs,
    vm: &VirtualMachine,
) -> PyResult<PyStrRef> {
    let DecodeArgs { encoding, errors } = args;
    let encoding = encoding
        .as_ref()
        .map_or(crate::codecs::DEFAULT_ENCODING, |s| s.as_str());
    vm.state
        .codec_registry
        .decode_text(zelf, encoding, errors, vm)
}

fn hex_impl_no_sep(bytes: &[u8]) -> String {
    let mut buf: Vec<u8> = vec![0; bytes.len() * 2];
    hex::encode_to_slice(bytes, buf.as_mut_slice()).unwrap();
    unsafe { String::from_utf8_unchecked(buf) }
}

fn hex_impl(bytes: &[u8], sep: u8, bytes_per_sep: isize) -> String {
    let len = bytes.len();

    let buf = if bytes_per_sep < 0 {
        let bytes_per_sep = core::cmp::min(len, (-bytes_per_sep) as usize);
        let chunks = (len - 1) / bytes_per_sep;
        let chunked = chunks * bytes_per_sep;
        let unchunked = len - chunked;
        let mut buf = vec![0; len * 2 + chunks];
        let mut j = 0;
        for i in (0..chunks).map(|i| i * bytes_per_sep) {
            hex::encode_to_slice(
                &bytes[i..i + bytes_per_sep],
                &mut buf[j..j + bytes_per_sep * 2],
            )
            .unwrap();
            j += bytes_per_sep * 2;
            buf[j] = sep;
            j += 1;
        }
        hex::encode_to_slice(&bytes[chunked..], &mut buf[j..j + unchunked * 2]).unwrap();
        buf
    } else {
        let bytes_per_sep = core::cmp::min(len, bytes_per_sep as usize);
        let chunks = (len - 1) / bytes_per_sep;
        let chunked = chunks * bytes_per_sep;
        let unchunked = len - chunked;
        let mut buf = vec![0; len * 2 + chunks];
        hex::encode_to_slice(&bytes[..unchunked], &mut buf[..unchunked * 2]).unwrap();
        let mut j = unchunked * 2;
        for i in (0..chunks).map(|i| i * bytes_per_sep + unchunked) {
            buf[j] = sep;
            j += 1;
            hex::encode_to_slice(
                &bytes[i..i + bytes_per_sep],
                &mut buf[j..j + bytes_per_sep * 2],
            )
            .unwrap();
            j += bytes_per_sep * 2;
        }
        buf
    };

    unsafe { String::from_utf8_unchecked(buf) }
}

pub fn bytes_to_hex(
    bytes: &[u8],
    sep: OptionalArg<Either<PyStrRef, PyBytesRef>>,
    bytes_per_sep: OptionalArg<isize>,
    vm: &VirtualMachine,
) -> PyResult<String> {
    if bytes.is_empty() {
        return Ok("".to_owned());
    }

    if let OptionalArg::Present(sep) = sep {
        let bytes_per_sep = bytes_per_sep.unwrap_or(1);
        if bytes_per_sep == 0 {
            return Ok(hex_impl_no_sep(bytes));
        }

        let s_guard;
        let b_guard;
        let sep = match &sep {
            Either::A(s) => {
                s_guard = s.as_str();
                s_guard.as_bytes()
            }
            Either::B(bytes) => {
                b_guard = bytes.as_bytes();
                b_guard
            }
        };

        if sep.len() != 1 {
            return Err(vm.new_value_error("sep must be length 1."));
        }
        let sep = sep[0];
        if sep > 127 {
            return Err(vm.new_value_error("sep must be ASCII."));
        }

        Ok(hex_impl(bytes, sep, bytes_per_sep))
    } else {
        Ok(hex_impl_no_sep(bytes))
    }
}

pub const fn is_py_ascii_whitespace(b: u8) -> bool {
    matches!(b, b'\t' | b'\n' | b'\x0C' | b'\r' | b' ' | b'\x0B')
}

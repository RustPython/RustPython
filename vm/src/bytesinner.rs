use bstr::ByteSlice;
use itertools::Itertools;
use num_bigint::BigInt;
use num_traits::ToPrimitive;

use crate::anystr::{self, AnyStr, AnyStrContainer, AnyStrWrapper};
use crate::builtins::bytearray::{PyByteArray, PyByteArrayRef};
use crate::builtins::bytes::{PyBytes, PyBytesRef};
use crate::builtins::int::{PyInt, PyIntRef};
use crate::builtins::pystr::{self, PyStr, PyStrRef};
use crate::builtins::singletons::PyNoneRef;
use crate::builtins::PyTypeRef;
use crate::byteslike::try_bytes_like;
use crate::cformat::CFormatBytes;
use crate::function::{OptionalArg, OptionalOption};
use crate::pyobject::{
    BorrowValue, Either, IdProtocol, PyComparisonValue, PyIterable, PyObjectRef, PyResult, PyValue,
    TryFromObject, TypeProtocol,
};
use crate::sliceable::PySliceableSequence;
use crate::slots::PyComparisonOp;
use crate::vm::VirtualMachine;
use rustpython_common::hash;

#[derive(Debug, Default, Clone)]
pub struct PyBytesInner {
    pub(crate) elements: Vec<u8>,
}

impl From<Vec<u8>> for PyBytesInner {
    fn from(elements: Vec<u8>) -> PyBytesInner {
        Self { elements }
    }
}

impl TryFromObject for PyBytesInner {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        bytes_from_object(vm, &obj).map(Self::from)
    }
}

#[derive(FromArgs)]
pub struct ByteInnerNewOptions {
    #[pyarg(any, optional)]
    source: OptionalArg<PyObjectRef>,
    #[pyarg(any, optional)]
    encoding: OptionalArg<PyStrRef>,
    #[pyarg(any, optional)]
    errors: OptionalArg<PyStrRef>,
}

impl ByteInnerNewOptions {
    fn get_value_from_string(
        s: PyStrRef,
        encoding: OptionalArg<PyStrRef>,
        errors: OptionalArg<PyStrRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyBytesInner> {
        // Handle bytes(string, encoding[, errors])
        let encoding = encoding
            .ok_or_else(|| vm.new_type_error("string argument without an encoding".to_owned()))?;
        let bytes = pystr::encode_string(s, Some(encoding), errors.into_option(), vm)?;
        Ok(bytes.borrow_value().to_vec().into())
    }

    fn get_value_from_source(source: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyBytesInner> {
        bytes_from_object(vm, &source).map(|x| x.into())
    }

    fn get_value_from_size(size: PyIntRef, vm: &VirtualMachine) -> PyResult<PyBytesInner> {
        let size = size.borrow_value().to_isize().ok_or_else(|| {
            vm.new_overflow_error("cannot fit 'int' into an index-sized integer".to_owned())
        })?;
        let size = if size < 0 {
            return Err(vm.new_value_error("negative count".to_owned()));
        } else {
            size as usize
        };
        Ok(vec![0; size].into())
    }

    fn check_args(self, vm: &VirtualMachine) -> PyResult<()> {
        if let OptionalArg::Present(_) = self.encoding {
            Err(vm.new_type_error("encoding without a string argument".to_owned()))
        } else if let OptionalArg::Present(_) = self.errors {
            Err(vm.new_type_error("errors without a string argument".to_owned()))
        } else {
            Ok(())
        }
    }

    pub fn get_bytes(mut self, cls: PyTypeRef, vm: &VirtualMachine) -> PyResult<PyBytesRef> {
        let inner = if let OptionalArg::Present(source) = self.source.take() {
            if cls.is(&PyBytes::class(vm)) {
                if let Ok(source) = source.clone().downcast_exact(vm) {
                    return self.check_args(vm).map(|()| source);
                }
            }

            match_class!(match source {
                s @ PyStr => Self::get_value_from_string(s, self.encoding, self.errors, vm),
                i @ PyInt => {
                    self.check_args(vm)?;
                    Self::get_value_from_size(i, vm)
                }
                obj => {
                    self.check_args(vm)?;
                    if let Some(bytes_method) = vm.get_method(obj.clone(), "__bytes__") {
                        let bytes = vm.invoke(&bytes_method?, ())?;
                        PyBytesInner::try_from_object(vm, bytes)
                    } else {
                        Self::get_value_from_source(obj, vm)
                    }
                }
            })
        } else {
            self.check_args(vm).map(|_| vec![].into())
        }?;

        PyBytes::from(inner).into_ref_with_type(vm, cls)
    }

    pub fn get_bytearray(
        mut self,
        cls: PyTypeRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyByteArrayRef> {
        let inner = if let OptionalArg::Present(source) = self.source.take() {
            match_class!(match source {
                s @ PyStr => Self::get_value_from_string(s, self.encoding, self.errors, vm),
                i @ PyInt => {
                    self.check_args(vm)?;
                    Self::get_value_from_size(i, vm)
                }
                obj => {
                    self.check_args(vm)?;
                    Self::get_value_from_source(obj, vm)
                }
            })
        } else {
            self.check_args(vm).map(|_| vec![].into())
        }?;

        PyByteArray::from(inner).into_ref_with_type(vm, cls)
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
    ) -> PyResult<(Vec<u8>, std::ops::Range<usize>)> {
        let sub = match self.sub {
            Either::A(v) => v.elements.to_vec(),
            Either::B(int) => vec![int.borrow_value().byte_or(vm)?],
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
                        fn_name, &v
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
    table: Either<PyBytesInner, PyNoneRef>,
    #[pyarg(any, optional)]
    delete: OptionalArg<PyBytesInner>,
}

impl ByteInnerTranslateOptions {
    pub fn get_value(self, vm: &VirtualMachine) -> PyResult<(Vec<u8>, Vec<u8>)> {
        let table = match self.table {
            Either::A(v) => v.elements.to_vec(),
            Either::B(_) => (0..=255).collect::<Vec<u8>>(),
        };

        if table.len() != 256 {
            return Err(
                vm.new_value_error("translation table must be 256 characters long".to_owned())
            );
        }

        let delete = match self.delete {
            OptionalArg::Present(byte) => byte.elements,
            _ => vec![],
        };

        Ok((table, delete))
    }
}

pub type ByteInnerSplitOptions<'a> = anystr::SplitArgs<'a, PyBytesInner>;

#[allow(clippy::len_without_is_empty)]
impl PyBytesInner {
    pub fn repr(&self) -> String {
        let mut res = String::with_capacity(self.elements.len());
        for i in self.elements.iter() {
            match i {
                9 => res.push_str("\\t"),
                10 => res.push_str("\\n"),
                13 => res.push_str("\\r"),
                32..=126 => res.push(*(i) as char),
                _ => res.push_str(&format!("\\x{:02x}", i)),
            }
        }
        res
    }

    pub fn len(&self) -> usize {
        self.elements.len()
    }

    pub fn cmp(
        &self,
        other: &PyObjectRef,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyComparisonValue {
        // TODO: bytes can compare with any object implemented buffer protocol
        // but not memoryview, and not equal if compare with unicode str(PyStr)
        PyComparisonValue::from_option(
            try_bytes_like(vm, other, |other| {
                op.eval_ord(self.elements.as_slice().cmp(other))
            })
            .ok(),
        )
    }

    pub fn hash(&self, vm: &VirtualMachine) -> hash::PyHash {
        vm.state.hash_secret.hash_bytes(&self.elements)
    }

    pub fn add(&self, other: &[u8]) -> Vec<u8> {
        self.elements.py_add(other)
    }

    pub fn contains(
        &self,
        needle: Either<PyBytesInner, PyIntRef>,
        vm: &VirtualMachine,
    ) -> PyResult<bool> {
        Ok(match needle {
            Either::A(byte) => self.elements.contains_str(byte.elements.as_slice()),
            Either::B(int) => self.elements.contains(&int.borrow_value().byte_or(vm)?),
        })
    }

    pub fn getitem(
        &self,
        name: &'static str,
        needle: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult {
        let obj = match self.elements.get_item(vm, needle, name)? {
            Either::A(byte) => vm.new_pyobj(byte),
            Either::B(bytes) => vm.ctx.new_bytes(bytes),
        };
        Ok(obj)
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
        !self.elements.is_empty() && self.elements.iter().all(|x| char::from(*x).is_digit(10))
    }

    pub fn islower(&self) -> bool {
        self.elements
            .py_iscase(char::is_lowercase, char::is_uppercase)
    }

    pub fn isupper(&self) -> bool {
        self.elements
            .py_iscase(char::is_uppercase, char::is_lowercase)
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
                65..=90 => new.push(w.to_ascii_lowercase()),
                97..=122 => new.push(w.to_ascii_uppercase()),
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

    pub fn fromhex(string: &str, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        let mut iter = string.bytes().enumerate();
        let mut bytes: Vec<u8> = Vec::with_capacity(string.len() / 2);
        let i = loop {
            let (i, b) = match iter.next() {
                Some(val) => val,
                None => {
                    return Ok(bytes);
                }
            };

            if is_py_ascii_whitespace(b) {
                continue;
            }

            let top = match b {
                b'0'..=b'9' => b - b'0',
                b'a'..=b'f' => 10 + b - b'a',
                b'A'..=b'F' => 10 + b - b'A',
                _ => break i,
            };

            let (i, b) = match iter.next() {
                Some(val) => val,
                None => break i + 1,
            };

            let bot = match b {
                b'0'..=b'9' => b - b'0',
                b'a'..=b'f' => 10 + b - b'a',
                b'A'..=b'F' => 10 + b - b'A',
                _ => break i,
            };

            bytes.push((top << 4) + bot);
        };

        Err(vm.new_value_error(format!(
            "non-hexadecimal number found in fromhex() arg at position {}",
            i
        )))
    }

    #[inline]
    fn _pad(
        &self,
        options: ByteInnerPaddingOptions,
        pad: fn(&[u8], usize, u8, usize) -> Vec<u8>,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<u8>> {
        let (width, fillchar) = options.get_value("center", vm)?;
        Ok(if self.len() as isize >= width {
            Vec::from(&self.elements[..])
        } else {
            pad(&self.elements, width as usize, fillchar, self.len())
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

    pub fn join(
        &self,
        iterable: PyIterable<PyBytesInner>,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<u8>> {
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

    pub fn maketrans(from: PyBytesInner, to: PyBytesInner, vm: &VirtualMachine) -> PyResult {
        let mut res = vec![];

        for i in 0..=255 {
            res.push(if let Some(position) = from.elements.find_byte(i) {
                to.elements[position]
            } else {
                i
            });
        }

        Ok(vm.ctx.new_bytes(res))
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

        for i in self.elements.iter() {
            if !delete.contains(&i) {
                res.push(table[*i as usize]);
            }
        }

        Ok(res)
    }

    pub fn strip(&self, chars: OptionalOption<PyBytesInner>) -> Vec<u8> {
        self.elements
            .py_strip(
                chars,
                |s, chars| s.trim_with(|c| chars.contains(&(c as u8))),
                |s| s.trim(),
            )
            .to_vec()
    }

    pub fn lstrip(&self, chars: OptionalOption<PyBytesInner>) -> Vec<u8> {
        self.elements
            .py_strip(
                chars,
                |s, chars| s.trim_start_with(|c| chars.contains(&(c as u8))),
                |s| s.trim_start(),
            )
            .to_vec()
    }

    pub fn rstrip(&self, chars: OptionalOption<PyBytesInner>) -> Vec<u8> {
        self.elements
            .py_strip(
                chars,
                |s, chars| s.trim_end_with(|c| chars.contains(&(c as u8))),
                |s| s.trim_end(),
            )
            .to_vec()
    }

    // new in Python 3.9
    pub fn removeprefix(&self, prefix: PyBytesInner) -> Vec<u8> {
        self.elements
            .py_removeprefix(&prefix.elements, prefix.elements.len(), |s, p| {
                s.starts_with(p)
            })
            .to_vec()
    }

    // new in Python 3.9
    pub fn removesuffix(&self, suffix: PyBytesInner) -> Vec<u8> {
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
    ) -> PyResult<PyObjectRef>
    where
        F: Fn(&[u8], &VirtualMachine) -> PyObjectRef,
    {
        let elements = self.elements.py_split(
            options,
            vm,
            |v, s, vm| v.split_str(s).map(|v| convert(v, vm)).collect(),
            |v, s, n, vm| v.splitn_str(n, s).map(|v| convert(v, vm)).collect(),
            |v, n, vm| v.py_split_whitespace(n, |v| convert(v, vm)),
        )?;
        Ok(vm.ctx.new_list(elements))
    }

    pub fn rsplit<F>(
        &self,
        options: ByteInnerSplitOptions,
        convert: F,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef>
    where
        F: Fn(&[u8], &VirtualMachine) -> PyObjectRef,
    {
        let mut elements = self.elements.py_split(
            options,
            vm,
            |v, s, vm| v.rsplit_str(s).map(|v| convert(v, vm)).collect(),
            |v, s, n, vm| v.rsplitn_str(n, s).map(|v| convert(v, vm)).collect(),
            |v, n, vm| v.py_rsplit_whitespace(n, |v| convert(v, vm)),
        )?;
        elements.reverse();
        Ok(vm.ctx.new_list(elements))
    }

    pub fn partition(
        &self,
        sub: &PyBytesInner,
        vm: &VirtualMachine,
    ) -> PyResult<(Vec<u8>, bool, Vec<u8>)> {
        self.elements.py_partition(
            &sub.elements,
            || self.elements.splitn_str(2, &sub.elements),
            vm,
        )
    }

    pub fn rpartition(
        &self,
        sub: &PyBytesInner,
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
        self.elements.py_splitlines(options, into_wrapper)
    }

    pub fn zfill(&self, width: isize) -> Vec<u8> {
        self.elements.py_zfill(width)
    }

    // len(self)>=1, from="", len(to)>=1, maxcount>=1
    fn replace_interleave(&self, to: PyBytesInner, maxcount: Option<usize>) -> Vec<u8> {
        let place_count = self.elements.len() + 1;
        let count = maxcount.map_or(place_count, |v| std::cmp::min(v, place_count)) - 1;
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

    fn replace_delete(&self, from: PyBytesInner, maxcount: Option<usize>) -> Vec<u8> {
        let count = count_substring(self.elements.as_slice(), from.elements.as_slice(), maxcount);
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

    pub fn replace_in_place(
        &self,
        from: PyBytesInner,
        to: PyBytesInner,
        maxcount: Option<usize>,
    ) -> Vec<u8> {
        let len = from.len();
        let mut iter = self.elements.find_iter(&from.elements);

        let mut new = if let Some(offset) = iter.next() {
            let mut new = self.elements.clone();
            new[offset..offset + len].clone_from_slice(to.elements.as_slice());
            if maxcount == Some(1) {
                return new;
            } else {
                new
            }
        } else {
            return self.elements.clone();
        };

        let mut count = maxcount.unwrap_or(std::usize::MAX) - 1;
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
        from: PyBytesInner,
        to: PyBytesInner,
        maxcount: Option<usize>,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<u8>> {
        let count = count_substring(self.elements.as_slice(), from.elements.as_slice(), maxcount);
        if count == 0 {
            // no matches, return unchanged
            return Ok(self.elements.clone());
        }

        // Check for overflow
        //    result_len = self_len + count * (to_len-from_len)
        debug_assert!(count > 0);
        if to.len() as isize - from.len() as isize
            > (std::isize::MAX - self.elements.len() as isize) / count as isize
        {
            return Err(vm.new_overflow_error("replace bytes is too long".to_owned()));
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
        from: PyBytesInner,
        to: PyBytesInner,
        maxcount: OptionalArg<isize>,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<u8>> {
        // stringlib_replace in CPython
        let maxcount = match maxcount {
            OptionalArg::Present(maxcount) if maxcount >= 0 => {
                if maxcount == 0 || self.elements.is_empty() {
                    // nothing to do; return the original bytes
                    return Ok(self.elements.clone());
                }
                Some(maxcount as usize)
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
            return Ok(self.replace_interleave(to, maxcount));
        }

        // Except for b"".replace(b"", b"A") == b"A" there is no way beyond this
        // point for an empty self bytes to generate a non-empty bytes
        // Special case so the remaining code always gets a non-empty bytes
        if self.elements.is_empty() {
            return Ok(self.elements.clone());
        }

        if to.elements.is_empty() {
            // delete all occurrences of 'from' bytes
            Ok(self.replace_delete(from, maxcount))
        } else if from.len() == to.len() {
            // Handle special case where both bytes have the same length
            Ok(self.replace_in_place(from, to, maxcount))
        } else {
            // Otherwise use the more generic algorithms
            self.replace_general(from, to, maxcount, vm)
        }
    }

    pub fn title(&self) -> Vec<u8> {
        let mut res = vec![];
        let mut spaced = true;

        for i in self.elements.iter() {
            match i {
                65..=90 | 97..=122 => {
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
        CFormatBytes::parse_from_bytes(self.elements.as_slice())
            .map_err(|err| vm.new_value_error(err.to_string()))?
            .format(vm, values)
    }

    pub fn repeat(&self, n: isize) -> Vec<u8> {
        self.elements.repeat(n.to_usize().unwrap_or(0))
    }
}

pub fn try_as_bytes<F, R>(obj: PyObjectRef, f: F) -> Option<R>
where
    F: Fn(&[u8]) -> R,
{
    match_class!(match obj {
        i @ PyBytes => Some(f(i.borrow_value())),
        j @ PyByteArray => Some(f(&j.borrow_value().elements)),
        _ => None,
    })
}

#[inline]
fn count_substring(haystack: &[u8], needle: &[u8], maxcount: Option<usize>) -> usize {
    let substrings = haystack.find_iter(needle);
    if let Some(maxcount) = maxcount {
        std::cmp::min(substrings.take(maxcount).count(), maxcount)
    } else {
        substrings.count()
    }
}

pub trait ByteOr: ToPrimitive {
    fn byte_or(&self, vm: &VirtualMachine) -> PyResult<u8> {
        match self.to_u8() {
            Some(value) => Ok(value),
            None => Err(vm.new_value_error("byte must be in range(0, 256)".to_owned())),
        }
    }
}

impl ByteOr for BigInt {}

impl<'s> AnyStrWrapper<'s> for PyBytesInner {
    type Str = [u8];
    fn as_ref(&self) -> &[u8] {
        &self.elements
    }
}

impl AnyStrContainer<[u8]> for Vec<u8> {
    fn new() -> Self {
        Vec::new()
    }

    fn with_capacity(capacity: usize) -> Self {
        Vec::with_capacity(capacity)
    }

    fn push_str(&mut self, other: &[u8]) {
        self.extend(other)
    }
}

const ASCII_WHITESPACES: [u8; 6] = [0x20, 0x09, 0x0a, 0x0c, 0x0d, 0x0b];

impl<'s> AnyStr<'s> for [u8] {
    type Char = u8;
    type Container = Vec<u8>;
    type CharIter = bstr::Chars<'s>;
    type ElementIter = std::iter::Copied<std::slice::Iter<'s, u8>>;

    fn element_bytes_len(_: u8) -> usize {
        1
    }

    fn to_container(&self) -> Self::Container {
        self.to_vec()
    }

    fn as_bytes(&self) -> &[u8] {
        self
    }

    fn as_utf8_str(&self) -> Result<&str, std::str::Utf8Error> {
        std::str::from_utf8(self)
    }

    fn chars(&'s self) -> Self::CharIter {
        bstr::ByteSlice::chars(self)
    }

    fn elements(&'s self) -> Self::ElementIter {
        self.iter().copied()
    }

    fn get_bytes(&self, range: std::ops::Range<usize>) -> &Self {
        &self[range]
    }

    fn get_chars(&self, range: std::ops::Range<usize>) -> &Self {
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
        let mut splited = Vec::new();
        let mut count = maxsplit;
        let mut haystack = self;
        while let Some(offset) = haystack.find_byteset(ASCII_WHITESPACES) {
            if offset != 0 {
                if count == 0 {
                    break;
                }
                splited.push(convert(&haystack[..offset]));
                count -= 1;
            }
            haystack = &haystack[offset + 1..];
        }
        if !haystack.is_empty() {
            splited.push(convert(haystack));
        }
        splited
    }

    fn py_rsplit_whitespace<F>(&self, maxsplit: isize, convert: F) -> Vec<PyObjectRef>
    where
        F: Fn(&Self) -> PyObjectRef,
    {
        let mut splited = Vec::new();
        let mut count = maxsplit;
        let mut haystack = self;
        while let Some(offset) = haystack.rfind_byteset(ASCII_WHITESPACES) {
            if offset + 1 != haystack.len() {
                if count == 0 {
                    break;
                }
                splited.push(convert(&haystack[offset + 1..]));
                count -= 1;
            }
            haystack = &haystack[..offset];
        }
        if !haystack.is_empty() {
            splited.push(convert(haystack));
        }
        splited
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
    vm.decode(zelf, encoding.clone(), errors)?
        .downcast::<PyStr>()
        .map_err(|obj| {
            vm.new_type_error(format!(
                "'{}' decoder returned '{}' instead of 'str'; use codecs.encode() to \
                     encode arbitrary types",
                encoding.as_ref().map_or("utf-8", |s| s.borrow_value()),
                obj.class().name,
            ))
        })
}

fn hex_impl_no_sep(bytes: &[u8]) -> String {
    let mut buf: Vec<u8> = vec![0; bytes.len() * 2];
    hex::encode_to_slice(bytes, buf.as_mut_slice()).unwrap();
    unsafe { String::from_utf8_unchecked(buf) }
}

fn hex_impl(bytes: &[u8], sep: u8, bytes_per_sep: isize) -> String {
    let len = bytes.len();

    let buf = if bytes_per_sep < 0 {
        let bytes_per_sep = std::cmp::min(len, (-bytes_per_sep) as usize);
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
        let bytes_per_sep = std::cmp::min(len, bytes_per_sep as usize);
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
                s_guard = s.borrow_value();
                s_guard.as_bytes()
            }
            Either::B(bytes) => {
                b_guard = bytes.borrow_value();
                b_guard
            }
        };

        if sep.len() != 1 {
            return Err(vm.new_value_error("sep must be length 1.".to_owned()));
        }
        let sep = sep[0];
        if sep > 127 {
            return Err(vm.new_value_error("sep must be ASCII.".to_owned()));
        }

        Ok(hex_impl(bytes, sep, bytes_per_sep))
    } else {
        Ok(hex_impl_no_sep(bytes))
    }
}

pub const fn is_py_ascii_whitespace(b: u8) -> bool {
    matches!(b, b'\t' | b'\n' | b'\x0C' | b'\r' | b' ' | b'\x0B')
}

pub fn bytes_from_object(vm: &VirtualMachine, obj: &PyObjectRef) -> PyResult<Vec<u8>> {
    if let Ok(elements) = try_bytes_like(vm, obj, |bytes| bytes.to_vec()) {
        return Ok(elements);
    }

    if let Ok(elements) = vm.map_iterable_object(obj, |x| value_from_object(vm, &x)) {
        return elements;
    }

    Err(vm.new_type_error(
        "can assign only bytes, buffers, or iterables of ints in range(0, 256)".to_owned(),
    ))
}

pub fn value_from_object(vm: &VirtualMachine, obj: &PyObjectRef) -> PyResult<u8> {
    vm.to_index(obj)?
        .borrow_value()
        .to_u8()
        .ok_or_else(|| vm.new_value_error("byte must be in range(0, 256)".to_owned()))
}

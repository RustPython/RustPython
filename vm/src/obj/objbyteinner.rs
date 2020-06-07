use bstr::ByteSlice;
use num_bigint::{BigInt, ToBigInt};
use num_traits::{One, Signed, ToPrimitive, Zero};
use std::convert::TryFrom;
use std::ops::Range;

use super::objbytearray::{PyByteArray, PyByteArrayRef};
use super::objbytes::{PyBytes, PyBytesRef};
use super::objint::{self, PyInt, PyIntRef};
use super::objlist::PyList;
use super::objmemory::PyMemoryView;
use super::objnone::PyNoneRef;
use super::objsequence::{PySliceableSequence, SequenceIndex};
use super::objslice::PySliceRef;
use super::objstr::{self, PyString, PyStringRef};
use super::pystr::{self, PyCommonString, PyCommonStringWrapper};
use crate::function::{OptionalArg, OptionalOption};
use crate::pyhash;
use crate::pyobject::{
    Either, PyComparisonValue, PyIterable, PyObjectRef, PyResult, TryFromObject, TypeProtocol,
};
use crate::vm::VirtualMachine;

#[derive(Debug, Default, Clone)]
pub struct PyByteInner {
    pub elements: Vec<u8>,
}

impl From<Vec<u8>> for PyByteInner {
    fn from(elements: Vec<u8>) -> PyByteInner {
        Self { elements }
    }
}

impl TryFromObject for PyByteInner {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        match_class!(match obj {
            i @ PyBytes => Ok(PyByteInner {
                elements: i.get_value().to_vec()
            }),
            j @ PyByteArray => Ok(PyByteInner {
                elements: j.borrow_value().elements.to_vec()
            }),
            k @ PyMemoryView => Ok(PyByteInner {
                elements: k.try_value().unwrap()
            }),
            l @ PyList => l.get_byte_inner(vm),
            obj => {
                let iter = vm.get_method_or_type_error(obj.clone(), "__iter__", || {
                    format!("a bytes-like object is required, not {}", obj.class())
                })?;
                let iter = PyIterable::from_method(iter);
                Ok(PyByteInner {
                    elements: iter.iter(vm)?.collect::<PyResult<_>>()?,
                })
            }
        })
    }
}

#[derive(FromArgs)]
pub struct ByteInnerNewOptions {
    #[pyarg(positional_only, optional = true)]
    val_option: OptionalArg<PyObjectRef>,
    #[pyarg(positional_or_keyword, optional = true)]
    encoding: OptionalArg<PyStringRef>,
}

impl ByteInnerNewOptions {
    pub fn get_value(self, vm: &VirtualMachine) -> PyResult<PyByteInner> {
        // First handle bytes(string, encoding[, errors])
        if let OptionalArg::Present(enc) = self.encoding {
            if let OptionalArg::Present(eval) = self.val_option {
                if let Ok(input) = eval.downcast::<PyString>() {
                    let bytes = objstr::encode_string(input, Some(enc), None, vm)?;
                    Ok(PyByteInner {
                        elements: bytes.get_value().to_vec(),
                    })
                } else {
                    Err(vm.new_type_error("encoding without a string argument".to_owned()))
                }
            } else {
                Err(vm.new_type_error("encoding without a string argument".to_owned()))
            }
        // Only one argument
        } else {
            let value = if let OptionalArg::Present(ival) = self.val_option {
                match_class!(match ival.clone() {
                    i @ PyInt => {
                        let size =
                            objint::get_value(&i.into_object())
                                .to_isize()
                                .ok_or_else(|| {
                                    vm.new_overflow_error(
                                        "cannot fit 'int' into an index-sized integer".to_owned(),
                                    )
                                })?;
                        let size = if size < 0 {
                            return Err(vm.new_value_error("negative count".to_owned()));
                        } else {
                            size as usize
                        };
                        Ok(vec![0; size])
                    }
                    _l @ PyString => {
                        return Err(
                            vm.new_type_error("string argument without an encoding".to_owned())
                        );
                    }
                    i @ PyBytes => Ok(i.get_value().to_vec()),
                    j @ PyByteArray => Ok(j.borrow_value().elements.to_vec()),
                    obj => {
                        // TODO: only support this method in the bytes() constructor
                        if let Some(bytes_method) = vm.get_method(obj.clone(), "__bytes__") {
                            let bytes = vm.invoke(&bytes_method?, vec![])?;
                            return PyByteInner::try_from_object(vm, bytes);
                        }
                        let elements = vm.extract_elements(&obj).or_else(|_| {
                            Err(vm.new_type_error(format!(
                                "cannot convert '{}' object to bytes",
                                obj.class().name
                            )))
                        })?;

                        let mut data_bytes = vec![];
                        for elem in elements {
                            let v = objint::to_int(vm, &elem, &BigInt::from(10))?;
                            if let Some(i) = v.to_u8() {
                                data_bytes.push(i);
                            } else {
                                return Err(
                                    vm.new_value_error("bytes must be in range(0, 256)".to_owned())
                                );
                            }
                        }
                        Ok(data_bytes)
                    }
                })
            } else {
                Ok(vec![])
            };
            match value {
                Ok(val) => Ok(PyByteInner { elements: val }),
                Err(err) => Err(err),
            }
        }
    }
}

#[derive(FromArgs)]
pub struct ByteInnerFindOptions {
    #[pyarg(positional_only, optional = false)]
    sub: Either<PyByteInner, PyIntRef>,
    #[pyarg(positional_only, default = "None")]
    start: Option<PyIntRef>,
    #[pyarg(positional_only, default = "None")]
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
            Either::B(int) => vec![int.as_bigint().byte_or(vm)?],
        };
        let range = pystr::adjust_indices(self.start, self.end, len);
        Ok((sub, range))
    }
}

#[derive(FromArgs)]
pub struct ByteInnerPaddingOptions {
    #[pyarg(positional_only, optional = false)]
    width: isize,
    #[pyarg(positional_only, optional = true)]
    fillchar: OptionalArg<PyObjectRef>,
}

impl ByteInnerPaddingOptions {
    fn get_value(self, fn_name: &str, vm: &VirtualMachine) -> PyResult<(isize, u8)> {
        let fillchar = if let OptionalArg::Present(v) = self.fillchar {
            match try_as_byte(v.clone()) {
                Some(x) if x.len() == 1 => x[0],
                _ => {
                    return Err(vm.new_type_error(format!(
                        "{}() argument 2 must be a byte string of length 1, not {}",
                        fn_name, &v
                    )));
                }
            }
        } else {
            b' ' // default is space
        };

        Ok((self.width, fillchar))
    }
}

#[derive(FromArgs)]
pub struct ByteInnerTranslateOptions {
    #[pyarg(positional_only, optional = false)]
    table: Either<PyByteInner, PyNoneRef>,
    #[pyarg(positional_or_keyword, optional = true)]
    delete: OptionalArg<PyByteInner>,
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

pub type ByteInnerSplitOptions = pystr::SplitArgs<PyByteInner, [u8], u8>;

#[derive(FromArgs)]
pub struct ByteInnerSplitlinesOptions {
    #[pyarg(positional_or_keyword, optional = true)]
    keepends: OptionalArg<bool>,
}

impl ByteInnerSplitlinesOptions {
    pub fn get_value(self) -> bool {
        match self.keepends.into_option() {
            Some(x) => x,
            None => false,
        }
        // if let OptionalArg::Present(value) = self.keepends {
        //     Ok(bool::try_from_object(vm, value)?)
        // } else {
        //     Ok(false)
        // }
    }
}

#[allow(clippy::len_without_is_empty)]
impl PyByteInner {
    pub fn repr(&self) -> PyResult<String> {
        let mut res = String::with_capacity(self.elements.len());
        for i in self.elements.iter() {
            match i {
                0..=8 => res.push_str(&format!("\\x0{}", i)),
                9 => res.push_str("\\t"),
                10 => res.push_str("\\n"),
                11 => res.push_str(&format!("\\x0{:x}", i)),
                13 => res.push_str("\\r"),
                32..=126 => res.push(*(i) as char),
                _ => res.push_str(&format!("\\x{:x}", i)),
            }
        }
        Ok(res)
    }

    pub fn len(&self) -> usize {
        self.elements.len()
    }

    #[inline]
    fn cmp<F>(&self, other: PyObjectRef, op: F, vm: &VirtualMachine) -> PyComparisonValue
    where
        F: Fn(&[u8], &[u8]) -> bool,
    {
        let r = PyBytesLike::try_from_object(vm, other)
            .map(|other| other.with_ref(|other| op(&self.elements, other)));
        PyComparisonValue::from_option(r.ok())
    }

    pub fn eq(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyComparisonValue {
        self.cmp(other, |a, b| a == b, vm)
    }

    pub fn ge(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyComparisonValue {
        self.cmp(other, |a, b| a >= b, vm)
    }

    pub fn le(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyComparisonValue {
        self.cmp(other, |a, b| a <= b, vm)
    }

    pub fn gt(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyComparisonValue {
        self.cmp(other, |a, b| a > b, vm)
    }

    pub fn lt(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyComparisonValue {
        self.cmp(other, |a, b| a < b, vm)
    }

    pub fn hash(&self) -> pyhash::PyHash {
        pyhash::hash_value(&self.elements)
    }

    pub fn add(&self, other: PyByteInner) -> Vec<u8> {
        self.elements
            .iter()
            .chain(other.elements.iter())
            .cloned()
            .collect::<Vec<u8>>()
    }

    pub fn contains(
        &self,
        needle: Either<PyByteInner, PyIntRef>,
        vm: &VirtualMachine,
    ) -> PyResult<bool> {
        Ok(match needle {
            Either::A(byte) => self.elements.contains_str(byte.elements.as_slice()),
            Either::B(int) => self.elements.contains(&int.as_bigint().byte_or(vm)?),
        })
    }

    pub fn getitem(&self, needle: SequenceIndex, vm: &VirtualMachine) -> PyResult {
        match needle {
            SequenceIndex::Int(int) => {
                if let Some(idx) = self.elements.get_pos(int) {
                    Ok(vm.new_int(self.elements[idx]))
                } else {
                    Err(vm.new_index_error("index out of range".to_owned()))
                }
            }
            SequenceIndex::Slice(slice) => {
                Ok(vm.ctx.new_bytes(self.elements.get_slice_items(vm, &slice)?))
            }
        }
    }

    fn setindex(&mut self, int: isize, object: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if let Some(idx) = self.elements.get_pos(int) {
            let result = match_class!(match object {
                i @ PyInt => {
                    if let Some(value) = i.as_bigint().to_u8() {
                        Ok(value)
                    } else {
                        Err(vm.new_value_error("byte must be in range(0, 256)".to_owned()))
                    }
                }
                _ => Err(vm.new_type_error("an integer is required".to_owned())),
            });
            let value = result?;
            self.elements[idx] = value;
            Ok(vm.new_int(value))
        } else {
            Err(vm.new_index_error("index out of range".to_owned()))
        }
    }

    fn setslice(
        &mut self,
        slice: PySliceRef,
        object: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult {
        let sec = match PyIterable::try_from_object(vm, object.clone()) {
            Ok(sec) => {
                let items: Result<Vec<PyObjectRef>, _> = sec.iter(vm)?.collect();
                Ok(items?
                    .into_iter()
                    .map(|obj| u8::try_from_object(vm, obj))
                    .collect::<PyResult<Vec<_>>>()?)
            }
            _ => match_class!(match object {
                i @ PyMemoryView => Ok(i.try_value().unwrap()),
                _ => Err(vm.new_index_error(
                    "can assign only bytes, buffers, or iterables of ints in range(0, 256)"
                        .to_owned()
                )),
            }),
        };
        let items = sec?;
        let mut range = self
            .elements
            .get_slice_range(&slice.start_index(vm)?, &slice.stop_index(vm)?);
        if range.end < range.start {
            range.end = range.start;
        }
        self.elements.splice(range, items);
        Ok(vm.ctx.new_bytes(self.elements.get_slice_items(vm, &slice)?))
    }

    pub fn setitem(
        &mut self,
        needle: SequenceIndex,
        object: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult {
        match needle {
            SequenceIndex::Int(int) => self.setindex(int, object, vm),
            SequenceIndex::Slice(slice) => self.setslice(slice, object, vm),
        }
    }

    pub fn delitem(&mut self, needle: SequenceIndex, vm: &VirtualMachine) -> PyResult<()> {
        match needle {
            SequenceIndex::Int(int) => {
                if let Some(idx) = self.elements.get_pos(int) {
                    self.elements.remove(idx);
                    Ok(())
                } else {
                    Err(vm.new_index_error("index out of range".to_owned()))
                }
            }
            SequenceIndex::Slice(slice) => self.delslice(slice, vm),
        }
    }

    // TODO: deduplicate this with the code in objlist
    fn delslice(&mut self, slice: PySliceRef, vm: &VirtualMachine) -> PyResult<()> {
        let start = slice.start_index(vm)?;
        let stop = slice.stop_index(vm)?;
        let step = slice.step_index(vm)?.unwrap_or_else(BigInt::one);

        if step.is_zero() {
            Err(vm.new_value_error("slice step cannot be zero".to_owned()))
        } else if step.is_positive() {
            let range = self.elements.get_slice_range(&start, &stop);
            if range.start < range.end {
                #[allow(clippy::range_plus_one)]
                match step.to_i32() {
                    Some(1) => {
                        self._del_slice(range);
                        Ok(())
                    }
                    Some(num) => {
                        self._del_stepped_slice(range, num as usize);
                        Ok(())
                    }
                    None => {
                        self._del_slice(range.start..range.start + 1);
                        Ok(())
                    }
                }
            } else {
                // no del to do
                Ok(())
            }
        } else {
            // calculate the range for the reverse slice, first the bounds needs to be made
            // exclusive around stop, the lower number
            let start = start.as_ref().map(|x| {
                if *x == (-1).to_bigint().unwrap() {
                    self.elements.len() + BigInt::one() //.to_bigint().unwrap()
                } else {
                    x + 1
                }
            });
            let stop = stop.as_ref().map(|x| {
                if *x == (-1).to_bigint().unwrap() {
                    self.elements.len().to_bigint().unwrap()
                } else {
                    x + 1
                }
            });
            let range = self.elements.get_slice_range(&stop, &start);
            if range.start < range.end {
                match (-step).to_i32() {
                    Some(1) => {
                        self._del_slice(range);
                        Ok(())
                    }
                    Some(num) => {
                        self._del_stepped_slice_reverse(range, num as usize);
                        Ok(())
                    }
                    None => {
                        self._del_slice(range.end - 1..range.end);
                        Ok(())
                    }
                }
            } else {
                // no del to do
                Ok(())
            }
        }
    }

    fn _del_slice(&mut self, range: Range<usize>) {
        self.elements.drain(range);
    }

    fn _del_stepped_slice(&mut self, range: Range<usize>, step: usize) {
        // no easy way to delete stepped indexes so here is what we'll do
        let mut deleted = 0;
        let elements = &mut self.elements;
        let mut indexes = range.clone().step_by(step).peekable();

        for i in range.clone() {
            // is this an index to delete?
            if indexes.peek() == Some(&i) {
                // record and move on
                indexes.next();
                deleted += 1;
            } else {
                // swap towards front
                elements.swap(i - deleted, i);
            }
        }
        // then drain (the values to delete should now be contiguous at the end of the range)
        elements.drain((range.end - deleted)..range.end);
    }

    fn _del_stepped_slice_reverse(&mut self, range: Range<usize>, step: usize) {
        // no easy way to delete stepped indexes so here is what we'll do
        let mut deleted = 0;
        let elements = &mut self.elements;
        let mut indexes = range.clone().rev().step_by(step).peekable();

        for i in range.clone().rev() {
            // is this an index to delete?
            if indexes.peek() == Some(&i) {
                // record and move on
                indexes.next();
                deleted += 1;
            } else {
                // swap towards back
                elements.swap(i + deleted, i);
            }
        }
        // then drain (the values to delete should now be contiguous at teh start of the range)
        elements.drain(range.start..(range.start + deleted));
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
        // CPython _Py_bytes_islower
        let mut cased = false;
        for b in self.elements.iter() {
            let c = *b as char;
            if c.is_uppercase() {
                return false;
            } else if !cased && c.is_lowercase() {
                cased = true
            }
        }
        cased
    }

    pub fn isupper(&self) -> bool {
        // CPython _Py_bytes_isupper
        let mut cased = false;
        for b in self.elements.iter() {
            let c = *b as char;
            if c.is_lowercase() {
                return false;
            } else if !cased && c.is_uppercase() {
                cased = true
            }
        }
        cased
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

    pub fn hex(&self) -> String {
        self.elements
            .iter()
            .map(|x| format!("{:02x}", x))
            .collect::<String>()
    }

    pub fn fromhex(string: &str, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        // first check for invalid character
        for (i, c) in string.char_indices() {
            if !c.is_digit(16) && !c.is_whitespace() {
                return Err(vm.new_value_error(format!(
                    "non-hexadecimal number found in fromhex() arg at position {}",
                    i
                )));
            }
        }

        // strip white spaces
        let stripped = string.split_whitespace().collect::<String>();

        // Hex is evaluated on 2 digits
        if stripped.len() % 2 != 0 {
            return Err(vm.new_value_error(format!(
                "non-hexadecimal number found in fromhex() arg at position {}",
                stripped.len() - 1
            )));
        }

        // parse even string
        Ok(stripped
            .chars()
            .collect::<Vec<char>>()
            .chunks(2)
            .map(|x| x.to_vec().iter().collect::<String>())
            .map(|x| u8::from_str_radix(&x, 16).unwrap())
            .collect::<Vec<u8>>())
    }

    #[inline]
    fn pad(
        &self,
        options: ByteInnerPaddingOptions,
        pad: fn(&[u8], usize, u8) -> Vec<u8>,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<u8>> {
        let (width, fillchar) = options.get_value("center", vm)?;
        Ok(if self.len() as isize >= width {
            Vec::from(&self.elements[..])
        } else {
            pad(&self.elements, width as usize, fillchar)
        })
    }

    pub fn center(
        &self,
        options: ByteInnerPaddingOptions,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<u8>> {
        self.pad(options, PyCommonString::<u8>::py_center, vm)
    }

    pub fn ljust(
        &self,
        options: ByteInnerPaddingOptions,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<u8>> {
        self.pad(options, PyCommonString::<u8>::py_ljust, vm)
    }

    pub fn rjust(
        &self,
        options: ByteInnerPaddingOptions,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<u8>> {
        self.pad(options, PyCommonString::<u8>::py_rjust, vm)
    }

    pub fn count(&self, options: ByteInnerFindOptions, vm: &VirtualMachine) -> PyResult<usize> {
        let (needle, range) = options.get_value(self.elements.len(), vm)?;
        Ok(self
            .elements
            .py_count(needle.as_slice(), range, |h, n| h.find_iter(n).count()))
    }

    pub fn join(&self, iter: PyIterable<PyByteInner>, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        let mut refs = Vec::new();
        for v in iter.iter(vm)? {
            let v = v?;
            if !refs.is_empty() {
                refs.extend(&self.elements);
            }
            refs.extend(v.elements);
        }

        Ok(refs)
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

    pub fn maketrans(from: PyByteInner, to: PyByteInner, vm: &VirtualMachine) -> PyResult {
        let mut res = vec![];

        for i in 0..=255 {
            res.push(
                if let Some(position) = from.elements.iter().position(|&x| x == i) {
                    to.elements[position]
                } else {
                    i
                },
            );
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

    pub fn strip(&self, chars: OptionalOption<PyByteInner>) -> Vec<u8> {
        self.elements
            .py_strip(
                chars,
                |s, chars| s.trim_with(|c| chars.contains(&(c as u8))),
                |s| s.trim(),
            )
            .to_vec()
    }

    pub fn lstrip(&self, chars: OptionalOption<PyByteInner>) -> Vec<u8> {
        self.elements
            .py_strip(
                chars,
                |s, chars| s.trim_start_with(|c| chars.contains(&(c as u8))),
                |s| s.trim_start(),
            )
            .to_vec()
    }

    pub fn rstrip(&self, chars: OptionalOption<PyByteInner>) -> Vec<u8> {
        self.elements
            .py_strip(
                chars,
                |s, chars| s.trim_end_with(|c| chars.contains(&(c as u8))),
                |s| s.trim_end(),
            )
            .to_vec()
    }

    // new in Python 3.9
    pub fn removeprefix(&self, prefix: PyByteInner) -> Vec<u8> {
        self.elements
            .py_removeprefix(&prefix.elements, prefix.elements.len(), |s, p| {
                s.starts_with(p)
            })
            .to_vec()
    }

    // new in Python 3.9
    pub fn removesuffix(&self, suffix: PyByteInner) -> Vec<u8> {
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
        sub: &PyByteInner,
        vm: &VirtualMachine,
    ) -> PyResult<(Vec<u8>, bool, Vec<u8>)> {
        if sub.elements.is_empty() {
            return Err(vm.new_value_error("empty separator".to_owned()));
        }

        let mut sp = self.elements.splitn_str(2, &sub.elements);
        let front = sp.next().unwrap().to_vec();
        let (has_mid, back) = if let Some(back) = sp.next() {
            (true, back.to_vec())
        } else {
            (false, Vec::new())
        };
        Ok((front, has_mid, back))
    }

    pub fn rpartition(
        &self,
        sub: &PyByteInner,
        vm: &VirtualMachine,
    ) -> PyResult<(Vec<u8>, bool, Vec<u8>)> {
        if sub.elements.is_empty() {
            return Err(vm.new_value_error("empty separator".to_owned()));
        }

        let mut sp = self.elements.rsplitn_str(2, &sub.elements);
        let back = sp.next().unwrap().to_vec();
        let (has_mid, front) = if let Some(front) = sp.next() {
            (true, front.to_vec())
        } else {
            (false, Vec::new())
        };
        Ok((front, has_mid, back))
    }

    pub fn expandtabs(&self, options: pystr::ExpandTabsArgs) -> Vec<u8> {
        let tabsize = options.tabsize();
        let mut counter: usize = 0;
        let mut res = vec![];

        if tabsize == 0 {
            return self
                .elements
                .iter()
                .cloned()
                .filter(|x| *x != b'\t')
                .collect::<Vec<u8>>();
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

    pub fn splitlines(&self, options: pystr::SplitLinesArgs) -> Vec<&[u8]> {
        let mut res = vec![];

        if self.elements.is_empty() {
            return vec![];
        }

        let mut prev_index = 0;
        let mut index = 0;
        let keep = if options.keepends { 1 } else { 0 };
        let slice = &self.elements;

        while index < slice.len() {
            match slice[index] {
                b'\n' => {
                    res.push(&slice[prev_index..index + keep]);
                    index += 1;
                    prev_index = index;
                }
                b'\r' => {
                    if index + 2 <= slice.len() && slice[index + 1] == b'\n' {
                        res.push(&slice[prev_index..index + keep + keep]);
                        index += 2;
                    } else {
                        res.push(&slice[prev_index..index + keep]);
                        index += 1;
                    }
                    prev_index = index;
                }
                _x => {
                    if index == slice.len() - 1 {
                        res.push(&slice[prev_index..=index]);
                        break;
                    }
                    index += 1
                }
            }
        }

        res
    }

    pub fn zfill(&self, width: isize) -> Vec<u8> {
        bytes_zfill(&self.elements, width.to_usize().unwrap_or(0))
    }

    // len(self)>=1, from="", len(to)>=1, maxcount>=1
    fn replace_interleave(&self, to: PyByteInner, maxcount: Option<usize>) -> Vec<u8> {
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

    fn replace_delete(&self, from: PyByteInner, maxcount: Option<usize>) -> Vec<u8> {
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
        from: PyByteInner,
        to: PyByteInner,
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
        from: PyByteInner,
        to: PyByteInner,
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
        from: PyByteInner,
        to: PyByteInner,
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

    pub fn repeat(&self, n: isize) -> Vec<u8> {
        if self.elements.is_empty() || n <= 0 {
            // We can multiple an empty vector by any integer, even if it doesn't fit in an isize.
            Vec::new()
        } else {
            let n = usize::try_from(n).unwrap();

            let mut new_value = Vec::with_capacity(n * self.elements.len());
            for _ in 0..n {
                new_value.extend(&self.elements);
            }

            new_value
        }
    }

    pub fn irepeat(&mut self, n: isize) {
        if self.elements.is_empty() {
            // We can multiple an empty vector by any integer, even if it doesn't fit in an isize.
            return;
        }

        if n <= 0 {
            self.elements.clear();
        } else {
            let n = usize::try_from(n).unwrap();

            let old = self.elements.clone();

            self.elements.reserve((n - 1) * old.len());
            for _ in 1..n {
                self.elements.extend(&old);
            }
        }
    }
}

pub fn try_as_byte(obj: PyObjectRef) -> Option<Vec<u8>> {
    match_class!(match obj {
        i @ PyBytes => Some(i.get_value().to_vec()),
        j @ PyByteArray => Some(j.borrow_value().elements.to_vec()),
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

pub enum PyBytesLike {
    Bytes(PyBytesRef),
    Bytearray(PyByteArrayRef),
}

impl TryFromObject for PyBytesLike {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        match_class!(match obj {
            b @ PyBytes => Ok(PyBytesLike::Bytes(b)),
            b @ PyByteArray => Ok(PyBytesLike::Bytearray(b)),
            obj => Err(vm.new_type_error(format!(
                "a bytes-like object is required, not {}",
                obj.class()
            ))),
        })
    }
}

impl PyBytesLike {
    pub fn to_cow(&self) -> std::borrow::Cow<[u8]> {
        match self {
            PyBytesLike::Bytes(b) => b.get_value().into(),
            PyBytesLike::Bytearray(b) => b.borrow_value().elements.clone().into(),
        }
    }

    #[inline]
    pub fn with_ref<R>(&self, f: impl FnOnce(&[u8]) -> R) -> R {
        match self {
            PyBytesLike::Bytes(b) => f(b.get_value()),
            PyBytesLike::Bytearray(b) => f(&b.borrow_value().elements),
        }
    }
}

pub fn bytes_zfill(bytes: &[u8], width: usize) -> Vec<u8> {
    if width <= bytes.len() {
        bytes.to_vec()
    } else {
        let (sign, s) = match bytes.first() {
            Some(_sign @ b'+') | Some(_sign @ b'-') => {
                (unsafe { bytes.get_unchecked(..1) }, &bytes[1..])
            }
            _ => (&b""[..], bytes),
        };
        let mut filled = Vec::new();
        filled.extend_from_slice(sign);
        filled.extend(std::iter::repeat(b'0').take(width - bytes.len()));
        filled.extend_from_slice(s);
        filled
    }
}

impl PyCommonStringWrapper<[u8]> for PyByteInner {
    fn as_ref(&self) -> &[u8] {
        &self.elements
    }
}

const ASCII_WHITESPACES: [u8; 6] = [0x20, 0x09, 0x0a, 0x0c, 0x0d, 0x0b];

impl PyCommonString<u8> for [u8] {
    type Container = Vec<u8>;

    fn with_capacity(capacity: usize) -> Self::Container {
        Vec::with_capacity(capacity)
    }

    fn get_bytes<'a>(&'a self, range: std::ops::Range<usize>) -> &'a Self {
        &self[range]
    }

    fn get_chars<'a>(&'a self, range: std::ops::Range<usize>) -> &'a Self {
        &self[range]
    }

    fn is_empty(&self) -> bool {
        Self::is_empty(self)
    }

    fn bytes_len(&self) -> usize {
        Self::len(self)
    }

    fn chars_len(&self) -> usize {
        Self::len(self)
    }

    fn py_split_whitespace<F>(&self, maxsplit: isize, convert: F) -> Vec<PyObjectRef>
    where
        F: Fn(&Self) -> PyObjectRef,
    {
        let mut splited = Vec::new();
        let mut count = maxsplit;
        let mut haystack = &self[..];
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
        let mut haystack = &self[..];
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

    fn py_pad(&self, left: usize, right: usize, fill: u8) -> Self::Container {
        let mut u = Vec::with_capacity(left + self.len() + right);
        u.extend(std::iter::repeat(fill).take(left));
        u.extend_from_slice(self);
        u.extend(std::iter::repeat(fill).take(right));
        u
    }
}

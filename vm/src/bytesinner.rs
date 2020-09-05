use bstr::ByteSlice;
use itertools::Itertools;
use num_bigint::BigInt;
use num_traits::ToPrimitive;

use crate::byteslike::PyBytesLike;
use crate::function::{OptionalArg, OptionalOption};
use crate::obj::objbytearray::PyByteArray;
use crate::obj::objbytes::PyBytes;
use crate::obj::objint::{self, PyInt, PyIntRef};
use crate::obj::objlist::PyList;
use crate::obj::objmemory::PyMemoryView;
use crate::obj::objnone::PyNoneRef;
use crate::obj::objsequence::{PySliceableSequence, PySliceableSequenceMut, SequenceIndex};
use crate::obj::objslice::PySliceRef;
use crate::obj::objstr::{self, PyString, PyStringRef};
use crate::pyobject::{
    BorrowValue, Either, PyComparisonValue, PyIterable, PyObjectRef, PyResult, TryFromObject,
    TypeProtocol,
};
use crate::pystr::{self, PyCommonString, PyCommonStringContainer, PyCommonStringWrapper};
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
        match_class!(match obj {
            i @ PyBytes => Ok(PyBytesInner {
                elements: i.borrow_value().to_vec()
            }),
            j @ PyByteArray => Ok(PyBytesInner {
                elements: j.borrow_value().elements.to_vec()
            }),
            k @ PyMemoryView => Ok(PyBytesInner {
                elements: k.try_bytes(|v| v.to_vec()).unwrap()
            }),
            l @ PyList => l.to_byte_inner(vm),
            obj => {
                let iter = vm.get_method_or_type_error(obj.clone(), "__iter__", || {
                    format!("a bytes-like object is required, not {}", obj.class())
                })?;
                let iter = PyIterable::from_method(iter);
                Ok(PyBytesInner {
                    elements: iter.iter(vm)?.collect::<PyResult<_>>()?,
                })
            }
        })
    }
}

#[derive(FromArgs)]
pub struct ByteInnerNewOptions {
    #[pyarg(positional_or_keyword, optional = true)]
    source: OptionalArg<PyObjectRef>,
    #[pyarg(positional_or_keyword, optional = true)]
    encoding: OptionalArg<PyStringRef>,
    #[pyarg(positional_or_keyword, optional = true)]
    errors: OptionalArg<PyStringRef>,
}

impl ByteInnerNewOptions {
    pub fn get_value(self, vm: &VirtualMachine) -> PyResult<PyBytesInner> {
        match self.source {
            OptionalArg::Missing => {
                if let OptionalArg::Present(_) = self.encoding {
                    Err(vm.new_type_error("encoding without a string argument".to_owned()))
                } else if let OptionalArg::Present(_) = self.errors {
                    Err(vm.new_type_error("errors without a string argument".to_owned()))
                } else {
                    Ok(PyBytesInner {
                        elements: Vec::new(),
                    })
                }
            }
            OptionalArg::Present(obj) => {
                match obj.downcast::<PyString>() {
                    Ok(s) => {
                        // Handle bytes(string, encoding[, errors])
                        if let OptionalArg::Present(enc) = self.encoding {
                            let bytes =
                                objstr::encode_string(s, Some(enc), self.errors.into_option(), vm)?;
                            Ok(PyBytesInner {
                                elements: bytes.borrow_value().to_vec(),
                            })
                        } else {
                            Err(vm.new_type_error("string argument without an encoding".to_owned()))
                        }
                    }
                    Err(obj) => {
                        if let OptionalArg::Present(_) = self.encoding {
                            Err(vm.new_type_error("encoding without a string argument".to_owned()))
                        } else if let OptionalArg::Present(_) = self.errors {
                            Err(vm.new_type_error("errors without a string argument".to_owned()))
                        } else {
                            let value = match_class!(match obj {
                                i @ PyInt => {
                                    let size = objint::get_value(&i.into_object())
                                        .to_isize()
                                        .ok_or_else(|| {
                                            vm.new_overflow_error(
                                                "cannot fit 'int' into an index-sized integer"
                                                    .to_owned(),
                                            )
                                        })?;
                                    let size = if size < 0 {
                                        return Err(vm.new_value_error("negative count".to_owned()));
                                    } else {
                                        size as usize
                                    };
                                    Ok(vec![0; size])
                                }
                                i @ PyBytes => Ok(i.borrow_value().to_vec()),
                                j @ PyByteArray => Ok(j.borrow_value().elements.to_vec()),
                                obj => {
                                    // TODO: only support this method in the bytes() constructor
                                    if let Some(bytes_method) =
                                        vm.get_method(obj.clone(), "__bytes__")
                                    {
                                        let bytes = vm.invoke(&bytes_method?, vec![])?;
                                        return PyBytesInner::try_from_object(vm, bytes);
                                    }
                                    let elements = vm.extract_elements::<PyIntRef>(&obj)?;
                                    // TODO: better error message
                                    // .map_err(|_| {
                                    //     vm.new_type_error(format!(
                                    //         "cannot convert '{}' object to bytes",
                                    //         obj.class().name
                                    //     ))
                                    // })?;

                                    elements
                                        .into_iter()
                                        .map(|elem| {
                                            elem.borrow_value().to_u8().ok_or_else(|| {
                                                vm.new_value_error(
                                                    "bytes must be in range(0, 256)".to_owned(),
                                                )
                                            })
                                        })
                                        .collect()
                                }
                            });

                            value.map(|v| PyBytesInner { elements: v })
                        }
                    }
                }
            }
        }
    }
}

#[derive(FromArgs)]
pub struct ByteInnerFindOptions {
    #[pyarg(positional_only, optional = false)]
    sub: Either<PyBytesInner, PyIntRef>,
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
            Either::B(int) => vec![int.borrow_value().byte_or(vm)?],
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
    #[pyarg(positional_only, optional = false)]
    table: Either<PyBytesInner, PyNoneRef>,
    #[pyarg(positional_or_keyword, optional = true)]
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

pub type ByteInnerSplitOptions<'a> = pystr::SplitArgs<'a, PyBytesInner, [u8], u8>;

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

    pub fn ne(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyComparisonValue {
        self.eq(other, vm).map(|v| !v)
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

    pub fn hash(&self, vm: &VirtualMachine) -> hash::PyHash {
        vm.state.hash_secret.hash_bytes(&self.elements)
    }

    pub fn add(&self, other: PyBytesInner) -> Vec<u8> {
        self.elements.py_add(&other.elements)
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

    pub fn getitem(&self, needle: SequenceIndex, vm: &VirtualMachine) -> PyResult {
        match needle {
            SequenceIndex::Int(int) => {
                if let Some(idx) = self.elements.get_pos(int) {
                    Ok(vm.ctx.new_int(self.elements[idx]))
                } else {
                    Err(vm.new_index_error("index out of range".to_owned()))
                }
            }
            SequenceIndex::Slice(slice) => {
                Ok(vm.ctx.new_bytes(self.elements.get_slice_items(vm, &slice)?))
            }
        }
    }

    fn setindex(&mut self, int: isize, object: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        if let Some(idx) = self.elements.get_pos(int) {
            let result = match_class!(match object {
                i @ PyInt => {
                    if let Some(value) = i.borrow_value().to_u8() {
                        Ok(value)
                    } else {
                        Err(vm.new_value_error("byte must be in range(0, 256)".to_owned()))
                    }
                }
                _ => Err(vm.new_type_error("an integer is required".to_owned())),
            });
            let value = result?;
            self.elements[idx] = value;
            Ok(())
        } else {
            Err(vm.new_index_error("index out of range".to_owned()))
        }
    }

    fn setslice(
        &mut self,
        slice: PySliceRef,
        object: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let sec = match PyIterable::try_from_object(vm, object.clone()) {
            Ok(sec) => {
                let items: Result<Vec<PyObjectRef>, _> = sec.iter(vm)?.collect();
                Ok(items?
                    .into_iter()
                    .map(|obj| u8::try_from_object(vm, obj))
                    .collect::<PyResult<Vec<_>>>()?)
            }
            _ => match_class!(match object {
                i @ PyMemoryView => Ok(i.try_bytes(|v| v.to_vec()).unwrap()),
                _ => Err(vm.new_value_error(
                    "can assign only bytes, buffers, or iterables of ints in range(0, 256)"
                        .to_owned()
                )),
            }),
        };
        let items = sec?;
        self.elements.set_slice_items(vm, &slice, items.as_slice())
    }

    pub fn setitem(
        &mut self,
        needle: SequenceIndex,
        object: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
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

    fn delslice(&mut self, slice: PySliceRef, vm: &VirtualMachine) -> PyResult<()> {
        self.elements.delete_slice(vm, &slice)
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
        self._pad(options, PyCommonString::<u8>::py_center, vm)
    }

    pub fn ljust(
        &self,
        options: ByteInnerPaddingOptions,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<u8>> {
        self._pad(options, PyCommonString::<u8>::py_ljust, vm)
    }

    pub fn rjust(
        &self,
        options: ByteInnerPaddingOptions,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<u8>> {
        self._pad(options, PyCommonString::<u8>::py_rjust, vm)
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

    pub fn expandtabs(&self, options: pystr::ExpandTabsArgs) -> Vec<u8> {
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

    pub fn splitlines<FW, W>(&self, options: pystr::SplitLinesArgs, into_wrapper: FW) -> Vec<W>
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

    pub fn cformat(&self, values: PyObjectRef, vm: &VirtualMachine) -> PyResult<String> {
        self.elements.py_cformat(values, vm)
    }

    pub fn repeat(&self, n: isize) -> Vec<u8> {
        self.elements.repeat(n.to_usize().unwrap_or(0))
    }

    pub fn irepeat(&mut self, n: isize) {
        if self.elements.is_empty() {
            // We can multiple an empty vector by any integer, even if it doesn't fit in an isize.
            return;
        }

        if n <= 0 {
            self.elements.clear();
        } else {
            let n = n.to_usize().unwrap(); // always positive by outer if condition

            let old = self.elements.clone();

            self.elements.reserve((n - 1) * old.len());
            for _ in 1..n {
                self.elements.extend(&old);
            }
        }
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

impl PyCommonStringWrapper<[u8]> for PyBytesInner {
    fn as_ref(&self) -> &[u8] {
        &self.elements
    }
}

impl PyCommonStringContainer<[u8]> for Vec<u8> {
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

impl<'s> PyCommonString<'s, u8> for [u8] {
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
}

#[derive(FromArgs)]
pub struct DecodeArgs {
    #[pyarg(positional_or_keyword, default = "None")]
    encoding: Option<PyStringRef>,
    #[pyarg(positional_or_keyword, default = "None")]
    errors: Option<PyStringRef>,
}

pub fn bytes_decode(
    zelf: PyObjectRef,
    args: DecodeArgs,
    vm: &VirtualMachine,
) -> PyResult<PyStringRef> {
    let DecodeArgs { encoding, errors } = args;
    vm.decode(zelf, encoding.clone(), errors)?
        .downcast::<PyString>()
        .map_err(|obj| {
            vm.new_type_error(format!(
                "'{}' decoder returned '{}' instead of 'str'; use codecs.encode() to \
                     encode arbitrary types",
                encoding.as_ref().map_or("utf-8", |s| s.borrow_value()),
                obj.lease_class().name,
            ))
        })
}

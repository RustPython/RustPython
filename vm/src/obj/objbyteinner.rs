use crate::obj::objint::PyIntRef;
use crate::obj::objnone::PyNoneRef;
use crate::obj::objslice::PySliceRef;
use crate::obj::objtuple::PyTupleRef;
use crate::pyhash;
use crate::pyobject::Either;
use crate::pyobject::PyRef;
use crate::pyobject::PyValue;
use crate::pyobject::TryFromObject;
use crate::pyobject::{PyIterable, PyObjectRef};
use core::convert::TryFrom;
use core::ops::Range;
use num_bigint::BigInt;

use crate::function::OptionalArg;
use crate::pyobject::{PyResult, TypeProtocol};
use crate::vm::VirtualMachine;

use super::objint;
use super::objsequence::{is_valid_slice_arg, PySliceableSequence};
use super::objstr::{PyString, PyStringRef};

use crate::obj::objint::PyInt;
use num_integer::Integer;
use num_traits::ToPrimitive;

use super::objbytearray::PyByteArray;
use super::objbytes::PyBytes;
use super::objmemory::PyMemoryView;

use super::objsequence;

#[derive(Debug, Default, Clone)]
pub struct PyByteInner {
    pub elements: Vec<u8>,
}

impl TryFromObject for PyByteInner {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        match_class!(obj,

            i @ PyBytes => Ok(PyByteInner{elements: i.get_value().to_vec()}),
            j @ PyByteArray => Ok(PyByteInner{elements: j.inner.borrow().elements.to_vec()}),
            k @ PyMemoryView => Ok(PyByteInner{elements: k.get_obj_value().unwrap()}),
            obj => Err(vm.new_type_error(format!(
                        "a bytes-like object is required, not {}",
                        obj.class()
                    )))
        )
    }
}

impl<B: PyValue> TryFromObject for Either<PyByteInner, PyRef<B>> {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        match PyByteInner::try_from_object(vm, obj.clone()) {
            Ok(a) => Ok(Either::A(a)),
            Err(_) => match obj.clone().downcast::<B>() {
                Ok(b) => Ok(Either::B(b)),
                Err(_) => Err(vm.new_type_error(format!(
                    "a bytes-like object or {} is required, not {}",
                    B::class(vm),
                    obj.class()
                ))),
            },
        }
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
                    let encoding = enc.as_str();
                    if encoding.to_lowercase() == "utf8" || encoding.to_lowercase() == "utf-8"
                    // TODO: different encoding
                    {
                        return Ok(PyByteInner {
                            elements: input.value.as_bytes().to_vec(),
                        });
                    } else {
                        return Err(
                            vm.new_value_error(format!("unknown encoding: {}", encoding)), //should be lookup error
                        );
                    }
                } else {
                    return Err(vm.new_type_error("encoding without a string argument".to_string()));
                }
            } else {
                return Err(vm.new_type_error("encoding without a string argument".to_string()));
            }
        // Only one argument
        } else {
            let value = if let OptionalArg::Present(ival) = self.val_option {
                match_class!(ival.clone(),
                    i @ PyInt => {
                            let size = objint::get_value(&i.into_object()).to_usize().unwrap();
                            Ok(vec![0; size])},
                    _l @ PyString=> {return Err(vm.new_type_error("string argument without an encoding".to_string()));},
                    obj => {
                        let elements = vm.extract_elements(&obj).or_else(|_| {Err(vm.new_type_error(format!(
                        "cannot convert {} object to bytes", obj.class().name)))});

                        let mut data_bytes = vec![];
                        for elem in elements.unwrap(){
                            let v = objint::to_int(vm, &elem, 10)?;
                            if let Some(i) = v.to_u8() {
                                data_bytes.push(i);
                            } else {
                                return Err(vm.new_value_error("bytes must be in range(0, 256)".to_string()));
                                }

                            }
                        Ok(data_bytes)
                        }
                )
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
    #[pyarg(positional_only, optional = true)]
    start: OptionalArg<Option<PyIntRef>>,
    #[pyarg(positional_only, optional = true)]
    end: OptionalArg<Option<PyIntRef>>,
}

impl ByteInnerFindOptions {
    pub fn get_value(
        self,
        elements: &[u8],
        vm: &VirtualMachine,
    ) -> PyResult<(Vec<u8>, Range<usize>)> {
        let sub = match self.sub {
            Either::A(v) => v.elements.to_vec(),
            Either::B(int) => vec![int.as_bigint().byte_or(vm)?],
        };

        let start = match self.start {
            OptionalArg::Present(Some(int)) => Some(int.as_bigint().clone()),
            _ => None,
        };

        let end = match self.end {
            OptionalArg::Present(Some(int)) => Some(int.as_bigint().clone()),
            _ => None,
        };

        let range = elements.to_vec().get_slice_range(&start, &end);

        Ok((sub, range))
    }
}

#[derive(FromArgs)]
pub struct ByteInnerPaddingOptions {
    #[pyarg(positional_only, optional = false)]
    width: PyIntRef,
    #[pyarg(positional_only, optional = true)]
    fillbyte: OptionalArg<PyObjectRef>,
}
impl ByteInnerPaddingOptions {
    fn get_value(self, fn_name: &str, len: usize, vm: &VirtualMachine) -> PyResult<(u8, usize)> {
        let fillbyte = if let OptionalArg::Present(v) = &self.fillbyte {
            match try_as_byte(&v) {
                Some(x) => {
                    if x.len() == 1 {
                        x[0]
                    } else {
                        return Err(vm.new_type_error(format!(
                            "{}() argument 2 must be a byte string of length 1, not {}",
                            fn_name, &v
                        )));
                    }
                }
                None => {
                    return Err(vm.new_type_error(format!(
                        "{}() argument 2 must be a byte string of length 1, not {}",
                        fn_name, &v
                    )));
                }
            }
        } else {
            b' ' // default is space
        };

        // <0 = no change
        let width = if let Some(x) = self.width.as_bigint().to_usize() {
            if x <= len {
                0
            } else {
                x
            }
        } else {
            0
        };

        let diff: usize = if width != 0 { width - len } else { 0 };

        Ok((fillbyte, diff))
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
                vm.new_value_error("translation table must be 256 characters long".to_string())
            );
        }

        let delete = match self.delete {
            OptionalArg::Present(byte) => byte.elements,
            _ => vec![],
        };

        Ok((table, delete))
    }
}

#[derive(FromArgs)]
pub struct ByteInnerSplitOptions {
    #[pyarg(positional_or_keyword, optional = true)]
    sep: OptionalArg<Option<PyByteInner>>,
    #[pyarg(positional_or_keyword, optional = true)]
    maxsplit: OptionalArg<PyIntRef>,
}

impl ByteInnerSplitOptions {
    pub fn get_value(self) -> PyResult<(Vec<u8>, i32)> {
        let sep = match self.sep.into_option() {
            Some(Some(bytes)) => bytes.elements,
            _ => vec![],
        };

        let maxsplit = if let OptionalArg::Present(value) = self.maxsplit {
            value.as_bigint().to_i32().unwrap()
        } else {
            -1
        };

        Ok((sep.clone(), maxsplit))
    }
}

#[derive(FromArgs)]
pub struct ByteInnerExpandtabsOptions {
    #[pyarg(positional_or_keyword, optional = true)]
    tabsize: OptionalArg<PyIntRef>,
}

impl ByteInnerExpandtabsOptions {
    pub fn get_value(self) -> usize {
        match self.tabsize.into_option() {
            Some(int) => int.as_bigint().to_usize().unwrap_or(0),
            None => 8,
        }
    }
}

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

    pub fn is_empty(&self) -> bool {
        self.elements.len() == 0
    }

    pub fn eq(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if let Ok(other) = PyByteInner::try_from_object(vm, other) {
            Ok(vm.new_bool(self.elements == other.elements))
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    pub fn ge(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if let Ok(other) = PyByteInner::try_from_object(vm, other) {
            Ok(vm.new_bool(self.elements >= other.elements))
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    pub fn le(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if let Ok(other) = PyByteInner::try_from_object(vm, other) {
            Ok(vm.new_bool(self.elements <= other.elements))
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    pub fn gt(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if let Ok(other) = PyByteInner::try_from_object(vm, other) {
            Ok(vm.new_bool(self.elements > other.elements))
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    pub fn lt(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if let Ok(other) = PyByteInner::try_from_object(vm, other) {
            Ok(vm.new_bool(self.elements < other.elements))
        } else {
            Ok(vm.ctx.not_implemented())
        }
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

    pub fn contains(&self, needle: Either<PyByteInner, PyIntRef>, vm: &VirtualMachine) -> PyResult {
        match needle {
            Either::A(byte) => {
                let other = &byte.elements[..];
                for (n, i) in self.elements.iter().enumerate() {
                    if n + other.len() <= self.len()
                        && *i == other[0]
                        && &self.elements[n..n + other.len()] == other
                    {
                        return Ok(vm.new_bool(true));
                    }
                }
                Ok(vm.new_bool(false))
            }
            Either::B(int) => {
                if self.elements.contains(&int.as_bigint().byte_or(vm)?) {
                    Ok(vm.new_bool(true))
                } else {
                    Ok(vm.new_bool(false))
                }
            }
        }
    }

    pub fn getitem(&self, needle: Either<PyIntRef, PySliceRef>, vm: &VirtualMachine) -> PyResult {
        match needle {
            Either::A(int) => {
                if let Some(idx) = self.elements.get_pos(int.as_bigint().to_i32().unwrap()) {
                    Ok(vm.new_int(self.elements[idx]))
                } else {
                    Err(vm.new_index_error("index out of range".to_string()))
                }
            }
            Either::B(slice) => Ok(vm
                .ctx
                .new_bytes(self.elements.get_slice_items(vm, slice.as_object())?)),
        }
    }

    pub fn isalnum(&self, vm: &VirtualMachine) -> PyResult {
        Ok(vm.new_bool(
            !self.elements.is_empty()
                && self
                    .elements
                    .iter()
                    .all(|x| char::from(*x).is_alphanumeric()),
        ))
    }

    pub fn isalpha(&self, vm: &VirtualMachine) -> PyResult {
        Ok(vm.new_bool(
            !self.elements.is_empty()
                && self.elements.iter().all(|x| char::from(*x).is_alphabetic()),
        ))
    }

    pub fn isascii(&self, vm: &VirtualMachine) -> PyResult {
        Ok(vm.new_bool(
            !self.elements.is_empty() && self.elements.iter().all(|x| char::from(*x).is_ascii()),
        ))
    }

    pub fn isdigit(&self, vm: &VirtualMachine) -> PyResult {
        Ok(vm.new_bool(
            !self.elements.is_empty() && self.elements.iter().all(|x| char::from(*x).is_digit(10)),
        ))
    }

    pub fn islower(&self, vm: &VirtualMachine) -> PyResult {
        Ok(vm.new_bool(
            !self.elements.is_empty()
                && self
                    .elements
                    .iter()
                    .filter(|x| !char::from(**x).is_whitespace())
                    .all(|x| char::from(*x).is_lowercase()),
        ))
    }

    pub fn isspace(&self, vm: &VirtualMachine) -> PyResult {
        Ok(vm.new_bool(
            !self.elements.is_empty()
                && self.elements.iter().all(|x| char::from(*x).is_whitespace()),
        ))
    }

    pub fn isupper(&self, vm: &VirtualMachine) -> PyResult {
        Ok(vm.new_bool(
            !self.elements.is_empty()
                && self
                    .elements
                    .iter()
                    .filter(|x| !char::from(**x).is_whitespace())
                    .all(|x| char::from(*x).is_uppercase()),
        ))
    }

    pub fn istitle(&self, vm: &VirtualMachine) -> PyResult {
        if self.elements.is_empty() {
            return Ok(vm.new_bool(false));
        }

        let mut iter = self.elements.iter().peekable();
        let mut prev_cased = false;

        while let Some(c) = iter.next() {
            let current = char::from(*c);
            let next = if let Some(k) = iter.peek() {
                char::from(**k)
            } else if current.is_uppercase() {
                return Ok(vm.new_bool(!prev_cased));
            } else {
                return Ok(vm.new_bool(prev_cased));
            };

            let is_cased = current.to_uppercase().next().unwrap() != current
                || current.to_lowercase().next().unwrap() != current;
            if (is_cased && next.is_uppercase() && !prev_cased)
                || (!is_cased && next.is_lowercase())
            {
                return Ok(vm.new_bool(false));
            }

            prev_cased = is_cased;
        }

        Ok(vm.new_bool(true))
    }

    pub fn lower(&self, _vm: &VirtualMachine) -> Vec<u8> {
        self.elements.to_ascii_lowercase()
    }

    pub fn upper(&self, _vm: &VirtualMachine) -> Vec<u8> {
        self.elements.to_ascii_uppercase()
    }

    pub fn capitalize(&self, _vm: &VirtualMachine) -> Vec<u8> {
        let mut new: Vec<u8> = Vec::new();
        if let Some((first, second)) = self.elements.split_first() {
            new.push(first.to_ascii_uppercase());
            second.iter().for_each(|x| new.push(x.to_ascii_lowercase()));
        }
        new
    }

    pub fn swapcase(&self, _vm: &VirtualMachine) -> Vec<u8> {
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

    pub fn hex(&self, vm: &VirtualMachine) -> PyResult {
        let bla = self
            .elements
            .iter()
            .map(|x| format!("{:02x}", x))
            .collect::<String>();
        Ok(vm.ctx.new_str(bla))
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

    pub fn center(
        &self,
        options: ByteInnerPaddingOptions,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<u8>> {
        let (fillbyte, diff) = options.get_value("center", self.len(), vm)?;

        let mut ln: usize = diff / 2;
        let mut rn: usize = ln;

        if diff.is_odd() && self.len() % 2 == 0 {
            ln += 1
        }

        if diff.is_odd() && self.len() % 2 != 0 {
            rn += 1
        }

        // merge all
        let mut res = vec![fillbyte; ln];
        res.extend_from_slice(&self.elements[..]);
        res.extend_from_slice(&vec![fillbyte; rn][..]);

        Ok(res)
    }

    pub fn ljust(
        &self,
        options: ByteInnerPaddingOptions,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<u8>> {
        let (fillbyte, diff) = options.get_value("ljust", self.len(), vm)?;

        // merge all
        let mut res = vec![];
        res.extend_from_slice(&self.elements[..]);
        res.extend_from_slice(&vec![fillbyte; diff][..]);

        Ok(res)
    }

    pub fn rjust(
        &self,
        options: ByteInnerPaddingOptions,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<u8>> {
        let (fillbyte, diff) = options.get_value("rjust", self.len(), vm)?;

        // merge all
        let mut res = vec![fillbyte; diff];
        res.extend_from_slice(&self.elements[..]);

        Ok(res)
    }

    pub fn count(&self, options: ByteInnerFindOptions, vm: &VirtualMachine) -> PyResult<usize> {
        let (sub, range) = options.get_value(&self.elements, vm)?;

        if sub.is_empty() {
            return Ok(self.len() + 1);
        }

        let mut total: usize = 0;
        let mut i_start = range.start;
        let i_end = range.end;

        for i in self.elements.do_slice(range) {
            if i_start + sub.len() <= i_end
                && i == sub[0]
                && &self.elements[i_start..(i_start + sub.len())] == sub.as_slice()
            {
                total += 1;
            }
            i_start += 1;
        }
        Ok(total)
    }

    pub fn join(&self, iter: PyIterable, vm: &VirtualMachine) -> PyResult {
        let mut refs = vec![];
        for v in iter.iter(vm)? {
            let v = v?;
            refs.extend(PyByteInner::try_from_object(vm, v)?.elements)
        }

        Ok(vm.ctx.new_bytes(refs))
    }

    pub fn startsendswith(
        &self,
        arg: Either<PyByteInner, PyTupleRef>,
        start: OptionalArg<PyObjectRef>,
        end: OptionalArg<PyObjectRef>,
        endswith: bool, // true for endswith, false for startswith
        vm: &VirtualMachine,
    ) -> PyResult {
        let suff = match arg {
            Either::A(byte) => byte.elements,
            Either::B(tuple) => {
                let mut flatten = vec![];
                for v in objsequence::get_elements(tuple.as_object()).to_vec() {
                    flatten.extend(PyByteInner::try_from_object(vm, v)?.elements)
                }
                flatten
            }
        };

        if suff.is_empty() {
            return Ok(vm.new_bool(true));
        }
        let range = self.elements.get_slice_range(
            &is_valid_slice_arg(start, vm)?,
            &is_valid_slice_arg(end, vm)?,
        );

        if range.end - range.start < suff.len() {
            return Ok(vm.new_bool(false));
        }

        let offset = if endswith {
            (range.end - suff.len())..range.end
        } else {
            0..suff.len()
        };

        Ok(vm.new_bool(suff.as_slice() == &self.elements.do_slice(range)[offset]))
    }

    pub fn find(
        &self,
        options: ByteInnerFindOptions,
        reverse: bool,
        vm: &VirtualMachine,
    ) -> PyResult<isize> {
        let (sub, range) = options.get_value(&self.elements, vm)?;
        // not allowed for this method
        if range.end < range.start {
            return Ok(-1isize);
        }

        let start = range.start;
        let end = range.end;

        if reverse {
            let slice = self.elements.do_slice_reverse(range);
            for (n, _) in slice.iter().enumerate() {
                if n + sub.len() <= slice.len() && &slice[n..n + sub.len()] == sub.as_slice() {
                    return Ok((end - n - 1) as isize);
                }
            }
        } else {
            let slice = self.elements.do_slice(range);
            for (n, _) in slice.iter().enumerate() {
                if n + sub.len() <= slice.len() && &slice[n..n + sub.len()] == sub.as_slice() {
                    return Ok((start + n) as isize);
                }
            }
        };
        Ok(-1isize)
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

    pub fn translate(&self, options: ByteInnerTranslateOptions, vm: &VirtualMachine) -> PyResult {
        let (table, delete) = options.get_value(vm)?;

        let mut res = vec![];

        for i in self.elements.iter() {
            if !delete.contains(&i) {
                res.push(table[*i as usize]);
            }
        }

        Ok(vm.ctx.new_bytes(res))
    }

    pub fn strip(
        &self,
        chars: OptionalArg<PyByteInner>,
        position: ByteInnerPosition,
        _vm: &VirtualMachine,
    ) -> PyResult<Vec<u8>> {
        let chars = if let OptionalArg::Present(bytes) = chars {
            bytes.elements
        } else {
            vec![b' ']
        };

        let mut start = 0;
        let mut end = self.len();

        if let ByteInnerPosition::Left | ByteInnerPosition::All = position {
            for (n, i) in self.elements.iter().enumerate() {
                if !chars.contains(i) {
                    start = n;
                    break;
                }
            }
        }

        if let ByteInnerPosition::Right | ByteInnerPosition::All = position {
            for (n, i) in self.elements.iter().rev().enumerate() {
                if !chars.contains(i) {
                    end = self.len() - n;
                    break;
                }
            }
        }
        Ok(self.elements[start..end].to_vec())
    }

    pub fn split(&self, options: ByteInnerSplitOptions, reverse: bool) -> PyResult<Vec<&[u8]>> {
        let (sep, maxsplit) = options.get_value()?;

        if self.elements.is_empty() {
            if !sep.is_empty() {
                return Ok(vec![&[]]);
            }
            return Ok(vec![]);
        }

        if reverse {
            Ok(split_slice_reverse(&self.elements, &sep, maxsplit))
        } else {
            Ok(split_slice(&self.elements, &sep, maxsplit))
        }
    }

    pub fn partition(&self, sep: &PyByteInner, reverse: bool) -> PyResult<(Vec<u8>, Vec<u8>)> {
        let splitted = if reverse {
            split_slice_reverse(&self.elements, &sep.elements, 1)
        } else {
            split_slice(&self.elements, &sep.elements, 1)
        };
        Ok((splitted[0].to_vec(), splitted[1].to_vec()))
    }

    pub fn expandtabs(&self, options: ByteInnerExpandtabsOptions) -> Vec<u8> {
        let tabsize = options.get_value();
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

    pub fn splitlines(&self, options: ByteInnerSplitlinesOptions) -> Vec<&[u8]> {
        let keepends = options.get_value();

        let mut res = vec![];

        if self.elements.is_empty() {
            return vec![];
        }

        let mut prev_index = 0;
        let mut index = 0;
        let keep = if keepends { 1 } else { 0 };
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

    pub fn zfill(&self, width: PyIntRef) -> Vec<u8> {
        if let Some(value) = width.as_bigint().to_usize() {
            if value < self.elements.len() {
                return self.elements.to_vec();
            }
            let mut res = vec![];
            if self.elements.starts_with(&[b'-']) {
                res.push(b'-');
                res.extend_from_slice(&vec![b'0'; value - self.elements.len()]);
                res.extend_from_slice(&self.elements[1..]);
            } else {
                res.extend_from_slice(&vec![b'0'; value - self.elements.len()]);
                res.extend_from_slice(&self.elements[0..]);
            }
            res
        } else {
            self.elements.to_vec()
        }
    }

    pub fn replace(
        &self,
        old: PyByteInner,
        new: PyByteInner,
        count: OptionalArg<PyIntRef>,
    ) -> PyResult<Vec<u8>> {
        let count = match count.into_option() {
            Some(int) => int
                .as_bigint()
                .to_u32()
                .unwrap_or(self.elements.len() as u32),
            None => self.elements.len() as u32,
        };

        let mut res = vec![];
        let mut index = 0;
        let mut done = 0;

        let slice = &self.elements;
        while index <= slice.len() - old.len() {
            if done == count {
                res.extend_from_slice(&slice[index..]);
                break;
            }
            if &slice[index..index + old.len()] == old.elements.as_slice() {
                res.extend_from_slice(&new.elements);
                index += old.len();
                done += 1;
            } else {
                res.push(slice[index]);
                index += 1
            }
        }

        Ok(res)
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

    pub fn repeat(&self, n: PyIntRef, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        if self.elements.is_empty() {
            // We can multiple an empty vector by any integer, even if it doesn't fit in an isize.
            return Ok(vec![]);
        }

        let n = n.as_bigint().to_isize().ok_or_else(|| {
            vm.new_overflow_error("can't multiply bytes that many times".to_string())
        })?;

        if n <= 0 {
            Ok(vec![])
        } else {
            let n = usize::try_from(n).unwrap();

            let mut new_value = Vec::with_capacity(n * self.elements.len());
            for _ in 0..n {
                new_value.extend(&self.elements);
            }

            Ok(new_value)
        }
    }

    pub fn irepeat(&mut self, n: PyIntRef, vm: &VirtualMachine) -> PyResult<()> {
        if self.elements.is_empty() {
            // We can multiple an empty vector by any integer, even if it doesn't fit in an isize.
            return Ok(());
        }

        let n = n.as_bigint().to_isize().ok_or_else(|| {
            vm.new_overflow_error("can't multiply bytes that many times".to_string())
        })?;

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

        Ok(())
    }
}

pub fn try_as_byte(obj: &PyObjectRef) -> Option<Vec<u8>> {
    match_class!(obj.clone(),

    i @ PyBytes => Some(i.get_value().to_vec()),
    j @ PyByteArray => Some(j.inner.borrow().elements.to_vec()),
    _ => None)
}

pub trait ByteOr: ToPrimitive {
    fn byte_or(&self, vm: &VirtualMachine) -> Result<u8, PyObjectRef> {
        match self.to_u8() {
            Some(value) => Ok(value),
            None => Err(vm.new_value_error("byte must be in range(0, 256)".to_string())),
        }
    }
}

impl ByteOr for BigInt {}

pub enum ByteInnerPosition {
    Left,
    Right,
    All,
}

fn split_slice<'a>(slice: &'a [u8], sep: &[u8], maxsplit: i32) -> Vec<&'a [u8]> {
    let mut splitted: Vec<&[u8]> = vec![];
    let mut prev_index = 0;
    let mut index = 0;
    let mut count = 0;
    let mut in_string = false;

    // No sep given, will split for any \t \n \r and space  = [9, 10, 13, 32]
    if sep.is_empty() {
        // split wihtout sep always trim left spaces for any maxsplit
        // so we have to ignore left spaces.
        loop {
            if [9, 10, 13, 32].contains(&slice[index]) {
                index += 1
            } else {
                prev_index = index;
                break;
            }
        }

        // most simple case
        if maxsplit == 0 {
            splitted.push(&slice[index..slice.len()]);
            return splitted;
        }

        // main loop. in_string means previous char is ascii char(true) or space(false)
        // loop from left to right
        loop {
            if [9, 10, 13, 32].contains(&slice[index]) {
                if in_string {
                    splitted.push(&slice[prev_index..index]);
                    in_string = false;
                    count += 1;
                    if count == maxsplit {
                        // while index < slice.len()
                        splitted.push(&slice[index + 1..slice.len()]);
                        break;
                    }
                }
            } else if !in_string {
                prev_index = index;
                in_string = true;
            }

            index += 1;

            // handle last item in slice
            if index == slice.len() {
                if in_string {
                    if [9, 10, 13, 32].contains(&slice[index - 1]) {
                        splitted.push(&slice[prev_index..index - 1]);
                    } else {
                        splitted.push(&slice[prev_index..index]);
                    }
                }
                break;
            }
        }
    } else {
        // sep is given, we match exact slice
        while index != slice.len() {
            if index + sep.len() >= slice.len() {
                if &slice[index..slice.len()] == sep {
                    splitted.push(&slice[prev_index..index]);
                    splitted.push(&[]);
                    break;
                }
                splitted.push(&slice[prev_index..slice.len()]);
                break;
            }

            if &slice[index..index + sep.len()] == sep {
                splitted.push(&slice[prev_index..index]);
                index += sep.len();
                prev_index = index;
                count += 1;
                if count == maxsplit {
                    // maxsplit reached, append, the remaing
                    splitted.push(&slice[prev_index..slice.len()]);
                    break;
                }
                continue;
            }

            index += 1;
        }
    }
    splitted
}

fn split_slice_reverse<'a>(slice: &'a [u8], sep: &[u8], maxsplit: i32) -> Vec<&'a [u8]> {
    let mut splitted: Vec<&[u8]> = vec![];
    let mut prev_index = slice.len();
    let mut index = slice.len();
    let mut count = 0;

    // No sep given, will split for any \t \n \r and space  = [9, 10, 13, 32]
    if sep.is_empty() {
        //adjust index
        index -= 1;

        // rsplit without sep always trim right spaces for any maxsplit
        // so we have to ignore right spaces.
        loop {
            if [9, 10, 13, 32].contains(&slice[index]) {
                index -= 1
            } else {
                break;
            }
        }
        prev_index = index + 1;

        // most simple case
        if maxsplit == 0 {
            splitted.push(&slice[0..=index]);
            return splitted;
        }

        // main loop. in_string means previous char is ascii char(true) or space(false)
        // loop from right to left and reverse result the end
        let mut in_string = true;
        loop {
            if [9, 10, 13, 32].contains(&slice[index]) {
                if in_string {
                    splitted.push(&slice[index + 1..prev_index]);
                    count += 1;
                    if count == maxsplit {
                        // maxsplit reached, append, the remaing
                        splitted.push(&slice[0..index]);
                        break;
                    }
                    in_string = false;
                    index -= 1;
                    continue;
                }
            } else if !in_string {
                in_string = true;
                if index == 0 {
                    splitted.push(&slice[0..1]);
                    break;
                }
                prev_index = index + 1;
            }
            if index == 0 {
                break;
            }
            index -= 1;
        }
    } else {
        // sep is give, we match exact slice going backwards
        while index != 0 {
            if index <= sep.len() {
                if &slice[0..index] == sep {
                    splitted.push(&slice[index..prev_index]);
                    splitted.push(&[]);
                    break;
                }
                splitted.push(&slice[0..prev_index]);
                break;
            }
            if &slice[(index - sep.len())..index] == sep {
                splitted.push(&slice[index..prev_index]);
                index -= sep.len();
                prev_index = index;
                count += 1;
                if count == maxsplit {
                    // maxsplit reached, append, the remaing
                    splitted.push(&slice[0..prev_index]);
                    break;
                }
                continue;
            }

            index -= 1;
        }
    }
    splitted.reverse();
    splitted
}

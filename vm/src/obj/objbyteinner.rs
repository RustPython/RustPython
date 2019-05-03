use crate::obj::objint::PyIntRef;
use crate::obj::objnone::PyNoneRef;
use crate::obj::objslice::PySliceRef;
use crate::obj::objtuple::PyTupleRef;
use crate::pyobject::Either;
use crate::pyobject::PyRef;
use crate::pyobject::PyValue;
use crate::pyobject::TryFromObject;
use crate::pyobject::{PyIterable, PyObjectRef};
use core::ops::Range;
use num_bigint::BigInt;

use crate::function::OptionalArg;

use crate::vm::VirtualMachine;

use crate::pyobject::{PyResult, TypeProtocol};

use crate::obj::objstr::{PyString, PyStringRef};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use super::objint;
use super::objsequence::{is_valid_slice_arg, PySliceableSequence};

use crate::obj::objint::PyInt;
use num_integer::Integer;
use num_traits::ToPrimitive;

use super::objbytearray::{get_value as get_value_bytearray, PyByteArray};
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
            j @ PyByteArray => Ok(PyByteInner{elements: get_value_bytearray(&j.as_object()).to_vec()}),
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

impl PyByteInner {
    pub fn repr(&self) -> PyResult<String> {
        let mut res = String::with_capacity(self.elements.len());
        for i in self.elements.iter() {
            match i {
                0..=8 => res.push_str(&format!("\\x0{}", i)),
                9 => res.push_str("\\t"),
                10 => res.push_str("\\n"),
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

    pub fn eq(&self, other: &PyByteInner, vm: &VirtualMachine) -> PyResult {
        if self.elements == other.elements {
            Ok(vm.new_bool(true))
        } else {
            Ok(vm.new_bool(false))
        }
    }

    pub fn ge(&self, other: &PyByteInner, vm: &VirtualMachine) -> PyResult {
        if self.elements >= other.elements {
            Ok(vm.new_bool(true))
        } else {
            Ok(vm.new_bool(false))
        }
    }

    pub fn le(&self, other: &PyByteInner, vm: &VirtualMachine) -> PyResult {
        if self.elements <= other.elements {
            Ok(vm.new_bool(true))
        } else {
            Ok(vm.new_bool(false))
        }
    }

    pub fn gt(&self, other: &PyByteInner, vm: &VirtualMachine) -> PyResult {
        if self.elements > other.elements {
            Ok(vm.new_bool(true))
        } else {
            Ok(vm.new_bool(false))
        }
    }

    pub fn lt(&self, other: &PyByteInner, vm: &VirtualMachine) -> PyResult {
        if self.elements < other.elements {
            Ok(vm.new_bool(true))
        } else {
            Ok(vm.new_bool(false))
        }
    }

    pub fn hash(&self) -> usize {
        let mut hasher = DefaultHasher::new();
        self.elements.hash(&mut hasher);
        hasher.finish() as usize
    }

    pub fn add(&self, other: &PyByteInner, _vm: &VirtualMachine) -> Vec<u8> {
        let elements: Vec<u8> = self
            .elements
            .iter()
            .chain(other.elements.iter())
            .cloned()
            .collect();
        elements
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
}

pub fn try_as_byte(obj: &PyObjectRef) -> Option<Vec<u8>> {
    match_class!(obj.clone(),

    i @ PyBytes => Some(i.get_value().to_vec()),
    j @ PyByteArray => Some(get_value_bytearray(&j.as_object()).to_vec()),
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

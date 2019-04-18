use crate::obj::objint::PyIntRef;
use crate::obj::objslice::PySlice;
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
use super::objtype;
use crate::obj::objint::PyInt;
use num_integer::Integer;
use num_traits::ToPrimitive;

use super::objbytearray::{get_value as get_value_bytearray, PyByteArray};
use super::objbytes::PyBytes;
use super::objmemory::PyMemoryView;
use super::objnone::PyNone;
use super::objsequence;

#[derive(Debug, Default, Clone)]
pub struct PyByteInner {
    pub elements: Vec<u8>,
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
    sub: PyObjectRef,
    #[pyarg(positional_only, optional = true)]
    start: OptionalArg<PyObjectRef>,
    #[pyarg(positional_only, optional = true)]
    end: OptionalArg<PyObjectRef>,
}

impl ByteInnerFindOptions {
    pub fn get_value(
        self,
        elements: &[u8],
        vm: &VirtualMachine,
    ) -> PyResult<(Vec<u8>, Range<usize>)> {
        let sub = match try_as_bytes_like(&self.sub) {
            Some(value) => value,
            None => match_class!(self.sub,
                i @ PyInt => vec![i.as_bigint().byte_or(vm)?],
                obj => {return Err(vm.new_type_error(format!("argument should be integer or bytes-like object, not {}", obj)));}),
        };
        let start = if let OptionalArg::Present(st) = self.start {
            match_class!(st,
            i @ PyInt => {Some(i.as_bigint().clone())},
            _obj @ PyNone => None,
            _=> {return Err(vm.new_type_error("slice indices must be integers or None or have an __index__ method".to_string()));}
            )
        } else {
            None
        };
        let end = if let OptionalArg::Present(e) = self.end {
            match_class!(e,
            i @ PyInt => {Some(i.as_bigint().clone())},
            _obj @ PyNone => None,
            _=> {return Err(vm.new_type_error("slice indices must be integers or None or have an __index__ method".to_string()));}
            )
        } else {
            None
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
    table: PyObjectRef,
    #[pyarg(positional_or_keyword, optional = true)]
    delete: OptionalArg<PyObjectRef>,
}

impl ByteInnerTranslateOptions {
    pub fn get_value(self, vm: &VirtualMachine) -> PyResult<(Vec<u8>, Vec<u8>)> {
        let table = match try_as_bytes_like(&self.table) {
            Some(value) => value,
            None => match_class!(self.table,

            _n @ PyNone => (0..=255).collect::<Vec<u8>>(),
            obj => {return Err(vm.new_type_error(format!("a bytes-like object is required, not {}", obj)));},
            ),
        };

        if table.len() != 256 {
            return Err(
                vm.new_value_error("translation table must be 256 characters long".to_string())
            );
        }

        let delete = if let OptionalArg::Present(value) = &self.delete {
            match try_as_bytes_like(&value) {
                Some(value) => value,
                None => {
                    return Err(vm.new_type_error(format!(
                        "a bytes-like object is required, not {}",
                        value
                    )));
                }
            }
        } else {
            vec![]
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

    pub fn contains(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        match try_as_bytes_like(&needle) {
            Some(value) => self.contains_bytes(&value, vm),
            None => match_class!(needle,
                i @ PyInt => self.contains_int(&i, vm),
                obj => {Err(vm.new_type_error(format!("a bytes-like object is required, not {}", obj)))}),
        }
    }

    fn contains_bytes(&self, other: &[u8], vm: &VirtualMachine) -> PyResult {
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

    fn contains_int(&self, int: &PyInt, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        if self.elements.contains(&int.as_bigint().byte_or(vm)?) {
            Ok(vm.new_bool(true))
        } else {
            Ok(vm.new_bool(false))
        }
    }

    pub fn getitem(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        match_class!(needle,
        int @ PyInt => //self.inner.getitem_int(&int, vm),
        {
            if let Some(idx) = self.elements.get_pos(int.as_bigint().to_i32().unwrap()) {
            Ok(vm.new_int(self.elements[idx]))
        } else {
            Err(vm.new_index_error("index out of range".to_string()))
        }
    },
        slice @ PySlice => //self.inner.getitem_slice(slice.as_object(), vm),
        {
        Ok(vm
            .ctx
            .new_bytes(self.elements.get_slice_items(vm, slice.as_object())?))
    },
        obj  => Err(vm.new_type_error(format!("byte indices must be integers or slices, not {}", obj))))
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
        // let fn_name = "center".to_string();
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
        // let fn_name = "ljust".to_string();
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
        // let fn_name = "rjust".to_string();
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
        for (index, v) in iter.iter(vm)?.enumerate() {
            let v = v?;
            match try_as_bytes_like(&v) {
                None => {
                    return Err(vm.new_type_error(format!(
                        "sequence item {}: expected a bytes-like object, {} found",
                        index,
                        &v.class().name,
                    )));
                }
                Some(value) => refs.extend(value),
            }
        }

        Ok(vm.ctx.new_bytes(refs))
    }

    pub fn startsendswith(
        &self,
        arg: PyObjectRef,
        start: OptionalArg<PyObjectRef>,
        end: OptionalArg<PyObjectRef>,
        endswith: bool, // true for endswith, false for startswith
        vm: &VirtualMachine,
    ) -> PyResult {
        let suff = if objtype::isinstance(&arg, &vm.ctx.tuple_type()) {
            let mut flatten = vec![];
            for v in objsequence::get_elements(&arg).to_vec() {
                match try_as_bytes_like(&v) {
                    None => {
                        return Err(vm.new_type_error(format!(
                            "a bytes-like object is required, not {}",
                            &v.class().name,
                        )));
                    }
                    Some(value) => flatten.extend(value),
                }
            }
            flatten
        } else {
            match try_as_bytes_like(&arg) {
                Some(value) => value,
                None => {
                    return Err(vm.new_type_error(format!(
                        "endswith first arg must be bytes or a tuple of bytes, not {}",
                        arg
                    )));
                }
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

    pub fn find(&self, options: ByteInnerFindOptions, vm: &VirtualMachine) -> PyResult<isize> {
        let (sub, range) = options.get_value(&self.elements, vm)?;
        // not allowed for this method
        if range.end < range.start {
            return Ok(-1isize);
        }

        let start = range.start;

        let slice = &self.elements[range];
        for (n, _) in slice.iter().enumerate() {
            if n + sub.len() <= slice.len() && &slice[n..n + sub.len()] == sub.as_slice() {
                return Ok((start + n) as isize);
            }
        }
        Ok(-1isize)
    }

    pub fn maketrans(from: PyObjectRef, to: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let mut res = vec![];

        let from = match try_as_bytes_like(&from) {
            Some(value) => value,
            None => {
                return Err(
                    vm.new_type_error(format!("a bytes-like object is required, not {}", from))
                );
            }
        };

        let to = match try_as_bytes_like(&to) {
            Some(value) => value,
            None => {
                return Err(
                    vm.new_type_error(format!("a bytes-like object is required, not {}", to))
                );
            }
        };

        for i in 0..=255 {
            res.push(if let Some(position) = from.iter().position(|&x| x == i) {
                to[position]
            } else {
                i
            });
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
}

pub fn try_as_byte(obj: &PyObjectRef) -> Option<Vec<u8>> {
    match_class!(obj.clone(),

    i @ PyBytes => Some(i.get_value().to_vec()),
    j @ PyByteArray => Some(get_value_bytearray(&j.as_object()).to_vec()),
    _ => None)
}

pub fn try_as_bytes_like(obj: &PyObjectRef) -> Option<Vec<u8>> {
    match_class!(obj.clone(),

    i @ PyBytes => Some(i.get_value().to_vec()),
    j @ PyByteArray => Some(get_value_bytearray(&j.as_object()).to_vec()),
    k @ PyMemoryView => Some(k.get_obj_value().unwrap()),
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

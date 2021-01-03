use crate::builtins::int::PyIntRef;
use crate::cformat::CFormatString;
use crate::function::{single_or_tuple_any, OptionalOption};
use crate::pyobject::{
    BorrowValue, PyIterator, PyObjectRef, PyResult, TryFromObject, TypeProtocol,
};
use crate::vm::VirtualMachine;
use num_traits::{cast::ToPrimitive, sign::Signed};
use std::str::FromStr;

#[derive(FromArgs)]
pub struct SplitArgs<'s, T: TryFromObject + AnyStrWrapper<'s>> {
    #[pyarg(any, default)]
    sep: Option<T>,
    #[pyarg(any, default = "-1")]
    maxsplit: isize,
    _phantom: std::marker::PhantomData<&'s ()>,
}

impl<'s, T: TryFromObject + AnyStrWrapper<'s>> SplitArgs<'s, T> {
    pub fn get_value(self, vm: &VirtualMachine) -> PyResult<(Option<T>, isize)> {
        let sep = if let Some(s) = self.sep {
            let sep = s.as_ref();
            if sep.is_empty() {
                return Err(vm.new_value_error("empty separator".to_owned()));
            }
            Some(s)
        } else {
            None
        };
        Ok((sep, self.maxsplit))
    }
}

#[derive(FromArgs)]
pub struct SplitLinesArgs {
    #[pyarg(any, default = "false")]
    pub keepends: bool,
}

#[derive(FromArgs)]
pub struct ExpandTabsArgs {
    #[pyarg(any, default = "8")]
    tabsize: isize,
}

impl ExpandTabsArgs {
    pub fn tabsize(&self) -> usize {
        self.tabsize.to_usize().unwrap_or(0)
    }
}

#[derive(FromArgs)]
pub struct StartsEndsWithArgs {
    #[pyarg(positional)]
    affix: PyObjectRef,
    #[pyarg(positional, default)]
    start: Option<PyIntRef>,
    #[pyarg(positional, default)]
    end: Option<PyIntRef>,
}

impl StartsEndsWithArgs {
    fn get_value(self, len: usize) -> (PyObjectRef, std::ops::Range<usize>) {
        let range = adjust_indices(self.start, self.end, len);
        (self.affix, range)
    }
}

fn saturate_to_isize(py_int: PyIntRef) -> isize {
    let big = py_int.borrow_value();
    big.to_isize().unwrap_or_else(|| {
        if big.is_negative() {
            std::isize::MIN
        } else {
            std::isize::MAX
        }
    })
}

// help get optional string indices
pub fn adjust_indices(
    start: Option<PyIntRef>,
    end: Option<PyIntRef>,
    len: usize,
) -> std::ops::Range<usize> {
    let mut start = start.map_or(0, saturate_to_isize);
    let mut end = end.map_or(len as isize, saturate_to_isize);
    if end > len as isize {
        end = len as isize;
    } else if end < 0 {
        end += len as isize;
        if end < 0 {
            end = 0;
        }
    }
    if start < 0 {
        start += len as isize;
        if start < 0 {
            start = 0;
        }
    }
    start as usize..end as usize
}

pub trait StringRange {
    fn is_normal(&self) -> bool;
}

impl StringRange for std::ops::Range<usize> {
    fn is_normal(&self) -> bool {
        self.start <= self.end
    }
}

pub trait AnyStrWrapper<'s> {
    type Str: ?Sized + AnyStr<'s>;
    fn as_ref(&self) -> &Self::Str;
}

pub trait AnyStrContainer<S>
where
    S: ?Sized,
{
    fn new() -> Self;
    fn with_capacity(capacity: usize) -> Self;
    fn push_str(&mut self, s: &S);
}

// TODO: GATs for `'s` once stabilized
pub trait AnyStr<'s>: 's {
    type Char: Copy;
    type Container: AnyStrContainer<Self> + Extend<Self::Char>;
    type CharIter: Iterator<Item = char> + 's;
    type ElementIter: Iterator<Item = Self::Char> + 's;

    fn element_bytes_len(c: Self::Char) -> usize;

    fn to_container(&self) -> Self::Container;
    fn as_bytes(&self) -> &[u8];
    fn as_utf8_str(&self) -> Result<&str, std::str::Utf8Error>;
    fn chars(&'s self) -> Self::CharIter;
    fn elements(&'s self) -> Self::ElementIter;
    fn get_bytes(&self, range: std::ops::Range<usize>) -> &Self;
    // FIXME: get_chars is expensive for str
    fn get_chars(&self, range: std::ops::Range<usize>) -> &Self;
    fn bytes_len(&self) -> usize;
    // fn chars_len(&self) -> usize;  // cannot access to cache here
    fn is_empty(&self) -> bool;

    fn py_add(&self, other: &Self) -> Self::Container {
        let mut new = Self::Container::with_capacity(self.bytes_len() + other.bytes_len());
        new.push_str(self);
        new.push_str(other);
        new
    }

    fn py_split<T, SP, SN, SW, R>(
        &self,
        args: SplitArgs<'s, T>,
        vm: &VirtualMachine,
        split: SP,
        splitn: SN,
        splitw: SW,
    ) -> PyResult<Vec<R>>
    where
        T: TryFromObject + AnyStrWrapper<'s, Str = Self>,
        SP: Fn(&Self, &Self, &VirtualMachine) -> Vec<R>,
        SN: Fn(&Self, &Self, usize, &VirtualMachine) -> Vec<R>,
        SW: Fn(&Self, isize, &VirtualMachine) -> Vec<R>,
    {
        let (sep, maxsplit) = args.get_value(vm)?;
        let splited = if let Some(pattern) = sep {
            if maxsplit < 0 {
                split(self, pattern.as_ref(), vm)
            } else {
                splitn(self, pattern.as_ref(), (maxsplit + 1) as usize, vm)
            }
        } else {
            splitw(self, maxsplit, vm)
        };
        Ok(splited)
    }
    fn py_split_whitespace<F>(&self, maxsplit: isize, convert: F) -> Vec<PyObjectRef>
    where
        F: Fn(&Self) -> PyObjectRef;
    fn py_rsplit_whitespace<F>(&self, maxsplit: isize, convert: F) -> Vec<PyObjectRef>
    where
        F: Fn(&Self) -> PyObjectRef;

    #[inline]
    fn py_startsendswith<T, F>(
        &self,
        args: StartsEndsWithArgs,
        func_name: &str,
        py_type_name: &str,
        func: F,
        vm: &VirtualMachine,
    ) -> PyResult<bool>
    where
        T: TryFromObject,
        F: Fn(&Self, &T) -> bool,
    {
        let (affix, range) = args.get_value(self.bytes_len());
        if range.is_normal() {
            let value = self.get_bytes(range);
            single_or_tuple_any(
                affix,
                &|s: &T| Ok(func(value, s)),
                &|o| {
                    format!(
                        "{} first arg must be {} or a tuple of {}, not {}",
                        func_name,
                        py_type_name,
                        py_type_name,
                        o.class(),
                    )
                },
                vm,
            )
        } else {
            Ok(false)
        }
    }

    #[inline]
    fn py_strip<'a, S, FC, FD>(
        &'a self,
        chars: OptionalOption<S>,
        func_chars: FC,
        func_default: FD,
    ) -> &'a Self
    where
        S: AnyStrWrapper<'s, Str = Self>,
        FC: Fn(&'a Self, &Self) -> &'a Self,
        FD: Fn(&'a Self) -> &'a Self,
    {
        let chars = chars.flatten();
        match chars {
            Some(chars) => func_chars(self, chars.as_ref()),
            None => func_default(self),
        }
    }

    #[inline]
    fn py_find<F>(&self, needle: &Self, range: std::ops::Range<usize>, find: F) -> Option<usize>
    where
        F: Fn(&Self, &Self) -> Option<usize>,
    {
        if range.is_normal() {
            let start = range.start;
            let index = find(self.get_chars(range), &needle)?;
            Some(start + index)
        } else {
            None
        }
    }

    #[inline]
    fn py_count<F>(&self, needle: &Self, range: std::ops::Range<usize>, count: F) -> usize
    where
        F: Fn(&Self, &Self) -> usize,
    {
        if range.is_normal() {
            count(self.get_chars(range), &needle)
        } else {
            0
        }
    }

    fn py_pad(&self, left: usize, right: usize, fillchar: Self::Char) -> Self::Container {
        let mut u = Self::Container::with_capacity(
            (left + right) * Self::element_bytes_len(fillchar) + self.bytes_len(),
        );
        u.extend(std::iter::repeat(fillchar).take(left));
        u.push_str(self);
        u.extend(std::iter::repeat(fillchar).take(right));
        u
    }

    fn py_center(&self, width: usize, fillchar: Self::Char, len: usize) -> Self::Container {
        let marg = width - len;
        let left = marg / 2 + (marg & width & 1);
        self.py_pad(left, marg - left, fillchar)
    }

    fn py_ljust(&self, width: usize, fillchar: Self::Char, len: usize) -> Self::Container {
        self.py_pad(0, width - len, fillchar)
    }

    fn py_rjust(&self, width: usize, fillchar: Self::Char, len: usize) -> Self::Container {
        self.py_pad(width - len, 0, fillchar)
    }

    fn py_join<'a>(
        &self,
        mut iter: PyIterator<'a, impl AnyStrWrapper<'s, Str = Self> + TryFromObject>,
    ) -> PyResult<Self::Container> {
        let mut joined = if let Some(elem) = iter.next() {
            elem?.as_ref().to_container()
        } else {
            return Ok(Self::Container::new());
        };
        for elem in iter {
            let elem = elem?;
            joined.push_str(self);
            joined.push_str(elem.as_ref());
        }
        Ok(joined)
    }

    fn py_partition<'a, F, S>(
        &'a self,
        sub: &Self,
        split: F,
        vm: &VirtualMachine,
    ) -> PyResult<(Self::Container, bool, Self::Container)>
    where
        F: Fn() -> S,
        S: std::iter::Iterator<Item = &'a Self>,
    {
        if sub.is_empty() {
            return Err(vm.new_value_error("empty separator".to_owned()));
        }

        let mut sp = split();
        let front = sp.next().unwrap().to_container();
        let (has_mid, back) = if let Some(back) = sp.next() {
            (true, back.to_container())
        } else {
            (false, Self::Container::new())
        };
        Ok((front, has_mid, back))
    }

    fn py_removeprefix<FC>(&self, prefix: &Self, prefix_len: usize, is_prefix: FC) -> &Self
    where
        FC: Fn(&Self, &Self) -> bool,
    {
        //if self.py_starts_with(prefix) {
        if is_prefix(&self, prefix) {
            self.get_bytes(prefix_len..self.bytes_len())
        } else {
            &self
        }
    }

    fn py_removesuffix<FC>(&self, suffix: &Self, suffix_len: usize, is_suffix: FC) -> &Self
    where
        FC: Fn(&Self, &Self) -> bool,
    {
        if is_suffix(&self, suffix) {
            self.get_bytes(0..self.bytes_len() - suffix_len)
        } else {
            &self
        }
    }

    fn py_splitlines<FW, W>(&self, options: SplitLinesArgs, into_wrapper: FW) -> Vec<W>
    where
        FW: Fn(&Self) -> W,
    {
        let keep = if options.keepends { 1 } else { 0 };
        let mut elements = Vec::new();
        let mut last_i = 0;
        let mut enumerated = self.as_bytes().iter().enumerate().peekable();
        while let Some((i, ch)) = enumerated.next() {
            let (end_len, i_diff) = match *ch {
                b'\n' => (keep, 1),
                b'\r' => {
                    let is_rn = enumerated.peek().map_or(false, |(_, ch)| **ch == b'\n');
                    if is_rn {
                        let _ = enumerated.next();
                        (keep + keep, 2)
                    } else {
                        (keep, 1)
                    }
                }
                _ => {
                    continue;
                }
            };
            let range = last_i..i + end_len;
            last_i = i + i_diff;
            elements.push(into_wrapper(self.get_bytes(range)));
        }
        if last_i != self.bytes_len() {
            elements.push(into_wrapper(self.get_bytes(last_i..self.bytes_len())));
        }
        elements
    }

    fn py_zfill(&self, width: isize) -> Vec<u8> {
        let width = width.to_usize().unwrap_or(0);
        rustpython_common::str::zfill(self.as_bytes(), width)
    }

    fn py_iscase<F, G>(&'s self, is_case: F, is_opposite: G) -> bool
    where
        F: Fn(char) -> bool,
        G: Fn(char) -> bool,
    {
        // Unified form of CPython functions:
        //  _Py_bytes_islower
        //   Py_bytes_isupper
        //  unicode_islower_impl
        //  unicode_isupper_impl
        let mut cased = false;
        for c in self.chars() {
            if is_opposite(c) {
                return false;
            } else if !cased && is_case(c) {
                cased = true
            }
        }
        cased
    }

    fn py_cformat(&self, values: PyObjectRef, vm: &VirtualMachine) -> PyResult<String> {
        let format_string = self.as_utf8_str().unwrap();
        CFormatString::from_str(format_string)
            .map_err(|err| vm.new_value_error(err.to_string()))?
            .format(vm, values)
    }
}

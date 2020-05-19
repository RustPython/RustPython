use crate::function::{single_or_tuple_any, OptionalOption};
use crate::obj::objint::PyIntRef;
use crate::pyobject::{PyObjectRef, PyResult, TryFromObject, TypeProtocol};
use crate::vm::VirtualMachine;
use num_traits::{cast::ToPrimitive, sign::Signed};

#[derive(FromArgs)]
pub struct SplitArgs<T, S, E>
where
    T: TryFromObject + PyCommonStringWrapper<S>,
    S: ?Sized + PyCommonString<E>,
{
    #[pyarg(positional_or_keyword, default = "None")]
    sep: Option<T>,
    #[pyarg(positional_or_keyword, default = "-1")]
    maxsplit: isize,
    _phantom1: std::marker::PhantomData<S>,
    _phantom2: std::marker::PhantomData<E>,
}

impl<T, S, E> SplitArgs<T, S, E>
where
    T: TryFromObject + PyCommonStringWrapper<S>,
    S: ?Sized + PyCommonString<E>,
{
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
    #[pyarg(positional_or_keyword, default = "false")]
    pub keepends: bool,
}

#[derive(FromArgs)]
pub struct ExpandTabsArgs {
    #[pyarg(positional_or_keyword, default = "8")]
    tabsize: isize,
}

impl ExpandTabsArgs {
    pub fn tabsize(&self) -> usize {
        self.tabsize.to_usize().unwrap_or(0)
    }
}

#[derive(FromArgs)]
pub struct StartsEndsWithArgs {
    #[pyarg(positional_only, optional = false)]
    affix: PyObjectRef,
    #[pyarg(positional_only, default = "None")]
    start: Option<PyIntRef>,
    #[pyarg(positional_only, default = "None")]
    end: Option<PyIntRef>,
}

impl StartsEndsWithArgs {
    fn get_value(self, len: usize) -> (PyObjectRef, std::ops::Range<usize>) {
        let range = adjust_indices(self.start, self.end, len);
        (self.affix, range)
    }
}

fn cap_to_isize(py_int: PyIntRef) -> isize {
    let big = py_int.as_bigint();
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
    let mut start = start.map_or(0, cap_to_isize);
    let mut end = end.map_or(len as isize, cap_to_isize);
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

pub trait PyCommonStringWrapper<S>
where
    S: ?Sized,
{
    fn as_ref(&self) -> &S;
}

pub trait PyCommonString<E> {
    type Container;

    fn with_capacity(capacity: usize) -> Self::Container;
    fn get_bytes<'a>(&'a self, range: std::ops::Range<usize>) -> &'a Self;
    // FIXME: get_chars is expensive for str
    fn get_chars<'a>(&'a self, range: std::ops::Range<usize>) -> &'a Self;
    fn bytes_len(&self) -> usize;
    fn chars_len(&self) -> usize;
    fn is_empty(&self) -> bool;

    fn py_split<T, SP, SN, SW, R>(
        &self,
        args: SplitArgs<T, Self, E>,
        vm: &VirtualMachine,
        split: SP,
        splitn: SN,
        splitw: SW,
    ) -> PyResult<Vec<R>>
    where
        T: TryFromObject + PyCommonStringWrapper<Self>,
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
                |s: &T| Ok(func(value, s)),
                |o| {
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
        S: PyCommonStringWrapper<Self>,
        FC: Fn(&'a Self, &Self) -> &'a Self,
        FD: Fn(&'a Self) -> &'a Self,
    {
        let chars = chars.flat_option();
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
            if let Some(index) = find(self.get_chars(range), &needle) {
                return Some(start + index);
            }
        }
        None
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

    fn py_pad(&self, left: usize, right: usize, fillchar: E) -> Self::Container;

    fn py_center(&self, width: usize, fillchar: E) -> Self::Container {
        let marg = width - self.chars_len();
        let left = marg / 2 + (marg & width & 1);
        self.py_pad(left, marg - left, fillchar)
    }

    fn py_ljust(&self, width: usize, fillchar: E) -> Self::Container {
        self.py_pad(0, width - self.chars_len(), fillchar)
    }

    fn py_rjust(&self, width: usize, fillchar: E) -> Self::Container {
        self.py_pad(width - self.chars_len(), 0, fillchar)
    }

    fn py_removeprefix<FC>(
        &self,
        prefix: &Self::Container,
        prefix_len: usize,
        is_prefix: FC,
    ) -> &Self
    where
        FC: Fn(&Self, &Self::Container) -> bool,
    {
        //if self.py_starts_with(prefix) {
        if is_prefix(&self, &prefix) {
            self.get_bytes(prefix_len..self.bytes_len())
        } else {
            &self
        }
    }

    fn py_removesuffix<FC>(
        &self,
        suffix: &Self::Container,
        suffix_len: usize,
        is_suffix: FC,
    ) -> &Self
    where
        FC: Fn(&Self, &Self::Container) -> bool,
    {
        if is_suffix(&self, &suffix) {
            self.get_bytes(0..self.bytes_len() - suffix_len)
        } else {
            &self
        }
    }
}

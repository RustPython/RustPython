use crate::function::{single_or_tuple_any, OptionalOption};
use crate::pyobject::{PyObjectRef, PyResult, TryFromObject, TypeProtocol};
use crate::vm::VirtualMachine;
use num_traits::cast::ToPrimitive;

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

// help get optional string indices
pub fn adjust_indices(
    start: OptionalOption<isize>,
    end: OptionalOption<isize>,
    len: usize,
) -> std::ops::Range<usize> {
    let mut start = start.flat_option().unwrap_or(0);
    let mut end = end.flat_option().unwrap_or(len as isize);
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
    fn get_slice(&self, range: std::ops::Range<usize>) -> &Self;
    fn len(&self) -> usize;
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

    #[allow(clippy::too_many_arguments)]
    #[inline]
    fn py_startsendswith<T, F>(
        &self,
        affix: PyObjectRef,
        start: OptionalOption<isize>,
        end: OptionalOption<isize>,
        func_name: &str,
        py_type_name: &str,
        func: F,
        vm: &VirtualMachine,
    ) -> PyResult<bool>
    where
        T: TryFromObject,
        F: Fn(&Self, &T) -> bool,
    {
        let range = adjust_indices(start, end, self.len());
        if range.is_normal() {
            let value = self.get_slice(range);
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
}

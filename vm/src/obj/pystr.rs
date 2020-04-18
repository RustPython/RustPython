use crate::function::{single_or_tuple_any, OptionalOption};
use crate::pyobject::{PyObjectRef, PyResult, TryFromObject, TypeProtocol};
use crate::vm::VirtualMachine;

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

pub trait PyCommonString<E> {
    fn get_slice(&self, range: std::ops::Range<usize>) -> &Self;
    fn len(&self) -> usize;

    fn py_split<SP, SN, SW, R>(
        &self,
        sep: Option<&Self>,
        maxsplit: isize,
        vm: &VirtualMachine,
        split: SP,
        splitn: SN,
        splitw: SW,
    ) -> Vec<R>
    where
        SP: Fn(&Self, &Self, &VirtualMachine) -> Vec<R>,
        SN: Fn(&Self, &Self, usize, &VirtualMachine) -> Vec<R>,
        SW: Fn(&Self, isize, &VirtualMachine) -> Vec<R>,
    {
        if let Some(pattern) = sep {
            if maxsplit < 0 {
                split(self, pattern, vm)
            } else {
                splitn(self, pattern, (maxsplit + 1) as usize, vm)
            }
        } else {
            splitw(self, maxsplit, vm)
        }
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
}

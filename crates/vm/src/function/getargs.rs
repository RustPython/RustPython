//! `PyArg_ParseTupleAndKeywords` argument matching.

use super::FuncArgs;
use crate::{PyObject, PyObjectRef, PyResult, VirtualMachine, common::wtf8::Wtf8};

/// The keyword list of a `PyArg_ParseTupleAndKeywords` call together with the
/// `|`, `$` and `:` markers of its format string. Names are matched to the
/// supplied arguments; converting them is left to the caller.
pub(crate) struct ArgSpec<'a> {
    /// What the `:name` suffix names the function in error messages.
    pub(crate) fname: &'a str,
    /// Every parameter name, in `kwlist` order. None may be positional-only.
    pub(crate) keywords: &'a [&'a str],
    /// Where `|` sits: how many leading parameters are required. Equal to the
    /// parameter count when the format string has no `|`.
    pub(crate) required: usize,
    /// Where `$` sits: how many leading parameters may be given positionally.
    /// Equal to the parameter count when the format string has no `$`.
    pub(crate) max_positional: usize,
}

impl ArgSpec<'_> {
    pub(crate) fn parse(
        &self,
        args: &FuncArgs,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<Option<PyObjectRef>>> {
        self.parse_with(args, |_, _, _| Ok(()), vm)
    }

    /// `check` runs for each supplied argument in `kwlist` order, so a
    /// conversion failure is reported ahead of the checks that follow it.
    pub(crate) fn parse_with(
        &self,
        args: &FuncArgs,
        check: impl Fn(usize, &PyObject, &VirtualMachine) -> PyResult<()>,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<Option<PyObjectRef>>> {
        let fname = self.fname;
        let len = self.keywords.len();
        let nargs = args.args.len();
        let mut nkwargs = args.kwargs.len();
        if nargs + nkwargs > len {
            // Saying "keyword" when nothing was passed positionally keeps the
            // message right for keyword-only signatures.
            return Err(vm.new_type_error(format!(
                "{fname}() takes at most {len} {}argument{} ({} given)",
                if nargs == 0 { "keyword " } else { "" },
                if len == 1 { "" } else { "s" },
                nargs + nkwargs,
            )));
        }

        let mut slots = vec![None; len];
        let mut exhausted = false;
        for (i, slot) in slots.iter_mut().enumerate() {
            if i == self.max_positional && self.max_positional < nargs {
                return Err(vm.new_type_error(if self.max_positional == 0 {
                    format!("{fname}() takes no positional arguments")
                } else {
                    format!(
                        "{fname}() takes {} {} positional argument{} ({nargs} given)",
                        if self.required < len {
                            "at most"
                        } else {
                            "exactly"
                        },
                        self.max_positional,
                        if self.max_positional == 1 { "" } else { "s" },
                    )
                }));
            }
            let current = if i < nargs {
                Some(args.args[i].clone())
            } else if nkwargs > 0 {
                args.kwargs
                    .get(self.keywords[i])
                    .inspect(|_| nkwargs -= 1)
                    .cloned()
            } else {
                None
            };
            if let Some(obj) = current {
                check(i, &obj, vm)?;
                *slot = Some(obj);
                continue;
            }
            if i < self.required {
                return Err(vm.new_type_error(format!(
                    "{fname}() missing required argument '{}' (pos {})",
                    self.keywords[i],
                    i + 1
                )));
            }
            // All the required arguments are in and no keyword is left over,
            // so nothing below can fail.
            if nkwargs == 0 {
                exhausted = true;
                break;
            }
        }

        if !exhausted && nkwargs > 0 {
            for (i, name) in self.keywords.iter().enumerate().take(nargs) {
                if args.kwargs.contains_key(name) {
                    return Err(vm.new_type_error(format!(
                        "argument for {fname}() given by name ('{name}') and position ({})",
                        i + 1
                    )));
                }
            }
            for key in args.kwargs.keys() {
                if !self.keywords.iter().any(|name| &**key == Wtf8::new(name)) {
                    return Err(vm.new_type_error(format!(
                        "{fname}() got an unexpected keyword argument '{key}'"
                    )));
                }
            }
            return Err(vm.new_type_error(format!("invalid keyword argument for {fname}()")));
        }
        Ok(slots)
    }
}

//! `PyArg_ParseTupleAndKeywords` argument matching.

use super::{FuncArgs, KwArgs, given_by_name_and_position_message};
use crate::{
    AsObject, PyObject, PyObjectRef, PyResult, TryFromObject, VirtualMachine, builtins::PyBaseExceptionRef,
    common::wtf8::{Wtf8, Wtf8Buf},
};

/// The keyword list of a `PyArg_ParseTupleAndKeywords` call together with the
/// `|`, `$` and `:` markers of its format string. Names are matched to the
/// supplied arguments; converting them is left to the caller.
pub struct ArgSpec<'a> {
    /// What the `:name` suffix names the function in error messages.
    pub fname: &'a str,
    /// Every parameter name, in `kwlist` order. None may be positional-only.
    pub keywords: &'a [&'a str],
    /// Where `|` sits: how many leading parameters are required. Equal to the
    /// parameter count when the format string has no `|`.
    pub required: usize,
    /// Where `$` sits: how many leading parameters may be given positionally.
    /// Equal to the parameter count when the format string has no `$`.
    pub max_positional: usize,
}

/// Sequential view of `ArgSpec` slots in kwlist order.
pub struct ArgSlots<'a> {
    spec: &'a ArgSpec<'a>,
    args: &'a [PyObjectRef],
    kwargs: &'a KwArgs,
    index: usize,
    nkwargs: usize,
    exhausted: bool,
}

impl ArgSpec<'_> {
    pub fn parse(
        &self,
        args: &FuncArgs,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<Option<PyObjectRef>>> {
        self.parse_with(args, |_, _, _| Ok(()), vm)
    }

    /// `check` runs for each supplied argument in `kwlist` order, so a
    /// conversion failure is reported ahead of the checks that follow it.
    /// It may retain converted values, avoiding a second conversion after
    /// validation and repeated calls to user-defined conversion methods.
    pub fn parse_with(
        &self,
        args: &FuncArgs,
        mut check: impl FnMut(usize, &PyObject, &VirtualMachine) -> PyResult<()>,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<Option<PyObjectRef>>> {
        self.bind(args, vm, |slots| {
            let mut parsed = vec![None; self.keywords.len()];
            for (i, slot) in parsed.iter_mut().enumerate() {
                let value = if i < self.required {
                    Some(slots.required(vm)?)
                } else {
                    slots.optional(vm)?
                };
                if let Some(ref obj) = value {
                    check(i, obj, vm)?;
                }
                *slot = value;
            }
            Ok(parsed)
        })
    }

    /// Bind only keywords while the caller retains ownership of raw arguments.
    /// This entry point supplies CPython's keyword typo suggestions.
    pub(crate) fn parse_keywords(
        &self,
        kwargs: &KwArgs,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<Option<PyObjectRef>>> {
        self.bind_parts(&[], kwargs, true, vm, |slots| {
            let mut parsed = Vec::with_capacity(self.keywords.len());
            for i in 0..self.keywords.len() {
                parsed.push(if i < self.required {
                    Some(slots.required(vm)?)
                } else {
                    slots.optional(vm)?
                });
            }
            Ok(parsed)
        })
    }

    /// Matches `args` to the kwlist and hands the slots to `bind` in kwlist order.
    pub fn bind<R, E>(
        &self,
        args: &FuncArgs,
        vm: &VirtualMachine,
        bind: impl FnOnce(&mut ArgSlots<'_>) -> Result<R, E>,
    ) -> Result<R, E>
    where
        E: From<PyBaseExceptionRef>,
    {
        self.bind_parts(&args.args, &args.kwargs, false, vm, bind)
    }

    fn bind_parts<R, E>(
        &self,
        args: &[PyObjectRef],
        kwargs: &KwArgs,
        suggest_keywords: bool,
        vm: &VirtualMachine,
        bind: impl FnOnce(&mut ArgSlots<'_>) -> Result<R, E>,
    ) -> Result<R, E>
    where
        E: From<PyBaseExceptionRef>,
    {
        let fname = self.fname;
        let len = self.keywords.len();
        let nargs = args.len();
        let nkwargs = kwargs.len();
        if nargs + nkwargs > len {
            // Saying "keyword" when nothing was passed positionally keeps the
            // message right for keyword-only signatures.
            return Err(E::from(vm.new_type_error(format!(
                "{fname}() takes at most {len} {}argument{} ({} given)",
                if nargs == 0 { "keyword " } else { "" },
                if len == 1 { "" } else { "s" },
                nargs + nkwargs,
            ))));
        }

        let mut slots = ArgSlots {
            spec: self,
            args,
            kwargs,
            index: 0,
            nkwargs,
            exhausted: false,
        };
        let result = bind(&mut slots)?;
        while slots.index < len {
            debug_assert!(
                slots.index >= self.required,
                "bind closure skipped a required argument"
            );
            let supplied = slots.take_current(vm)?;
            debug_assert!(
                supplied.is_none(),
                "bind closure skipped a supplied argument"
            );
            let _ = supplied;
        }
        if !slots.exhausted && slots.nkwargs > 0 {
            for (i, name) in self.keywords.iter().enumerate().take(nargs) {
                if kwargs.contains_key(name) {
                    return Err(E::from(vm.new_type_error(
                        given_by_name_and_position_message(Some(fname), name, i + 1),
                    )));
                }
            }
            for key in kwargs.keys() {
                if !self.keywords.iter().any(|name| &**key == Wtf8::new(name)) {
                    let mut message =
                        Wtf8Buf::from(format!("{fname}() got an unexpected keyword argument '"));
                    if suggest_keywords {
                        // Preserve lone surrogates in the error message.
                        message.push_wtf8(key);
                    } else {
                        message.push_str(&key.to_string());
                    }
                    message.push_str("'");
                    // CPython skips suggestions for names that are not UTF-8.
                    if suggest_keywords && key.as_str().is_ok() {
                        let names: Vec<_> = self
                            .keywords
                            .iter()
                            .map(|name| vm.ctx.new_str(*name))
                            .collect();
                        let name = vm.ctx.new_str(&**key);
                        if let Some(suggestion) = crate::suggestion::calculate_keyword_suggestions(
                            names.iter().map(|name| name.as_object()),
                            name.as_object(),
                        ) {
                            message.push_str(&format!(". Did you mean '{suggestion}'?"));
                        }
                    }
                    return Err(E::from(vm.new_type_error(message)));
                }
            }
            return Err(E::from(
                vm.new_type_error(format!("invalid keyword argument for {fname}()")),
            ));
        }
        Ok(result)
    }
}

impl ArgSlots<'_> {
    /// The next slot, which must be filled: the "missing required argument" error otherwise.
    pub fn required(&mut self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        self.check_cursor(vm)?;
        debug_assert!(
            self.index < self.spec.required,
            "bind cursor requested a required argument past the required prefix"
        );
        if self.index >= self.spec.required {
            return Err(vm.new_system_error(format!(
                "{}() bind cursor requested a required argument past the required prefix",
                self.spec.fname
            )));
        }
        let i = self.index;
        match self.take_current(vm)? {
            Some(obj) => Ok(obj),
            None => self.missing_required(i, vm),
        }
    }

    /// The next slot, `None` when the caller left it out.
    pub fn optional(&mut self, vm: &VirtualMachine) -> PyResult<Option<PyObjectRef>> {
        self.check_cursor(vm)?;
        let i = self.index;
        match self.take_current(vm)? {
            Some(obj) => Ok(Some(obj)),
            None if i < self.spec.required => self.missing_required(i, vm),
            None => Ok(None),
        }
    }

    /// `required` converted with `TryFromObject`.
    pub fn required_as<T: TryFromObject>(&mut self, vm: &VirtualMachine) -> PyResult<T> {
        T::try_from_object(vm, self.required(vm)?)
    }

    /// `optional` converted with `TryFromObject`.
    pub fn optional_as<T: TryFromObject>(&mut self, vm: &VirtualMachine) -> PyResult<Option<T>> {
        match self.optional(vm)? {
            Some(obj) => T::try_from_object(vm, obj).map(Some),
            None => Ok(None),
        }
    }

    fn check_cursor(&self, vm: &VirtualMachine) -> PyResult<()> {
        debug_assert!(
            self.index < self.spec.keywords.len(),
            "bind cursor moved past the end of the parameter list"
        );
        if self.index >= self.spec.keywords.len() {
            return Err(vm.new_system_error(format!(
                "{}() bind cursor moved past the end of the parameter list",
                self.spec.fname
            )));
        }
        Ok(())
    }

    fn missing_required<T>(&self, i: usize, vm: &VirtualMachine) -> PyResult<T> {
        Err(vm.new_type_error(format!(
            "{}() missing required argument '{}' (pos {})",
            self.spec.fname,
            self.spec.keywords[i],
            i + 1
        )))
    }

    /// One iteration of the matching loop for the current index.
    fn take_current(&mut self, vm: &VirtualMachine) -> PyResult<Option<PyObjectRef>> {
        let i = self.index;
        self.index += 1;
        if self.exhausted {
            return Ok(None);
        }
        let fname = self.spec.fname;
        let len = self.spec.keywords.len();
        let nargs = self.args.len();
        if i == self.spec.max_positional && self.spec.max_positional < nargs {
            return Err(vm.new_type_error(if self.spec.max_positional == 0 {
                format!("{fname}() takes no positional arguments")
            } else {
                format!(
                    "{fname}() takes {} {} positional argument{} ({nargs} given)",
                    if self.spec.required < len {
                        "at most"
                    } else {
                        "exactly"
                    },
                    self.spec.max_positional,
                    if self.spec.max_positional == 1 {
                        ""
                    } else {
                        "s"
                    },
                )
            }));
        }
        let current = if i < nargs {
            Some(self.args[i].clone())
        } else if self.nkwargs > 0 {
            self.args
                .kwargs
                .get(self.spec.keywords[i])
                .inspect(|_| self.nkwargs -= 1)
                .cloned()
        } else {
            None
        };
        if current.is_none() && i >= self.spec.required && self.nkwargs == 0 {
            // All the required arguments are in and no keyword is left over,
            // so nothing below can fail.
            self.exhausted = true;
        }
        Ok(current)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::function::KwArgs;

    const SPEC: ArgSpec<'static> = ArgSpec {
        fname: "example",
        keywords: &["first", "second", "third"],
        required: 1,
        max_positional: 3,
    };

    #[test]
    fn mutable_converters_run_once_in_parameter_order() {
        crate::Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let args = FuncArgs::new(
                vec![vm.ctx.none()],
                core::iter::once(("third", vm.ctx.none())).collect::<KwArgs>(),
            );
            let mut visited = Vec::new();
            let mut retained = [None, None, None];
            let parsed = match SPEC.parse_with(
                &args,
                |i, obj, _vm| {
                    visited.push(i);
                    retained[i] = Some(obj.to_owned());
                    Ok(())
                },
                vm,
            ) {
                Ok(parsed) => parsed,
                Err(_) => panic!("parse_with failed"),
            };
            assert_eq!(visited, [0, 2]);
            assert_eq!(parsed.len(), 3);
            assert!(retained[0].is_some());
            assert!(retained[1].is_none());
            assert!(retained[2].is_some());
        });
    }

    #[test]
    fn supplied_values_convert_before_missing_arguments() {
        crate::Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let spec = ArgSpec {
                required: 2,
                ..SPEC
            };
            let args = FuncArgs::from(vec![vm.ctx.none()]);
            let mut visited = Vec::new();
            let result = spec.parse_with(
                &args,
                |i, _obj, _vm| {
                    visited.push(i);
                    Ok(())
                },
                vm,
            );
            assert!(result.is_err());
            assert_eq!(visited, [0]);
        });
    }

    #[test]
    fn supplied_values_convert_before_leftover_keywords() {
        crate::Interpreter::without_stdlib(Default::default()).enter(|vm| {
            for keyword in ["first", "unexpected"] {
                let args = FuncArgs::new(
                    vec![vm.ctx.none()],
                    core::iter::once((keyword, vm.ctx.none())).collect::<KwArgs>(),
                );
                let mut visited = Vec::new();
                let result = SPEC.parse_with(
                    &args,
                    |i, _obj, _vm| {
                        visited.push(i);
                        Ok(())
                    },
                    vm,
                );
                assert!(result.is_err());
                assert_eq!(visited, [0]);
            }
        });
    }

    #[test]
    fn total_argument_overflow_does_not_run_converters() {
        crate::Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let args = FuncArgs::from(vec![vm.ctx.none(); 4]);
            let mut calls = 0;
            let result = SPEC.parse_with(
                &args,
                |_, _obj, _vm| {
                    calls += 1;
                    Ok(())
                },
                vm,
            );
            assert!(result.is_err());
            assert_eq!(calls, 0);
        });
    }

    #[test]
    fn conversion_failure_stops_before_later_parameters() {
        crate::Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let args = FuncArgs::from(vec![vm.ctx.none(); 3]);
            let mut visited = Vec::new();
            let result = SPEC.parse_with(
                &args,
                |i, _obj, vm| {
                    visited.push(i);
                    Err(vm.new_value_error("conversion failed"))
                },
                vm,
            );
            assert!(result.is_err());
            assert_eq!(visited, [0]);
        });
    }
}

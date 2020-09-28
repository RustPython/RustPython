pub(crate) use _json::make_module;
mod machinery;

#[pymodule]
mod _json {
    use super::*;
    use crate::builtins::pystr::PyStrRef;
    use crate::builtins::{pybool, pytype::PyTypeRef};
    use crate::exceptions::PyBaseExceptionRef;
    use crate::function::{FuncArgs, OptionalArg};
    use crate::iterator;
    use crate::pyobject::{
        BorrowValue, IdProtocol, IntoPyObject, PyObjectRef, PyRef, PyResult, PyValue, StaticType,
        TryFromObject,
    };
    use crate::slots::Callable;
    use crate::VirtualMachine;

    use num_bigint::BigInt;
    use std::str::FromStr;

    #[pyattr(name = "make_scanner")]
    #[pyclass(name = "Scanner")]
    #[derive(Debug)]
    struct JsonScanner {
        strict: bool,
        object_hook: Option<PyObjectRef>,
        object_pairs_hook: Option<PyObjectRef>,
        parse_float: Option<PyObjectRef>,
        parse_int: Option<PyObjectRef>,
        parse_constant: PyObjectRef,
        ctx: PyObjectRef,
    }

    impl PyValue for JsonScanner {
        fn class(_vm: &VirtualMachine) -> &PyTypeRef {
            Self::static_type()
        }
    }

    #[pyimpl(with(Callable))]
    impl JsonScanner {
        #[pyslot]
        fn tp_new(cls: PyTypeRef, ctx: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
            let strict = pybool::boolval(vm, vm.get_attribute(ctx.clone(), "strict")?)?;
            let object_hook = vm.option_if_none(vm.get_attribute(ctx.clone(), "object_hook")?);
            let object_pairs_hook =
                vm.option_if_none(vm.get_attribute(ctx.clone(), "object_pairs_hook")?);
            let parse_float = vm.get_attribute(ctx.clone(), "parse_float")?;
            let parse_float =
                if vm.is_none(&parse_float) || parse_float.is(&vm.ctx.types.float_type) {
                    None
                } else {
                    Some(parse_float)
                };
            let parse_int = vm.get_attribute(ctx.clone(), "parse_int")?;
            let parse_int = if vm.is_none(&parse_int) || parse_int.is(&vm.ctx.types.int_type) {
                None
            } else {
                Some(parse_int)
            };
            let parse_constant = vm.get_attribute(ctx.clone(), "parse_constant")?;

            Self {
                strict,
                object_hook,
                object_pairs_hook,
                parse_float,
                parse_int,
                parse_constant,
                ctx,
            }
            .into_ref_with_type(vm, cls)
        }

        fn parse(
            &self,
            s: &str,
            pystr: PyStrRef,
            idx: usize,
            scan_once: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult {
            let c = s
                .chars()
                .next()
                .ok_or_else(|| iterator::stop_iter_with_value(vm.ctx.new_int(idx), vm))?;
            let next_idx = idx + c.len_utf8();
            match c {
                '"' => {
                    return scanstring(pystr, next_idx, OptionalArg::Present(self.strict), vm)
                        .map(|x| x.into_pyobject(vm))
                }
                '{' => {
                    // TODO: parse the object in rust
                    let parse_obj = vm.get_attribute(self.ctx.clone(), "parse_object")?;
                    return vm.invoke(
                        &parse_obj,
                        (
                            vm.ctx
                                .new_tuple(vec![pystr.into_object(), vm.ctx.new_int(next_idx)]),
                            self.strict,
                            scan_once,
                            self.object_hook.clone(),
                            self.object_pairs_hook.clone(),
                        ),
                    );
                }
                '[' => {
                    // TODO: parse the array in rust
                    let parse_array = vm.get_attribute(self.ctx.clone(), "parse_array")?;
                    return vm.invoke(
                        &parse_array,
                        vec![
                            vm.ctx
                                .new_tuple(vec![pystr.into_object(), vm.ctx.new_int(next_idx)]),
                            scan_once,
                        ],
                    );
                }
                _ => {}
            }

            macro_rules! parse_const {
                ($s:literal, $val:expr) => {
                    if s.starts_with($s) {
                        return Ok(vm.ctx.new_tuple(vec![$val, vm.ctx.new_int(idx + $s.len())]));
                    }
                };
            }

            parse_const!("null", vm.ctx.none());
            parse_const!("true", vm.ctx.new_bool(true));
            parse_const!("false", vm.ctx.new_bool(false));

            if let Some((res, len)) = self.parse_number(s, vm) {
                return Ok(vm.ctx.new_tuple(vec![res?, vm.ctx.new_int(idx + len)]));
            }

            macro_rules! parse_constant {
                ($s:literal) => {
                    if s.starts_with($s) {
                        return Ok(vm.ctx.new_tuple(vec![
                            vm.invoke(&self.parse_constant, ($s.to_owned(),))?,
                            vm.ctx.new_int(idx + $s.len()),
                        ]));
                    }
                };
            }

            parse_constant!("NaN");
            parse_constant!("Infinity");
            parse_constant!("-Infinity");

            Err(iterator::stop_iter_with_value(vm.ctx.new_int(idx), vm))
        }

        fn parse_number(&self, s: &str, vm: &VirtualMachine) -> Option<(PyResult, usize)> {
            let mut has_neg = false;
            let mut has_decimal = false;
            let mut has_exponent = false;
            let mut has_e_sign = false;
            let mut i = 0;
            for c in s.chars() {
                match c {
                    '-' if i == 0 => has_neg = true,
                    n if n.is_ascii_digit() => {}
                    '.' if !has_decimal => has_decimal = true,
                    'e' | 'E' if !has_exponent => has_exponent = true,
                    '+' | '-' if !has_e_sign => has_e_sign = true,
                    _ => break,
                }
                i += 1;
            }
            if i == 0 || (i == 1 && has_neg) {
                return None;
            }
            let buf = &s[..i];
            let ret = if has_decimal || has_exponent {
                // float
                if let Some(ref parse_float) = self.parse_float {
                    vm.invoke(parse_float, (buf.to_owned(),))
                } else {
                    Ok(vm.ctx.new_float(f64::from_str(buf).unwrap()))
                }
            } else if let Some(ref parse_int) = self.parse_int {
                vm.invoke(parse_int, (buf.to_owned(),))
            } else {
                Ok(vm.ctx.new_int(BigInt::from_str(buf).unwrap()))
            };
            Some((ret, buf.len()))
        }

        fn call(zelf: &PyRef<Self>, pystr: PyStrRef, idx: isize, vm: &VirtualMachine) -> PyResult {
            if idx < 0 {
                return Err(vm.new_value_error("idx cannot be negative".to_owned()));
            }
            let idx = idx as usize;
            let mut chars = pystr.borrow_value().chars();
            if idx > 0 {
                chars
                    .nth(idx - 1)
                    .ok_or_else(|| iterator::stop_iter_with_value(vm.ctx.new_int(idx), vm))?;
            }
            zelf.parse(
                chars.as_str(),
                pystr.clone(),
                idx,
                zelf.clone().into_object(),
                vm,
            )
        }
    }

    impl Callable for JsonScanner {
        fn call(zelf: &PyRef<Self>, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            let (pystr, idx) = args.bind::<(PyStrRef, isize)>(vm)?;
            JsonScanner::call(zelf, pystr, idx, vm)
        }
    }

    fn encode_string(s: &str, ascii_only: bool) -> String {
        let mut buf = Vec::<u8>::with_capacity(s.len() + 2);
        machinery::write_json_string(s, ascii_only, &mut buf)
            // SAFETY: writing to a vec can't fail
            .unwrap_or_else(|_| unsafe { std::hint::unreachable_unchecked() });
        // SAFETY: we only output valid utf8 from write_json_string
        unsafe { String::from_utf8_unchecked(buf) }
    }

    #[pyfunction]
    fn encode_basestring(s: PyStrRef) -> String {
        encode_string(s.borrow_value(), false)
    }

    #[pyfunction]
    fn encode_basestring_ascii(s: PyStrRef) -> String {
        encode_string(s.borrow_value(), true)
    }

    fn py_decode_error(
        e: machinery::DecodeError,
        s: PyStrRef,
        vm: &VirtualMachine,
    ) -> PyBaseExceptionRef {
        let get_error = || -> PyResult<_> {
            let cls = vm.try_class("json", "JSONDecodeError")?;
            let exc = vm.invoke(cls.as_object(), (e.msg, s, e.pos))?;
            PyBaseExceptionRef::try_from_object(vm, exc)
        };
        match get_error() {
            Ok(x) | Err(x) => x,
        }
    }

    #[pyfunction]
    fn scanstring(
        s: PyStrRef,
        end: usize,
        strict: OptionalArg<bool>,
        vm: &VirtualMachine,
    ) -> PyResult<(String, usize)> {
        machinery::scanstring(s.borrow_value(), end, strict.unwrap_or(true))
            .map_err(|e| py_decode_error(e, s, vm))
    }
}

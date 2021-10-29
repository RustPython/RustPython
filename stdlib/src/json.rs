pub(crate) use _json::make_module;
mod machinery;

#[pymodule]
mod _json {
    use super::machinery;
    use crate::vm::{
        builtins::{PyBaseExceptionRef, PyStrRef, PyTypeRef},
        function::{IntoPyObject, IntoPyResult, OptionalArg},
        protocol::PyIterReturn,
        types::{Callable, Constructor},
        IdProtocol, PyObjectRef, PyObjectView, PyResult, PyValue, VirtualMachine,
    };
    use num_bigint::BigInt;
    use std::str::FromStr;

    #[pyattr(name = "make_scanner")]
    #[pyclass(name = "Scanner")]
    #[derive(Debug, PyValue)]
    struct JsonScanner {
        strict: bool,
        object_hook: Option<PyObjectRef>,
        object_pairs_hook: Option<PyObjectRef>,
        parse_float: Option<PyObjectRef>,
        parse_int: Option<PyObjectRef>,
        parse_constant: PyObjectRef,
        ctx: PyObjectRef,
    }

    impl Constructor for JsonScanner {
        type Args = PyObjectRef;

        fn py_new(cls: PyTypeRef, ctx: Self::Args, vm: &VirtualMachine) -> PyResult {
            let strict = ctx.clone().get_attr("strict", vm)?.try_to_bool(vm)?;
            let object_hook = vm.option_if_none(ctx.clone().get_attr("object_hook", vm)?);
            let object_pairs_hook =
                vm.option_if_none(ctx.clone().get_attr("object_pairs_hook", vm)?);
            let parse_float = ctx.clone().get_attr("parse_float", vm)?;
            let parse_float =
                if vm.is_none(&parse_float) || parse_float.is(&vm.ctx.types.float_type) {
                    None
                } else {
                    Some(parse_float)
                };
            let parse_int = ctx.clone().get_attr("parse_int", vm)?;
            let parse_int = if vm.is_none(&parse_int) || parse_int.is(&vm.ctx.types.int_type) {
                None
            } else {
                Some(parse_int)
            };
            let parse_constant = ctx.clone().get_attr("parse_constant", vm)?;

            Self {
                strict,
                object_hook,
                object_pairs_hook,
                parse_float,
                parse_int,
                parse_constant,
                ctx,
            }
            .into_pyresult_with_type(vm, cls)
        }
    }

    #[pyimpl(with(Callable, Constructor))]
    impl JsonScanner {
        fn parse(
            &self,
            s: &str,
            pystr: PyStrRef,
            idx: usize,
            scan_once: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyIterReturn> {
            let c = match s.chars().next() {
                Some(c) => c,
                None => {
                    return Ok(PyIterReturn::StopIteration(Some(
                        vm.ctx.new_int(idx).into(),
                    )))
                }
            };
            let next_idx = idx + c.len_utf8();
            match c {
                '"' => {
                    return scanstring(pystr, next_idx, OptionalArg::Present(self.strict), vm)
                        .map(|x| PyIterReturn::Return(x.into_pyobject(vm)))
                }
                '{' => {
                    // TODO: parse the object in rust
                    let parse_obj = self.ctx.clone().get_attr("parse_object", vm)?;
                    return PyIterReturn::from_pyresult(
                        vm.invoke(
                            &parse_obj,
                            (
                                (pystr, next_idx),
                                self.strict,
                                scan_once,
                                self.object_hook.clone(),
                                self.object_pairs_hook.clone(),
                            ),
                        ),
                        vm,
                    );
                }
                '[' => {
                    // TODO: parse the array in rust
                    let parse_array = self.ctx.clone().get_attr("parse_array", vm)?;
                    return PyIterReturn::from_pyresult(
                        vm.invoke(&parse_array, ((pystr, next_idx), scan_once)),
                        vm,
                    );
                }
                _ => {}
            }

            macro_rules! parse_const {
                ($s:literal, $val:expr) => {
                    if s.starts_with($s) {
                        return Ok(PyIterReturn::Return(
                            vm.new_tuple(($val, idx + $s.len())).into(),
                        ));
                    }
                };
            }

            parse_const!("null", vm.ctx.none());
            parse_const!("true", true);
            parse_const!("false", false);

            if let Some((res, len)) = self.parse_number(s, vm) {
                return Ok(PyIterReturn::Return(vm.new_tuple((res?, idx + len)).into()));
            }

            macro_rules! parse_constant {
                ($s:literal) => {
                    if s.starts_with($s) {
                        return Ok(PyIterReturn::Return(
                            vm.new_tuple((
                                vm.invoke(&self.parse_constant, ($s.to_owned(),))?,
                                idx + $s.len(),
                            ))
                            .into(),
                        ));
                    }
                };
            }

            parse_constant!("NaN");
            parse_constant!("Infinity");
            parse_constant!("-Infinity");

            Ok(PyIterReturn::StopIteration(Some(
                vm.ctx.new_int(idx).into(),
            )))
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
                    Ok(vm.ctx.new_float(f64::from_str(buf).unwrap()).into())
                }
            } else if let Some(ref parse_int) = self.parse_int {
                vm.invoke(parse_int, (buf.to_owned(),))
            } else {
                Ok(vm.new_pyobj(BigInt::from_str(buf).unwrap()))
            };
            Some((ret, buf.len()))
        }
    }

    impl Callable for JsonScanner {
        type Args = (PyStrRef, isize);
        fn call(
            zelf: &PyObjectView<Self>,
            (pystr, idx): Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult {
            if idx < 0 {
                return Err(vm.new_value_error("idx cannot be negative".to_owned()));
            }
            let idx = idx as usize;
            let mut chars = pystr.as_str().chars();
            if idx > 0 && chars.nth(idx - 1).is_none() {
                PyIterReturn::StopIteration(Some(vm.ctx.new_int(idx).into())).into_pyresult(vm)
            } else {
                zelf.parse(
                    chars.as_str(),
                    pystr.clone(),
                    idx,
                    zelf.to_owned().into(),
                    vm,
                )
                .and_then(|x| x.into_pyresult(vm))
            }
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
        encode_string(s.as_str(), false)
    }

    #[pyfunction]
    fn encode_basestring_ascii(s: PyStrRef) -> String {
        encode_string(s.as_str(), true)
    }

    fn py_decode_error(
        e: machinery::DecodeError,
        s: PyStrRef,
        vm: &VirtualMachine,
    ) -> PyBaseExceptionRef {
        let get_error = || -> PyResult<_> {
            let cls = vm.try_class("json", "JSONDecodeError")?;
            let exc = vm.invoke(cls.as_object(), (e.msg, s, e.pos))?;
            exc.try_into_value(vm)
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
        machinery::scanstring(s.as_str(), end, strict.unwrap_or(true))
            .map_err(|e| py_decode_error(e, s, vm))
    }
}

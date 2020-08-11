pub(crate) use _json::make_module;
mod machinery;

#[pymodule]
mod _json {
    use crate::obj::objiter;
    use crate::obj::objstr::PyStringRef;
    use crate::obj::{objbool, objtype::PyClassRef};
    use crate::pyobject::{
        BorrowValue, IdProtocol, PyClassImpl, PyObjectRef, PyRef, PyResult, PyValue,
    };
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
        fn class(vm: &VirtualMachine) -> PyClassRef {
            vm.class("_json", "make_scanner")
        }
    }

    #[pyimpl]
    impl JsonScanner {
        #[pyslot]
        fn tp_new(cls: PyClassRef, ctx: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
            let strict = objbool::boolval(vm, vm.get_attribute(ctx.clone(), "strict")?)?;
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
            pystr: PyStringRef,
            idx: usize,
            scan_once: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult {
            let c = s
                .chars()
                .next()
                .ok_or_else(|| objiter::stop_iter_with_value(vm.ctx.new_int(idx), vm))?;
            let next_idx = idx + c.len_utf8();
            match c {
                '"' => {
                    // TODO: parse the string in rust
                    let parse_str = vm.get_attribute(self.ctx.clone(), "parse_string")?;
                    return vm.invoke(
                        &parse_str,
                        vec![
                            pystr.into_object(),
                            vm.ctx.new_int(next_idx),
                            vm.ctx.new_bool(self.strict),
                        ],
                    );
                }
                '{' => {
                    // TODO: parse the object in rust
                    let parse_obj = vm.get_attribute(self.ctx.clone(), "parse_object")?;
                    return vm.invoke(
                        &parse_obj,
                        vec![
                            vm.ctx
                                .new_tuple(vec![pystr.into_object(), vm.ctx.new_int(next_idx)]),
                            vm.ctx.new_bool(self.strict),
                            scan_once,
                            self.object_hook.clone().unwrap_or_else(|| vm.get_none()),
                            self.object_pairs_hook
                                .clone()
                                .unwrap_or_else(|| vm.get_none()),
                        ],
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

            parse_const!("null", vm.get_none());
            parse_const!("true", vm.ctx.new_bool(true));
            parse_const!("false", vm.ctx.new_bool(false));

            if let Some((res, len)) = self.parse_number(s, vm) {
                return Ok(vm.ctx.new_tuple(vec![res?, vm.ctx.new_int(idx + len)]));
            }

            macro_rules! parse_constant {
                ($s:literal) => {
                    if s.starts_with($s) {
                        return Ok(vm.ctx.new_tuple(vec![
                            vm.invoke(&self.parse_constant, vec![vm.ctx.new_str($s.to_owned())])?,
                            vm.ctx.new_int(idx + $s.len()),
                        ]));
                    }
                };
            }

            parse_constant!("NaN");
            parse_constant!("Infinity");
            parse_constant!("-Infinity");

            Err(objiter::stop_iter_with_value(vm.ctx.new_int(idx), vm))
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
                    vm.invoke(parse_float, vec![vm.ctx.new_str(buf.to_owned())])
                } else {
                    Ok(vm.ctx.new_float(f64::from_str(buf).unwrap()))
                }
            } else if let Some(ref parse_int) = self.parse_int {
                vm.invoke(parse_int, vec![vm.ctx.new_str(buf.to_owned())])
            } else {
                Ok(vm.ctx.new_int(BigInt::from_str(buf).unwrap()))
            };
            Some((ret, buf.len()))
        }

        #[pyslot]
        fn call(
            zelf: PyRef<Self>,
            pystr: PyStringRef,
            idx: isize,
            vm: &VirtualMachine,
        ) -> PyResult {
            if idx < 0 {
                return Err(vm.new_value_error("idx cannot be negative".to_owned()));
            }
            let idx = idx as usize;
            let mut chars = pystr.borrow_value().chars();
            if idx > 0 {
                chars
                    .nth(idx - 1)
                    .ok_or_else(|| objiter::stop_iter_with_value(vm.ctx.new_int(idx), vm))?;
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

    fn encode_string(s: &str, ascii_only: bool) -> String {
        let mut buf = Vec::<u8>::with_capacity(s.len() + 2);
        super::machinery::write_json_string(s, ascii_only, &mut buf)
            // writing to a vec can't fail
            .unwrap_or_else(|_| unsafe { std::hint::unreachable_unchecked() });
        // TODO: verify that the implementation is correct enough to use `from_utf8_unchecked`
        String::from_utf8(buf).expect("invalid utf-8 in json output")
    }

    #[pyfunction]
    fn encode_basestring(s: PyStringRef) -> String {
        encode_string(s.borrow_value(), false)
    }

    #[pyfunction]
    fn encode_basestring_ascii(s: PyStringRef) -> String {
        encode_string(s.borrow_value(), true)
    }
}

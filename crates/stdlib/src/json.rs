pub(crate) use _json::make_module;
mod machinery;

#[pymodule]
mod _json {
    use super::machinery;
    use crate::vm::{
        AsObject, Py, PyObjectRef, PyPayload, PyResult, VirtualMachine,
        builtins::{PyBaseExceptionRef, PyStrRef, PyType},
        convert::{ToPyObject, ToPyResult},
        function::{IntoFuncArgs, OptionalArg},
        protocol::PyIterReturn,
        types::{Callable, Constructor},
    };
    use core::str::FromStr;
    use malachite_bigint::BigInt;
    use rustpython_common::wtf8::Wtf8Buf;
    use std::collections::HashMap;

    /// Skip JSON whitespace characters (space, tab, newline, carriage return).
    /// Works with a character iterator and returns the number of characters skipped.
    #[inline]
    fn skip_whitespace_chars<I>(chars: &mut std::iter::Peekable<I>) -> usize
    where
        I: Iterator<Item = char>,
    {
        flame_guard!("_json::skip_whitespace_chars");
        let mut count = 0;
        while let Some(&c) = chars.peek() {
            match c {
                ' ' | '\t' | '\n' | '\r' => {
                    chars.next();
                    count += 1;
                }
                _ => break,
            }
        }
        count
    }

    /// Check if a character iterator starts with a given pattern.
    /// This avoids byte/char index mismatch issues with non-ASCII strings.
    #[inline]
    fn starts_with_chars<I>(mut chars: I, pattern: &str) -> bool
    where
        I: Iterator<Item = char>,
    {
        for expected in pattern.chars() {
            match chars.next() {
                Some(c) if c == expected => continue,
                _ => return false,
            }
        }
        true
    }

    #[pyattr(name = "make_scanner")]
    #[pyclass(name = "Scanner", traverse)]
    #[derive(Debug, PyPayload)]
    struct JsonScanner {
        #[pytraverse(skip)]
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

        fn py_new(_cls: &Py<PyType>, ctx: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            let strict = ctx.get_attr("strict", vm)?.try_to_bool(vm)?;
            let object_hook = vm.option_if_none(ctx.get_attr("object_hook", vm)?);
            let object_pairs_hook = vm.option_if_none(ctx.get_attr("object_pairs_hook", vm)?);
            let parse_float = ctx.get_attr("parse_float", vm)?;
            let parse_float = if vm.is_none(&parse_float) || parse_float.is(vm.ctx.types.float_type)
            {
                None
            } else {
                Some(parse_float)
            };
            let parse_int = ctx.get_attr("parse_int", vm)?;
            let parse_int = if vm.is_none(&parse_int) || parse_int.is(vm.ctx.types.int_type) {
                None
            } else {
                Some(parse_int)
            };
            let parse_constant = ctx.get_attr("parse_constant", vm)?;

            Ok(Self {
                strict,
                object_hook,
                object_pairs_hook,
                parse_float,
                parse_int,
                parse_constant,
                ctx,
            })
        }
    }

    #[pyclass(with(Callable, Constructor))]
    impl JsonScanner {
        fn parse(
            &self,
            s: &str,
            pystr: PyStrRef,
            idx: usize,
            scan_once: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyIterReturn> {
            flame_guard!("JsonScanner::parse");
            let c = match s.chars().next() {
                Some(c) => c,
                None => {
                    return Ok(PyIterReturn::StopIteration(Some(
                        vm.ctx.new_int(idx).into(),
                    )));
                }
            };
            let next_idx = idx + c.len_utf8();
            match c {
                '"' => {
                    return scanstring(pystr, next_idx, OptionalArg::Present(self.strict), vm)
                        .map(|x| PyIterReturn::Return(x.to_pyobject(vm)));
                }
                '{' => {
                    // Parse object in Rust
                    let mut memo = HashMap::new();
                    return self
                        .parse_object(pystr, next_idx, &scan_once, &mut memo, vm)
                        .map(|(obj, end)| PyIterReturn::Return(vm.new_tuple((obj, end)).into()));
                }
                '[' => {
                    // Parse array in Rust
                    let mut memo = HashMap::new();
                    return self
                        .parse_array(pystr, next_idx, &scan_once, &mut memo, vm)
                        .map(|(obj, end)| PyIterReturn::Return(vm.new_tuple((obj, end)).into()));
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
                            vm.new_tuple((self.parse_constant.call(($s,), vm)?, idx + $s.len()))
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
            flame_guard!("JsonScanner::parse_number");
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
                    parse_float.call((buf,), vm)
                } else {
                    Ok(vm.ctx.new_float(f64::from_str(buf).unwrap()).into())
                }
            } else if let Some(ref parse_int) = self.parse_int {
                parse_int.call((buf,), vm)
            } else {
                Ok(vm.new_pyobj(BigInt::from_str(buf).unwrap()))
            };
            Some((ret, buf.len()))
        }

        /// Parse a number from a character iterator.
        /// Returns (result, character_count) where character_count is the number of chars consumed.
        fn parse_number_from_chars<I>(
            &self,
            chars: I,
            vm: &VirtualMachine,
        ) -> Option<(PyResult, usize)>
        where
            I: Iterator<Item = char>,
        {
            flame_guard!("JsonScanner::parse_number_from_chars");
            let mut buf = String::new();
            let mut has_neg = false;
            let mut has_decimal = false;
            let mut has_exponent = false;
            let mut has_e_sign = false;

            for c in chars {
                let i = buf.len();
                match c {
                    '-' if i == 0 => has_neg = true,
                    n if n.is_ascii_digit() => {}
                    '.' if !has_decimal => has_decimal = true,
                    'e' | 'E' if !has_exponent => has_exponent = true,
                    '+' | '-' if !has_e_sign => has_e_sign = true,
                    _ => break,
                }
                buf.push(c);
            }

            let len = buf.len();
            if len == 0 || (len == 1 && has_neg) {
                return None;
            }

            let ret = if has_decimal || has_exponent {
                if let Some(ref parse_float) = self.parse_float {
                    parse_float.call((&buf,), vm)
                } else {
                    Ok(vm.ctx.new_float(f64::from_str(&buf).unwrap()).into())
                }
            } else if let Some(ref parse_int) = self.parse_int {
                parse_int.call((&buf,), vm)
            } else {
                Ok(vm.new_pyobj(BigInt::from_str(&buf).unwrap()))
            };
            Some((ret, len))
        }

        /// Parse a JSON object starting after the opening '{'.
        /// Returns (parsed_object, end_character_index).
        fn parse_object(
            &self,
            pystr: PyStrRef,
            start_idx: usize, // Character index right after '{'
            scan_once: &PyObjectRef,
            memo: &mut HashMap<String, PyStrRef>,
            vm: &VirtualMachine,
        ) -> PyResult<(PyObjectRef, usize)> {
            flame_guard!("JsonScanner::parse_object");

            let s = pystr.as_str();
            let mut chars = s.chars().skip(start_idx).peekable();
            let mut idx = start_idx;

            // Skip initial whitespace
            idx += skip_whitespace_chars(&mut chars);

            // Check for empty object
            match chars.peek() {
                Some('}') => {
                    return self.finalize_object(vec![], idx + 1, vm);
                }
                Some('"') => {
                    // Continue to parse first key
                }
                Some(_) | None => {
                    return Err(self.make_decode_error(
                        "Expecting property name enclosed in double quotes",
                        pystr,
                        idx,
                        vm,
                    ));
                }
            }

            let mut pairs: Vec<(PyObjectRef, PyObjectRef)> = Vec::new();

            loop {
                // We're now at '"', skip it
                chars.next();
                idx += 1;

                // Parse key string using existing scanstring
                let (key_wtf8, key_end) = machinery::scanstring(pystr.as_wtf8(), idx, self.strict)
                    .map_err(|e| py_decode_error(e, pystr.clone(), vm))?;

                // Key memoization - reuse existing key strings
                let key_str = key_wtf8.to_string();
                let key: PyObjectRef = match memo.get(&key_str) {
                    Some(cached) => cached.clone().into(),
                    None => {
                        let py_key = vm.ctx.new_str(key_str.clone());
                        memo.insert(key_str, py_key.clone());
                        py_key.into()
                    }
                };

                // Update position and rebuild iterator
                idx = key_end;
                chars = s.chars().skip(idx).peekable();

                // Skip whitespace after key
                idx += skip_whitespace_chars(&mut chars);

                // Expect ':' delimiter
                match chars.peek() {
                    Some(':') => {
                        chars.next();
                        idx += 1;
                    }
                    _ => {
                        return Err(self.make_decode_error(
                            "Expecting ':' delimiter",
                            pystr,
                            idx,
                            vm,
                        ));
                    }
                }

                // Skip whitespace after ':'
                idx += skip_whitespace_chars(&mut chars);

                // Parse value recursively using scan_once
                let (value, value_end) =
                    self.call_scan_once(scan_once, pystr.clone(), idx, memo, vm)?;

                pairs.push((key, value));
                idx = value_end;
                chars = s.chars().skip(idx).peekable();

                // Skip whitespace after value
                idx += skip_whitespace_chars(&mut chars);

                // Check for ',' or '}'
                match chars.peek() {
                    Some('}') => {
                        idx += 1;
                        break;
                    }
                    Some(',') => {
                        let comma_idx = idx;
                        chars.next();
                        idx += 1;

                        // Skip whitespace after comma
                        idx += skip_whitespace_chars(&mut chars);

                        // Next must be '"'
                        match chars.peek() {
                            Some('"') => {
                                // Continue to next key-value pair
                            }
                            Some('}') => {
                                // Trailing comma before end of object
                                return Err(self.make_decode_error(
                                    "Illegal trailing comma before end of object",
                                    pystr,
                                    comma_idx,
                                    vm,
                                ));
                            }
                            _ => {
                                return Err(self.make_decode_error(
                                    "Expecting property name enclosed in double quotes",
                                    pystr,
                                    idx,
                                    vm,
                                ));
                            }
                        }
                    }
                    _ => {
                        return Err(self.make_decode_error(
                            "Expecting ',' delimiter",
                            pystr,
                            idx,
                            vm,
                        ));
                    }
                }
            }

            self.finalize_object(pairs, idx, vm)
        }

        /// Parse a JSON array starting after the opening '['.
        /// Returns (parsed_array, end_character_index).
        fn parse_array(
            &self,
            pystr: PyStrRef,
            start_idx: usize, // Character index right after '['
            scan_once: &PyObjectRef,
            memo: &mut HashMap<String, PyStrRef>,
            vm: &VirtualMachine,
        ) -> PyResult<(PyObjectRef, usize)> {
            flame_guard!("JsonScanner::parse_array");

            let s = pystr.as_str();
            let mut chars = s.chars().skip(start_idx).peekable();
            let mut idx = start_idx;

            // Skip initial whitespace
            idx += skip_whitespace_chars(&mut chars);

            // Check for empty array
            if chars.peek() == Some(&']') {
                return Ok((vm.ctx.new_list(vec![]).into(), idx + 1));
            }

            let mut values: Vec<PyObjectRef> = Vec::new();

            loop {
                // Parse value
                let (value, value_end) =
                    self.call_scan_once(scan_once, pystr.clone(), idx, memo, vm)?;

                values.push(value);
                idx = value_end;
                chars = s.chars().skip(idx).peekable();

                // Skip whitespace after value
                idx += skip_whitespace_chars(&mut chars);

                match chars.peek() {
                    Some(']') => {
                        idx += 1;
                        break;
                    }
                    Some(',') => {
                        let comma_idx = idx;
                        chars.next();
                        idx += 1;
                        // Skip whitespace after comma
                        idx += skip_whitespace_chars(&mut chars);

                        // Check for trailing comma
                        if chars.peek() == Some(&']') {
                            return Err(self.make_decode_error(
                                "Illegal trailing comma before end of array",
                                pystr,
                                comma_idx,
                                vm,
                            ));
                        }
                    }
                    _ => {
                        return Err(self.make_decode_error(
                            "Expecting ',' delimiter",
                            pystr,
                            idx,
                            vm,
                        ));
                    }
                }
            }

            Ok((vm.ctx.new_list(values).into(), idx))
        }

        /// Finalize object construction with hooks.
        fn finalize_object(
            &self,
            pairs: Vec<(PyObjectRef, PyObjectRef)>,
            end_idx: usize,
            vm: &VirtualMachine,
        ) -> PyResult<(PyObjectRef, usize)> {
            let result = if let Some(ref pairs_hook) = self.object_pairs_hook {
                // object_pairs_hook takes priority - pass list of tuples
                let pairs_list: Vec<PyObjectRef> = pairs
                    .into_iter()
                    .map(|(k, v)| vm.new_tuple((k, v)).into())
                    .collect();
                pairs_hook.call((vm.ctx.new_list(pairs_list),), vm)?
            } else {
                // Build a dict from pairs
                let dict = vm.ctx.new_dict();
                for (key, value) in pairs {
                    dict.set_item(&*key, value, vm)?;
                }

                // Apply object_hook if present
                let dict_obj: PyObjectRef = dict.into();
                if let Some(ref hook) = self.object_hook {
                    hook.call((dict_obj,), vm)?
                } else {
                    dict_obj
                }
            };

            Ok((result, end_idx))
        }

        /// Call scan_once and handle the result.
        /// Uses character iterators to avoid byte/char index mismatch with non-ASCII strings.
        fn call_scan_once(
            &self,
            scan_once: &PyObjectRef,
            pystr: PyStrRef,
            idx: usize,
            memo: &mut HashMap<String, PyStrRef>,
            vm: &VirtualMachine,
        ) -> PyResult<(PyObjectRef, usize)> {
            let s = pystr.as_str();
            let chars = s.chars().skip(idx).peekable();

            let first_char = match chars.clone().next() {
                Some(c) => c,
                None => return Err(self.make_decode_error("Expecting value", pystr, idx, vm)),
            };

            match first_char {
                '"' => {
                    // String
                    let (wtf8, end) = machinery::scanstring(pystr.as_wtf8(), idx + 1, self.strict)
                        .map_err(|e| py_decode_error(e, pystr.clone(), vm))?;
                    let py_str = vm.ctx.new_str(wtf8.to_string());
                    Ok((py_str.into(), end))
                }
                '{' => {
                    // Object
                    self.parse_object(pystr, idx + 1, scan_once, memo, vm)
                }
                '[' => {
                    // Array
                    self.parse_array(pystr, idx + 1, scan_once, memo, vm)
                }
                'n' if starts_with_chars(chars.clone(), "null") => {
                    // null
                    Ok((vm.ctx.none(), idx + 4))
                }
                't' if starts_with_chars(chars.clone(), "true") => {
                    // true
                    Ok((vm.ctx.new_bool(true).into(), idx + 4))
                }
                'f' if starts_with_chars(chars.clone(), "false") => {
                    // false
                    Ok((vm.ctx.new_bool(false).into(), idx + 5))
                }
                'N' if starts_with_chars(chars.clone(), "NaN") => {
                    // NaN
                    let result = self.parse_constant.call(("NaN",), vm)?;
                    Ok((result, idx + 3))
                }
                'I' if starts_with_chars(chars.clone(), "Infinity") => {
                    // Infinity
                    let result = self.parse_constant.call(("Infinity",), vm)?;
                    Ok((result, idx + 8))
                }
                '-' => {
                    // -Infinity or negative number
                    if starts_with_chars(chars.clone(), "-Infinity") {
                        let result = self.parse_constant.call(("-Infinity",), vm)?;
                        return Ok((result, idx + 9));
                    }
                    // Negative number - collect number characters
                    if let Some((result, len)) = self.parse_number_from_chars(chars, vm) {
                        return Ok((result?, idx + len));
                    }
                    Err(self.make_decode_error("Expecting value", pystr, idx, vm))
                }
                c if c.is_ascii_digit() => {
                    // Positive number
                    if let Some((result, len)) = self.parse_number_from_chars(chars, vm) {
                        return Ok((result?, idx + len));
                    }
                    Err(self.make_decode_error("Expecting value", pystr, idx, vm))
                }
                _ => {
                    // Fall back to scan_once for unrecognized input
                    let result = scan_once.call((pystr.clone(), idx as isize), vm);

                    match result {
                        Ok(tuple) => {
                            use crate::vm::builtins::PyTupleRef;
                            let tuple: PyTupleRef = tuple.try_into_value(vm)?;
                            if tuple.len() != 2 {
                                return Err(vm.new_value_error("scan_once must return 2-tuple"));
                            }
                            let value = tuple.as_slice()[0].clone();
                            let end_idx: isize = tuple.as_slice()[1].try_to_value(vm)?;
                            Ok((value, end_idx as usize))
                        }
                        Err(err) if err.fast_isinstance(vm.ctx.exceptions.stop_iteration) => {
                            Err(self.make_decode_error("Expecting value", pystr, idx, vm))
                        }
                        Err(err) => Err(err),
                    }
                }
            }
        }

        /// Create a decode error.
        fn make_decode_error(
            &self,
            msg: &str,
            s: PyStrRef,
            pos: usize,
            vm: &VirtualMachine,
        ) -> PyBaseExceptionRef {
            let err = machinery::DecodeError::new(msg, pos);
            py_decode_error(err, s, vm)
        }
    }

    impl Callable for JsonScanner {
        type Args = (PyStrRef, isize);
        fn call(zelf: &Py<Self>, (pystr, idx): Self::Args, vm: &VirtualMachine) -> PyResult {
            if idx < 0 {
                return Err(vm.new_value_error("idx cannot be negative"));
            }
            let idx = idx as usize;
            let mut chars = pystr.as_str().chars();
            if idx > 0 && chars.nth(idx - 1).is_none() {
                PyIterReturn::StopIteration(Some(vm.ctx.new_int(idx).into())).to_pyresult(vm)
            } else {
                zelf.parse(
                    chars.as_str(),
                    pystr.clone(),
                    idx,
                    zelf.to_owned().into(),
                    vm,
                )
                .and_then(|x| x.to_pyresult(vm))
            }
        }
    }

    fn encode_string(s: &str, ascii_only: bool) -> String {
        flame_guard!("_json::encode_string");
        let mut buf = Vec::<u8>::with_capacity(s.len() + 2);
        machinery::write_json_string(s, ascii_only, &mut buf)
            // SAFETY: writing to a vec can't fail
            .unwrap_or_else(|_| unsafe { core::hint::unreachable_unchecked() });
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
            let exc = PyType::call(&cls, (e.msg, s, e.pos).into_args(vm), vm)?;
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
    ) -> PyResult<(Wtf8Buf, usize)> {
        flame_guard!("_json::scanstring");
        machinery::scanstring(s.as_wtf8(), end, strict.unwrap_or(true))
            .map_err(|e| py_decode_error(e, s, vm))
    }
}

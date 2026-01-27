pub(crate) use _json::module_def;
mod machinery;

#[pymodule]
mod _json {
    use super::machinery;
    use crate::vm::{
        AsObject, Py, PyObjectRef, PyPayload, PyResult, VirtualMachine,
        builtins::{PyBaseExceptionRef, PyStrRef, PyType},
        convert::ToPyResult,
        function::{IntoFuncArgs, OptionalArg},
        protocol::PyIterReturn,
        types::{Callable, Constructor},
    };
    use core::str::FromStr;
    use malachite_bigint::BigInt;
    use rustpython_common::wtf8::Wtf8Buf;
    use std::collections::HashMap;

    /// Skip JSON whitespace characters (space, tab, newline, carriage return).
    /// Works with a byte slice and returns the number of bytes skipped.
    /// Since all JSON whitespace chars are ASCII, bytes == chars.
    #[inline]
    fn skip_whitespace(bytes: &[u8]) -> usize {
        flame_guard!("_json::skip_whitespace");
        let mut count = 0;
        for &b in bytes {
            match b {
                b' ' | b'\t' | b'\n' | b'\r' => count += 1,
                _ => break,
            }
        }
        count
    }

    /// Check if a byte slice starts with a given ASCII pattern.
    #[inline]
    fn starts_with_bytes(bytes: &[u8], pattern: &[u8]) -> bool {
        bytes.len() >= pattern.len() && &bytes[..pattern.len()] == pattern
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
            pystr: PyStrRef,
            char_idx: usize,
            byte_idx: usize,
            scan_once: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyIterReturn> {
            flame_guard!("JsonScanner::parse");
            let bytes = pystr.as_str().as_bytes();
            let wtf8 = pystr.as_wtf8();

            let first_byte = match bytes.get(byte_idx) {
                Some(&b) => b,
                None => {
                    return Ok(PyIterReturn::StopIteration(Some(
                        vm.ctx.new_int(char_idx).into(),
                    )));
                }
            };

            match first_byte {
                b'"' => {
                    // Parse string - pass slice starting after the quote
                    let (wtf8_result, chars_consumed, _bytes_consumed) =
                        machinery::scanstring(&wtf8[byte_idx + 1..], char_idx + 1, self.strict)
                            .map_err(|e| py_decode_error(e, pystr.clone(), vm))?;
                    let end_char_idx = char_idx + 1 + chars_consumed;
                    return Ok(PyIterReturn::Return(
                        vm.new_tuple((wtf8_result, end_char_idx)).into(),
                    ));
                }
                b'{' => {
                    // Parse object in Rust
                    let mut memo = HashMap::new();
                    return self
                        .parse_object(pystr, char_idx + 1, byte_idx + 1, &scan_once, &mut memo, vm)
                        .map(|(obj, end_char, _end_byte)| {
                            PyIterReturn::Return(vm.new_tuple((obj, end_char)).into())
                        });
                }
                b'[' => {
                    // Parse array in Rust
                    let mut memo = HashMap::new();
                    return self
                        .parse_array(pystr, char_idx + 1, byte_idx + 1, &scan_once, &mut memo, vm)
                        .map(|(obj, end_char, _end_byte)| {
                            PyIterReturn::Return(vm.new_tuple((obj, end_char)).into())
                        });
                }
                _ => {}
            }

            let s = &pystr.as_str()[byte_idx..];

            macro_rules! parse_const {
                ($s:literal, $val:expr) => {
                    if s.starts_with($s) {
                        return Ok(PyIterReturn::Return(
                            vm.new_tuple(($val, char_idx + $s.len())).into(),
                        ));
                    }
                };
            }

            parse_const!("null", vm.ctx.none());
            parse_const!("true", true);
            parse_const!("false", false);

            if let Some((res, len)) = self.parse_number(s, vm) {
                return Ok(PyIterReturn::Return(
                    vm.new_tuple((res?, char_idx + len)).into(),
                ));
            }

            macro_rules! parse_constant {
                ($s:literal) => {
                    if s.starts_with($s) {
                        return Ok(PyIterReturn::Return(
                            vm.new_tuple((
                                self.parse_constant.call(($s,), vm)?,
                                char_idx + $s.len(),
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
                vm.ctx.new_int(char_idx).into(),
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

        /// Parse a JSON object starting after the opening '{'.
        /// Returns (parsed_object, end_char_index, end_byte_index).
        fn parse_object(
            &self,
            pystr: PyStrRef,
            start_char_idx: usize,
            start_byte_idx: usize,
            scan_once: &PyObjectRef,
            memo: &mut HashMap<String, PyStrRef>,
            vm: &VirtualMachine,
        ) -> PyResult<(PyObjectRef, usize, usize)> {
            flame_guard!("JsonScanner::parse_object");

            let bytes = pystr.as_str().as_bytes();
            let wtf8 = pystr.as_wtf8();
            let mut char_idx = start_char_idx;
            let mut byte_idx = start_byte_idx;

            // Skip initial whitespace
            let ws = skip_whitespace(&bytes[byte_idx..]);
            char_idx += ws;
            byte_idx += ws;

            // Check for empty object
            match bytes.get(byte_idx) {
                Some(b'}') => {
                    return self.finalize_object(vec![], char_idx + 1, byte_idx + 1, vm);
                }
                Some(b'"') => {
                    // Continue to parse first key
                }
                _ => {
                    return Err(self.make_decode_error(
                        "Expecting property name enclosed in double quotes",
                        pystr,
                        char_idx,
                        vm,
                    ));
                }
            }

            let mut pairs: Vec<(PyObjectRef, PyObjectRef)> = Vec::new();

            loop {
                // We're now at '"', skip it
                char_idx += 1;
                byte_idx += 1;

                // Parse key string using scanstring with byte slice
                let (key_wtf8, chars_consumed, bytes_consumed) =
                    machinery::scanstring(&wtf8[byte_idx..], char_idx, self.strict)
                        .map_err(|e| py_decode_error(e, pystr.clone(), vm))?;

                char_idx += chars_consumed;
                byte_idx += bytes_consumed;

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

                // Skip whitespace after key
                let ws = skip_whitespace(&bytes[byte_idx..]);
                char_idx += ws;
                byte_idx += ws;

                // Expect ':' delimiter
                match bytes.get(byte_idx) {
                    Some(b':') => {
                        char_idx += 1;
                        byte_idx += 1;
                    }
                    _ => {
                        return Err(self.make_decode_error(
                            "Expecting ':' delimiter",
                            pystr,
                            char_idx,
                            vm,
                        ));
                    }
                }

                // Skip whitespace after ':'
                let ws = skip_whitespace(&bytes[byte_idx..]);
                char_idx += ws;
                byte_idx += ws;

                // Parse value recursively
                let (value, value_char_end, value_byte_end) =
                    self.call_scan_once(scan_once, pystr.clone(), char_idx, byte_idx, memo, vm)?;

                pairs.push((key, value));
                char_idx = value_char_end;
                byte_idx = value_byte_end;

                // Skip whitespace after value
                let ws = skip_whitespace(&bytes[byte_idx..]);
                char_idx += ws;
                byte_idx += ws;

                // Check for ',' or '}'
                match bytes.get(byte_idx) {
                    Some(b'}') => {
                        char_idx += 1;
                        byte_idx += 1;
                        break;
                    }
                    Some(b',') => {
                        let comma_char_idx = char_idx;
                        char_idx += 1;
                        byte_idx += 1;

                        // Skip whitespace after comma
                        let ws = skip_whitespace(&bytes[byte_idx..]);
                        char_idx += ws;
                        byte_idx += ws;

                        // Next must be '"'
                        match bytes.get(byte_idx) {
                            Some(b'"') => {
                                // Continue to next key-value pair
                            }
                            Some(b'}') => {
                                // Trailing comma before end of object
                                return Err(self.make_decode_error(
                                    "Illegal trailing comma before end of object",
                                    pystr,
                                    comma_char_idx,
                                    vm,
                                ));
                            }
                            _ => {
                                return Err(self.make_decode_error(
                                    "Expecting property name enclosed in double quotes",
                                    pystr,
                                    char_idx,
                                    vm,
                                ));
                            }
                        }
                    }
                    _ => {
                        return Err(self.make_decode_error(
                            "Expecting ',' delimiter",
                            pystr,
                            char_idx,
                            vm,
                        ));
                    }
                }
            }

            self.finalize_object(pairs, char_idx, byte_idx, vm)
        }

        /// Parse a JSON array starting after the opening '['.
        /// Returns (parsed_array, end_char_index, end_byte_index).
        fn parse_array(
            &self,
            pystr: PyStrRef,
            start_char_idx: usize,
            start_byte_idx: usize,
            scan_once: &PyObjectRef,
            memo: &mut HashMap<String, PyStrRef>,
            vm: &VirtualMachine,
        ) -> PyResult<(PyObjectRef, usize, usize)> {
            flame_guard!("JsonScanner::parse_array");

            let bytes = pystr.as_str().as_bytes();
            let mut char_idx = start_char_idx;
            let mut byte_idx = start_byte_idx;

            // Skip initial whitespace
            let ws = skip_whitespace(&bytes[byte_idx..]);
            char_idx += ws;
            byte_idx += ws;

            // Check for empty array
            if bytes.get(byte_idx) == Some(&b']') {
                return Ok((vm.ctx.new_list(vec![]).into(), char_idx + 1, byte_idx + 1));
            }

            let mut values: Vec<PyObjectRef> = Vec::new();

            loop {
                // Parse value
                let (value, value_char_end, value_byte_end) =
                    self.call_scan_once(scan_once, pystr.clone(), char_idx, byte_idx, memo, vm)?;

                values.push(value);
                char_idx = value_char_end;
                byte_idx = value_byte_end;

                // Skip whitespace after value
                let ws = skip_whitespace(&bytes[byte_idx..]);
                char_idx += ws;
                byte_idx += ws;

                match bytes.get(byte_idx) {
                    Some(b']') => {
                        char_idx += 1;
                        byte_idx += 1;
                        break;
                    }
                    Some(b',') => {
                        let comma_char_idx = char_idx;
                        char_idx += 1;
                        byte_idx += 1;

                        // Skip whitespace after comma
                        let ws = skip_whitespace(&bytes[byte_idx..]);
                        char_idx += ws;
                        byte_idx += ws;

                        // Check for trailing comma
                        if bytes.get(byte_idx) == Some(&b']') {
                            return Err(self.make_decode_error(
                                "Illegal trailing comma before end of array",
                                pystr,
                                comma_char_idx,
                                vm,
                            ));
                        }
                    }
                    _ => {
                        return Err(self.make_decode_error(
                            "Expecting ',' delimiter",
                            pystr,
                            char_idx,
                            vm,
                        ));
                    }
                }
            }

            Ok((vm.ctx.new_list(values).into(), char_idx, byte_idx))
        }

        /// Finalize object construction with hooks.
        fn finalize_object(
            &self,
            pairs: Vec<(PyObjectRef, PyObjectRef)>,
            end_char_idx: usize,
            end_byte_idx: usize,
            vm: &VirtualMachine,
        ) -> PyResult<(PyObjectRef, usize, usize)> {
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

            Ok((result, end_char_idx, end_byte_idx))
        }

        /// Call scan_once and handle the result.
        /// Returns (value, end_char_idx, end_byte_idx).
        fn call_scan_once(
            &self,
            scan_once: &PyObjectRef,
            pystr: PyStrRef,
            char_idx: usize,
            byte_idx: usize,
            memo: &mut HashMap<String, PyStrRef>,
            vm: &VirtualMachine,
        ) -> PyResult<(PyObjectRef, usize, usize)> {
            let s = pystr.as_str();
            let bytes = s.as_bytes();
            let wtf8 = pystr.as_wtf8();

            let first_byte = match bytes.get(byte_idx) {
                Some(&b) => b,
                None => return Err(self.make_decode_error("Expecting value", pystr, char_idx, vm)),
            };

            match first_byte {
                b'"' => {
                    // String - pass slice starting after the quote
                    let (wtf8_result, chars_consumed, bytes_consumed) =
                        machinery::scanstring(&wtf8[byte_idx + 1..], char_idx + 1, self.strict)
                            .map_err(|e| py_decode_error(e, pystr.clone(), vm))?;
                    let py_str = vm.ctx.new_str(wtf8_result.to_string());
                    Ok((
                        py_str.into(),
                        char_idx + 1 + chars_consumed,
                        byte_idx + 1 + bytes_consumed,
                    ))
                }
                b'{' => {
                    // Object
                    self.parse_object(pystr, char_idx + 1, byte_idx + 1, scan_once, memo, vm)
                }
                b'[' => {
                    // Array
                    self.parse_array(pystr, char_idx + 1, byte_idx + 1, scan_once, memo, vm)
                }
                b'n' if starts_with_bytes(&bytes[byte_idx..], b"null") => {
                    // null
                    Ok((vm.ctx.none(), char_idx + 4, byte_idx + 4))
                }
                b't' if starts_with_bytes(&bytes[byte_idx..], b"true") => {
                    // true
                    Ok((vm.ctx.new_bool(true).into(), char_idx + 4, byte_idx + 4))
                }
                b'f' if starts_with_bytes(&bytes[byte_idx..], b"false") => {
                    // false
                    Ok((vm.ctx.new_bool(false).into(), char_idx + 5, byte_idx + 5))
                }
                b'N' if starts_with_bytes(&bytes[byte_idx..], b"NaN") => {
                    // NaN
                    let result = self.parse_constant.call(("NaN",), vm)?;
                    Ok((result, char_idx + 3, byte_idx + 3))
                }
                b'I' if starts_with_bytes(&bytes[byte_idx..], b"Infinity") => {
                    // Infinity
                    let result = self.parse_constant.call(("Infinity",), vm)?;
                    Ok((result, char_idx + 8, byte_idx + 8))
                }
                b'-' => {
                    // -Infinity or negative number
                    if starts_with_bytes(&bytes[byte_idx..], b"-Infinity") {
                        let result = self.parse_constant.call(("-Infinity",), vm)?;
                        return Ok((result, char_idx + 9, byte_idx + 9));
                    }
                    // Negative number - numbers are ASCII so len == bytes
                    if let Some((result, len)) = self.parse_number(&s[byte_idx..], vm) {
                        return Ok((result?, char_idx + len, byte_idx + len));
                    }
                    Err(self.make_decode_error("Expecting value", pystr, char_idx, vm))
                }
                b'0'..=b'9' => {
                    // Positive number - numbers are ASCII so len == bytes
                    if let Some((result, len)) = self.parse_number(&s[byte_idx..], vm) {
                        return Ok((result?, char_idx + len, byte_idx + len));
                    }
                    Err(self.make_decode_error("Expecting value", pystr, char_idx, vm))
                }
                _ => {
                    // Fall back to scan_once for unrecognized input
                    // Note: This path requires char_idx for Python compatibility
                    let result = scan_once.call((pystr.clone(), char_idx as isize), vm);

                    match result {
                        Ok(tuple) => {
                            use crate::vm::builtins::PyTupleRef;
                            let tuple: PyTupleRef = tuple.try_into_value(vm)?;
                            if tuple.len() != 2 {
                                return Err(vm.new_value_error("scan_once must return 2-tuple"));
                            }
                            let value = tuple.as_slice()[0].clone();
                            let end_char_idx: isize = tuple.as_slice()[1].try_to_value(vm)?;
                            // For fallback, we need to calculate byte_idx from char_idx
                            // This is expensive but fallback should be rare
                            let end_byte_idx = s
                                .char_indices()
                                .nth(end_char_idx as usize)
                                .map(|(i, _)| i)
                                .unwrap_or(s.len());
                            Ok((value, end_char_idx as usize, end_byte_idx))
                        }
                        Err(err) if err.fast_isinstance(vm.ctx.exceptions.stop_iteration) => {
                            Err(self.make_decode_error("Expecting value", pystr, char_idx, vm))
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
        fn call(zelf: &Py<Self>, (pystr, char_idx): Self::Args, vm: &VirtualMachine) -> PyResult {
            if char_idx < 0 {
                return Err(vm.new_value_error("idx cannot be negative"));
            }
            let char_idx = char_idx as usize;
            let s = pystr.as_str();

            // Calculate byte index from char index (O(char_idx) but only at entry point)
            let byte_idx = if char_idx == 0 {
                0
            } else {
                match s.char_indices().nth(char_idx) {
                    Some((byte_i, _)) => byte_i,
                    None => {
                        // char_idx is beyond the string length
                        return PyIterReturn::StopIteration(Some(vm.ctx.new_int(char_idx).into()))
                            .to_pyresult(vm);
                    }
                }
            };

            zelf.parse(
                pystr.clone(),
                char_idx,
                byte_idx,
                zelf.to_owned().into(),
                vm,
            )
            .and_then(|x| x.to_pyresult(vm))
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
        let wtf8 = s.as_wtf8();

        // Convert char index `end` to byte index
        let byte_idx = if end == 0 {
            0
        } else {
            wtf8.code_point_indices()
                .nth(end)
                .map(|(i, _)| i)
                .ok_or_else(|| {
                    py_decode_error(
                        machinery::DecodeError::new("Unterminated string starting at", end - 1),
                        s.clone(),
                        vm,
                    )
                })?
        };

        let (result, chars_consumed, _bytes_consumed) =
            machinery::scanstring(&wtf8[byte_idx..], end, strict.unwrap_or(true))
                .map_err(|e| py_decode_error(e, s, vm))?;

        Ok((result, end + chars_consumed))
    }
}

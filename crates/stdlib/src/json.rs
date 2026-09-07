pub(crate) use _json::module_def;

#[pymodule]
mod _json {
    use crate::vm::{
        AsObject, Py, PyObjectRef, PyPayload, PyResult, VirtualMachine,
        builtins::{
            PyBaseExceptionRef, PyDict, PyFloat, PyInt, PyList, PyStr, PyStrRef, PyTuple, PyType,
        },
        convert::ToPyResult,
        function::{IntoFuncArgs, OptionalArg},
        protocol::PyIterReturn,
        types::{Callable, Constructor, PyComparisonOp},
    };
    use core::cell::RefCell;
    use core::str::FromStr;
    use malachite_bigint::BigInt;
    use rustpython_common::{
        json,
        wtf8::{Wtf8, Wtf8Buf},
    };
    use std::collections::{HashMap, HashSet};

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
            let bytes = pystr.as_bytes();
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
                    let (wtf8_result, end_char_idx, _bytes_consumed) =
                        json::scan_string(&wtf8[byte_idx + 1..], char_idx + 1, self.strict)
                            .map_err(|e| py_decode_error(e, pystr.clone(), vm))?;
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

            let rest = &bytes[byte_idx..];

            macro_rules! parse_const {
                ($s:literal, $val:expr) => {
                    if rest.starts_with($s.as_bytes()) {
                        return Ok(PyIterReturn::Return(
                            vm.new_tuple(($val, char_idx + $s.len())).into(),
                        ));
                    }
                };
            }

            parse_const!("null", vm.ctx.none());
            parse_const!("true", true);
            parse_const!("false", false);

            if let Some((res, len)) = self.parse_number(rest, vm) {
                return Ok(PyIterReturn::Return(
                    vm.new_tuple((res?, char_idx + len)).into(),
                ));
            }

            macro_rules! parse_constant {
                ($s:literal) => {
                    if rest.starts_with($s.as_bytes()) {
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

        fn parse_number(&self, bytes: &[u8], vm: &VirtualMachine) -> Option<(PyResult, usize)> {
            flame_guard!("JsonScanner::parse_number");
            // RFC 8259 defines JSON numbers in ASCII syntax, including digits,
            // '-', '.', 'e'/'E', and an optional exponent sign, so byte iteration
            // is equivalent to char iteration here.
            let mut i = 0;
            if bytes.get(i) == Some(&b'-') {
                i += 1;
            }
            match bytes.get(i) {
                Some(b'0') => i += 1,
                Some(b'1'..=b'9') => {
                    i += 1;
                    while matches!(bytes.get(i), Some(b'0'..=b'9')) {
                        i += 1;
                    }
                }
                _ => return None,
            }

            let mut is_float = false;
            if bytes.get(i) == Some(&b'.') && matches!(bytes.get(i + 1), Some(b'0'..=b'9')) {
                is_float = true;
                i += 2;
                while matches!(bytes.get(i), Some(b'0'..=b'9')) {
                    i += 1;
                }
            }

            if matches!(bytes.get(i), Some(b'e' | b'E')) {
                let mut exponent_end = i + 1;
                if matches!(bytes.get(exponent_end), Some(b'+' | b'-')) {
                    exponent_end += 1;
                }
                if matches!(bytes.get(exponent_end), Some(b'0'..=b'9')) {
                    is_float = true;
                    exponent_end += 1;
                    while matches!(bytes.get(exponent_end), Some(b'0'..=b'9')) {
                        exponent_end += 1;
                    }
                    i = exponent_end;
                }
            }

            // SAFETY: the loop above accepts only ASCII bytes, so bytes[..i] is valid UTF-8.
            let buf = unsafe { core::str::from_utf8_unchecked(&bytes[..i]) };
            let ret = if is_float {
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
            memo: &mut HashMap<Wtf8Buf, PyStrRef>,
            vm: &VirtualMachine,
        ) -> PyResult<(PyObjectRef, usize, usize)> {
            flame_guard!("JsonScanner::parse_object");

            let bytes = pystr.as_bytes();
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
                let (key_wtf8, end_char_idx, bytes_consumed) =
                    json::scan_string(&wtf8[byte_idx..], char_idx, self.strict)
                        .map_err(|e| py_decode_error(e, pystr.clone(), vm))?;

                char_idx = end_char_idx;
                byte_idx += bytes_consumed;

                // Key memoization - reuse existing key strings.
                // Keyed by Wtf8Buf so lone surrogates in keys (legal per Python str)
                // are preserved; using String here would lossy-collapse surrogates to U+FFFD.
                let key: PyObjectRef = match memo.get(&key_wtf8) {
                    Some(cached) => cached.clone().into(),
                    None => {
                        let py_key = vm.ctx.new_str(key_wtf8.clone());
                        memo.insert(key_wtf8, py_key.clone());
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
            memo: &mut HashMap<Wtf8Buf, PyStrRef>,
            vm: &VirtualMachine,
        ) -> PyResult<(PyObjectRef, usize, usize)> {
            flame_guard!("JsonScanner::parse_array");

            let bytes = pystr.as_bytes();
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
            memo: &mut HashMap<Wtf8Buf, PyStrRef>,
            vm: &VirtualMachine,
        ) -> PyResult<(PyObjectRef, usize, usize)> {
            // Recursion guard: parse_object/parse_array recurse into call_scan_once
            // for each child value. Without this, a deeply-nested input like
            // `'[' * 50000 + ']' * 50000` overflows the native Rust stack and
            // crashes the process with SIGSEGV. Matches CPython's
            // _Py_EnterRecursiveCall in Modules/_json.c.
            vm.with_recursion("while decoding a JSON object from a string", || {
                let bytes = pystr.as_bytes();
                let wtf8 = pystr.as_wtf8();

                let first_byte = match bytes.get(byte_idx) {
                    Some(&b) => b,
                    None => {
                        return Err(self.make_decode_error("Expecting value", pystr, char_idx, vm));
                    }
                };

                match first_byte {
                    b'"' => {
                        // String - pass slice starting after the quote.
                        // Feed the Wtf8Buf directly to new_str; routing through
                        // .to_string() here would lossy-collapse surrogates to U+FFFD.
                        let (wtf8_result, end_char_idx, bytes_consumed) =
                            json::scan_string(&wtf8[byte_idx + 1..], char_idx + 1, self.strict)
                                .map_err(|e| py_decode_error(e, pystr.clone(), vm))?;
                        let py_str = vm.ctx.new_str(wtf8_result);
                        Ok((py_str.into(), end_char_idx, byte_idx + 1 + bytes_consumed))
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
                        if let Some((result, len)) = self.parse_number(&bytes[byte_idx..], vm) {
                            return Ok((result?, char_idx + len, byte_idx + len));
                        }
                        Err(self.make_decode_error("Expecting value", pystr, char_idx, vm))
                    }
                    b'0'..=b'9' => {
                        // Positive number - numbers are ASCII so len == bytes
                        if let Some((result, len)) = self.parse_number(&bytes[byte_idx..], vm) {
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
                                let end_byte_idx = wtf8
                                    .code_point_indices()
                                    .nth(end_char_idx as usize)
                                    .map_or(wtf8.len(), |(i, _)| i);
                                Ok((value, end_char_idx as usize, end_byte_idx))
                            }
                            Err(err) if err.fast_isinstance(vm.ctx.exceptions.stop_iteration) => {
                                Err(self.make_decode_error("Expecting value", pystr, char_idx, vm))
                            }
                            Err(err) => Err(err),
                        }
                    }
                }
            })
        }

        /// Create a decode error.
        fn make_decode_error(
            &self,
            msg: &str,
            s: PyStrRef,
            pos: usize,
            vm: &VirtualMachine,
        ) -> PyBaseExceptionRef {
            let err = json::DecodeError {
                msg: msg.to_owned(),
                pos,
            };
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
            let wtf8 = pystr.as_wtf8();

            // Calculate byte index from char index (O(char_idx) but only at entry point)
            let byte_idx = if char_idx == 0 {
                0
            } else {
                match wtf8.code_point_indices().nth(char_idx) {
                    Some((byte_i, _)) => byte_i,
                    None => {
                        // char_idx is beyond the string length
                        return PyIterReturn::StopIteration(Some(vm.ctx.new_int(char_idx).into()))
                            .to_pyresult(vm);
                    }
                }
            };

            zelf.parse(pystr, char_idx, byte_idx, zelf.to_owned().into(), vm)
                .and_then(|x| x.to_pyresult(vm))
        }
    }

    #[pyfunction]
    fn encode_basestring(s: PyStrRef) -> Wtf8Buf {
        json::encode_string(s.as_wtf8(), false)
    }

    #[pyfunction]
    fn encode_basestring_ascii(s: PyStrRef) -> Wtf8Buf {
        json::encode_string(s.as_wtf8(), true)
    }

    /// Which Rust-native string escaper (if any) the `encoder` callable is
    /// identical to, so we can skip the generic call machinery for the
    /// extremely hot "encode a string" path.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum EncoderKind {
        Ascii,
        Basic,
        Generic,
    }

    /// Cached `(encode_basestring_ascii, encode_basestring)` refs from the
    /// `_json` module, keyed on the module's identity so a reload is
    /// detected and the cache refreshed. This avoids re-importing `_json`
    /// and re-resolving both attributes by name on every `JsonEncoder`
    /// construction (i.e. every `json.dumps` call), which only needs to
    /// identity-compare `encoder` against these two builtins.
    struct BuiltinEscapers {
        module: PyObjectRef,
        ascii_fn: PyObjectRef,
        basic_fn: PyObjectRef,
    }

    thread_local! {
        static BUILTIN_ESCAPERS: RefCell<Option<BuiltinEscapers>> = const { RefCell::new(None) };
    }

    fn builtin_escaper_refs(vm: &VirtualMachine) -> PyResult<(PyObjectRef, PyObjectRef)> {
        let module = vm.import("_json", 0)?;

        let hit = BUILTIN_ESCAPERS.with_borrow(|c| {
            c.as_ref()
                .filter(|c| c.module.is(&module))
                .map(|c| (c.ascii_fn.clone(), c.basic_fn.clone()))
        });
        if let Some(r) = hit {
            return Ok(r);
        }

        let ascii_fn = module.get_attr("encode_basestring_ascii", vm)?;
        let basic_fn = module.get_attr("encode_basestring", vm)?;
        let result = (ascii_fn.clone(), basic_fn.clone());
        BUILTIN_ESCAPERS.with_borrow_mut(|c| {
            *c = Some(BuiltinEscapers {
                module,
                ascii_fn,
                basic_fn,
            })
        });
        Ok(result)
    }

    /// `_json.make_encoder`, mirroring CPython's `Modules/_json.c` `PyEncoderObject`.
    ///
    /// Encodes directly into a single growable buffer instead of building a
    /// generator of chunks, then returns that buffer as a one-element tuple
    /// (matching what `json/encoder.py`'s `encode()` does with the result:
    /// `''.join(chunks)`).
    #[pyattr(name = "make_encoder")]
    #[pyclass(name = "Encoder", traverse)]
    #[derive(Debug, PyPayload)]
    struct JsonEncoder {
        #[pytraverse(skip)]
        check_circular: bool,
        default: PyObjectRef,
        encoder: PyObjectRef,
        #[pytraverse(skip)]
        encoder_kind: EncoderKind,
        #[pytraverse(skip)]
        indent: Option<Wtf8Buf>,
        key_separator: PyStrRef,
        item_separator: PyStrRef,
        #[pytraverse(skip)]
        sort_keys: bool,
        #[pytraverse(skip)]
        skipkeys: bool,
        #[pytraverse(skip)]
        allow_nan: bool,
        #[pytraverse(skip)]
        markers: crate::vm::common::lock::PyMutex<HashSet<usize>>,
    }

    impl Constructor for JsonEncoder {
        // markers, default, encoder, indent, key_separator, item_separator,
        // sort_keys, skipkeys, allow_nan -- exactly as `encoder.py` calls
        // `c_make_encoder`.
        // Nested tuple: `FromArgs` is only implemented up to 8-tuples, and
        // nested tuples count as a single top-level slot while still
        // consuming their own positional arguments in order.
        type Args = (
            PyObjectRef,
            PyObjectRef,
            PyObjectRef,
            PyObjectRef,
            PyStrRef,
            PyStrRef,
            (PyObjectRef, PyObjectRef, PyObjectRef),
        );

        fn py_new(
            _cls: &Py<PyType>,
            (
                markers,
                default,
                encoder,
                indent,
                key_separator,
                item_separator,
                (sort_keys, skipkeys, allow_nan),
            ): Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult<Self> {
            let check_circular = if vm.is_none(&markers) {
                false
            } else if markers.downcast_ref::<PyDict>().is_some() {
                true
            } else {
                return Err(vm.new_type_error(format!(
                    "make_encoder() argument 1 must be dict or None, not {}",
                    markers.class().name()
                )));
            };

            let indent = if vm.is_none(&indent) {
                None
            } else if let Some(s) = indent.downcast_ref::<PyStr>() {
                Some(s.as_wtf8().to_owned())
            } else {
                return Err(vm.new_type_error(format!(
                    "make_encoder() argument 4 must be str or None, not {}",
                    indent.class().name()
                )));
            };

            let sort_keys = sort_keys.try_to_bool(vm)?;
            let skipkeys = skipkeys.try_to_bool(vm)?;
            let allow_nan = allow_nan.try_to_bool(vm)?;

            // Detect (by identity) whether `encoder` is one of our own
            // builtin string escapers, so the hot string-encoding path can
            // call straight into Rust instead of going through a generic
            // Python call. The two builtin refs are cached (see
            // `builtin_escaper_refs`) so this doesn't re-import `_json` and
            // re-resolve both attributes on every encoder construction
            // (i.e. every `json.dumps` call).
            let encoder_kind = {
                let (ascii_fn, basic_fn) = builtin_escaper_refs(vm)?;
                if encoder.is(&ascii_fn) {
                    EncoderKind::Ascii
                } else if encoder.is(&basic_fn) {
                    EncoderKind::Basic
                } else {
                    EncoderKind::Generic
                }
            };

            Ok(Self {
                check_circular,
                default,
                encoder,
                encoder_kind,
                indent,
                key_separator,
                item_separator,
                sort_keys,
                skipkeys,
                allow_nan,
                markers: crate::vm::common::lock::PyMutex::new(HashSet::new()),
            })
        }
    }

    /// Attach a note to an exception (mirroring `BaseException.add_note`, as
    /// used all over `_make_iterencode` to report which container/item was
    /// being serialized when an error occurred), then return it unchanged so
    /// it can be re-raised.
    fn add_note(exc: PyBaseExceptionRef, note: Wtf8Buf, vm: &VirtualMachine) -> PyBaseExceptionRef {
        // `PyBaseException::add_note` is `pub`, so call it directly instead
        // of going through the generic attribute-lookup + call machinery.
        let _ = exc.clone().add_note(vm.ctx.new_str(note), vm);
        exc
    }

    #[pyclass(with(Callable, Constructor))]
    impl JsonEncoder {
        /// Push `obj`'s id onto the recursion-detection set (a stand-in for
        /// CPython's `markers` dict -- nothing outside this call ever reads
        /// that dict back, so we don't need to actually mutate a real
        /// Python dict object). Returns the id to later pass to
        /// `pop_marker`, or `None` if circular-reference checking is
        /// disabled.
        fn push_marker(&self, obj: &PyObjectRef, vm: &VirtualMachine) -> PyResult<Option<usize>> {
            if !self.check_circular {
                return Ok(None);
            }
            let id = obj.get_id();
            if !self.markers.lock().insert(id) {
                return Err(vm.new_value_error("Circular reference detected"));
            }
            Ok(Some(id))
        }

        fn pop_marker(&self, marker: Option<usize>) {
            if let Some(id) = marker {
                self.markers.lock().remove(&id);
            }
        }

        /// `floatstr`: format a float per JSON rules, honouring `allow_nan`.
        fn float_repr(&self, value: f64, vm: &VirtualMachine) -> PyResult<String> {
            let special = if value.is_nan() {
                Some("NaN")
            } else if value.is_infinite() {
                Some(if value > 0.0 { "Infinity" } else { "-Infinity" })
            } else {
                None
            };
            match special {
                None => Ok(crate::vm::literal::float::to_string(value)),
                Some(text) => {
                    if !self.allow_nan {
                        return Err(vm.new_value_error(format!(
                            "Out of range float values are not JSON compliant: {}",
                            crate::vm::literal::float::to_string(value)
                        )));
                    }
                    Ok(text.to_owned())
                }
            }
        }

        /// Call `self.encoder` (or the Rust-native fast path) and write the
        /// resulting escaped/quoted string into `out`.
        fn write_via_encoder(
            &self,
            arg: PyObjectRef,
            out: &mut Wtf8Buf,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let result = self.encoder.call((arg,), vm)?;
            let s = result.downcast::<PyStr>().map_err(|o| {
                vm.new_type_error(format!(
                    "encoder() must return a str, not {}",
                    o.class().name()
                ))
            })?;
            out.push_wtf8(s.as_wtf8());
            Ok(())
        }

        fn write_text(&self, text: &Wtf8, out: &mut Wtf8Buf, vm: &VirtualMachine) -> PyResult<()> {
            match self.encoder_kind {
                EncoderKind::Ascii => {
                    out.push_wtf8(&json::encode_string(text, true));
                    Ok(())
                }
                EncoderKind::Basic => {
                    out.push_wtf8(&json::encode_string(text, false));
                    Ok(())
                }
                EncoderKind::Generic => {
                    let s = vm.ctx.new_str(text.to_owned());
                    self.write_via_encoder(s.into(), out, vm)
                }
            }
        }

        /// Same as `write_text`, but for an original `str` object -- passed
        /// through to a user-supplied `encoder` as-is (preserving identity
        /// and any subclass) instead of allocating a fresh `str`.
        fn write_str_obj(
            &self,
            obj: &PyObjectRef,
            s: &Py<PyStr>,
            out: &mut Wtf8Buf,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            match self.encoder_kind {
                EncoderKind::Ascii => {
                    out.push_wtf8(&json::encode_string(s.as_wtf8(), true));
                    Ok(())
                }
                EncoderKind::Basic => {
                    out.push_wtf8(&json::encode_string(s.as_wtf8(), false));
                    Ok(())
                }
                EncoderKind::Generic => self.write_via_encoder(obj.clone(), out, vm),
            }
        }

        fn push_newline_indent(&self, out: &mut Wtf8Buf, level: isize) {
            if let Some(indent) = &self.indent {
                out.push_char('\n');
                for _ in 0..level.max(0) {
                    out.push_wtf8(indent);
                }
            }
        }

        /// Compute the (already-escaped-and-quoted) representation of a
        /// dict key. Mirrors CPython's key coercion in
        /// `encoder_encode_key`/`_iterencode_dict`: `str` keys are used
        /// as-is; `float`/bool/`None`/`int` keys are converted to their
        /// JSON text first, then that text is *also* run through the
        /// string encoder (so e.g. `True` becomes `"true"`, quotes
        /// included). Returns `Ok(None)` when the key should be skipped
        /// (`skipkeys=True`).
        fn write_dict_key(
            &self,
            key: &PyObjectRef,
            out: &mut Wtf8Buf,
            vm: &VirtualMachine,
        ) -> PyResult<bool> {
            if let Some(s) = key.downcast_ref::<PyStr>() {
                self.write_str_obj(key, s, out, vm)?;
                return Ok(true);
            }
            if let Some(f) = key.downcast_ref::<PyFloat>() {
                let text = self.float_repr(f.to_f64(), vm)?;
                self.write_text(&Wtf8Buf::from(text), out, vm)?;
                return Ok(true);
            }
            if key.is(&vm.ctx.true_value) {
                self.write_text(Wtf8::new("true"), out, vm)?;
                return Ok(true);
            }
            if key.is(&vm.ctx.false_value) {
                self.write_text(Wtf8::new("false"), out, vm)?;
                return Ok(true);
            }
            if vm.is_none(key) {
                self.write_text(Wtf8::new("null"), out, vm)?;
                return Ok(true);
            }
            if let Some(i) = key.downcast_ref::<PyInt>() {
                self.write_text(&Wtf8Buf::from(i.to_str_radix_10()), out, vm)?;
                return Ok(true);
            }
            if self.skipkeys {
                return Ok(false);
            }
            Err(vm.new_type_error(format!(
                "keys must be str, int, float, bool or None, not {}",
                key.class().name()
            )))
        }

        /// `sorted(dct.items())`: sort dict items by key. Since dict keys
        /// are unique, comparing keys alone reproduces tuple-comparison
        /// semantics (the first differing element always decides).
        fn sort_items(
            &self,
            mut items: Vec<(PyObjectRef, PyObjectRef)>,
            vm: &VirtualMachine,
        ) -> PyResult<Vec<(PyObjectRef, PyObjectRef)>> {
            // Use the VM's fallible timsort (the same one backing
            // `list.sort`/`sorted`) instead of `Vec::sort_by`: a Python-level
            // key comparison (`__lt__`) can raise partway through (e.g.
            // comparing `str` and `int` keys), and `Vec::sort_by`'s
            // comparator is infallible -- swallowing the error and reporting
            // `Ordering::Equal` instead corrupts the sort's invariants and
            // makes Rust's sort implementation panic with "comparison
            // function does not correctly implement a total order",
            // aborting the whole interpreter instead of propagating a
            // catchable Python `TypeError` like CPython does.
            crate::vm::sorting::timsort(&mut items, &mut |a, b| {
                a.0.rich_compare_bool(&b.0, PyComparisonOp::Lt, vm)
            })?;
            Ok(items)
        }

        /// Encode a `list` or `tuple`. `len`/`get` re-read the live
        /// collection on every step (rather than working off a
        /// pre-collected snapshot) so that a `default()`/nested-encoder
        /// callback that shrinks the list mid-encode is observed the same
        /// way CPython's `PySequence_Fast`-free direct indexing observes it
        /// (see `test_encode_mutated`: deleting the last element on every
        /// callback call causes encoding to stop early once the shrinking
        /// list's length catches up with the read cursor).
        fn encode_list(
            &self,
            obj: &PyObjectRef,
            level: isize,
            out: &mut Wtf8Buf,
            vm: &VirtualMachine,
            len: impl Fn() -> usize,
            get: impl Fn(usize) -> PyObjectRef,
        ) -> PyResult<()> {
            if len() == 0 {
                out.push_str("[]");
                return Ok(());
            }
            let marker = self.push_marker(obj, vm)?;
            out.push_char('[');
            let new_level = if self.indent.is_some() {
                level + 1
            } else {
                level
            };
            let mut i = 0;
            while i < len() {
                let value = get(i);
                if i == 0 {
                    self.push_newline_indent(out, new_level);
                } else {
                    out.push_wtf8(self.item_separator.as_wtf8());
                    self.push_newline_indent(out, new_level);
                }
                if let Err(e) = self.encode_value(&value, new_level, out, vm) {
                    self.pop_marker(marker);
                    return Err(add_note(
                        e,
                        Wtf8Buf::from(format!("when serializing {} item {i}", obj.class().name())),
                        vm,
                    ));
                }
                i += 1;
            }
            self.push_newline_indent(out, level);
            out.push_char(']');
            self.pop_marker(marker);
            Ok(())
        }

        /// Fetch `dct.items()` for encoding. For an exact `dict` this reads
        /// the native table directly (matching CPython's fast path); for a
        /// `dict` subclass (e.g. `collections.OrderedDict`) it calls the
        /// real (possibly overridden) `.items()` method instead, so a
        /// subclass's custom ordering/behavior is respected.
        fn dict_items(
            &self,
            obj: &PyObjectRef,
            dict: &PyDict,
            vm: &VirtualMachine,
        ) -> PyResult<Vec<(PyObjectRef, PyObjectRef)>> {
            if obj.class().is(vm.ctx.types.dict_type) {
                return Ok(dict.items_vec());
            }
            let items_obj = obj.get_attr("items", vm)?.call((), vm)?;
            vm.extract_elements_with(&items_obj, |item| {
                use crate::vm::builtins::PyTupleRef;
                let tuple: PyTupleRef = item.try_into_value(vm)?;
                if tuple.len() != 2 {
                    return Err(vm.new_value_error("items() must return 2-tuples"));
                }
                let slice = tuple.as_slice();
                Ok((slice[0].clone(), slice[1].clone()))
            })
        }

        fn encode_dict(
            &self,
            obj: &PyObjectRef,
            dict: &PyDict,
            level: isize,
            out: &mut Wtf8Buf,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let mut items = self.dict_items(obj, dict, vm)?;
            if items.is_empty() {
                out.push_str("{}");
                return Ok(());
            }
            let marker = self.push_marker(obj, vm)?;
            if self.sort_keys {
                items = match self.sort_items(items, vm) {
                    Ok(items) => items,
                    Err(e) => {
                        self.pop_marker(marker);
                        return Err(e);
                    }
                };
            }
            out.push_char('{');
            let new_level = if self.indent.is_some() {
                level + 1
            } else {
                level
            };
            let mut first = true;
            for (key, value) in &items {
                let mut key_out = Wtf8Buf::new();
                let wrote = match self.write_dict_key(key, &mut key_out, vm) {
                    Ok(wrote) => wrote,
                    Err(e) => {
                        self.pop_marker(marker);
                        return Err(e);
                    }
                };
                if !wrote {
                    continue;
                }
                if first {
                    first = false;
                    self.push_newline_indent(out, new_level);
                } else {
                    out.push_wtf8(self.item_separator.as_wtf8());
                    self.push_newline_indent(out, new_level);
                }
                out.push_wtf8(&key_out);
                out.push_wtf8(self.key_separator.as_wtf8());
                if let Err(e) = self.encode_value(value, new_level, out, vm) {
                    self.pop_marker(marker);
                    let mut note =
                        Wtf8Buf::from(format!("when serializing {} item ", obj.class().name()));
                    if let Ok(r) = key.repr(vm) {
                        note.push_wtf8(r.as_wtf8());
                    }
                    return Err(add_note(e, note, vm));
                }
            }
            if !first {
                self.push_newline_indent(out, level);
            }
            out.push_char('}');
            self.pop_marker(marker);
            Ok(())
        }

        fn encode_default(
            &self,
            obj: &PyObjectRef,
            level: isize,
            out: &mut Wtf8Buf,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let marker = self.push_marker(obj, vm)?;
            let new_obj = match self.default.call((obj.clone(),), vm) {
                Ok(v) => v,
                Err(e) => {
                    self.pop_marker(marker);
                    return Err(e);
                }
            };
            let result = self.encode_value(&new_obj, level, out, vm);
            self.pop_marker(marker);
            result.map_err(|e| {
                add_note(
                    e,
                    Wtf8Buf::from(format!("when serializing {} object", obj.class().name())),
                    vm,
                )
            })
        }

        fn encode_value(
            &self,
            obj: &PyObjectRef,
            level: isize,
            out: &mut Wtf8Buf,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            vm.with_recursion("while encoding a JSON object", || {
                if let Some(s) = obj.downcast_ref::<PyStr>() {
                    self.write_str_obj(obj, s, out, vm)
                } else if vm.is_none(obj) {
                    out.push_str("null");
                    Ok(())
                } else if obj.is(&vm.ctx.true_value) {
                    out.push_str("true");
                    Ok(())
                } else if obj.is(&vm.ctx.false_value) {
                    out.push_str("false");
                    Ok(())
                } else if let Some(i) = obj.downcast_ref::<PyInt>() {
                    out.push_str(&i.to_str_radix_10());
                    Ok(())
                } else if let Some(f) = obj.downcast_ref::<PyFloat>() {
                    let text = self.float_repr(f.to_f64(), vm)?;
                    out.push_str(&text);
                    Ok(())
                } else if let Some(list) = obj.downcast_ref::<PyList>() {
                    self.encode_list(
                        obj,
                        level,
                        out,
                        vm,
                        || list.borrow_vec().len(),
                        |i| list.borrow_vec()[i].clone(),
                    )
                } else if let Some(tuple) = obj.downcast_ref::<PyTuple>() {
                    let slice = tuple.as_slice();
                    self.encode_list(obj, level, out, vm, || slice.len(), |i| slice[i].clone())
                } else if let Some(dict) = obj.downcast_ref::<PyDict>() {
                    self.encode_dict(obj, dict, level, out, vm)
                } else {
                    self.encode_default(obj, level, out, vm)
                }
            })
        }
    }

    impl Callable for JsonEncoder {
        type Args = (PyObjectRef, PyObjectRef);

        fn call(zelf: &Py<Self>, (obj, indent_level): Self::Args, vm: &VirtualMachine) -> PyResult {
            let level: isize = indent_level.try_index(vm)?.try_to_primitive(vm)?;
            let mut out = Wtf8Buf::new();
            zelf.encode_value(&obj, level, &mut out, vm)?;
            Ok(vm.new_tuple((out,)).into())
        }
    }

    fn py_decode_error(
        e: json::DecodeError,
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
                        json::DecodeError {
                            msg: "Unterminated string starting at".to_owned(),
                            pos: end - 1,
                        },
                        s.clone(),
                        vm,
                    )
                })?
        };

        let (result, end_char_idx, _bytes_consumed) =
            json::scan_string(&wtf8[byte_idx..], end, strict.unwrap_or(true))
                .map_err(|e| py_decode_error(e, s, vm))?;

        Ok((result, end_char_idx))
    }
}

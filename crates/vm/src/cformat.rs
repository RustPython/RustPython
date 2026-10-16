//cspell:ignore bytesobject

//! Implementation of Printf-Style string formatting
//! as per the [Python Docs](https://docs.python.org/3/library/stdtypes.html#printf-style-string-formatting).

use itertools::Itertools;
use num_traits::cast::ToPrimitive;

use crate::{
    AsObject, PyObject, PyObjectRef, PyResult, TryFromObject, VirtualMachine,
    builtins::{
        PyBaseExceptionRef, PyByteArray, PyBytes, PyFloat, PyInt, PyStr,
        int::check_int_to_str_digits, try_f64_to_bigint, tuple,
    },
    common::{
        cformat::{
            CCharacterType, CConversionFlags, CFormatContext, CFormatConversion, CFormatErrorType,
            CFormatPrecision, CFormatQuantity, CFormatSpec, CFormatType, CNumberType, FormatBuf,
            FormatChar, consume_length, parse_flags, parse_format_type, parse_precision,
            parse_quantity, parse_spec_mapping_key,
        },
        wtf8::{CodePoint, Wtf8, Wtf8Buf},
    },
    function::ArgIntoFloat,
    protocol::{BufferFlags, PyBuffer},
    stdlib::builtins,
};

#[derive(Clone, Copy)]
enum FormatArg<'a> {
    Single,
    Tuple(usize),
    Mapping(&'a PyObject),
}

impl FormatArg<'_> {
    fn message(self, vm: &VirtualMachine, message: String) -> PyResult<String> {
        let context = match self {
            Self::Single => "format argument".to_owned(),
            Self::Tuple(index) => format!("format argument {index}"),
            Self::Mapping(key) => format!("format argument {}", key.repr(vm)?),
        };
        Ok(format!("{context}: {message}"))
    }

    fn type_error(self, vm: &VirtualMachine, message: String) -> PyBaseExceptionRef {
        match self.message(vm, message) {
            Ok(message) => vm.new_type_error(message),
            Err(error) => error,
        }
    }

    fn overflow_error(self, vm: &VirtualMachine, message: String) -> PyBaseExceptionRef {
        match self.message(vm, message) {
            Ok(message) => vm.new_overflow_error(message),
            Err(error) => error,
        }
    }
}

fn format_number_error(
    vm: &VirtualMachine,
    spec: &CFormatSpec,
    obj: &PyObject,
    arg: FormatArg<'_>,
) -> PyBaseExceptionRef {
    let type_name = match obj.class().fully_qualified_name(vm) {
        Ok(name) => name,
        Err(error) => return error,
    };
    let required = match spec.format_type {
        CFormatType::Number(CNumberType::Octal | CNumberType::HexLower | CNumberType::HexUpper) => {
            "an integer"
        }
        _ => "a real number",
    };
    arg.type_error(
        vm,
        format!(
            "%{} requires {required}, not {type_name}",
            spec.format_type.to_char()
        ),
    )
}

fn format_decimal_object(
    vm: &VirtualMachine,
    spec: &CFormatSpec,
    obj: &PyObject,
    arg: FormatArg<'_>,
) -> PyResult<String> {
    let type_error = || format_number_error(vm, spec, obj, arg);
    // Decimal conversions prefer __int__; __index__ is only a fallback when absent.
    let i = obj
        .number()
        .int(vm)
        .or_else(|| obj.try_index_opt(vm))
        .ok_or_else(type_error)?
        .map_err(|error| {
            if error.fast_isinstance(vm.ctx.exceptions.type_error) {
                type_error()
            } else {
                error
            }
        })?;
    check_int_to_str_digits(i.as_bigint(), vm)?;
    Ok(spec.format_number(i.as_bigint()))
}

fn format_index_object(
    vm: &VirtualMachine,
    spec: &CFormatSpec,
    obj: &PyObject,
    arg: FormatArg<'_>,
) -> PyResult<String> {
    let type_error = || format_number_error(vm, spec, obj, arg);
    let value = obj
        .try_index_opt(vm)
        .ok_or_else(type_error)?
        .map_err(|error| {
            if error.fast_isinstance(vm.ctx.exceptions.type_error) {
                type_error()
            } else {
                error
            }
        })?;
    Ok(spec.format_number(value.as_bigint()))
}

fn format_float_object(
    vm: &VirtualMachine,
    spec: &CFormatSpec,
    obj: &PyObject,
    arg: FormatArg<'_>,
) -> PyResult<String> {
    let value = ArgIntoFloat::try_from_object(vm, obj.to_owned()).map_err(|error| {
        if error.fast_isinstance(vm.ctx.exceptions.type_error) {
            format_number_error(vm, spec, obj, arg)
        } else {
            error
        }
    })?;
    Ok(spec.format_float(value.into()))
}

fn format_character_error(
    vm: &VirtualMachine,
    obj: &PyObject,
    arg: FormatArg<'_>,
    context: CFormatContext,
) -> PyBaseExceptionRef {
    let what = match context {
        CFormatContext::Str => obj
            .downcast_ref::<PyStr>()
            .map(|s| format!("a string of length {}", s.char_len())),
        CFormatContext::Bytes => obj
            .downcast_ref::<PyBytes>()
            .map(|b| format!("a bytes object of length {}", b.as_bytes().len()))
            .or_else(|| {
                obj.downcast_ref::<PyByteArray>()
                    .map(|b| format!("a bytearray object of length {}", b.borrow_buf().len()))
            }),
    };
    let what = match what.map_or_else(|| obj.class().fully_qualified_name(vm), Ok) {
        Ok(what) => what,
        Err(error) => return error,
    };
    let required = match context {
        CFormatContext::Str => "an integer or a unicode character",
        CFormatContext::Bytes => "an integer in range(256) or a single byte",
    };
    arg.type_error(vm, format!("%c requires {required}, not {what}"))
}

fn spec_format_bytes(
    vm: &VirtualMachine,
    spec: &CFormatSpec,
    obj: PyObjectRef,
    arg: FormatArg<'_>,
) -> PyResult<Vec<u8>> {
    match &spec.format_type {
        CFormatType::Unsupported { .. } => {
            unreachable!("unsupported format was rejected before conversion")
        }
        // Unlike strings, %r and %a are identical for bytes: the behaviour corresponds to
        // %a for strings (not %r)
        CFormatType::String(CFormatConversion::Repr | CFormatConversion::Ascii) => {
            Ok(spec.format_bytes(builtins::ascii(obj, vm)?.as_bytes()))
        }
        // %b and %s are equivalent for bytes formatting.
        // Mirrors CPython's format_obj() in bytesobject.c
        CFormatType::Bytes | CFormatType::String(CFormatConversion::Str) => {
            if let Some(bytes) = obj.downcast_ref::<PyBytes>() {
                return Ok(spec.format_bytes(bytes.as_bytes()));
            }
            if let Some(bytearray) = obj.downcast_ref::<PyByteArray>() {
                return Ok(spec.format_bytes(&bytearray.borrow_buf()));
            }
            if let Some(method) = vm.get_special_method(&obj, identifier!(vm, __bytes__))? {
                let bytes = method.invoke((), vm)?;
                let bytes = bytes.downcast_ref::<PyBytes>().ok_or_else(|| {
                    match (
                        obj.class().fully_qualified_name(vm),
                        bytes.class().fully_qualified_name(vm),
                    ) {
                        (Ok(source), Ok(result)) => vm.new_type_error(format!(
                            "{source}.__bytes__() must return a bytes, not {result}"
                        )),
                        (Err(error), _) | (_, Err(error)) => error,
                    }
                })?;
                return Ok(spec.format_bytes(bytes.as_bytes()));
            }
            if obj.check_buffer() {
                let buffer = PyBuffer::from_object(vm, &obj, BufferFlags::FULL_RO)?;
                return Ok(buffer.contiguous_or_collect(|bytes| spec.format_bytes(bytes)));
            }
            let msg = format!(
                "%b requires a bytes-like object, or an object that \
                    implements __bytes__, not {}",
                obj.class().fully_qualified_name(vm)?
            );
            Err(arg.type_error(vm, msg))
        }
        CFormatType::Number(number_type) => match number_type {
            CNumberType::DecimalD | CNumberType::DecimalI | CNumberType::DecimalU => {
                if let Some(i) = obj.downcast_ref::<PyInt>() {
                    check_int_to_str_digits(i.as_bigint(), vm)?;
                    Ok(spec.format_number(i.as_bigint()).into_bytes())
                } else if let Some(f) = obj.downcast_ref_if_exact::<PyFloat>(vm) {
                    let bigint = try_f64_to_bigint(f.to_f64(), vm)?;
                    check_int_to_str_digits(&bigint, vm)?;
                    Ok(spec.format_number(&bigint).into_bytes())
                } else {
                    format_decimal_object(vm, spec, &obj, arg).map(String::into_bytes)
                }
            }
            _ => {
                // CPython parity: `%x` / `%o` / `%X` accept any object with
                // `__index__`, not just PyInt. Mirrors PyNumber_Index dispatch.
                if let Some(i) = obj.downcast_ref::<PyInt>() {
                    Ok(spec.format_number(i.as_bigint()).into_bytes())
                } else {
                    format_index_object(vm, spec, &obj, arg).map(String::into_bytes)
                }
            }
        },
        CFormatType::Float(_) => format_float_object(vm, spec, &obj, arg).map(String::into_bytes),
        CFormatType::Character(CCharacterType::Character) => {
            // CPython parity: bytes `%c` accepts a single byte or any object
            // with `__index__` in range(256).
            if let Some(b) = obj.downcast_ref::<PyBytes>() {
                if b.as_bytes().len() == 1 {
                    return Ok(spec.format_char(b.as_bytes()[0]));
                }
                return Err(format_character_error(vm, &obj, arg, CFormatContext::Bytes));
            } else if let Some(ba) = obj.downcast_ref::<PyByteArray>() {
                let buf = ba.borrow_buf();
                if buf.len() == 1 {
                    return Ok(spec.format_char(buf[0]));
                }
                drop(buf);
                return Err(format_character_error(vm, &obj, arg, CFormatContext::Bytes));
            }
            let int = if let Some(i) = obj.downcast_ref::<PyInt>() {
                i.to_owned()
            } else if let Some(int_result) = obj.try_index_opt(vm) {
                int_result?
            } else {
                return Err(format_character_error(vm, &obj, arg, CFormatContext::Bytes));
            };
            let ch = int
                .try_to_primitive::<u8>(vm)
                .map_err(|_| arg.overflow_error(vm, "%c argument not in range(256)".to_owned()))?;
            Ok(spec.format_char(ch))
        }
    }
}

fn spec_format_string(
    vm: &VirtualMachine,
    spec: &CFormatSpec,
    obj: PyObjectRef,
    arg: FormatArg<'_>,
) -> PyResult<Wtf8Buf> {
    match &spec.format_type {
        CFormatType::Unsupported { .. } => {
            unreachable!("unsupported format was rejected before conversion")
        }
        CFormatType::String(conversion) => {
            let result = match conversion {
                CFormatConversion::Ascii => builtins::ascii(obj, vm)?.as_wtf8().to_owned(),
                CFormatConversion::Str => obj.str(vm)?.as_wtf8().to_owned(),
                CFormatConversion::Repr => obj.repr(vm)?.as_wtf8().to_owned(),
            };
            Ok(spec.format_string(result))
        }
        CFormatType::Bytes => {
            // 'b' is rejected at parse time in Str context, see `CFormatContext`.
            unreachable!("%b cannot be parsed in a str format string")
        }
        CFormatType::Number(number_type) => match number_type {
            CNumberType::DecimalD | CNumberType::DecimalI | CNumberType::DecimalU => {
                if let Some(i) = obj.downcast_ref::<PyInt>() {
                    check_int_to_str_digits(i.as_bigint(), vm)?;
                    Ok(spec.format_number(i.as_bigint()).into())
                } else if let Some(f) = obj.downcast_ref_if_exact::<PyFloat>(vm) {
                    let bigint = try_f64_to_bigint(f.to_f64(), vm)?;
                    check_int_to_str_digits(&bigint, vm)?;
                    Ok(spec.format_number(&bigint).into())
                } else {
                    format_decimal_object(vm, spec, &obj, arg).map(Into::into)
                }
            }
            _ => {
                // CPython parity: `%x` / `%o` / `%X` accept any object with
                // `__index__`, not just PyInt. Mirrors PyNumber_Index dispatch.
                if let Some(i) = obj.downcast_ref::<PyInt>() {
                    Ok(spec.format_number(i.as_bigint()).into())
                } else {
                    format_index_object(vm, spec, &obj, arg).map(Into::into)
                }
            }
        },
        CFormatType::Float(_) => format_float_object(vm, spec, &obj, arg).map(Into::into),
        CFormatType::Character(CCharacterType::Character) => {
            // CPython parity: `%c` accepts a single-char str or any object with
            // `__index__` (the latter via PyNumber_Index dispatch).
            if let Some(s) = obj.downcast_ref::<PyStr>() {
                if let Ok(ch) = s.as_wtf8().code_points().exactly_one() {
                    return Ok(spec.format_char(ch));
                }
                return Err(format_character_error(vm, &obj, arg, CFormatContext::Str));
            }
            let int = if let Some(i) = obj.downcast_ref::<PyInt>() {
                i.to_owned()
            } else if let Some(int_result) = obj.try_index_opt(vm) {
                int_result.map_err(|error| {
                    if error.fast_isinstance(vm.ctx.exceptions.type_error) {
                        format_character_error(vm, &obj, arg, CFormatContext::Str)
                    } else {
                        error
                    }
                })?
            } else {
                return Err(format_character_error(vm, &obj, arg, CFormatContext::Str));
            };
            let ch = int
                .as_bigint()
                .to_u32()
                .and_then(CodePoint::from_u32)
                .ok_or_else(|| {
                    arg.overflow_error(vm, "%c argument not in range(0x110000)".to_owned())
                })?;
            Ok(spec.format_char(ch))
        }
    }
}

struct FormatValues<'a> {
    object: &'a PyObject,
    tuple: Option<&'a [PyObjectRef]>,
    consumed: usize,
}

impl<'a> FormatValues<'a> {
    fn new(object: &'a PyObject) -> Self {
        Self {
            object,
            tuple: object
                .downcast_ref::<tuple::PyTuple>()
                .map(|tuple| tuple.as_slice()),
            consumed: 0,
        }
    }

    fn count(&self) -> usize {
        self.tuple.map_or(1, <[PyObjectRef]>::len)
    }

    fn next(
        &mut self,
        allow_single: bool,
        vm: &VirtualMachine,
    ) -> PyResult<(PyObjectRef, FormatArg<'static>)> {
        let value = if let Some(tuple) = self.tuple {
            tuple.get(self.consumed).cloned()
        } else if allow_single && self.consumed == 0 {
            Some(self.object.to_owned())
        } else {
            None
        };
        let value = value.ok_or_else(|| {
            vm.new_type_error(format!(
                "not enough arguments for format string (got {})",
                self.count()
            ))
        })?;
        self.consumed += 1;
        let arg = if self.tuple.is_some() {
            FormatArg::Tuple(self.consumed)
        } else {
            FormatArg::Single
        };
        Ok((value, arg))
    }

    fn star(&mut self, precision: bool, vm: &VirtualMachine) -> PyResult<isize> {
        let (value, arg) = self.next(false, vm)?;
        let Some(value_int) = value.downcast_ref::<PyInt>() else {
            return Err(arg.type_error(
                vm,
                format!(
                    "* requires int, not {}",
                    value.class().fully_qualified_name(vm)?
                ),
            ));
        };
        let converted = if precision {
            value_int.as_bigint().to_i32().map(|value| value as isize)
        } else {
            value_int.as_bigint().to_isize()
        };
        converted.ok_or_else(|| {
            arg.overflow_error(
                vm,
                format!(
                    "too big for {}",
                    if precision { "precision" } else { "width" }
                ),
            )
        })
    }
}

fn parse_error(vm: &VirtualMachine, error: CFormatErrorType, start: usize) -> PyBaseExceptionRef {
    let message = match error {
        CFormatErrorType::UnmatchedKeyParentheses => {
            format!("stray % or incomplete format key at position {start}")
        }
        CFormatErrorType::WidthTooBig => format!("width too big at position {start}"),
        CFormatErrorType::PrecisionTooBig => format!("precision too big at position {start}"),
        _ => format!("stray % at position {start}"),
    };
    vm.new_value_error(message)
}

fn unsupported_format_error(
    vm: &VirtualMachine,
    ch: CodePoint,
    start: usize,
    index: usize,
    context: CFormatContext,
) -> PyBaseExceptionRef {
    let value = ch.to_u32();
    let c = ch.to_char_lossy();
    if c.is_ascii_alphabetic() {
        return vm.new_value_error(format!("unsupported format %{c} at position {start}"));
    }
    let character = if c == '\'' {
        "\"'\"".to_owned()
    } else if (32..127).contains(&value) {
        format!("'{c}'")
    } else if context == CFormatContext::Bytes {
        format!("with code 0x{value:02x}")
    } else if ch
        .to_char()
        .is_some_and(rustpython_unicode::classify::is_printable)
    {
        format!("'{c}' (U+{value:04X})")
    } else {
        format!("U+{value:04X}")
    };
    vm.new_value_error(format!(
        "stray % at position {start} or unexpected format character {character} at position {index}"
    ))
}

trait FormatOutput: FormatBuf {
    fn reserve_output(&mut self, additional: usize, vm: &VirtualMachine) -> PyResult<()>;
    fn push_output(&mut self, character: Self::Char, vm: &VirtualMachine) -> PyResult<()>;
}

impl FormatOutput for Vec<u8> {
    fn reserve_output(&mut self, additional: usize, vm: &VirtualMachine) -> PyResult<()> {
        self.try_reserve_exact(additional)
            .map_err(|_| vm.new_memory_error(""))
    }

    fn push_output(&mut self, character: u8, vm: &VirtualMachine) -> PyResult<()> {
        self.try_reserve(1).map_err(|_| vm.new_memory_error(""))?;
        self.push(character);
        Ok(())
    }
}

impl FormatOutput for Wtf8Buf {
    fn reserve_output(&mut self, additional: usize, vm: &VirtualMachine) -> PyResult<()> {
        self.try_reserve_exact(additional)
            .map_err(|_| vm.new_memory_error(""))
    }

    fn push_output(&mut self, character: CodePoint, vm: &VirtualMachine) -> PyResult<()> {
        self.try_reserve(character.len_wtf8())
            .map_err(|_| vm.new_memory_error(""))?;
        self.push(character);
        Ok(())
    }
}

fn pad_output<S: FormatOutput>(
    vm: &VirtualMachine,
    spec: &CFormatSpec,
    mut value: S,
) -> PyResult<S> {
    let Some(CFormatQuantity::Amount(width)) = spec.min_field_width else {
        return Ok(value);
    };
    let fill = width.saturating_sub(value.chars().count());
    if fill == 0 {
        return Ok(value);
    }
    if spec.flags.contains(CConversionFlags::LEFT_ADJUST) {
        value.reserve_output(fill, vm)?;
        value.extend(core::iter::repeat_n(S::Char::from(b' '), fill));
        return Ok(value);
    }
    let zero_pad = spec.flags.contains(CConversionFlags::ZERO_PAD)
        && matches!(
            spec.format_type,
            CFormatType::Number(_) | CFormatType::Float(_)
        );
    let prefix = if zero_pad {
        let sign = usize::from(
            value
                .chars()
                .next()
                .is_some_and(|c| c.eq_char('-') || c.eq_char('+') || c.eq_char(' ')),
        );
        let alternate = spec.flags.contains(CConversionFlags::ALTERNATE_FORM)
            && matches!(
                spec.format_type,
                CFormatType::Number(
                    CNumberType::Octal | CNumberType::HexLower | CNumberType::HexUpper
                )
            );
        sign + if alternate { 2 } else { 0 }
    } else {
        0
    };
    let size = value
        .len()
        .checked_add(fill)
        .ok_or_else(|| vm.new_memory_error(""))?;
    let mut output = S::default();
    output.reserve_output(size, vm)?;
    output.extend(value.chars().take(prefix));
    output.extend(core::iter::repeat_n(
        S::Char::from(if zero_pad { b'0' } else { b' ' }),
        fill,
    ));
    output.extend(value.chars().skip(prefix));
    Ok(output)
}

fn cformat<S: FormatOutput>(
    vm: &VirtualMachine,
    characters: impl Iterator<Item = S::Char>,
    values_obj: &PyObject,
    context: CFormatContext,
    make_key: impl Fn(S) -> PyObjectRef,
    format_value: impl Fn(&VirtualMachine, &CFormatSpec, PyObjectRef, FormatArg<'_>) -> PyResult<S>,
) -> PyResult<S> {
    let mut characters = characters.enumerate().peekable();
    let mut values = FormatValues::new(values_obj);
    let is_mapping = values_obj.class().slots().as_mapping.has_subscript()
        && values.tuple.is_none()
        && !values_obj.fast_isinstance(vm.ctx.types.str_type)
        && (context == CFormatContext::Str
            || (!values_obj.fast_isinstance(vm.ctx.types.bytes_type)
                && !values_obj.fast_isinstance(vm.ctx.types.bytearray_type)));
    let mut mapping_used = false;
    let mut mapped_value = None;
    let mut result = S::default();

    while let Some((start, character)) = characters.next() {
        if !character.eq_char('%') {
            result.push_output(character, vm)?;
            continue;
        }
        if characters
            .next_if(|(_, character)| character.eq_char('%'))
            .is_some()
        {
            result.push_output(S::Char::from(b'%'), vm)?;
            continue;
        }

        // Fetch mappings and star arguments as their fields are parsed, so
        // callbacks and earlier errors precede later malformed fields.
        let keyed = characters
            .peek()
            .is_some_and(|(_, character)| character.eq_char('('));
        if keyed && !is_mapping {
            return Err(vm.new_type_error(format!(
                "format requires a mapping, not {}",
                values_obj.class().fully_qualified_name(vm)?
            )));
        }
        if !keyed && mapping_used {
            return Err(vm.new_value_error(format!(
                "format requires a parenthesised mapping key at position {start}"
            )));
        }
        let mapping_key = parse_spec_mapping_key::<S, _>(&mut characters)
            .map_err(|(error, _)| parse_error(vm, error, start))?
            .map(&make_key);
        if let Some(key) = &mapping_key {
            mapping_used = true;
            // Release the previous mapped argument before invoking the next lookup.
            drop(mapped_value.take());
            mapped_value = Some(values_obj.get_item(key.as_object(), vm)?);
        }

        let mut flags = parse_flags(&mut characters);
        let mut min_field_width = parse_quantity(
            &mut characters,
            isize::MAX as usize,
            CFormatErrorType::WidthTooBig,
        )
        .map_err(|(error, _)| parse_error(vm, error, start))?;
        let star_mapping_error = || {
            vm.new_value_error(format!(
                "* cannot be used with a parenthesised mapping key at position {start}"
            ))
        };
        if min_field_width == Some(CFormatQuantity::FromValuesTuple) {
            if mapping_used {
                return Err(star_mapping_error());
            }
            let width = values.star(false, vm)?;
            if width < 0 {
                flags.insert(CConversionFlags::LEFT_ADJUST);
            }
            // Negating PY_SSIZE_T_MIN leaves a negative width in CPython,
            // which imposes no padding requirement.
            min_field_width = Some(CFormatQuantity::Amount(
                width.checked_abs().unwrap_or(0) as usize
            ));
        }
        let mut precision =
            parse_precision(&mut characters).map_err(|(error, _)| parse_error(vm, error, start))?;
        if precision == Some(CFormatPrecision::Quantity(CFormatQuantity::FromValuesTuple)) {
            if mapping_used {
                return Err(star_mapping_error());
            }
            precision = Some(CFormatPrecision::Quantity(CFormatQuantity::Amount(
                values.star(true, vm)?.max(0) as usize,
            )));
        }
        consume_length(&mut characters);
        let format_type = parse_format_type(&mut characters, context)
            .map_err(|(error, _)| parse_error(vm, error, start))?;
        let spec = CFormatSpec {
            flags,
            min_field_width,
            precision,
            format_type,
        };
        let (value, arg) = if let Some(key) = &mapping_key {
            (
                mapped_value.as_ref().unwrap().clone(),
                FormatArg::Mapping(key),
            )
        } else {
            values.next(true, vm)?
        };
        if let CFormatType::Unsupported { ch, index } = format_type {
            return Err(unsupported_format_error(vm, ch, start, index, context));
        }
        // Convert before reserving field padding: user callbacks must still
        // run when an otherwise valid width cannot be allocated.
        let mut unpadded_spec = spec;
        unpadded_spec.min_field_width = None;
        let value = format_value(vm, &unpadded_spec, value, arg)?;
        let value = pad_output(vm, &spec, value)?;
        result.reserve_output(value.len(), vm)?;
        result = result.concat(value);
    }

    if !is_mapping && values.consumed < values.count() {
        let kind = match context {
            CFormatContext::Str => "string",
            CFormatContext::Bytes => "bytes",
        };
        return Err(vm.new_type_error(format!(
            "not all arguments converted during {kind} formatting (required {}, got {})",
            values.consumed,
            values.count()
        )));
    }
    Ok(result)
}

pub(crate) fn cformat_bytes(
    vm: &VirtualMachine,
    format_string: &[u8],
    values_obj: &PyObject,
) -> PyResult<Vec<u8>> {
    cformat(
        vm,
        format_string.iter().copied(),
        values_obj,
        CFormatContext::Bytes,
        |key| vm.ctx.new_bytes(key).into(),
        spec_format_bytes,
    )
}

pub(crate) fn cformat_string(
    vm: &VirtualMachine,
    format_string: &Wtf8,
    values_obj: &PyObject,
) -> PyResult<Wtf8Buf> {
    cformat(
        vm,
        format_string.code_points(),
        values_obj,
        CFormatContext::Str,
        |key| vm.ctx.new_str(key).into(),
        spec_format_string,
    )
}

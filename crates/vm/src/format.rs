use crate::{
    PyObject, PyResult, VirtualMachine, builtins::PyBaseExceptionRef, convert::ToPyException,
    function::FuncArgs, stdlib::builtins,
};

use crate::common::format::*;
use crate::common::wtf8::{Wtf8, Wtf8Buf};

/// Get locale information from C `localeconv()` for the 'n' format specifier.
#[cfg(any(unix, windows))]
pub(crate) fn get_locale_info() -> LocaleInfo {
    let lc = crate::host_env::locale::localeconv_data();
    #[allow(
        clippy::unnecessary_cast,
        reason = "libc::c_char is not u8 on all platforms"
    )]
    let mut grouping = lc.grouping.iter().map(|&c| c as u8).collect::<Vec<u8>>();
    if !grouping.is_empty() {
        grouping.push(0);
    }
    LocaleInfo {
        thousands_sep: String::from_utf8_lossy(&lc.thousands_sep).into_owned(),
        decimal_point: if lc.decimal_point.is_empty() {
            ".".to_string()
        } else {
            String::from_utf8_lossy(&lc.decimal_point).into_owned()
        },
        grouping,
    }
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn get_locale_info() -> LocaleInfo {
    LocaleInfo {
        thousands_sep: String::new(),
        decimal_point: ".".to_string(),
        grouping: vec![],
    }
}

pub(crate) fn parse_format_spec(
    object: &PyObject,
    spec: &str,
    vm: &VirtualMachine,
) -> PyResult<FormatSpec> {
    FormatSpec::parse(spec).map_err(|err| format_spec_error(err, object, spec, vm))
}

pub(crate) fn format_spec_error(
    err: FormatSpecError,
    object: &PyObject,
    spec: &str,
    vm: &VirtualMachine,
) -> PyBaseExceptionRef {
    match err {
        FormatSpecError::InvalidFormatSpecifier => vm.new_value_error(format!(
            "Invalid format specifier '{spec}' for object of type '{}'",
            object.class().name()
        )),
        FormatSpecError::DecimalDigitsTooMany => {
            vm.new_value_error("Too many decimal digits in format string")
        }
        FormatSpecError::PrecisionTooBig => vm.new_value_error("Precision too big"),
        FormatSpecError::PrecisionMissing => {
            vm.new_value_error("Format specifier missing precision")
        }
        FormatSpecError::UnspecifiedFormat(c1, c2) => {
            vm.new_value_error(format!("Cannot specify '{c1}' with '{c2}'."))
        }
        FormatSpecError::ExclusiveFormat(c1, c2) => {
            vm.new_value_error(format!("Cannot specify both '{c1}' and '{c2}'."))
        }
        FormatSpecError::UnknownFormatCode(c, s) => vm.new_value_error(format!(
            "Unknown format code '{c}' for object of type '{s}'"
        )),
        FormatSpecError::PrecisionNotAllowed => {
            vm.new_value_error("Precision not allowed in integer format specifier")
        }
        FormatSpecError::NotAllowed(s) => {
            vm.new_value_error(format!("{s} not allowed with integer format specifier 'c'"))
        }
        FormatSpecError::UnableToConvert => vm.new_value_error("Unable to convert int to float"),
        FormatSpecError::CodeNotInRange => vm.new_overflow_error("%c arg not in range(0x110000)"),
        FormatSpecError::IntTooLargeForCLong => {
            vm.new_overflow_error("Python int too large to convert to C long")
        }
        FormatSpecError::ZeroPadding => {
            vm.new_value_error("Zero padding is not allowed in complex format specifier")
        }
        FormatSpecError::AlignmentFlag => {
            vm.new_value_error("'=' alignment flag is not allowed in complex format specifier")
        }
        FormatSpecError::NegativeZeroCoercionNotAllowed(type_name) => vm.new_value_error(format!(
            "Negative zero coercion (z) not allowed in {type_name} format specifier"
        )),
        FormatSpecError::StringAlignmentFlag => {
            vm.new_value_error("'=' alignment not allowed in string format specifier")
        }
        FormatSpecError::StringSpecNotAllowed(s) => {
            vm.new_value_error(format!("{s} not allowed in string format specifier"))
        }
        FormatSpecError::NotImplemented(c, s) => vm.new_value_error(format!(
            "Format code '{c}' for object of type '{s}' not implemented yet"
        )),
    }
}

impl ToPyException for FormatParseError {
    fn to_pyexception(&self, vm: &VirtualMachine) -> PyBaseExceptionRef {
        // Matched exhaustively on purpose: a catch-all sent most of these to
        // "Unexpected error parsing format string", which tells the reader nothing
        // about the format string they mistyped.
        match self {
            Self::UnmatchedBracket | Self::MissingRightBracket => {
                vm.new_value_error("expected '}' before end of string")
            }
            Self::MissingRightSquareBracket => vm.new_value_error("Missing ']' in format string"),
            Self::MissingStartBracket => {
                vm.new_value_error("Single '}' encountered in format string")
            }
            // A brace in literal text that was not doubled, i.e. a stray `{` with
            // nothing after it. `parse_spec` reports this rather than
            // UnmatchedBracket so that "{" and "a{" are separated from "{0" and
            // "a{b", which are fields left open.
            Self::UnescapedStartBracketInLiteral => {
                vm.new_value_error("Single '{' encountered in format string")
            }
            Self::InvalidFormatSpecifier => vm.new_value_error("unmatched '{' in format spec"),
            Self::MissingConversion => {
                vm.new_value_error("end of string while looking for conversion specifier")
            }
            Self::MissingConversionColon => {
                vm.new_value_error("expected ':' after conversion specifier")
            }
            Self::UnexpectedStartBracket => vm.new_value_error("unexpected '{' in field name"),
            Self::EmptyAttribute => vm.new_value_error("Empty attribute in format string"),
            Self::InvalidCharacterAfterRightBracket => {
                vm.new_value_error("Only '.' or '[' may follow ']' in format field specifier")
            }
            Self::TooManyDecimalDigits => {
                vm.new_value_error("Too many decimal digits in format string")
            }
        }
    }
}

fn format_internal(
    vm: &VirtualMachine,
    format: &FormatString,
    recursion_depth: usize,
    field_func: &mut impl FnMut(FieldType) -> PyResult,
) -> PyResult<Wtf8Buf> {
    let mut final_string = Wtf8Buf::new();
    for part in &format.format_parts {
        let pystr;
        let result_string: &Wtf8 = match part {
            FormatPart::Field {
                field_name,
                conversion_spec,
                format_spec,
            } => {
                let (field_type, parts) =
                    FieldName::split(field_name).map_err(|e| e.to_pyexception(vm))?;

                let mut argument = field_func(field_type)?;

                for name_part in parts {
                    match name_part.map_err(|e| e.to_pyexception(vm))? {
                        FieldNamePart::Attribute(attribute) => {
                            argument = argument.get_attr(&vm.ctx.new_str(attribute), vm)?;
                        }
                        FieldNamePart::Index(index) => {
                            argument = argument.get_item(&index, vm)?;
                        }
                        FieldNamePart::StringIndex(index) => {
                            argument = argument.get_item(&index, vm)?;
                        }
                    }
                }

                let argument = match conversion_spec {
                    None => argument,
                    Some(c) => match FormatConversion::from_char(*c) {
                        Some(FormatConversion::Str) => argument.str(vm)?.into(),
                        Some(FormatConversion::Repr) => argument.repr(vm)?.into(),
                        Some(FormatConversion::Ascii) => builtins::ascii(argument, vm)?.into(),
                        None => {
                            return Err(
                                vm.new_value_error(format!("Unknown conversion specifier {c}"))
                            );
                        }
                    },
                };

                let expanded_spec;
                let format_spec = if format_spec.contains("{".as_ref()) {
                    // Only replacement-bearing specs consume the expansion budget.
                    if recursion_depth == 1 {
                        return Err(vm.new_value_error("Max string recursion exceeded"));
                    }
                    let nested_format =
                        FormatString::from_str(format_spec).map_err(|e| e.to_pyexception(vm))?;
                    expanded_spec =
                        format_internal(vm, &nested_format, recursion_depth - 1, field_func)?;
                    &expanded_spec
                } else {
                    format_spec
                };

                // FIXME: compiler can intern specs using parser tree. Then this call can be interned_str
                pystr = vm.format(&argument, vm.ctx.new_str(format_spec.clone()))?;
                pystr.as_wtf8()
            }
            FormatPart::Literal(literal) => literal,
        };
        final_string.push_wtf8(result_string);
    }
    Ok(final_string)
}

pub(crate) fn format(
    format: &FormatString,
    arguments: &FuncArgs,
    vm: &VirtualMachine,
) -> PyResult<Wtf8Buf> {
    let mut auto_argument_index: usize = 0;
    let mut seen_index = false;
    format_internal(vm, format, 2, &mut |field_type| match field_type {
        FieldType::Auto => {
            if seen_index {
                return Err(vm.new_value_error(
                    "cannot switch from manual field specification to automatic field numbering",
                ));
            }
            auto_argument_index += 1;
            let idx = auto_argument_index - 1;
            arguments.args.get(idx).cloned().ok_or_else(|| {
                vm.new_index_error(format!(
                    "Replacement index {idx} out of range for positional args tuple"
                ))
            })
        }
        FieldType::Index(index) => {
            if auto_argument_index != 0 {
                return Err(vm.new_value_error(
                    "cannot switch from automatic field numbering to manual field specification",
                ));
            }
            seen_index = true;
            arguments.args.get(index).cloned().ok_or_else(|| {
                vm.new_index_error(format!(
                    "Replacement index {index} out of range for positional args tuple"
                ))
            })
        }
        FieldType::Keyword(keyword) => keyword
            .as_str()
            .ok()
            .and_then(|keyword| arguments.get_optional_kwarg(keyword))
            .ok_or_else(|| vm.new_key_error(vm.ctx.new_str(keyword).into())),
    })
}

pub(crate) fn format_map(
    format: &FormatString,
    dict: &PyObject,
    vm: &VirtualMachine,
) -> PyResult<Wtf8Buf> {
    format_internal(vm, format, 2, &mut |field_type| match field_type {
        FieldType::Auto | FieldType::Index(_) => {
            Err(vm.new_value_error("Format string contains positional fields"))
        }
        FieldType::Keyword(keyword) => dict.get_item(&keyword, vm),
    })
}

use crate::{
    PyObject, PyResult, VirtualMachine,
    builtins::PyBaseExceptionRef,
    convert::{IntoPyException, ToPyException},
    function::FuncArgs,
    stdlib::builtins,
};

use crate::common::format::*;
use crate::common::wtf8::{Wtf8, Wtf8Buf};

impl IntoPyException for FormatSpecError {
    fn into_pyexception(self, vm: &VirtualMachine) -> PyBaseExceptionRef {
        match self {
            Self::DecimalDigitsTooMany => {
                vm.new_value_error("Too many decimal digits in format string")
            }
            Self::PrecisionTooBig => vm.new_value_error("Precision too big"),
            Self::InvalidFormatSpecifier => vm.new_value_error("Invalid format specifier"),
            Self::UnspecifiedFormat(c1, c2) => {
                let msg = format!("Cannot specify '{c1}' with '{c2}'.");
                vm.new_value_error(msg)
            }
            Self::ExclusiveFormat(c1, c2) => {
                let msg = format!("Cannot specify both '{c1}' and '{c2}'.");
                vm.new_value_error(msg)
            }
            Self::UnknownFormatCode(c, s) => {
                let msg = format!("Unknown format code '{c}' for object of type '{s}'");
                vm.new_value_error(msg)
            }
            Self::PrecisionNotAllowed => {
                vm.new_value_error("Precision not allowed in integer format specifier")
            }
            Self::NotAllowed(s) => {
                let msg = format!("{s} not allowed with integer format specifier 'c'");
                vm.new_value_error(msg)
            }
            Self::UnableToConvert => vm.new_value_error("Unable to convert int to float"),
            Self::CodeNotInRange => vm.new_overflow_error("%c arg not in range(0x110000)"),
            Self::ZeroPadding => {
                vm.new_value_error("Zero padding is not allowed in complex format specifier")
            }
            Self::AlignmentFlag => {
                vm.new_value_error("'=' alignment flag is not allowed in complex format specifier")
            }
            Self::NotImplemented(c, s) => {
                let msg = format!("Format code '{c}' for object of type '{s}' not implemented yet");
                vm.new_value_error(msg)
            }
        }
    }
}

impl ToPyException for FormatParseError {
    fn to_pyexception(&self, vm: &VirtualMachine) -> PyBaseExceptionRef {
        match self {
            Self::UnmatchedBracket => vm.new_value_error("expected '}' before end of string"),
            _ => vm.new_value_error("Unexpected error parsing format string"),
        }
    }
}

fn format_internal(
    vm: &VirtualMachine,
    format: &FormatString,
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
                let FieldName { field_type, parts } =
                    FieldName::parse(field_name).map_err(|e| e.to_pyexception(vm))?;

                let mut argument = field_func(field_type)?;

                for name_part in parts {
                    match name_part {
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

                let nested_format =
                    FormatString::from_str(format_spec).map_err(|e| e.to_pyexception(vm))?;
                let format_spec = format_internal(vm, &nested_format, field_func)?;

                let argument = match conversion_spec.and_then(FormatConversion::from_char) {
                    Some(FormatConversion::Str) => argument.str(vm)?.into(),
                    Some(FormatConversion::Repr) => argument.repr(vm)?.into(),
                    Some(FormatConversion::Ascii) => {
                        vm.ctx.new_str(builtins::ascii(argument, vm)?).into()
                    }
                    Some(FormatConversion::Bytes) => {
                        vm.call_method(&argument, identifier!(vm, decode).as_str(), ())?
                    }
                    None => argument,
                };

                // FIXME: compiler can intern specs using parser tree. Then this call can be interned_str
                pystr = vm.format(&argument, vm.ctx.new_str(format_spec))?;
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
    format_internal(vm, format, &mut |field_type| match field_type {
        FieldType::Auto => {
            if seen_index {
                return Err(vm.new_value_error(
                    "cannot switch from manual field specification to automatic field numbering",
                ));
            }
            auto_argument_index += 1;
            arguments
                .args
                .get(auto_argument_index - 1)
                .cloned()
                .ok_or_else(|| vm.new_index_error("tuple index out of range"))
        }
        FieldType::Index(index) => {
            if auto_argument_index != 0 {
                return Err(vm.new_value_error(
                    "cannot switch from automatic field numbering to manual field specification",
                ));
            }
            seen_index = true;
            arguments
                .args
                .get(index)
                .cloned()
                .ok_or_else(|| vm.new_index_error("tuple index out of range"))
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
    format_internal(vm, format, &mut |field_type| match field_type {
        FieldType::Auto | FieldType::Index(_) => {
            Err(vm.new_value_error("Format string contains positional fields"))
        }
        FieldType::Keyword(keyword) => dict.get_item(&keyword, vm),
    })
}

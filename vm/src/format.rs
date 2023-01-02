use crate::{
    builtins::{PyBaseExceptionRef, PyStrRef},
    common::format::*,
    convert::ToPyException,
    function::FuncArgs,
    stdlib::builtins,
    AsObject, PyObject, PyObjectRef, PyResult, VirtualMachine,
};

impl ToPyException for FormatParseError {
    fn to_pyexception(&self, vm: &VirtualMachine) -> PyBaseExceptionRef {
        match self {
            FormatParseError::UnmatchedBracket => {
                vm.new_value_error("expected '}' before end of string".to_owned())
            }
            _ => vm.new_value_error("Unexpected error parsing format string".to_owned()),
        }
    }
}

fn format_internal(
    vm: &VirtualMachine,
    format: &FormatString,
    field_func: &mut impl FnMut(FieldType) -> PyResult,
) -> PyResult<String> {
    let mut final_string = String::new();
    for part in &format.format_parts {
        let pystr;
        let result_string: &str = match part {
            FormatPart::Field {
                field_name,
                preconversion_spec,
                format_spec,
            } => {
                let FieldName { field_type, parts } =
                    FieldName::parse(field_name.as_str()).map_err(|e| e.to_pyexception(vm))?;

                let mut argument = field_func(field_type)?;

                for name_part in parts {
                    match name_part {
                        FieldNamePart::Attribute(attribute) => {
                            argument = argument.get_attr(attribute.as_str(), vm)?;
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

                pystr = call_object_format(vm, argument, *preconversion_spec, &format_spec)?;
                pystr.as_ref()
            }
            FormatPart::Literal(literal) => literal,
        };
        final_string.push_str(result_string);
    }
    Ok(final_string)
}

pub(crate) fn format(
    format: &FormatString,
    arguments: &FuncArgs,
    vm: &VirtualMachine,
) -> PyResult<String> {
    let mut auto_argument_index: usize = 0;
    let mut seen_index = false;
    format_internal(vm, format, &mut |field_type| match field_type {
        FieldType::Auto => {
            if seen_index {
                return Err(vm.new_value_error(
                    "cannot switch from manual field specification to automatic field numbering"
                        .to_owned(),
                ));
            }
            auto_argument_index += 1;
            arguments
                .args
                .get(auto_argument_index - 1)
                .cloned()
                .ok_or_else(|| vm.new_index_error("tuple index out of range".to_owned()))
        }
        FieldType::Index(index) => {
            if auto_argument_index != 0 {
                return Err(vm.new_value_error(
                    "cannot switch from automatic field numbering to manual field specification"
                        .to_owned(),
                ));
            }
            seen_index = true;
            arguments
                .args
                .get(index)
                .cloned()
                .ok_or_else(|| vm.new_index_error("tuple index out of range".to_owned()))
        }
        FieldType::Keyword(keyword) => arguments
            .get_optional_kwarg(&keyword)
            .ok_or_else(|| vm.new_key_error(vm.ctx.new_str(keyword).into())),
    })
}

pub(crate) fn format_map(
    format: &FormatString,
    dict: &PyObject,
    vm: &VirtualMachine,
) -> PyResult<String> {
    format_internal(vm, format, &mut |field_type| match field_type {
        FieldType::Auto | FieldType::Index(_) => {
            Err(vm.new_value_error("Format string contains positional fields".to_owned()))
        }
        FieldType::Keyword(keyword) => dict.get_item(&keyword, vm),
    })
}

pub fn call_object_format(
    vm: &VirtualMachine,
    argument: PyObjectRef,
    preconversion_spec: Option<char>,
    format_spec: &str,
) -> PyResult<PyStrRef> {
    let argument = match preconversion_spec.and_then(FormatPreconversor::from_char) {
        Some(FormatPreconversor::Str) => argument.str(vm)?.into(),
        Some(FormatPreconversor::Repr) => argument.repr(vm)?.into(),
        Some(FormatPreconversor::Ascii) => vm.ctx.new_str(builtins::ascii(argument, vm)?).into(),
        Some(FormatPreconversor::Bytes) => {
            vm.call_method(&argument, identifier!(vm, decode).as_str(), ())?
        }
        None => argument,
    };
    let result = vm.call_special_method(argument, identifier!(vm, __format__), (format_spec,))?;
    result.downcast().map_err(|result| {
        vm.new_type_error(format!(
            "__format__ must return a str, not {}",
            &result.class().name()
        ))
    })
}

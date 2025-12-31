/* String builtin module
 */

pub(crate) use _string::make_module;

#[pymodule]
mod _string {
    use crate::common::ascii;
    use crate::common::format::{
        FieldName, FieldNamePart, FieldType, FormatPart, FormatString, FromTemplate,
    };
    use crate::common::wtf8::{CodePoint, Wtf8Buf};
    use crate::{
        PyObjectRef, PyResult, VirtualMachine,
        builtins::{PyList, PyStrRef},
        convert::ToPyException,
        convert::ToPyObject,
    };
    use core::mem;

    fn create_format_part(
        literal: Wtf8Buf,
        field_name: Option<Wtf8Buf>,
        format_spec: Option<Wtf8Buf>,
        conversion_spec: Option<CodePoint>,
        vm: &VirtualMachine,
    ) -> PyObjectRef {
        let tuple = (
            literal,
            field_name,
            format_spec,
            conversion_spec.map(|c| c.to_string()),
        );
        tuple.to_pyobject(vm)
    }

    #[pyfunction]
    fn formatter_parser(text: PyStrRef, vm: &VirtualMachine) -> PyResult<PyList> {
        let format_string =
            FormatString::from_str(text.as_wtf8()).map_err(|e| e.to_pyexception(vm))?;

        let mut result: Vec<PyObjectRef> = Vec::new();
        let mut literal = Wtf8Buf::new();
        for part in format_string.format_parts {
            match part {
                FormatPart::Field {
                    field_name,
                    conversion_spec,
                    format_spec,
                } => {
                    result.push(create_format_part(
                        mem::take(&mut literal),
                        Some(field_name),
                        Some(format_spec),
                        conversion_spec,
                        vm,
                    ));
                }
                FormatPart::Literal(text) => literal.push_wtf8(&text),
            }
        }
        if !literal.is_empty() {
            result.push(create_format_part(
                mem::take(&mut literal),
                None,
                None,
                None,
                vm,
            ));
        }
        Ok(result.into())
    }

    #[pyfunction]
    fn formatter_field_name_split(
        text: PyStrRef,
        vm: &VirtualMachine,
    ) -> PyResult<(PyObjectRef, PyList)> {
        let field_name = FieldName::parse(text.as_wtf8()).map_err(|e| e.to_pyexception(vm))?;

        let first = match field_name.field_type {
            FieldType::Auto => vm.ctx.new_str(ascii!("")).into(),
            FieldType::Index(index) => index.to_pyobject(vm),
            FieldType::Keyword(attribute) => attribute.to_pyobject(vm),
        };

        let rest = field_name
            .parts
            .iter()
            .map(|p| match p {
                FieldNamePart::Attribute(attribute) => (true, attribute).to_pyobject(vm),
                FieldNamePart::StringIndex(index) => (false, index).to_pyobject(vm),
                FieldNamePart::Index(index) => (false, *index).to_pyobject(vm),
            })
            .collect();

        Ok((first, rest))
    }
}

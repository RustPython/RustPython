/* String builtin module
 */

pub(crate) use _string::make_module;

#[pymodule]
mod _string {
    use std::mem;

    use crate::builtins::list::PyList;
    use crate::builtins::pystr::PyStrRef;
    use crate::exceptions::IntoPyException;
    use crate::format::{
        FieldName, FieldNamePart, FieldType, FormatPart, FormatString, FromTemplate,
    };
    use crate::pyobject::{BorrowValue, IntoPyObject, PyObjectRef, PyResult};
    use crate::vm::VirtualMachine;

    fn create_format_part(
        literal: String,
        field_name: Option<String>,
        format_spec: Option<String>,
        preconversion_spec: Option<char>,
        vm: &VirtualMachine,
    ) -> PyObjectRef {
        let tuple = (
            literal,
            field_name,
            format_spec,
            preconversion_spec.map(|c| c.to_string()),
        );
        tuple.into_pyobject(vm)
    }

    #[pyfunction]
    fn formatter_parser(text: PyStrRef, vm: &VirtualMachine) -> PyResult<PyList> {
        let format_string =
            FormatString::from_str(text.borrow_value()).map_err(|e| e.into_pyexception(vm))?;

        let mut result = Vec::new();
        let mut literal = String::new();
        for part in format_string.format_parts {
            match part {
                FormatPart::Field {
                    field_name,
                    preconversion_spec,
                    format_spec,
                } => {
                    result.push(create_format_part(
                        mem::take(&mut literal),
                        Some(field_name),
                        Some(format_spec),
                        preconversion_spec,
                        vm,
                    ));
                }
                FormatPart::Literal(text) => literal.push_str(&text),
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
        let field_name =
            FieldName::parse(text.borrow_value()).map_err(|e| e.into_pyexception(vm))?;

        let first = match field_name.field_type {
            FieldType::Auto => vm.ctx.new_str("".to_owned()),
            FieldType::Index(index) => index.into_pyobject(vm),
            FieldType::Keyword(attribute) => attribute.into_pyobject(vm),
        };

        let rest = field_name
            .parts
            .iter()
            .map(|p| match p {
                FieldNamePart::Attribute(attribute) => (true, attribute).into_pyobject(vm),
                FieldNamePart::StringIndex(index) => (false, index).into_pyobject(vm),
                FieldNamePart::Index(index) => (false, *index).into_pyobject(vm),
            })
            .collect();

        Ok((first, rest))
    }
}

/* String builtin module
 */

pub(crate) use _string::module_def;

#[pymodule]
mod _string {
    use crate::common::ascii;
    use crate::common::format::{
        FieldName, FieldNamePart, FieldNameParts, FieldType, FormatPart, FormatString, FromTemplate,
    };
    use crate::common::wtf8::{CodePoint, Wtf8Buf};
    use crate::{
        Py, PyObjectRef, PyPayload, PyResult, VirtualMachine,
        builtins::{PyList, PyStrRef},
        common::lock::PyMutex,
        convert::ToPyException,
        convert::ToPyObject,
        protocol::PyIterReturn,
        types::{IterNext, Iterable, SelfIter},
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
    fn formatter_parser(object: PyStrRef, vm: &VirtualMachine) -> PyResult<PyList> {
        let format_string =
            FormatString::from_str(object.as_wtf8()).map_err(|e| e.to_pyexception(vm))?;

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

    #[pyclass(no_attr, name = "fieldnameiterator", module = "builtins", traverse)]
    #[derive(Debug, PyPayload)]
    struct FieldNameIterator {
        #[pytraverse(skip)]
        parts: PyMutex<FieldNameParts>,
    }

    #[pyclass(with(IterNext, Iterable), flags(DISALLOW_INSTANTIATION))]
    impl FieldNameIterator {}

    impl SelfIter for FieldNameIterator {}

    impl IterNext for FieldNameIterator {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            let part = zelf.parts.lock().next();
            let Some(part) = part else {
                return Ok(PyIterReturn::StopIteration(None));
            };
            let value = match part.map_err(|e| e.to_pyexception(vm))? {
                FieldNamePart::Attribute(attribute) => (true, attribute).to_pyobject(vm),
                FieldNamePart::StringIndex(index) => (false, index).to_pyobject(vm),
                FieldNamePart::Index(index) => (false, index).to_pyobject(vm),
            };
            Ok(PyIterReturn::Return(value))
        }
    }

    #[pyfunction]
    fn formatter_field_name_split(
        object: PyStrRef,
        vm: &VirtualMachine,
    ) -> PyResult<(PyObjectRef, PyObjectRef)> {
        let (field_type, parts) =
            FieldName::split(object.as_wtf8()).map_err(|e| e.to_pyexception(vm))?;

        let first = match field_type {
            FieldType::Auto => vm.ctx.new_str(ascii!("")).into(),
            FieldType::Index(index) => index.to_pyobject(vm),
            FieldType::Keyword(attribute) => attribute.to_pyobject(vm),
        };
        let rest = FieldNameIterator {
            parts: PyMutex::new(parts),
        }
        .into_ref(&vm.ctx);
        Ok((first, rest.into()))
    }
}

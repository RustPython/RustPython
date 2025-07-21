use super::constant::{Constant, ConstantLiteral};
use super::*;

fn ruff_fstring_value_into_iter(
    mut fstring_value: ruff::FStringValue,
) -> impl Iterator<Item = ruff::FStringPart> + 'static {
    let default = ruff::FStringPart::FString(ruff::FString {
        range: Default::default(),
        elements: Default::default(),
        flags: ruff::FStringFlags::empty(),
    });
    (0..fstring_value.as_slice().len()).map(move |i| {
        let fstring_value = &mut fstring_value;
        let tmp = fstring_value.into_iter().nth(i).unwrap();
        std::mem::replace(tmp, default.clone())
    })
}

fn ruff_fstring_element_into_iter(
    mut fstring_element: ruff::FStringElements,
) -> impl Iterator<Item = ruff::FStringElement> + 'static {
    let default = ruff::FStringElement::Literal(ruff::FStringLiteralElement {
        range: Default::default(),
        value: Default::default(),
    });
    (0..fstring_element.into_iter().len()).map(move |i| {
        let fstring_element = &mut fstring_element;
        let tmp = fstring_element.into_iter().nth(i).unwrap();
        std::mem::replace(tmp, default.clone())
    })
}

fn fstring_part_to_joined_str_part(fstring_part: ruff::FStringPart) -> Vec<JoinedStrPart> {
    match fstring_part {
        ruff::FStringPart::Literal(ruff::StringLiteral {
            range,
            value,
            flags,
        }) => {
            vec![JoinedStrPart::Constant(Constant::new_str(
                value,
                flags.prefix(),
                range,
            ))]
        }
        ruff::FStringPart::FString(ruff::FString {
            range: _,
            elements,
            flags: _, // TODO
        }) => ruff_fstring_element_into_iter(elements)
            .map(ruff_fstring_element_to_joined_str_part)
            .collect(),
    }
}

fn ruff_fstring_element_to_joined_str_part(element: ruff::FStringElement) -> JoinedStrPart {
    match element {
        ruff::FStringElement::Literal(ruff::FStringLiteralElement { range, value }) => {
            JoinedStrPart::Constant(Constant::new_str(
                value,
                ruff::str_prefix::StringLiteralPrefix::Empty,
                range,
            ))
        }
        ruff::FStringElement::Expression(ruff::FStringExpressionElement {
            range,
            expression,
            debug_text: _, // TODO: What is this?
            conversion,
            format_spec,
        }) => JoinedStrPart::FormattedValue(FormattedValue {
            value: expression,
            conversion,
            format_spec: ruff_format_spec_to_joined_str(format_spec),
            range,
        }),
    }
}

fn ruff_format_spec_to_joined_str(
    format_spec: Option<Box<ruff::FStringFormatSpec>>,
) -> Option<Box<JoinedStr>> {
    match format_spec {
        None => None,
        Some(format_spec) => {
            let ruff::FStringFormatSpec { range, elements } = *format_spec;
            let values: Vec<_> = ruff_fstring_element_into_iter(elements)
                .map(ruff_fstring_element_to_joined_str_part)
                .collect();
            let values = values.into_boxed_slice();
            Some(Box::new(JoinedStr { range, values }))
        }
    }
}

fn ruff_fstring_element_to_ruff_fstring_part(element: ruff::FStringElement) -> ruff::FStringPart {
    match element {
        ruff::FStringElement::Literal(value) => {
            let ruff::FStringLiteralElement { range, value } = value;
            ruff::FStringPart::Literal(ruff::StringLiteral {
                range,
                value,
                flags: ruff::StringLiteralFlags::empty(),
            })
        }
        ruff::FStringElement::Expression(value) => {
            let ruff::FStringExpressionElement {
                range,
                expression,
                debug_text,
                conversion,
                format_spec,
            } = value;
            ruff::FStringPart::FString(ruff::FString {
                range,
                elements: vec![ruff::FStringElement::Expression(
                    ruff::FStringExpressionElement {
                        range,
                        expression,
                        debug_text,
                        conversion,
                        format_spec,
                    },
                )]
                .into(),
                flags: ruff::FStringFlags::empty(),
            })
        }
    }
}

fn joined_str_to_ruff_format_spec(
    joined_str: Option<Box<JoinedStr>>,
) -> Option<Box<ruff::FStringFormatSpec>> {
    match joined_str {
        None => None,
        Some(joined_str) => {
            let JoinedStr { range, values } = *joined_str;
            let elements: Vec<_> = Box::into_iter(values)
                .map(joined_str_part_to_ruff_fstring_element)
                .collect();
            let format_spec = ruff::FStringFormatSpec {
                range,
                elements: elements.into(),
            };
            Some(Box::new(format_spec))
        }
    }
}

#[derive(Debug)]
pub(super) struct JoinedStr {
    pub(super) range: TextRange,
    pub(super) values: Box<[JoinedStrPart]>,
}

impl JoinedStr {
    pub(super) fn into_expr(self) -> ruff::Expr {
        let Self { range, values } = self;
        ruff::Expr::FString(ruff::ExprFString {
            range: Default::default(),
            value: match values.len() {
                // ruff represents an empty fstring like this:
                0 => ruff::FStringValue::single(ruff::FString {
                    range,
                    elements: vec![].into(),
                    flags: ruff::FStringFlags::empty(),
                }),
                1 => ruff::FStringValue::single(
                    Box::<[_]>::into_iter(values)
                        .map(joined_str_part_to_ruff_fstring_element)
                        .map(|element| ruff::FString {
                            range,
                            elements: vec![element].into(),
                            flags: ruff::FStringFlags::empty(),
                        })
                        .next()
                        .expect("FString has exactly one part"),
                ),
                _ => ruff::FStringValue::concatenated(
                    Box::<[_]>::into_iter(values)
                        .map(joined_str_part_to_ruff_fstring_element)
                        .map(ruff_fstring_element_to_ruff_fstring_part)
                        .collect(),
                ),
            },
        })
    }
}

fn joined_str_part_to_ruff_fstring_element(part: JoinedStrPart) -> ruff::FStringElement {
    match part {
        JoinedStrPart::FormattedValue(value) => {
            ruff::FStringElement::Expression(ruff::FStringExpressionElement {
                range: value.range,
                expression: value.value.clone(),
                debug_text: None, // TODO: What is this?
                conversion: value.conversion,
                format_spec: joined_str_to_ruff_format_spec(value.format_spec),
            })
        }
        JoinedStrPart::Constant(value) => {
            ruff::FStringElement::Literal(ruff::FStringLiteralElement {
                range: value.range,
                value: match value.value {
                    ConstantLiteral::Str { value, .. } => value,
                    _ => todo!(),
                },
            })
        }
    }
}

// constructor
impl Node for JoinedStr {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self { values, range } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprJoinedStr::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item(
            "values",
            BoxedSlice(values).ast_to_object(vm, source_file),
            vm,
        )
        .unwrap();
        node_add_location(&dict, range, vm, source_file);
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let values: BoxedSlice<_> = Node::ast_from_object(
            vm,
            source_file,
            get_node_field(vm, &object, "values", "JoinedStr")?,
        )?;
        Ok(Self {
            values: values.0,
            range: range_from_object(vm, source_file, object, "JoinedStr")?,
        })
    }
}

#[derive(Debug)]
pub(super) enum JoinedStrPart {
    FormattedValue(FormattedValue),
    Constant(Constant),
}

// constructor
impl Node for JoinedStrPart {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        match self {
            Self::FormattedValue(value) => value.ast_to_object(vm, source_file),
            Self::Constant(value) => value.ast_to_object(vm, source_file),
        }
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let cls = object.class();
        if cls.is(pyast::NodeExprFormattedValue::static_type()) {
            Ok(Self::FormattedValue(Node::ast_from_object(
                vm,
                source_file,
                object,
            )?))
        } else {
            Ok(Self::Constant(Node::ast_from_object(
                vm,
                source_file,
                object,
            )?))
        }
    }
}

#[derive(Debug)]
pub(super) struct FormattedValue {
    value: Box<ruff::Expr>,
    conversion: ruff::ConversionFlag,
    format_spec: Option<Box<JoinedStr>>,
    range: TextRange,
}

// constructor
impl Node for FormattedValue {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            value,
            conversion,
            format_spec,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprFormattedValue::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item("conversion", conversion.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item(
            "format_spec",
            format_spec.ast_to_object(vm, source_file),
            vm,
        )
        .unwrap();
        node_add_location(&dict, range, vm, source_file);
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            value: Node::ast_from_object(
                vm,
                source_file,
                get_node_field(vm, &object, "value", "FormattedValue")?,
            )?,
            conversion: Node::ast_from_object(
                vm,
                source_file,
                get_node_field(vm, &object, "conversion", "FormattedValue")?,
            )?,
            format_spec: get_node_field_opt(vm, &object, "format_spec")?
                .map(|obj| Node::ast_from_object(vm, source_file, obj))
                .transpose()?,
            range: range_from_object(vm, source_file, object, "FormattedValue")?,
        })
    }
}

pub(super) fn fstring_to_object(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    expression: ruff::ExprFString,
) -> PyObjectRef {
    let ruff::ExprFString { range, value } = expression;
    let values: Vec<_> = ruff_fstring_value_into_iter(value)
        .flat_map(fstring_part_to_joined_str_part)
        .collect();
    let values = values.into_boxed_slice();
    let c = JoinedStr { range, values };
    c.ast_to_object(vm, source_file)
}

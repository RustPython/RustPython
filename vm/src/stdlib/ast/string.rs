use super::constant::Constant;
use super::*;

impl Node for ruff::ExprStringLiteral {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let Self { range, value } = self;
        let c = Constant::new_str(value.to_str(), range);
        c.ast_to_object(vm)
    }

    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        todo!()
    }
}

impl Node for ruff::ExprFString {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let Self { range, value } = self;
        let values: Vec<_> = value
            .into_iter()
            .flat_map(fstring_part_to_joined_str_part)
            .collect();
        let values = values.into_boxed_slice();
        let c = JoinedStr { range, values };
        c.ast_to_object(vm)
    }

    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        todo!()
    }
}

fn fstring_part_to_joined_str_part(fstring_part: &ruff::FStringPart) -> Vec<JoinedStrPart> {
    match fstring_part {
        ruff::FStringPart::Literal(ruff::StringLiteral {
            range,
            value,
            flags: _, // TODO
        }) => vec![JoinedStrPart::Constant(Constant::new_str(value, *range))],
        ruff::FStringPart::FString(ruff::FString {
            range: _,
            elements,
            flags: _, // TODO
        }) => elements
            .into_iter()
            .map(fstring_element_to_joined_str_part)
            .collect(),
    }
}

fn fstring_element_to_joined_str_part(element: &ruff::FStringElement) -> JoinedStrPart {
    match element {
        ruff::FStringElement::Literal(ruff::FStringLiteralElement { range, value }) => {
            JoinedStrPart::Constant(Constant::new_str(value, *range))
        }
        ruff::FStringElement::Expression(ruff::FStringExpressionElement {
            range,
            expression,
            debug_text: _, // TODO: What is this?
            conversion,
            format_spec,
        }) => JoinedStrPart::FormattedValue(FormattedValue {
            value: expression.clone(),
            conversion: *conversion,
            format_spec: format_spec_helper(format_spec),
            range: *range,
        }),
    }
}

fn format_spec_helper(
    format_spec: &Option<Box<ruff::FStringFormatSpec>>,
) -> Option<Box<JoinedStr>> {
    match format_spec.as_deref() {
        None => None,
        Some(ruff::FStringFormatSpec { range, elements }) => {
            let values: Vec<_> = elements
                .into_iter()
                .map(fstring_element_to_joined_str_part)
                .collect();
            let values = values.into_boxed_slice();
            Some(Box::new(JoinedStr {
                values,
                range: *range,
            }))
        }
    }
}

struct JoinedStr {
    values: Box<[JoinedStrPart]>,
    range: TextRange,
}

// constructor
impl Node for JoinedStr {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let Self { values, range } = self;
        let node = NodeAst
            .into_ref_with_type(vm, gen::NodeExprJoinedStr::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("values", BoxedSlice(values).ast_to_object(vm), vm)
            .unwrap();
        node_add_location(&dict, range, vm);
        node.into()
    }
    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        let values: BoxedSlice<_> =
            Node::ast_from_object(vm, get_node_field(vm, &object, "values", "JoinedStr")?)?;
        Ok(Self {
            values: values.0,
            range: range_from_object(vm, object, "JoinedStr")?,
        })
    }
}

enum JoinedStrPart {
    FormattedValue(FormattedValue),
    Constant(Constant),
}

// constructor
impl Node for JoinedStrPart {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        match self {
            JoinedStrPart::FormattedValue(value) => value.ast_to_object(vm),
            JoinedStrPart::Constant(value) => value.ast_to_object(vm),
        }
    }
    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        let cls = object.class();
        if cls.is(gen::NodeExprFormattedValue::static_type()) {
            Ok(Self::FormattedValue(Node::ast_from_object(vm, object)?))
        } else {
            Ok(Self::Constant(Node::ast_from_object(vm, object)?))
        }
    }
}

struct FormattedValue {
    value: Box<ruff::Expr>,
    conversion: ruff::ConversionFlag,
    format_spec: Option<Box<JoinedStr>>,
    range: TextRange,
}

// constructor
impl Node for FormattedValue {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            value,
            conversion,
            format_spec,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, gen::NodeExprFormattedValue::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(vm), vm).unwrap();
        dict.set_item("conversion", conversion.ast_to_object(vm), vm)
            .unwrap();
        dict.set_item("format_spec", format_spec.ast_to_object(vm), vm)
            .unwrap();
        node_add_location(&dict, range, vm);
        node.into()
    }
    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            value: Node::ast_from_object(
                vm,
                get_node_field(vm, &object, "value", "FormattedValue")?,
            )?,
            conversion: Node::ast_from_object(
                vm,
                get_node_field(vm, &object, "conversion", "FormattedValue")?,
            )?,
            format_spec: get_node_field_opt(vm, &object, "format_spec")?
                .map(|obj| Node::ast_from_object(vm, obj))
                .transpose()?,
            range: range_from_object(vm, object, "FormattedValue")?,
        })
    }
}

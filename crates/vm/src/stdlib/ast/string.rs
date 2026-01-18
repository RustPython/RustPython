use super::constant::{Constant, ConstantLiteral};
use super::*;

fn ruff_fstring_element_into_iter(
    mut fstring_element: ast::InterpolatedStringElements,
) -> impl Iterator<Item = ast::InterpolatedStringElement> {
    let default = ast::InterpolatedStringElement::Literal(ast::InterpolatedStringLiteralElement {
        node_index: Default::default(),
        range: Default::default(),
        value: Default::default(),
    });
    fstring_element
        .iter_mut()
        .map(move |elem| core::mem::replace(elem, default.clone()))
        .collect::<Vec<_>>()
        .into_iter()
}

fn ruff_fstring_element_to_joined_str_part(
    element: ast::InterpolatedStringElement,
) -> JoinedStrPart {
    match element {
        ast::InterpolatedStringElement::Literal(ast::InterpolatedStringLiteralElement {
            range,
            value,
            node_index: _,
        }) => JoinedStrPart::Constant(Constant::new_str(
            value,
            ast::str_prefix::StringLiteralPrefix::Empty,
            range,
        )),
        ast::InterpolatedStringElement::Interpolation(ast::InterpolatedElement {
            range,
            expression,
            debug_text: _, // TODO: What is this?
            conversion,
            format_spec,
            node_index: _,
        }) => JoinedStrPart::FormattedValue(FormattedValue {
            value: expression,
            conversion,
            format_spec: ruff_format_spec_to_joined_str(format_spec),
            range,
        }),
    }
}

fn ruff_format_spec_to_joined_str(
    format_spec: Option<Box<ast::InterpolatedStringFormatSpec>>,
) -> Option<Box<JoinedStr>> {
    match format_spec {
        None => None,
        Some(format_spec) => {
            let ast::InterpolatedStringFormatSpec {
                range,
                elements,
                node_index: _,
            } = *format_spec;
            let values: Vec<_> = ruff_fstring_element_into_iter(elements)
                .map(ruff_fstring_element_to_joined_str_part)
                .collect();
            let values = values.into_boxed_slice();
            Some(Box::new(JoinedStr { range, values }))
        }
    }
}

fn ruff_fstring_element_to_ruff_fstring_part(
    element: ast::InterpolatedStringElement,
) -> ast::FStringPart {
    match element {
        ast::InterpolatedStringElement::Literal(value) => {
            let ast::InterpolatedStringLiteralElement {
                node_index,
                range,
                value,
            } = value;
            ast::FStringPart::Literal(ast::StringLiteral {
                node_index,
                range,
                value,
                flags: ast::StringLiteralFlags::empty(),
            })
        }
        ast::InterpolatedStringElement::Interpolation(ast::InterpolatedElement {
            range, ..
        }) => ast::FStringPart::FString(ast::FString {
            node_index: Default::default(),
            range,
            elements: vec![element].into(),
            flags: ast::FStringFlags::empty(),
        }),
    }
}

fn joined_str_to_ruff_format_spec(
    joined_str: Option<Box<JoinedStr>>,
) -> Option<Box<ast::InterpolatedStringFormatSpec>> {
    match joined_str {
        None => None,
        Some(joined_str) => {
            let JoinedStr { range, values } = *joined_str;
            let elements: Vec<_> = Box::into_iter(values)
                .map(joined_str_part_to_ruff_fstring_element)
                .collect();
            let format_spec = ast::InterpolatedStringFormatSpec {
                node_index: Default::default(),
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
    pub(super) fn into_expr(self) -> ast::Expr {
        let Self { range, values } = self;
        ast::Expr::FString(ast::ExprFString {
            node_index: Default::default(),
            range: Default::default(),
            value: match values.len() {
                // ruff represents an empty fstring like this:
                0 => ast::FStringValue::single(ast::FString {
                    node_index: Default::default(),
                    range,
                    elements: vec![].into(),
                    flags: ast::FStringFlags::empty(),
                }),
                1 => ast::FStringValue::single(
                    Box::<[_]>::into_iter(values)
                        .map(joined_str_part_to_ruff_fstring_element)
                        .map(|element| ast::FString {
                            node_index: Default::default(),
                            range,
                            elements: vec![element].into(),
                            flags: ast::FStringFlags::empty(),
                        })
                        .next()
                        .expect("FString has exactly one part"),
                ),
                _ => ast::FStringValue::concatenated(
                    Box::<[_]>::into_iter(values)
                        .map(joined_str_part_to_ruff_fstring_element)
                        .map(ruff_fstring_element_to_ruff_fstring_part)
                        .collect(),
                ),
            },
        })
    }
}

fn joined_str_part_to_ruff_fstring_element(part: JoinedStrPart) -> ast::InterpolatedStringElement {
    match part {
        JoinedStrPart::FormattedValue(value) => {
            ast::InterpolatedStringElement::Interpolation(ast::InterpolatedElement {
                node_index: Default::default(),
                range: value.range,
                expression: value.value.clone(),
                debug_text: None, // TODO: What is this?
                conversion: value.conversion,
                format_spec: joined_str_to_ruff_format_spec(value.format_spec),
            })
        }
        JoinedStrPart::Constant(value) => {
            ast::InterpolatedStringElement::Literal(ast::InterpolatedStringLiteralElement {
                node_index: Default::default(),
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
    value: Box<ast::Expr>,
    conversion: ast::ConversionFlag,
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
    expression: ast::ExprFString,
) -> PyObjectRef {
    let ast::ExprFString {
        range,
        mut value,
        node_index: _,
    } = expression;
    let default_part = ast::FStringPart::FString(ast::FString {
        node_index: Default::default(),
        range: Default::default(),
        elements: Default::default(),
        flags: ast::FStringFlags::empty(),
    });
    let mut values = Vec::new();
    for i in 0..value.as_slice().len() {
        let part = core::mem::replace(value.iter_mut().nth(i).unwrap(), default_part.clone());
        match part {
            ast::FStringPart::Literal(ast::StringLiteral {
                range,
                value,
                flags,
                node_index: _,
            }) => {
                values.push(JoinedStrPart::Constant(Constant::new_str(
                    value,
                    flags.prefix(),
                    range,
                )));
            }
            ast::FStringPart::FString(ast::FString {
                range: _,
                elements,
                flags: _,
                node_index: _,
            }) => {
                for element in ruff_fstring_element_into_iter(elements) {
                    values.push(ruff_fstring_element_to_joined_str_part(element));
                }
            }
        }
    }
    let c = JoinedStr {
        range,
        values: values.into_boxed_slice(),
    };
    c.ast_to_object(vm, source_file)
}

// ===== TString (Template String) Support =====

fn ruff_tstring_element_to_template_str_part(
    element: ast::InterpolatedStringElement,
    source_file: &SourceFile,
) -> TemplateStrPart {
    match element {
        ast::InterpolatedStringElement::Literal(ast::InterpolatedStringLiteralElement {
            range,
            value,
            node_index: _,
        }) => TemplateStrPart::Constant(Constant::new_str(
            value,
            ast::str_prefix::StringLiteralPrefix::Empty,
            range,
        )),
        ast::InterpolatedStringElement::Interpolation(ast::InterpolatedElement {
            range,
            expression,
            debug_text,
            conversion,
            format_spec,
            node_index: _,
        }) => {
            // Get the expression source text for the "str" field
            let expr_str = debug_text
                .map(|dt| dt.leading.to_string() + &dt.trailing)
                .unwrap_or_else(|| source_file.slice(expression.range()).to_string());
            TemplateStrPart::Interpolation(TStringInterpolation {
                value: expression,
                str: expr_str,
                conversion,
                format_spec: ruff_format_spec_to_template_str(format_spec, source_file),
                range,
            })
        }
    }
}

fn ruff_format_spec_to_template_str(
    format_spec: Option<Box<ast::InterpolatedStringFormatSpec>>,
    source_file: &SourceFile,
) -> Option<Box<TemplateStr>> {
    match format_spec {
        None => None,
        Some(format_spec) => {
            let ast::InterpolatedStringFormatSpec {
                range,
                elements,
                node_index: _,
            } = *format_spec;
            let values: Vec<_> = ruff_fstring_element_into_iter(elements)
                .map(|e| ruff_tstring_element_to_template_str_part(e, source_file))
                .collect();
            let values = values.into_boxed_slice();
            Some(Box::new(TemplateStr { range, values }))
        }
    }
}

#[derive(Debug)]
pub(super) struct TemplateStr {
    pub(super) range: TextRange,
    pub(super) values: Box<[TemplateStrPart]>,
}

// constructor
impl Node for TemplateStr {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self { values, range } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprTemplateStr::static_type().to_owned())
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
            get_node_field(vm, &object, "values", "TemplateStr")?,
        )?;
        Ok(Self {
            values: values.0,
            range: range_from_object(vm, source_file, object, "TemplateStr")?,
        })
    }
}

#[derive(Debug)]
pub(super) enum TemplateStrPart {
    Interpolation(TStringInterpolation),
    Constant(Constant),
}

// constructor
impl Node for TemplateStrPart {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        match self {
            Self::Interpolation(value) => value.ast_to_object(vm, source_file),
            Self::Constant(value) => value.ast_to_object(vm, source_file),
        }
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let cls = object.class();
        if cls.is(pyast::NodeExprInterpolation::static_type()) {
            Ok(Self::Interpolation(Node::ast_from_object(
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
pub(super) struct TStringInterpolation {
    value: Box<ast::Expr>,
    str: String,
    conversion: ast::ConversionFlag,
    format_spec: Option<Box<TemplateStr>>,
    range: TextRange,
}

// constructor
impl Node for TStringInterpolation {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            value,
            str,
            conversion,
            format_spec,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprInterpolation::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item("str", vm.ctx.new_str(str).into(), vm)
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
        let str_obj = get_node_field(vm, &object, "str", "Interpolation")?;
        let str_val: String = str_obj.try_into_value(vm)?;
        Ok(Self {
            value: Node::ast_from_object(
                vm,
                source_file,
                get_node_field(vm, &object, "value", "Interpolation")?,
            )?,
            str: str_val,
            conversion: Node::ast_from_object(
                vm,
                source_file,
                get_node_field(vm, &object, "conversion", "Interpolation")?,
            )?,
            format_spec: get_node_field_opt(vm, &object, "format_spec")?
                .map(|obj| Node::ast_from_object(vm, source_file, obj))
                .transpose()?,
            range: range_from_object(vm, source_file, object, "Interpolation")?,
        })
    }
}

pub(super) fn tstring_to_object(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    expression: ast::ExprTString,
) -> PyObjectRef {
    let ast::ExprTString {
        range,
        mut value,
        node_index: _,
    } = expression;
    let default_tstring = ast::TString {
        node_index: Default::default(),
        range: Default::default(),
        elements: Default::default(),
        flags: ast::TStringFlags::empty(),
    };
    let mut values = Vec::new();
    for i in 0..value.as_slice().len() {
        let tstring = core::mem::replace(value.iter_mut().nth(i).unwrap(), default_tstring.clone());
        for element in ruff_fstring_element_into_iter(tstring.elements) {
            values.push(ruff_tstring_element_to_template_str_part(
                element,
                source_file,
            ));
        }
    }
    let c = TemplateStr {
        range,
        values: values.into_boxed_slice(),
    };
    c.ast_to_object(vm, source_file)
}

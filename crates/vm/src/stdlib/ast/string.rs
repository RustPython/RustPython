use super::constant::{Constant, ConstantLiteral};
use super::*;
use crate::warn;
use ast::str_prefix::StringLiteralPrefix;

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

fn push_joined_str_literal(
    output: &mut Vec<JoinedStrPart>,
    pending: &mut Option<(String, StringLiteralPrefix, TextRange)>,
) {
    if let Some((value, prefix, range)) = pending.take()
        && !value.is_empty()
    {
        output.push(JoinedStrPart::Constant(Constant::new_str(
            value, prefix, range,
        )));
    }
}

fn normalize_joined_str_parts(values: Vec<JoinedStrPart>) -> Vec<JoinedStrPart> {
    let mut output = Vec::with_capacity(values.len());
    let mut pending: Option<(String, StringLiteralPrefix, TextRange)> = None;

    for part in values {
        match part {
            JoinedStrPart::Constant(constant) => {
                let ConstantLiteral::Str { value, prefix } = constant.value else {
                    push_joined_str_literal(&mut output, &mut pending);
                    output.push(JoinedStrPart::Constant(constant));
                    continue;
                };
                let value: String = value.into();
                if let Some((pending_value, _, _)) = pending.as_mut() {
                    pending_value.push_str(&value);
                } else {
                    pending = Some((value, prefix, constant.range));
                }
            }
            JoinedStrPart::FormattedValue(value) => {
                push_joined_str_literal(&mut output, &mut pending);
                output.push(JoinedStrPart::FormattedValue(value));
            }
        }
    }

    push_joined_str_literal(&mut output, &mut pending);
    output
}

fn push_template_str_literal(
    output: &mut Vec<TemplateStrPart>,
    pending: &mut Option<(String, StringLiteralPrefix, TextRange)>,
) {
    if let Some((value, prefix, range)) = pending.take()
        && !value.is_empty()
    {
        output.push(TemplateStrPart::Constant(Constant::new_str(
            value, prefix, range,
        )));
    }
}

fn normalize_template_str_parts(values: Vec<TemplateStrPart>) -> Vec<TemplateStrPart> {
    let mut output = Vec::with_capacity(values.len());
    let mut pending: Option<(String, StringLiteralPrefix, TextRange)> = None;

    for part in values {
        match part {
            TemplateStrPart::Constant(constant) => {
                let ConstantLiteral::Str { value, prefix } = constant.value else {
                    push_template_str_literal(&mut output, &mut pending);
                    output.push(TemplateStrPart::Constant(constant));
                    continue;
                };
                let value: String = value.into();
                if let Some((pending_value, _, _)) = pending.as_mut() {
                    pending_value.push_str(&value);
                } else {
                    pending = Some((value, prefix, constant.range));
                }
            }
            TemplateStrPart::Interpolation(value) => {
                push_template_str_literal(&mut output, &mut pending);
                output.push(TemplateStrPart::Interpolation(value));
            }
        }
    }

    push_template_str_literal(&mut output, &mut pending);
    output
}

fn warn_invalid_escape_sequences_in_format_spec(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    range: TextRange,
) {
    let source = source_file.source_text();
    let start = range.start().to_usize();
    let end = range.end().to_usize();
    if start >= end || end > source.len() {
        return;
    }
    let mut raw = &source[start..end];
    if raw.starts_with(':') {
        raw = &raw[1..];
    }

    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            continue;
        }
        let Some(next) = chars.next() else {
            break;
        };
        let valid = match next {
            '\\' | '\'' | '"' | 'a' | 'b' | 'f' | 'n' | 'r' | 't' | 'v' => true,
            '\n' => true,
            '\r' => {
                if let Some('\n') = chars.peek().copied() {
                    chars.next();
                }
                true
            }
            '0'..='7' => {
                for _ in 0..2 {
                    if let Some('0'..='7') = chars.peek().copied() {
                        chars.next();
                    } else {
                        break;
                    }
                }
                true
            }
            'x' => {
                for _ in 0..2 {
                    if chars.peek().is_some_and(|c| c.is_ascii_hexdigit()) {
                        chars.next();
                    } else {
                        break;
                    }
                }
                true
            }
            'u' => {
                for _ in 0..4 {
                    if chars.peek().is_some_and(|c| c.is_ascii_hexdigit()) {
                        chars.next();
                    } else {
                        break;
                    }
                }
                true
            }
            'U' => {
                for _ in 0..8 {
                    if chars.peek().is_some_and(|c| c.is_ascii_hexdigit()) {
                        chars.next();
                    } else {
                        break;
                    }
                }
                true
            }
            'N' => {
                if let Some('{') = chars.peek().copied() {
                    chars.next();
                    for c in chars.by_ref() {
                        if c == '}' {
                            break;
                        }
                    }
                }
                true
            }
            _ => false,
        };
        if !valid {
            let message = vm.ctx.new_str(format!(
                "\"\\{next}\" is an invalid escape sequence. Such sequences will not work in the future. Did you mean \"\\\\{next}\"? A raw string is also an option."
            ));
            let _ = warn::warn(
                message.into(),
                Some(vm.ctx.exceptions.syntax_warning.to_owned()),
                1,
                None,
                vm,
            );
        }
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
            let range = if range.start() > ruff_text_size::TextSize::from(0) {
                TextRange::new(
                    range.start() - ruff_text_size::TextSize::from(1),
                    range.end(),
                )
            } else {
                range
            };
            let values: Vec<_> = ruff_fstring_element_into_iter(elements)
                .map(ruff_fstring_element_to_joined_str_part)
                .collect();
            let values = normalize_joined_str_parts(values).into_boxed_slice();
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
    let values = normalize_joined_str_parts(values);
    for part in &values {
        if let JoinedStrPart::FormattedValue(value) = part
            && let Some(format_spec) = &value.format_spec
        {
            warn_invalid_escape_sequences_in_format_spec(vm, source_file, format_spec.range);
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
            let expr_range =
                extend_expr_range_with_wrapping_parens(source_file, range, expression.range())
                    .unwrap_or_else(|| expression.range());
            let expr_str = if let Some(debug_text) = debug_text {
                let expr_source = source_file.slice(expr_range);
                let mut expr_with_debug = String::with_capacity(
                    debug_text.leading.len() + expr_source.len() + debug_text.trailing.len(),
                );
                expr_with_debug.push_str(&debug_text.leading);
                expr_with_debug.push_str(expr_source);
                expr_with_debug.push_str(&debug_text.trailing);
                strip_interpolation_expr(&expr_with_debug)
            } else {
                tstring_interpolation_expr_str(source_file, range, expr_range)
            };
            TemplateStrPart::Interpolation(TStringInterpolation {
                value: expression,
                str: expr_str,
                conversion,
                format_spec: ruff_format_spec_to_joined_str(format_spec),
                range,
            })
        }
    }
}

fn tstring_interpolation_expr_str(
    source_file: &SourceFile,
    interpolation_range: TextRange,
    expr_range: TextRange,
) -> String {
    let expr_range =
        extend_expr_range_with_wrapping_parens(source_file, interpolation_range, expr_range)
            .unwrap_or(expr_range);
    let start = interpolation_range.start() + TextSize::from(1);
    let start = if start > expr_range.end() {
        expr_range.start()
    } else {
        start
    };
    let expr_source = source_file.slice(TextRange::new(start, expr_range.end()));
    strip_interpolation_expr(expr_source)
}

fn extend_expr_range_with_wrapping_parens(
    source_file: &SourceFile,
    interpolation_range: TextRange,
    expr_range: TextRange,
) -> Option<TextRange> {
    let left_slice = source_file.slice(TextRange::new(
        interpolation_range.start(),
        expr_range.start(),
    ));
    let mut left_char: Option<(usize, char)> = None;
    for (idx, ch) in left_slice
        .char_indices()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        if !ch.is_whitespace() {
            left_char = Some((idx, ch));
            break;
        }
    }
    let (left_idx, left_ch) = left_char?;
    if left_ch != '(' {
        return None;
    }

    let right_slice =
        source_file.slice(TextRange::new(expr_range.end(), interpolation_range.end()));
    let mut right_char: Option<(usize, char)> = None;
    for (idx, ch) in right_slice.char_indices() {
        if !ch.is_whitespace() {
            right_char = Some((idx, ch));
            break;
        }
    }
    let (right_idx, right_ch) = right_char?;
    if right_ch != ')' {
        return None;
    }

    let left_pos = interpolation_range.start() + TextSize::from(left_idx as u32);
    let right_pos = expr_range.end() + TextSize::from(right_idx as u32);
    Some(TextRange::new(left_pos, right_pos + TextSize::from(1)))
}

fn strip_interpolation_expr(expr_source: &str) -> String {
    let mut end = expr_source.len();
    for (idx, ch) in expr_source.char_indices().rev() {
        if ch.is_whitespace() || ch == '=' {
            end = idx;
            continue;
        }
        end = idx + ch.len_utf8();
        break;
    }
    expr_source[..end].to_owned()
}

#[derive(Debug)]
pub(super) struct TemplateStr {
    pub(super) range: TextRange,
    pub(super) values: Box<[TemplateStrPart]>,
}

pub(super) fn template_str_to_expr(
    vm: &VirtualMachine,
    template: TemplateStr,
) -> PyResult<ast::Expr> {
    let TemplateStr { range, values } = template;
    let elements = template_parts_to_elements(vm, values)?;
    let tstring = ast::TString {
        range,
        node_index: Default::default(),
        elements,
        flags: ast::TStringFlags::empty(),
    };
    Ok(ast::Expr::TString(ast::ExprTString {
        node_index: Default::default(),
        range,
        value: ast::TStringValue::single(tstring),
    }))
}

pub(super) fn interpolation_to_expr(
    vm: &VirtualMachine,
    interpolation: TStringInterpolation,
) -> PyResult<ast::Expr> {
    let part = TemplateStrPart::Interpolation(interpolation);
    let elements = template_parts_to_elements(vm, vec![part].into_boxed_slice())?;
    let range = TextRange::default();
    let tstring = ast::TString {
        range,
        node_index: Default::default(),
        elements,
        flags: ast::TStringFlags::empty(),
    };
    Ok(ast::Expr::TString(ast::ExprTString {
        node_index: Default::default(),
        range,
        value: ast::TStringValue::single(tstring),
    }))
}

fn template_parts_to_elements(
    vm: &VirtualMachine,
    values: Box<[TemplateStrPart]>,
) -> PyResult<ast::InterpolatedStringElements> {
    let mut elements = Vec::with_capacity(values.len());
    for value in values.into_vec() {
        elements.push(template_part_to_element(vm, value)?);
    }
    Ok(ast::InterpolatedStringElements::from(elements))
}

fn template_part_to_element(
    vm: &VirtualMachine,
    part: TemplateStrPart,
) -> PyResult<ast::InterpolatedStringElement> {
    match part {
        TemplateStrPart::Constant(constant) => {
            let ConstantLiteral::Str { value, .. } = constant.value else {
                return Err(
                    vm.new_type_error("TemplateStr constant values must be strings".to_owned())
                );
            };
            Ok(ast::InterpolatedStringElement::Literal(
                ast::InterpolatedStringLiteralElement {
                    range: constant.range,
                    node_index: Default::default(),
                    value,
                },
            ))
        }
        TemplateStrPart::Interpolation(interpolation) => {
            let TStringInterpolation {
                value,
                conversion,
                format_spec,
                range,
                ..
            } = interpolation;
            let format_spec = joined_str_to_ruff_format_spec(format_spec);
            Ok(ast::InterpolatedStringElement::Interpolation(
                ast::InterpolatedElement {
                    range,
                    node_index: Default::default(),
                    expression: value,
                    debug_text: None,
                    conversion,
                    format_spec,
                },
            ))
        }
    }
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
    format_spec: Option<Box<JoinedStr>>,
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
    let values = normalize_template_str_parts(values);
    let c = TemplateStr {
        range,
        values: values.into_boxed_slice(),
    };
    c.ast_to_object(vm, source_file)
}

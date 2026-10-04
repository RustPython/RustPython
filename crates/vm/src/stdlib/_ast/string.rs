use super::constant::{Constant, ConstantLiteral};
use super::*;
use ast::str_prefix::StringLiteralPrefix;
use rustpython_common::wtf8::Wtf8Buf;

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

fn push_ruff_fstring_element(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    flags: ast::AnyStringFlags,
    element: ast::InterpolatedStringElement,
    output: &mut Vec<JoinedStrPart>,
) {
    match element {
        ast::InterpolatedStringElement::Literal(literal) => {
            output.push(JoinedStrPart::Constant(Constant::new_str(
                rustpython_codegen::interpolated_string_literal_value(source_file, &literal, flags),
                ast::str_prefix::StringLiteralPrefix::Empty,
                literal.range,
            )));
        }
        ast::InterpolatedStringElement::Interpolation(ast::InterpolatedElement {
            range,
            expression,
            debug_text,
            mut conversion,
            format_spec,
            node_index: _,
            runtime_str: _,
            runtime_interpolation_format_spec: _,
            runtime_formatted_value_format_spec,
        }) => {
            if let Some(debug_text) = &debug_text {
                output.push(JoinedStrPart::Constant(interpolation_debug_constant(
                    source_file,
                    debug_text,
                    expression.range(),
                )));
                conversion = debug_conversion(conversion, format_spec.is_some());
            }
            let runtime_format_spec = runtime_formatted_value_format_spec.or_else(|| {
                ruff_format_spec_to_joined_str(vm, source_file, flags, format_spec)
                    .map(|joined_str| Box::new(joined_str.into_expr(false)))
            });
            output.push(JoinedStrPart::FormattedValue(FormattedValue {
                value: expression,
                conversion,
                format_spec: runtime_format_spec,
                range,
            }));
        }
    }
}

/// The literal an `{expr=}` interpolation puts in front of its value.
fn interpolation_debug_constant(
    source_file: &SourceFile,
    debug_text: &ast::DebugText,
    expression_range: TextRange,
) -> Constant {
    let (text, range) =
        rustpython_codegen::interpolation_debug_text(source_file, debug_text, expression_range);
    Constant::new_str(text, ast::str_prefix::StringLiteralPrefix::Empty, range)
}

/// A `{expr=}` interpolation takes `repr` unless it was told otherwise.
fn debug_conversion(conversion: ast::ConversionFlag, has_format_spec: bool) -> ast::ConversionFlag {
    if matches!(conversion, ast::ConversionFlag::None) && !has_format_spec {
        ast::ConversionFlag::Repr
    } else {
        conversion
    }
}

fn push_joined_str_literal(
    output: &mut Vec<JoinedStrPart>,
    pending: &mut Option<(Wtf8Buf, StringLiteralPrefix, TextRange)>,
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
    let mut pending: Option<(Wtf8Buf, StringLiteralPrefix, TextRange)> = None;

    for part in values {
        match part {
            JoinedStrPart::Constant(constant) => {
                let ConstantLiteral::Str { value, prefix } = constant.value else {
                    push_joined_str_literal(&mut output, &mut pending);
                    output.push(JoinedStrPart::Constant(constant));
                    continue;
                };
                if let Some((pending_value, _, pending_range)) = pending.as_mut() {
                    pending_value.push_wtf8(&value);
                    // Folded literals span from the first of them to the last.
                    *pending_range = TextRange::new(pending_range.start(), constant.range.end());
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
    pending: &mut Option<(Wtf8Buf, StringLiteralPrefix, TextRange)>,
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
    let mut pending: Option<(Wtf8Buf, StringLiteralPrefix, TextRange)> = None;

    for part in values {
        match part {
            TemplateStrPart::Constant(constant) => {
                let ConstantLiteral::Str { value, prefix } = constant.value else {
                    push_template_str_literal(&mut output, &mut pending);
                    output.push(TemplateStrPart::Constant(constant));
                    continue;
                };
                if let Some((pending_value, _, pending_range)) = pending.as_mut() {
                    pending_value.push_wtf8(&value);
                    // Folded literals span from the first of them to the last.
                    *pending_range = TextRange::new(pending_range.start(), constant.range.end());
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

#[cfg(feature = "parser")]
fn warn_invalid_escape_sequences_in_format_spec<E>(
    source_file: &SourceFile,
    range: TextRange,
    emit_warning: &mut impl FnMut(usize, char) -> Result<(), E>,
) -> Result<(), E> {
    let source = source_file.source_text();
    let start = range.start().to_usize();
    let end = range.end().to_usize();
    if start >= end || end > source.len() {
        return Ok(());
    }
    let Some(raw) = source.get(start..end) else {
        return Ok(());
    };
    let mut chars = raw.char_indices().peekable();
    while let Some((offset, ch)) = chars.next() {
        if ch != '\\' {
            continue;
        }
        let Some((_, next)) = chars.next() else {
            break;
        };
        let valid = match next {
            '\\' | '\'' | '"' | 'a' | 'b' | 'f' | 'n' | 'r' | 't' | 'v' => true,
            '\n' => true,
            '\r' => {
                if let Some((_, '\n')) = chars.peek().copied() {
                    chars.next();
                }
                true
            }
            '0'..='7' => {
                for _ in 0..2 {
                    if let Some((_, '0'..='7')) = chars.peek().copied() {
                        chars.next();
                    } else {
                        break;
                    }
                }
                true
            }
            'x' => {
                for _ in 0..2 {
                    if chars.peek().is_some_and(|(_, c)| c.is_ascii_hexdigit()) {
                        chars.next();
                    } else {
                        break;
                    }
                }
                true
            }
            'u' => {
                for _ in 0..4 {
                    if chars.peek().is_some_and(|(_, c)| c.is_ascii_hexdigit()) {
                        chars.next();
                    } else {
                        break;
                    }
                }
                true
            }
            'U' => {
                for _ in 0..8 {
                    if chars.peek().is_some_and(|(_, c)| c.is_ascii_hexdigit()) {
                        chars.next();
                    } else {
                        break;
                    }
                }
                true
            }
            'N' => {
                if let Some((_, '{')) = chars.peek().copied() {
                    chars.next();
                    for (_, c) in chars.by_ref() {
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
            return emit_warning(start + offset, next);
        }
    }
    Ok(())
}

/// Keep AST parsing's existing format-spec diagnostics at a fallible boundary,
/// rather than emitting warnings from the infallible Python AST conversion.
#[cfg(feature = "parser")]
pub(super) fn emit_format_spec_warnings<E>(
    statements: &[ast::Stmt],
    expressions: &[ast::Expr],
    source_file: &SourceFile,
    emit_warning: &mut impl FnMut(usize, char) -> Result<(), E>,
) -> Result<(), E> {
    use ast::visitor::{self, Visitor};
    use std::collections::HashSet;

    struct LiteralVisitor<'a, F, E> {
        source_file: &'a SourceFile,
        emit_warning: F,
        emitted: HashSet<usize>,
        error: Option<E>,
    }

    impl<F, E> LiteralVisitor<'_, F, E>
    where
        F: FnMut(usize, char) -> Result<(), E>,
    {
        fn check_literal(&mut self, mut range: TextRange, interpolated: bool) {
            if self.error.is_some() {
                return;
            }
            // Ruff leaves the interpolation delimiter outside the literal range.
            // Include it so a trailing backslash can still diagnose \{ or \}.
            if interpolated
                && self
                    .source_file
                    .source_text()
                    .as_bytes()
                    .get(range.end().to_usize())
                    .is_some_and(|byte| matches!(byte, b'{' | b'}'))
            {
                range = TextRange::new(range.start(), range.end() + TextSize::from(1));
            }
            let result = warn_invalid_escape_sequences_in_format_spec(
                self.source_file,
                range,
                &mut |offset, ch| {
                    if !self.emitted.contains(&offset) {
                        (self.emit_warning)(offset, ch)?;
                        self.emitted.insert(offset);
                    }
                    Ok(())
                },
            );
            if let Err(error) = result {
                self.error = Some(error);
            }
        }

        fn visit_elements<'a>(&mut self, elements: &'a ast::InterpolatedStringElements, raw: bool)
        where
            Self: Visitor<'a>,
        {
            for element in elements {
                match element {
                    ast::InterpolatedStringElement::Literal(literal) => {
                        if !raw {
                            self.check_literal(literal.range, true);
                        }
                    }
                    ast::InterpolatedStringElement::Interpolation(interpolation) => {
                        self.visit_expr(&interpolation.expression);
                        if let Some(spec) = &interpolation.format_spec {
                            self.visit_elements(&spec.elements, raw);
                        }
                    }
                }
            }
        }
    }

    // Visit only literal spans within an existing format-spec warning region.
    // An enclosing spec can include nested raw strings, nonraw expressions in a
    // raw f-string, and comments; scanning its entire source range conflates them.
    impl<'a, F, E> Visitor<'a> for LiteralVisitor<'_, F, E>
    where
        F: FnMut(usize, char) -> Result<(), E>,
    {
        fn visit_expr(&mut self, expr: &'a ast::Expr) {
            if self.error.is_none() {
                visitor::walk_expr(self, expr);
            }
        }

        fn visit_string_literal(&mut self, literal: &'a ast::StringLiteral) {
            if !matches!(
                literal.flags.prefix(),
                ast::str_prefix::StringLiteralPrefix::Raw { .. }
            ) {
                self.check_literal(literal.range, false);
            }
        }

        fn visit_bytes_literal(&mut self, literal: &'a ast::BytesLiteral) {
            if !literal.flags.prefix().is_raw() {
                self.check_literal(literal.range, false);
            }
        }

        fn visit_f_string(&mut self, fstring: &'a ast::FString) {
            self.visit_elements(&fstring.elements, fstring.flags.prefix().is_raw());
        }

        fn visit_t_string(&mut self, tstring: &'a ast::TString) {
            self.visit_elements(&tstring.elements, tstring.flags.prefix().is_raw());
        }
    }

    struct FormatSpecVisitor<'a, F, E> {
        literals: LiteralVisitor<'a, F, E>,
    }

    impl<'a, F, E> Visitor<'a> for FormatSpecVisitor<'_, F, E>
    where
        F: FnMut(usize, char) -> Result<(), E>,
    {
        fn visit_expr(&mut self, expr: &'a ast::Expr) {
            if self.literals.error.is_some() {
                return;
            }
            if let ast::Expr::FString(fstring) = expr {
                // Conversion checked all enclosing specs before converting their
                // expressions, including across concatenated f-string parts.
                for part in fstring.value.as_slice() {
                    if let ast::FStringPart::FString(part) = part {
                        self.visit_format_specs(&part.elements, part.flags.prefix().is_raw());
                    }
                }
            }
            visitor::walk_expr(self, expr);
        }

        fn visit_t_string(&mut self, tstring: &'a ast::TString) {
            let raw = tstring.flags.prefix().is_raw();
            for element in &tstring.elements {
                if let ast::InterpolatedStringElement::Interpolation(interpolation) = element {
                    self.visit_expr(&interpolation.expression);
                    if let Some(spec) = &interpolation.format_spec {
                        // Template specs become JoinedStr nodes during conversion;
                        // their nested format specs already emitted warnings too.
                        self.visit_format_specs(&spec.elements, raw);
                        for element in &spec.elements {
                            self.visit_interpolated_string_element(element);
                        }
                    }
                }
            }
        }
    }

    impl<F, E> FormatSpecVisitor<'_, F, E>
    where
        F: FnMut(usize, char) -> Result<(), E>,
    {
        fn visit_format_specs(&mut self, elements: &ast::InterpolatedStringElements, raw: bool) {
            for element in elements {
                if let ast::InterpolatedStringElement::Interpolation(interpolation) = element
                    && let Some(spec) = &interpolation.format_spec
                {
                    self.literals.visit_elements(&spec.elements, raw);
                }
            }
        }
    }

    let mut visitor = FormatSpecVisitor {
        literals: LiteralVisitor {
            source_file,
            emit_warning,
            emitted: HashSet::new(),
            error: None,
        },
    };
    visitor.visit_body(statements);
    for expr in expressions {
        visitor.visit_expr(expr);
    }
    visitor.literals.error.map_or(Ok(()), Err)
}

fn ruff_format_spec_to_joined_str(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    flags: ast::AnyStringFlags,
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
            // The `:` that opens a format spec belongs to its range, and the
            // parser leaves it out of the outermost one.
            let opened_by_colon = source_file
                .source_text()
                .as_bytes()
                .get(..range.start().to_usize())
                .and_then(<[u8]>::last)
                == Some(&b':');
            let range = if opened_by_colon {
                TextRange::new(
                    range.start() - ruff_text_size::TextSize::from(1),
                    range.end(),
                )
            } else {
                range
            };
            let mut values = Vec::new();
            for element in ruff_fstring_element_into_iter(elements) {
                push_ruff_fstring_element(vm, source_file, flags, element, &mut values);
            }
            let values = normalize_joined_str_parts(values).into_boxed_slice();
            Some(Box::new(JoinedStr {
                range,
                values,
                runtime_values: None,
            }))
        }
    }
}

fn dummy_string_literal() -> ast::StringLiteral {
    ast::StringLiteral {
        node_index: Default::default(),
        range: Default::default(),
        value: "".into(),
        flags: ast::StringLiteralFlags::empty(),
    }
}

fn dummy_fstring() -> ast::FString {
    ast::FString {
        node_index: Default::default(),
        range: Default::default(),
        elements: Default::default(),
        flags: ast::FStringFlags::empty(),
    }
}

fn take_fstring_part(part: ast::FStringPartMut<'_>) -> ast::FStringPart {
    match part {
        ast::FStringPartMut::Literal(literal) => {
            ast::FStringPart::Literal(core::mem::replace(literal, dummy_string_literal()))
        }
        ast::FStringPartMut::FString(fstring) => {
            ast::FStringPart::FString(core::mem::replace(fstring, dummy_fstring()))
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

fn format_spec_expr_to_ruff_format_spec(
    format_spec: Option<Box<ast::Expr>>,
) -> Option<Box<ast::InterpolatedStringFormatSpec>> {
    let format_spec = format_spec?;
    let ast::Expr::FString(mut fstring) = *format_spec else {
        return None;
    };
    let ast::ExprFString {
        range,
        ref mut value,
        node_index: _,
        runtime_joined_str: _,
        runtime_values: _,
    } = fstring;
    let mut elements = Vec::new();
    for part in value {
        match take_fstring_part(part) {
            ast::FStringPart::Literal(ast::StringLiteral {
                range,
                value,
                node_index: _,
                flags: _,
            }) => elements.push(ast::InterpolatedStringElement::Literal(
                ast::InterpolatedStringLiteralElement {
                    node_index: Default::default(),
                    range,
                    value,
                },
            )),
            ast::FStringPart::FString(ast::FString {
                elements: fstring_elements,
                ..
            }) => {
                elements.extend(ruff_fstring_element_into_iter(fstring_elements));
            }
        }
    }
    Some(Box::new(ast::InterpolatedStringFormatSpec {
        node_index: Default::default(),
        range,
        elements: elements.into(),
    }))
}

#[derive(Debug)]
pub(super) struct JoinedStr {
    pub(super) range: TextRange,
    pub(super) values: Box<[JoinedStrPart]>,
    pub(super) runtime_values: Option<Vec<Option<ast::Expr>>>,
}

impl JoinedStr {
    pub(super) fn into_expr(self, from_ast_object: bool) -> ast::Expr {
        let Self {
            range,
            values,
            runtime_values: mut raw_runtime_values,
        } = self;
        let values = if values.iter().any(joined_str_part_requires_runtime_values) {
            if raw_runtime_values.is_none() {
                raw_runtime_values = Some(
                    values
                        .into_vec()
                        .into_iter()
                        .map(|part| joined_str_part_to_expr(from_ast_object, part))
                        .map(Some)
                        .collect(),
                );
            }
            Vec::new().into_boxed_slice()
        } else {
            values
        };
        let (runtime_joined_str, runtime_values) =
            raw_runtime_values.take().map_or((None, None), |values| {
                if values.iter().any(Option::is_none) {
                    (None, Some(values))
                } else {
                    (Some(values.into_iter().flatten().collect()), None)
                }
            });
        ast::Expr::FString(ast::ExprFString {
            node_index: Default::default(),
            range,
            value: match values.len() {
                0 => ast::FStringValue::single(ast::FString {
                    node_index: Default::default(),
                    range,
                    elements: vec![].into(),
                    flags: ast::FStringFlags::empty(),
                }),
                1 => ast::FStringValue::single(
                    Box::<[_]>::into_iter(values)
                        .map(|part| joined_str_part_to_ruff_fstring_element(from_ast_object, part))
                        .map(Option::unwrap)
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
                        .map(|part| joined_str_part_to_ruff_fstring_element(from_ast_object, part))
                        .map(Option::unwrap)
                        .map(ruff_fstring_element_to_ruff_fstring_part)
                        .collect(),
                ),
            },
            runtime_joined_str,
            runtime_values,
        })
    }
}

fn joined_str_part_requires_runtime_values(part: &JoinedStrPart) -> bool {
    matches!(
        part,
        JoinedStrPart::Constant(Constant {
            value,
            ..
        }) if !matches!(value, ConstantLiteral::Str { .. })
    )
}

fn joined_str_part_to_expr(from_ast_object: bool, part: JoinedStrPart) -> ast::Expr {
    match part {
        JoinedStrPart::FormattedValue(value) => formatted_value_to_expr(from_ast_object, value),
        JoinedStrPart::Constant(value) => value.into_expr(),
    }
}

fn joined_str_part_to_ruff_fstring_element(
    from_ast_object: bool,
    part: JoinedStrPart,
) -> Option<ast::InterpolatedStringElement> {
    match part {
        JoinedStrPart::FormattedValue(value) => {
            let format_spec = value.format_spec.clone();
            let runtime_formatted_value_format_spec = (from_ast_object && format_spec.is_some())
                .then_some(format_spec.clone())
                .flatten();
            Some(ast::InterpolatedStringElement::Interpolation(
                ast::InterpolatedElement {
                    node_index: Default::default(),
                    range: value.range,
                    expression: value.value.clone(),
                    debug_text: None,
                    conversion: value.conversion,
                    format_spec: format_spec_expr_to_ruff_format_spec(format_spec),
                    runtime_str: None,
                    runtime_interpolation_format_spec: None,
                    runtime_formatted_value_format_spec,
                },
            ))
        }
        JoinedStrPart::Constant(value) => {
            let Constant { range, value, .. } = value;
            let ConstantLiteral::Str { value, .. } = value else {
                return None;
            };
            Some(ast::InterpolatedStringElement::Literal(
                ast::InterpolatedStringLiteralElement {
                    node_index: Default::default(),
                    range,
                    value: value.to_string_lossy().into(),
                },
            ))
        }
    }
}

// constructor
pub(super) fn joined_str_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: &PyObject,
    range: TextRange,
) -> PyResult<JoinedStr> {
    let values: Vec<Option<ast::Expr>> =
        get_node_list_field(vm, source_file, object, "values", "JoinedStr")?;
    Ok(JoinedStr {
        values: Vec::new().into_boxed_slice(),
        runtime_values: Some(values),
        range,
    })
}

impl Node for JoinedStr {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            values,
            runtime_values,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprJoinedStr::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        let values = if let Some(runtime_values) = runtime_values {
            BoxedSlice(runtime_values.into_boxed_slice()).ast_to_object(vm, source_file)
        } else {
            BoxedSlice(values).ast_to_object(vm, source_file)
        };
        dict.set_item("values", values, vm).unwrap();
        node_add_location(&dict, range, vm, source_file);
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, &object, "JoinedStr")?;
        joined_str_from_object_with_range(vm, source_file, &object, range)
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
        if is_node_instance(vm, &object, pyast::NodeExprFormattedValue::static_type())? {
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
    format_spec: Option<Box<ast::Expr>>,
    range: TextRange,
}

// constructor
pub(super) fn formatted_value_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: &PyObject,
    range: TextRange,
) -> PyResult<FormattedValue> {
    Ok(FormattedValue {
        value: get_required_node_field(vm, source_file, object, "value", "FormattedValue")?,
        conversion: Node::ast_from_object(
            vm,
            source_file,
            get_node_field(vm, object, "conversion", "FormattedValue")?,
        )?,
        format_spec: get_node_field_opt(vm, object, "format_spec")?
            .map(|obj| Node::ast_from_object(vm, source_file, obj))
            .transpose()?,
        range,
    })
}

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
        let range = range_from_object(vm, source_file, &object, "FormattedValue")?;
        formatted_value_from_object_with_range(vm, source_file, &object, range)
    }
}

pub(super) fn formatted_value_to_expr(
    from_ast_object: bool,
    formatted: FormattedValue,
) -> ast::Expr {
    let range = formatted.range;
    JoinedStr {
        range,
        values: vec![JoinedStrPart::FormattedValue(formatted)].into_boxed_slice(),
        runtime_values: None,
    }
    .into_expr(from_ast_object)
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
        runtime_joined_str,
        runtime_values,
    } = expression;
    if let Some(joined_str) = runtime_joined_str {
        return JoinedStr {
            range,
            values: Vec::new().into_boxed_slice(),
            runtime_values: Some(joined_str.into_iter().map(Some).collect()),
        }
        .ast_to_object(vm, source_file);
    }

    if let Some(values) = runtime_values {
        return JoinedStr {
            range,
            values: Vec::new().into_boxed_slice(),
            runtime_values: Some(values),
        }
        .ast_to_object(vm, source_file);
    }

    let mut values = Vec::new();
    for part in &mut value {
        match take_fstring_part(part) {
            ast::FStringPart::Literal(literal) => {
                values.push(JoinedStrPart::Constant(Constant::new_str(
                    rustpython_codegen::string_literal_part_value(source_file, &literal),
                    literal.flags.prefix(),
                    literal.range,
                )));
            }
            ast::FStringPart::FString(ast::FString {
                range: _,
                elements,
                flags,
                node_index: _,
            }) => {
                for element in ruff_fstring_element_into_iter(elements) {
                    push_ruff_fstring_element(vm, source_file, flags.into(), element, &mut values);
                }
            }
        }
    }
    let values = normalize_joined_str_parts(values);
    let c = JoinedStr {
        range,
        values: values.into_boxed_slice(),
        runtime_values: None,
    };
    c.ast_to_object(vm, source_file)
}

// ===== TString (Template String) Support =====

fn push_ruff_tstring_element(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    flags: ast::AnyStringFlags,
    element: ast::InterpolatedStringElement,
    output: &mut Vec<TemplateStrPart>,
) {
    match element {
        ast::InterpolatedStringElement::Literal(literal) => {
            output.push(TemplateStrPart::Constant(Constant::new_str(
                rustpython_codegen::interpolated_string_literal_value(source_file, &literal, flags),
                ast::str_prefix::StringLiteralPrefix::Empty,
                literal.range,
            )));
        }
        ast::InterpolatedStringElement::Interpolation(interpolation) => {
            let expr_str =
                rustpython_codegen::interpolation_expression_text(source_file, &interpolation)
                    .unwrap_or_else(|| {
                        source_file
                            .source_text()
                            .slice(interpolation.expression.range())
                            .to_owned()
                    });
            let ast::InterpolatedElement {
                range,
                expression,
                debug_text,
                mut conversion,
                format_spec,
                runtime_str,
                runtime_interpolation_format_spec,
                ..
            } = interpolation;
            if let Some(debug_text) = &debug_text {
                output.push(TemplateStrPart::Constant(interpolation_debug_constant(
                    source_file,
                    debug_text,
                    expression.range(),
                )));
                conversion = debug_conversion(conversion, format_spec.is_some());
            }
            let runtime_interpolation = super::constant::runtime_interpolation_object(
                vm,
                runtime_str,
                runtime_interpolation_format_spec,
            );
            output.push(TemplateStrPart::Interpolation(TStringInterpolation {
                value: expression,
                str: runtime_interpolation
                    .as_ref()
                    .map_or_else(|| vm.ctx.new_str(expr_str).into(), |(str, _)| str.clone()),
                conversion,
                format_spec: runtime_interpolation
                    .and_then(|(_, format_spec)| format_spec)
                    .or_else(|| {
                        ruff_format_spec_to_joined_str(vm, source_file, flags, format_spec)
                            .map(|joined_str| Box::new(joined_str.into_expr(false)))
                    }),
                range,
            }));
        }
    }
}

#[derive(Debug)]
pub(super) struct TemplateStr {
    pub(super) range: TextRange,
    pub(super) values: Box<[TemplateStrPart]>,
    pub(super) runtime_values: Option<Vec<Option<ast::Expr>>>,
}

pub(super) fn template_str_to_expr(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    template: TemplateStr,
) -> PyResult<ast::Expr> {
    let TemplateStr {
        range,
        values,
        runtime_values: raw_runtime_values,
    } = template;
    let elements = template_parts_to_elements(vm, source_file, values)?;
    let tstring = ast::TString {
        range,
        node_index: Default::default(),
        elements,
        flags: ast::TStringFlags::empty(),
    };
    let (runtime_template_str, runtime_values) =
        raw_runtime_values.map_or((None, None), |values| {
            if values.iter().any(Option::is_none) {
                (None, Some(values))
            } else {
                (Some(values.into_iter().flatten().collect()), None)
            }
        });
    Ok(ast::Expr::TString(ast::ExprTString {
        node_index: Default::default(),
        range,
        value: ast::TStringValue::single(tstring),
        runtime_template_str,
        runtime_values,
    }))
}

pub(super) fn interpolation_to_expr(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    interpolation: TStringInterpolation,
) -> PyResult<ast::Expr> {
    let range = interpolation.range;
    let part = TemplateStrPart::Interpolation(interpolation);
    let elements = template_parts_to_elements(vm, source_file, vec![part].into_boxed_slice())?;
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
        runtime_template_str: None,
        runtime_values: None,
    }))
}

fn template_parts_to_elements(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    values: Box<[TemplateStrPart]>,
) -> PyResult<ast::InterpolatedStringElements> {
    let mut elements = Vec::with_capacity(values.len());
    for value in values.into_vec() {
        elements.push(template_part_to_element(vm, source_file, value)?);
    }
    Ok(ast::InterpolatedStringElements::from(elements))
}

fn template_part_to_element(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    part: TemplateStrPart,
) -> PyResult<ast::InterpolatedStringElement> {
    match part {
        TemplateStrPart::Constant(constant) => {
            let ConstantLiteral::Str { value, .. } = constant.value else {
                return Err(vm.new_type_error("TemplateStr constant values must be strings"));
            };
            Ok(ast::InterpolatedStringElement::Literal(
                ast::InterpolatedStringLiteralElement {
                    range: constant.range,
                    node_index: Default::default(),
                    value: value.to_string_lossy().into(),
                },
            ))
        }
        TemplateStrPart::Interpolation(interpolation) => {
            let TStringInterpolation {
                value,
                str,
                conversion,
                format_spec,
                range,
            } = interpolation;
            let str_constant =
                super::constant::constant_object_to_constant_data(vm, source_file, str)?;
            let runtime_str = Some(super::constant::constant_data_to_ast_constant_value(
                str_constant,
            ));
            let runtime_interpolation_format_spec = format_spec.clone();
            let format_spec = format_spec_expr_to_ruff_format_spec(format_spec);
            Ok(ast::InterpolatedStringElement::Interpolation(
                ast::InterpolatedElement {
                    range,
                    node_index: Default::default(),
                    expression: value,
                    debug_text: None,
                    conversion,
                    format_spec,
                    runtime_str,
                    runtime_interpolation_format_spec,
                    runtime_formatted_value_format_spec: None,
                },
            ))
        }
    }
}

// constructor
pub(super) fn template_str_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: &PyObject,
    range: TextRange,
) -> PyResult<TemplateStr> {
    let values: Vec<Option<ast::Expr>> =
        get_node_list_field(vm, source_file, object, "values", "TemplateStr")?;
    Ok(TemplateStr {
        values: Vec::new().into_boxed_slice(),
        runtime_values: Some(values),
        range,
    })
}

impl Node for TemplateStr {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            values,
            runtime_values,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprTemplateStr::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        let values = if let Some(runtime_values) = runtime_values {
            BoxedSlice(runtime_values.into_boxed_slice()).ast_to_object(vm, source_file)
        } else {
            BoxedSlice(values).ast_to_object(vm, source_file)
        };
        dict.set_item("values", values, vm).unwrap();
        node_add_location(&dict, range, vm, source_file);
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, &object, "TemplateStr")?;
        template_str_from_object_with_range(vm, source_file, &object, range)
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
        if is_node_instance(vm, &object, pyast::NodeExprInterpolation::static_type())? {
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
    str: PyObjectRef,
    conversion: ast::ConversionFlag,
    format_spec: Option<Box<ast::Expr>>,
    range: TextRange,
}

// constructor
pub(super) fn tstring_interpolation_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: &PyObject,
    range: TextRange,
) -> PyResult<TStringInterpolation> {
    let value = get_required_node_field(vm, source_file, object, "value", "Interpolation")?;
    let str = get_node_field(vm, object, "str", "Interpolation")?;
    let conversion = Node::ast_from_object(
        vm,
        source_file,
        get_node_field(vm, object, "conversion", "Interpolation")?,
    )?;
    let format_spec: Option<Box<ast::Expr>> = get_node_field_opt(vm, object, "format_spec")?
        .map(|obj| Node::ast_from_object(vm, source_file, obj))
        .transpose()?;
    Ok(TStringInterpolation {
        value,
        str,
        conversion,
        format_spec,
        range,
    })
}

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
        dict.set_item("str", str, vm).unwrap();
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
        let range = range_from_object(vm, source_file, &object, "Interpolation")?;
        tstring_interpolation_from_object_with_range(vm, source_file, &object, range)
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
        runtime_template_str,
        runtime_values,
    } = expression;
    if let Some(template_str) = runtime_template_str {
        return TemplateStr {
            range,
            values: Vec::new().into_boxed_slice(),
            runtime_values: Some(template_str.into_iter().map(Some).collect()),
        }
        .ast_to_object(vm, source_file);
    }

    if let Some(values) = runtime_values {
        return TemplateStr {
            range,
            values: Vec::new().into_boxed_slice(),
            runtime_values: Some(values),
        }
        .ast_to_object(vm, source_file);
    }

    if let [tstring] = value.as_slice()
        && let Some(ast::InterpolatedStringElement::Interpolation(interp)) =
            tstring.elements.iter().next()
        && tstring.elements.get(1).is_none()
        && let Some((str, format_spec)) = super::constant::runtime_interpolation_object(
            vm,
            interp.runtime_str.clone(),
            interp.runtime_interpolation_format_spec.clone(),
        )
        && let Some(interpolation) =
            standalone_tstring_interpolation_to_object(vm, source_file, &value, str, format_spec)
    {
        return interpolation;
    }

    let default_tstring = ast::TString {
        node_index: Default::default(),
        range: Default::default(),
        elements: Default::default(),
        flags: ast::TStringFlags::empty(),
    };
    let mut values = Vec::new();
    for i in 0..value.as_slice().len() {
        let tstring = core::mem::replace(value.iter_mut().nth(i).unwrap(), default_tstring.clone());
        let flags = tstring.flags.into();
        for element in ruff_fstring_element_into_iter(tstring.elements) {
            push_ruff_tstring_element(vm, source_file, flags, element, &mut values);
        }
    }
    let values = normalize_template_str_parts(values);
    let c = TemplateStr {
        range,
        values: values.into_boxed_slice(),
        runtime_values: None,
    };
    c.ast_to_object(vm, source_file)
}

fn standalone_tstring_interpolation_to_object(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    value: &ast::TStringValue,
    str: PyObjectRef,
    format_spec: Option<Box<ast::Expr>>,
) -> Option<PyObjectRef> {
    let [tstring] = value.as_slice() else {
        return None;
    };
    let mut elements = tstring.elements.iter();
    let ast::InterpolatedStringElement::Interpolation(interp) = elements.next()? else {
        return None;
    };
    if elements.next().is_some() {
        return None;
    }
    let interpolation = TStringInterpolation {
        value: interp.expression.clone(),
        str,
        conversion: interp.conversion,
        format_spec: format_spec.or_else(|| {
            ruff_format_spec_to_joined_str(
                vm,
                source_file,
                ast::TStringFlags::empty().into(),
                interp.format_spec.clone(),
            )
            .map(|joined_str| Box::new(joined_str.into_expr(false)))
        }),
        range: interp.range,
    };
    Some(interpolation.ast_to_object(vm, source_file))
}

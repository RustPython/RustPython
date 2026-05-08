use alloc::{boxed::Box, string::String, vec::Vec};

use ruff_python_ast::{
    self as ast, AtomicNodeIndex, ConversionFlag, Expr, ExprFString, FString, FStringFlags,
    FStringValue, HasNodeIndex, InterpolatedElement, InterpolatedStringElement,
    InterpolatedStringElements, InterpolatedStringFormatSpec, InterpolatedStringLiteralElement,
    Operator,
    visitor::transformer::{self, Transformer},
};
use ruff_text_size::{Ranged, TextRange};

const MAXDIGITS: usize = 3;
const F_LJUST: u8 = 1;

pub(crate) fn preprocess_mod(module: &mut ast::Mod) {
    let preprocessor = AstPreprocessor;
    match module {
        ast::Mod::Module(module) => preprocessor.visit_body(&mut module.body),
        ast::Mod::Expression(expr) => preprocessor.visit_expr(&mut expr.body),
    }
}

struct AstPreprocessor;

impl Transformer for AstPreprocessor {
    fn visit_expr(&self, expr: &mut Expr) {
        transformer::walk_expr(self, expr);
        if let Some(optimized) = optimize_format(expr) {
            *expr = optimized;
        }
    }
}

fn optimize_format(expr: &Expr) -> Option<Expr> {
    let Expr::BinOp(binop) = expr else {
        return None;
    };
    if !matches!(binop.op, Operator::Mod) {
        return None;
    }
    let Expr::StringLiteral(format) = binop.left.as_ref() else {
        return None;
    };
    let Expr::Tuple(tuple) = binop.right.as_ref() else {
        return None;
    };
    if tuple
        .elts
        .iter()
        .any(|expr| matches!(expr, Expr::Starred(_)))
    {
        return None;
    }

    let elements = parse_format(format.value.to_str(), &tuple.elts)?;
    Some(Expr::FString(ExprFString {
        node_index: binop.node_index.clone(),
        range: binop.range,
        value: FStringValue::single(FString {
            range: binop.range,
            node_index: binop.node_index.clone(),
            elements: InterpolatedStringElements::from(elements),
            flags: FStringFlags::empty(),
        }),
    }))
}

fn parse_format(format: &str, args: &[Expr]) -> Option<Vec<InterpolatedStringElement>> {
    let chars: Vec<char> = format.chars().collect();
    let mut elements = Vec::with_capacity(args.len().saturating_mul(2).saturating_add(1));
    let mut pos = 0;
    let mut arg_idx = 0;

    loop {
        if let Some(literal) = parse_literal(&chars, &mut pos) {
            elements.push(literal.into());
        }
        if pos >= chars.len() {
            break;
        }
        if arg_idx >= args.len() {
            return None;
        }
        debug_assert_eq!(chars[pos], '%');
        pos += 1;
        let formatted = parse_format_arg(&chars, &mut pos, args[arg_idx].clone())?;
        elements.push(formatted.into());
        arg_idx += 1;
    }

    (arg_idx == args.len()).then_some(elements)
}

fn parse_literal(chars: &[char], pos: &mut usize) -> Option<InterpolatedStringLiteralElement> {
    let start = *pos;
    let mut has_percents = false;
    while *pos < chars.len() {
        if chars[*pos] != '%' {
            *pos += 1;
        } else if *pos + 1 < chars.len() && chars[*pos + 1] == '%' {
            has_percents = true;
            *pos += 2;
        } else {
            break;
        }
    }
    if *pos == start {
        return None;
    }

    let mut value = String::new();
    let mut i = start;
    while i < *pos {
        if has_percents && chars[i] == '%' && i + 1 < *pos && chars[i + 1] == '%' {
            value.push('%');
            i += 2;
        } else {
            value.push(chars[i]);
            i += 1;
        }
    }

    Some(generated_literal(value))
}

fn parse_format_arg(chars: &[char], pos: &mut usize, arg: Expr) -> Option<InterpolatedElement> {
    let (spec, flags, width, precision) = simple_format_arg_parse(chars, pos)?;
    let conversion = match spec {
        's' => ConversionFlag::Str,
        'r' => ConversionFlag::Repr,
        'a' => ConversionFlag::Ascii,
        _ => return None,
    };

    let mut format_spec = String::new();
    if flags & F_LJUST == 0
        && let Some(width) = width
        && width > 0
    {
        format_spec.push('>');
    }
    if let Some(width) = width {
        format_spec.push_str(&width.to_string());
    }
    if let Some(precision) = precision {
        format_spec.push('.');
        format_spec.push_str(&precision.to_string());
    }

    let range = arg.range();
    let format_spec = (!format_spec.is_empty()).then(|| {
        Box::new(InterpolatedStringFormatSpec {
            range: TextRange::default(),
            node_index: AtomicNodeIndex::NONE,
            elements: InterpolatedStringElements::from(vec![generated_literal(format_spec).into()]),
        })
    });

    Some(InterpolatedElement {
        range,
        node_index: arg.node_index().clone(),
        expression: Box::new(arg),
        debug_text: None,
        conversion,
        format_spec,
    })
}

fn simple_format_arg_parse(
    chars: &[char],
    pos: &mut usize,
) -> Option<(char, u8, Option<u16>, Option<u16>)> {
    let mut flags = 0;
    let mut ch = next_char(chars, pos)?;
    loop {
        match ch {
            '-' => flags |= F_LJUST,
            '+' | ' ' | '#' | '0' => {}
            _ => break,
        }
        ch = next_char(chars, pos)?;
    }

    let width = parse_digits(chars, pos, &mut ch)?;
    let precision = if ch == '.' {
        ch = next_char(chars, pos)?;
        parse_digits(chars, pos, &mut ch)?
    } else {
        None
    };

    Some((ch, flags, width, precision))
}

fn parse_digits(chars: &[char], pos: &mut usize, ch: &mut char) -> Option<Option<u16>> {
    if !ch.is_ascii_digit() {
        return Some(None);
    }

    let mut value = 0u16;
    let mut digits = 0usize;
    while ch.is_ascii_digit() {
        value = value * 10 + (*ch as u16 - b'0' as u16);
        *ch = next_char(chars, pos)?;
        digits += 1;
        if digits >= MAXDIGITS {
            return None;
        }
    }
    Some(Some(value))
}

fn next_char(chars: &[char], pos: &mut usize) -> Option<char> {
    let ch = chars.get(*pos).copied()?;
    *pos += 1;
    Some(ch)
}

fn generated_literal(value: String) -> InterpolatedStringLiteralElement {
    InterpolatedStringLiteralElement {
        range: TextRange::default(),
        node_index: AtomicNodeIndex::NONE,
        value: value.into_boxed_str(),
    }
}

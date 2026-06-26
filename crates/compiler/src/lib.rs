pub use ruff_python_ast::token::TokenKind;
use ruff_python_parser::ParseErrorType;
use ruff_source_file::{PositionEncoding, SourceFile, SourceFileBuilder, SourceLocation};
use ruff_text_size::{Ranged, TextSize, TextSlice};
use rustpython_codegen::{compile, symboltable};
use thiserror::Error;

pub use rustpython_codegen::compile::CompileOpts;
pub use rustpython_compiler_core::{Mode, bytecode::CodeObject};

// these modules are out of repository. re-exporting them here for convenience.
pub use ruff_python_ast as ast;
pub use ruff_python_parser as parser;
pub use rustpython_codegen as codegen;
pub use rustpython_compiler_core as core;

#[derive(Error, Debug)]
pub enum CompileErrorType {
    #[error(transparent)]
    Codegen(#[from] codegen::error::CodegenErrorType),
    #[error(transparent)]
    Parse(#[from] ParseErrorType),
}

#[derive(Error, Debug)]
pub struct ParseError {
    #[source]
    pub error: ParseErrorType,
    pub raw_location: ruff_text_size::TextRange,
    pub location: SourceLocation,
    pub end_location: SourceLocation,
    pub source_path: String,
    /// Set when the error is an unclosed bracket (converted from EOF).
    pub is_unclosed_bracket: bool,
}

impl ::core::fmt::Display for ParseError {
    fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
        self.error.fmt(f)
    }
}

#[derive(Error, Debug)]
pub enum CompileError {
    #[error(transparent)]
    Codegen(#[from] codegen::error::CodegenError),
    #[error(transparent)]
    Parse(#[from] ParseError),
}

impl CompileError {
    #[must_use]
    pub fn from_ruff_parse_error(error: parser::ParseError, source_file: &SourceFile) -> Self {
        let raw_location = error.location;
        let diagnostic = match cpython_parse_diagnostic_override(&error, source_file) {
            Some(diagnostic) => diagnostic,
            None => default_parse_diagnostic(error, source_file),
        };

        Self::Parse(ParseError {
            error: diagnostic.error,
            raw_location,
            location: diagnostic.location,
            end_location: diagnostic.end_location,
            source_path: source_file.name().to_owned(),
            is_unclosed_bracket: diagnostic.is_unclosed_bracket,
        })
    }

    fn from_source_error(
        source_file: &SourceFile,
        message: String,
        start: usize,
        end: usize,
    ) -> Self {
        let start = TextSize::new(start as u32);
        let end = TextSize::new(end as u32);
        let (location, end_location) = source_locations(source_file, start, end);
        Self::Parse(ParseError {
            error: parser::ParseErrorType::OtherError(message),
            raw_location: ruff_text_size::TextRange::new(start, end),
            location,
            end_location,
            source_path: source_file.name().to_owned(),
            is_unclosed_bracket: false,
        })
    }

    #[must_use]
    pub const fn location(&self) -> Option<SourceLocation> {
        match self {
            Self::Codegen(codegen_error) => codegen_error.location,
            Self::Parse(parse_error) => Some(parse_error.location),
        }
    }

    #[must_use]
    pub const fn python_location(&self) -> (usize, usize) {
        if let Some(location) = self.location() {
            (location.line.get(), location.character_offset.get())
        } else {
            (0, 0)
        }
    }

    #[must_use]
    pub fn python_end_location(&self) -> Option<(usize, usize)> {
        match self {
            Self::Codegen(_) => None,
            Self::Parse(parse_error) => Some((
                parse_error.end_location.line.get(),
                parse_error.end_location.character_offset.get(),
            )),
        }
    }

    #[must_use]
    pub fn source_path(&self) -> &str {
        match self {
            Self::Codegen(codegen_error) => &codegen_error.source_path,
            Self::Parse(parse_error) => &parse_error.source_path,
        }
    }
}

fn source_location(source_file: &SourceFile, offset: TextSize) -> SourceLocation {
    source_file
        .to_source_code()
        .source_location(offset, PositionEncoding::Utf8)
}

fn source_locations(
    source_file: &SourceFile,
    start: TextSize,
    end: TextSize,
) -> (SourceLocation, SourceLocation) {
    let source_code = source_file.to_source_code();
    (
        source_code.source_location(start, PositionEncoding::Utf8),
        source_code.source_location(end, PositionEncoding::Utf8),
    )
}

struct NormalizedParseDiagnostic {
    error: parser::ParseErrorType,
    location: SourceLocation,
    end_location: SourceLocation,
    is_unclosed_bracket: bool,
}

impl NormalizedParseDiagnostic {
    const fn new(
        error: parser::ParseErrorType,
        location: SourceLocation,
        end_location: SourceLocation,
    ) -> Self {
        Self {
            error,
            location,
            end_location,
            is_unclosed_bracket: false,
        }
    }

    fn other(source_file: &SourceFile, message: String, start: usize, end: usize) -> Self {
        let (location, end_location) = source_locations(
            source_file,
            TextSize::new(start as u32),
            TextSize::new(end as u32),
        );
        Self::new(
            parser::ParseErrorType::OtherError(message),
            location,
            end_location,
        )
    }

    const fn with_unclosed_bracket(mut self, is_unclosed_bracket: bool) -> Self {
        self.is_unclosed_bracket = is_unclosed_bracket;
        self
    }
}

fn cpython_parse_diagnostic_override(
    error: &parser::ParseError,
    source_file: &SourceFile,
) -> Option<NormalizedParseDiagnostic> {
    let source_text = source_file.source_text();

    macro_rules! source_error {
        ($expr:expr) => {
            if let Some((message, start, end)) = $expr {
                return Some(NormalizedParseDiagnostic::other(
                    source_file,
                    message,
                    start,
                    end,
                ));
            }
        };
    }

    if let Some((message, offset)) = invalid_number_literal_error(source_text) {
        return Some(NormalizedParseDiagnostic::other(
            source_file,
            message,
            offset,
            offset,
        ));
    }
    source_error!(invalid_legacy_statement_error(source_text));
    source_error!(non_printable_character_error(source_text));
    source_error!(invalid_interpolated_string_error(source_text));

    if let Some((message, start, end, unclosed)) = bracket_syntax_error(source_text) {
        return Some(
            NormalizedParseDiagnostic::other(source_file, message, start, end)
                .with_unclosed_bracket(unclosed),
        );
    }

    if matches!(
        &error.error,
        parser::ParseErrorType::Lexical(parser::LexicalErrorType::LineContinuationError)
    ) {
        let loc = source_location(source_file, error.location.start() + TextSize::from(1));
        return Some(NormalizedParseDiagnostic::new(
            error.error.clone(),
            loc,
            loc,
        ));
    }

    source_error!(unterminated_string_error(source_text));
    source_error!(expected_indented_block_error(error, source_text));

    if matches!(
        &error.error,
        parser::ParseErrorType::Lexical(parser::LexicalErrorType::Eof)
    ) {
        return Some(eof_parse_diagnostic(error, source_file));
    }

    source_error!(invalid_type_param_error(source_text));
    source_error!(invalid_comprehension_error(source_text));
    source_error!(invalid_parameter_star_annotation_error(source_text));
    source_error!(invalid_parameter_list_error(source_text));
    source_error!(invalid_call_argument_error(source_text));

    if is_missing_comma_between_literals(error) {
        let (loc, end_loc) = adjusted_error_locations(source_file, error.location);
        let msg = "invalid syntax. Perhaps you forgot a comma?".into();
        return Some(NormalizedParseDiagnostic::new(
            parser::ParseErrorType::OtherError(msg),
            loc,
            end_loc,
        ));
    }

    source_error!(invalid_dict_error(source_text));
    source_error!(invalid_collection_assignment_error(source_text));
    source_error!(invalid_group_error(source_text));
    source_error!(invalid_def_type_params_error(source_text));
    source_error!(invalid_expression_error(source_text));
    source_error!(invalid_named_expression_error(source_text));
    source_error!(invalid_plain_assignment_error(source_text));
    source_error!(expression_assignment_error(source_text));
    source_error!(invalid_annotation_target_error(source_text));
    source_error!(invalid_assignment_target_error(source_text));
    source_error!(invalid_augassign_target_error(source_text));
    source_error!(invalid_for_target_error(source_text));
    source_error!(invalid_with_target_error(source_text));
    source_error!(invalid_delete_target_error(source_text));
    source_error!(invalid_standalone_except_error(source_text));
    source_error!(invalid_import_statement_error(source_text));
    source_error!(invalid_import_target_error(source_text));
    source_error!(invalid_except_as_target_error(source_text));
    source_error!(invalid_match_mapping_rest_wildcard_error(source_text));
    source_error!(invalid_match_as_target_error(source_text));
    source_error!(invalid_for_if_clause_error(source_text));
    source_error!(invalid_if_expression_statement_error(source_text));
    source_error!(invalid_else_elif_error(source_text));
    source_error!(mixed_except_handlers_error(source_text));

    if matches!(
        &error.error,
        parser::ParseErrorType::Lexical(parser::LexicalErrorType::IndentationError)
    ) {
        let end_loc = source_line_end_location(source_file, error.location.start());
        return Some(NormalizedParseDiagnostic::new(
            error.error.clone(),
            end_loc,
            end_loc,
        ));
    }

    if matches!(
        &error.error,
        parser::ParseErrorType::InvalidAssignmentTarget
    ) {
        return Some(invalid_assignment_target_diagnostic(error, source_file));
    }

    if matches!(
        &error.error,
        parser::ParseErrorType::InvalidNamedAssignmentTarget
    ) {
        let (loc, end_loc) = adjusted_error_locations(source_file, error.location);
        let target = source_file.source_text().slice(error.location);
        let msg = format!("cannot use assignment expressions with {target}");
        return Some(NormalizedParseDiagnostic::new(
            parser::ParseErrorType::OtherError(msg),
            loc,
            end_loc,
        ));
    }

    None
}

fn eof_parse_diagnostic(
    error: &parser::ParseError,
    source_file: &SourceFile,
) -> NormalizedParseDiagnostic {
    let source_text = source_file.source_text();
    if let Some((bracket_char, bracket_offset)) = find_unclosed_bracket(source_text) {
        let loc = source_location(source_file, TextSize::new(bracket_offset as u32));
        let end_loc = SourceLocation {
            line: loc.line,
            character_offset: loc.character_offset.saturating_add(1),
        };
        let msg = format!("'{bracket_char}' was never closed");
        NormalizedParseDiagnostic::new(parser::ParseErrorType::OtherError(msg), loc, end_loc)
            .with_unclosed_bracket(true)
    } else {
        let end_loc = source_line_end_location(source_file, error.location.start());
        NormalizedParseDiagnostic::new(error.error.clone(), end_loc, end_loc)
    }
}

fn invalid_assignment_target_diagnostic(
    error: &parser::ParseError,
    source_file: &SourceFile,
) -> NormalizedParseDiagnostic {
    let (loc, end_loc) = adjusted_error_locations(source_file, error.location);
    let expr_str = source_file.source_text().slice(error.location);

    let msg = parser::parse_expression(expr_str).map_or_else(
        |_| match expr_str {
            "yield" => "assignment to yield expression not possible".into(),
            _ => format!("cannot assign to {expr_str}"),
        },
        |parsed| match *parsed.syntax().body {
            ast::Expr::Call(_) => "cannot assign to function call".into(),
            ast::Expr::BinOp(_) => "cannot assign to expression".into(),
            ast::Expr::If(_) => "cannot assign to conditional expression".into(),
            ast::Expr::Generator(_) => "cannot assign to generator expression".into(),
            ast::Expr::FString(_) => "invalid syntax".into(),
            ast::Expr::StringLiteral(_)
            | ast::Expr::BytesLiteral(_)
            | ast::Expr::NumberLiteral(_) => {
                "cannot assign to literal here. Maybe you meant '==' instead of '='?".into()
            }
            ast::Expr::EllipsisLiteral(_) => {
                "cannot assign to ellipsis here. Maybe you meant '==' instead of '='?".into()
            }
            _ => format!("cannot assign to {expr_str}"),
        },
    );

    NormalizedParseDiagnostic::new(parser::ParseErrorType::OtherError(msg), loc, end_loc)
}

fn default_parse_diagnostic(
    error: parser::ParseError,
    source_file: &SourceFile,
) -> NormalizedParseDiagnostic {
    let (loc, end_loc) = adjusted_error_locations(source_file, error.location);
    NormalizedParseDiagnostic::new(error.error, loc, end_loc)
}

fn adjusted_error_locations(
    source_file: &SourceFile,
    range: ruff_text_size::TextRange,
) -> (SourceLocation, SourceLocation) {
    let mut locations = source_locations(source_file, range.start(), range.end());
    if locations.1.character_offset.get() == 1 && locations.1.line > locations.0.line {
        locations.1 = source_location(source_file, range.end() - TextSize::from(1));
        locations.1.character_offset = locations.1.character_offset.saturating_add(1);
    }
    locations
}

fn source_line_end_location(source_file: &SourceFile, offset: TextSize) -> SourceLocation {
    let loc = source_location(source_file, offset);
    let line_idx = loc.line.to_zero_indexed();
    let line = source_file
        .source_text()
        .split('\n')
        .nth(line_idx)
        .unwrap_or("");
    let line_end_col = line.chars().count() + 1;
    SourceLocation {
        line: loc.line,
        character_offset: ruff_source_file::OneIndexed::new(line_end_col)
            .unwrap_or(loc.character_offset),
    }
}

fn is_missing_comma_between_literals(error: &parser::ParseError) -> bool {
    matches!(
        &error.error,
        parser::ParseErrorType::ExpectedToken { expected, found }
            if matches!((expected, found), (TokenKind::Comma, TokenKind::Int))
    )
}

fn is_ascii_identifier_char(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphanumeric()
}

fn numeric_keyword_suffix(rest: &[u8]) -> bool {
    rest.starts_with(b"and")
        || rest.starts_with(b"else")
        || rest.starts_with(b"for")
        || rest.starts_with(b"if")
        || rest.starts_with(b"in")
        || rest.starts_with(b"is")
        || rest.starts_with(b"or")
        || rest.starts_with(b"not")
}

fn consume_decimal_digits(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() {
        match bytes[index] {
            b'0'..=b'9' => index += 1,
            b'_' if bytes
                .get(index + 1)
                .is_some_and(|byte| byte.is_ascii_digit()) =>
            {
                index += 2;
            }
            _ => break,
        }
    }
    index
}

fn consume_radix_digits(bytes: &[u8], mut index: usize, is_digit: impl Fn(u8) -> bool) -> usize {
    while index < bytes.len() {
        if is_digit(bytes[index]) {
            index += 1;
        } else if bytes.get(index) == Some(&b'_')
            && bytes.get(index + 1).is_some_and(|&byte| is_digit(byte))
        {
            index += 2;
        } else {
            break;
        }
    }
    index
}

fn invalid_radix_literal_error(
    bytes: &[u8],
    start: usize,
    kind: &'static str,
    is_digit: impl Fn(u8) -> bool,
) -> Option<(String, usize)> {
    let mut index = start + 2;
    let mut has_digit = false;
    loop {
        let Some(&byte) = bytes.get(index) else {
            return Some((format!("invalid {kind} literal"), start + 1));
        };
        if byte == b'_' {
            let Some(&next) = bytes.get(index + 1) else {
                return Some((format!("invalid {kind} literal"), index));
            };
            if is_digit(next) {
                has_digit = true;
                index += 2;
                continue;
            }
            if next.is_ascii_digit() && matches!(kind, "binary" | "octal") {
                return Some((
                    format!("invalid digit '{}' in {kind} literal", next as char),
                    index + 1,
                ));
            }
            return Some((format!("invalid {kind} literal"), index));
        }
        if is_digit(byte) {
            has_digit = true;
            index += 1;
            continue;
        }
        if byte.is_ascii_digit() && matches!(kind, "binary" | "octal") {
            return Some((
                format!("invalid digit '{}' in {kind} literal", byte as char),
                index,
            ));
        }
        if has_digit {
            return None;
        }
        return Some((format!("invalid {kind} literal"), start + 1));
    }
}

fn decimal_tail_error(bytes: &[u8], mut index: usize) -> Option<usize> {
    loop {
        while bytes.get(index).is_some_and(|byte| byte.is_ascii_digit()) {
            index += 1;
        }
        if bytes.get(index) != Some(&b'_') {
            return None;
        }
        let underscore = index;
        index += 1;
        if !bytes.get(index).is_some_and(|byte| byte.is_ascii_digit()) {
            return Some(underscore);
        }
    }
}

fn decimal_tail_end(bytes: &[u8], mut index: usize) -> usize {
    loop {
        while bytes.get(index).is_some_and(|byte| byte.is_ascii_digit()) {
            index += 1;
        }
        if bytes.get(index) == Some(&b'_')
            && bytes
                .get(index + 1)
                .is_some_and(|byte| byte.is_ascii_digit())
        {
            index += 2;
        } else {
            return index;
        }
    }
}

fn invalid_decimal_literal_error(bytes: &[u8], start: usize) -> Option<(String, usize)> {
    if bytes.get(start) == Some(&b'.') {
        return None;
    }
    let message = "invalid decimal literal".to_owned();
    if let Some(offset) = decimal_tail_error(bytes, start) {
        return Some((message, offset));
    }

    let mut index = decimal_tail_end(bytes, start);
    if bytes.get(index) == Some(&b'.') {
        if bytes.get(index + 1) == Some(&b'_') {
            return Some((message, index));
        }
        if let Some(offset) = decimal_tail_error(bytes, index + 1) {
            return Some((message, offset));
        }
        index = decimal_tail_end(bytes, index + 1);
    }
    if matches!(bytes.get(index), Some(b'e' | b'E')) {
        let exponent = index;
        index += 1;
        let sign = if matches!(bytes.get(index), Some(b'+' | b'-')) {
            let sign = index;
            index += 1;
            Some(sign)
        } else {
            None
        };
        if !bytes.get(index).is_some_and(|byte| byte.is_ascii_digit()) {
            return Some((message, sign.unwrap_or(exponent)));
        }
        if let Some(offset) = decimal_tail_error(bytes, index) {
            return Some((message, offset));
        }
    }
    None
}

fn leading_zero_decimal_literal_error(bytes: &[u8], start: usize) -> Option<(String, usize)> {
    if bytes.get(start) != Some(&b'0') {
        return None;
    }
    let mut index = start;
    loop {
        match bytes.get(index) {
            Some(b'0') => index += 1,
            Some(b'_')
                if bytes
                    .get(index + 1)
                    .is_some_and(|byte| byte.is_ascii_digit()) =>
            {
                index += 1;
            }
            _ => break,
        }
    }
    if bytes.get(index).is_some_and(|byte| byte.is_ascii_digit()) {
        let after_digits = decimal_tail_end(bytes, index);
        if !matches!(
            bytes.get(after_digits),
            Some(b'.' | b'e' | b'E' | b'j' | b'J')
        ) {
            return Some((
                "leading zeros in decimal integer literals are not permitted; use an 0o prefix for octal integers".to_owned(),
                start,
            ));
        }
    }
    None
}

fn invalid_numeric_literal_error(bytes: &[u8], start: usize) -> Option<(String, usize)> {
    if bytes.get(start) == Some(&b'0') {
        match bytes.get(start + 1) {
            Some(b'x' | b'X') => {
                return invalid_radix_literal_error(bytes, start, "hexadecimal", |byte| {
                    byte.is_ascii_hexdigit()
                });
            }
            Some(b'o' | b'O') => {
                return invalid_radix_literal_error(bytes, start, "octal", |byte| {
                    matches!(byte, b'0'..=b'7')
                });
            }
            Some(b'b' | b'B') => {
                return invalid_radix_literal_error(bytes, start, "binary", |byte| {
                    matches!(byte, b'0' | b'1')
                });
            }
            _ => {}
        }
        if let Some(err) = leading_zero_decimal_literal_error(bytes, start) {
            return Some(err);
        }
    }
    invalid_decimal_literal_error(bytes, start)
}

fn consume_exponent(bytes: &[u8], index: usize) -> usize {
    if !matches!(bytes.get(index), Some(b'e' | b'E')) {
        return index;
    }
    let mut cursor = index + 1;
    if matches!(bytes.get(cursor), Some(b'+' | b'-')) {
        cursor += 1;
    }
    if bytes.get(cursor).is_some_and(|byte| byte.is_ascii_digit()) {
        consume_decimal_digits(bytes, cursor)
    } else {
        index
    }
}

fn number_literal_end(bytes: &[u8], start: usize) -> Option<(&'static str, usize)> {
    if bytes.get(start) == Some(&b'.') {
        if !bytes
            .get(start + 1)
            .is_some_and(|byte| byte.is_ascii_digit())
        {
            return None;
        }
        let mut index = consume_decimal_digits(bytes, start + 1);
        index = consume_exponent(bytes, index);
        if matches!(bytes.get(index), Some(b'j' | b'J')) {
            return Some(("imaginary", index + 1));
        }
        return Some(("decimal", index));
    }

    if !bytes.get(start).is_some_and(|byte| byte.is_ascii_digit()) {
        return None;
    }

    if bytes.get(start) == Some(&b'0') {
        match bytes.get(start + 1) {
            Some(b'x' | b'X') => {
                let end = consume_radix_digits(bytes, start + 2, |byte| byte.is_ascii_hexdigit());
                return Some(("hexadecimal", end));
            }
            Some(b'o' | b'O') => {
                let end =
                    consume_radix_digits(bytes, start + 2, |byte| matches!(byte, b'0'..=b'7'));
                return Some(("octal", end));
            }
            Some(b'b' | b'B') => {
                let end =
                    consume_radix_digits(bytes, start + 2, |byte| matches!(byte, b'0' | b'1'));
                return Some(("binary", end));
            }
            _ => {}
        }
    }

    let mut index = consume_decimal_digits(bytes, start);
    if bytes.get(index) == Some(&b'.') {
        index = consume_decimal_digits(bytes, index + 1);
    }
    index = consume_exponent(bytes, index);
    if matches!(bytes.get(index), Some(b'j' | b'J')) {
        return Some(("imaginary", index + 1));
    }
    Some(("decimal", index))
}

fn skip_quoted_string(bytes: &[u8], mut index: usize) -> usize {
    let quote = bytes[index];
    let triple = bytes.get(index + 1) == Some(&quote) && bytes.get(index + 2) == Some(&quote);
    let quote_len = if triple { 3 } else { 1 };
    index += quote_len;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index = (index + 2).min(bytes.len());
        } else if triple
            && bytes.get(index) == Some(&quote)
            && bytes.get(index + 1) == Some(&quote)
            && bytes.get(index + 2) == Some(&quote)
        {
            return index + 3;
        } else if !triple && bytes[index] == quote {
            return index + 1;
        } else {
            index += 1;
        }
    }
    index
}

fn invalid_number_literal_error(source: &str) -> Option<(String, usize)> {
    let bytes = source.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'#' => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'\'' | b'"' => {
                index = skip_quoted_string(bytes, index);
            }
            byte if byte >= 0x80 || byte == b'_' || byte.is_ascii_alphabetic() => {
                index += 1;
                while index < bytes.len()
                    && (bytes[index] >= 0x80 || is_ascii_identifier_char(bytes[index]))
                {
                    index += 1;
                }
            }
            b'.' | b'0'..=b'9' => {
                if let Some(err) = invalid_numeric_literal_error(bytes, index) {
                    return Some(err);
                }
                let Some((kind, end)) = number_literal_end(bytes, index) else {
                    index += 1;
                    continue;
                };
                if end > index {
                    if source[end..].starts_with('⁄') {
                        return Some(("invalid character '⁄' (U+2044)".to_owned(), end));
                    }
                    if bytes
                        .get(end)
                        .is_some_and(|byte| *byte < 128 && is_ascii_identifier_char(*byte))
                        && !numeric_keyword_suffix(&bytes[end..])
                    {
                        return Some((format!("invalid {kind} literal"), end.saturating_sub(1)));
                    }
                }
                index = end.max(index + 1);
            }
            _ => index += 1,
        }
    }
    None
}

fn cpython_indented_block_clause(message: &str) -> Option<&'static str> {
    let clause = message.strip_prefix("Expected an indented block after ")?;
    Some(match clause {
        "`if` statement" => "'if' statement",
        "`elif` clause" => "'elif' statement",
        "`else` clause" => "'else' statement",
        "`for` statement" => "'for' statement",
        "`with` statement" => "'with' statement",
        "`while` statement" => "'while' statement",
        "`try` statement" => "'try' statement",
        "`except` clause" => "'except' statement",
        "`finally` clause" => "'finally' statement",
        "`match` statement" => "'match' statement",
        "`case` block" => "'case' statement",
        "`class` definition" => "class definition",
        "function definition" => "function definition",
        _ => return None,
    })
}

fn previous_non_empty_line_number(source: &str, offset: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut index = offset.min(bytes.len());
    while index > 0 {
        let line_end = index;
        while index > 0 && bytes[index - 1] != b'\n' {
            index -= 1;
        }
        let line_start = index;
        let content_start = skip_horizontal_whitespace(bytes, line_start);
        let mut content_end = line_end;
        while content_end > content_start
            && matches!(
                bytes.get(content_end - 1),
                Some(b' ' | b'\t' | b'\r' | b'\x0c')
            )
        {
            content_end -= 1;
        }
        if content_start < content_end {
            return Some(
                source[..line_start]
                    .bytes()
                    .filter(|byte| *byte == b'\n')
                    .count()
                    + 1,
            );
        }
        index = line_start.saturating_sub(1);
    }
    None
}

fn expected_indented_block_error(
    error: &parser::ParseError,
    source: &str,
) -> Option<(String, usize, usize)> {
    let parser::ParseErrorType::OtherError(message) = &error.error else {
        return None;
    };
    let mut clause = cpython_indented_block_clause(message)?;
    let start = error.location.start().to_usize();
    let end = error.location.end().to_usize();
    let line = previous_non_empty_line_number(source, start)?;
    if clause == "'except' statement"
        && let Some(previous_line) = previous_non_empty_line(source, start)
        && matches!(
            previous_line.trim_start(),
            line if line.starts_with("except*") || line.starts_with("except *")
        )
    {
        clause = "'except*' statement";
    }
    Some((
        format!("expected an indented block after {clause} on line {line}"),
        start,
        end,
    ))
}

fn previous_non_empty_line(source: &str, offset: usize) -> Option<&str> {
    let bytes = source.as_bytes();
    let mut index = offset.min(bytes.len());
    while index > 0 {
        let line_end = index;
        while index > 0 && bytes[index - 1] != b'\n' {
            index -= 1;
        }
        let line_start = index;
        let mut content_start = line_start;
        while content_start < line_end
            && matches!(bytes[content_start], b' ' | b'\t' | b'\n' | b'\r' | b'\x0c')
        {
            content_start += 1;
        }
        let mut content_end = line_end;
        while content_end > content_start
            && matches!(bytes[content_end - 1], b' ' | b'\t' | b'\r' | b'\x0c')
        {
            content_end -= 1;
        }
        if content_start < content_end {
            return source.get(line_start..line_end);
        }
        index = line_start.saturating_sub(1);
    }
    None
}

fn starts_identifier(bytes: &[u8], index: usize, word: &[u8]) -> bool {
    bytes.get(index..index + word.len()) == Some(word)
        && index
            .checked_sub(1)
            .and_then(|before| bytes.get(before))
            .is_none_or(|byte| !is_ascii_identifier_char(*byte))
        && bytes
            .get(index + word.len())
            .is_none_or(|byte| !is_ascii_identifier_char(*byte))
}

fn is_plain_assignment_operator(bytes: &[u8], index: usize) -> bool {
    bytes.get(index) == Some(&b'=')
        && bytes.get(index + 1) != Some(&b'=')
        && !matches!(
            index.checked_sub(1).and_then(|before| bytes.get(before)),
            Some(b'=' | b'!' | b'<' | b'>' | b':')
        )
}

fn is_simple_keyword_name(bytes: &[u8], mut start: usize, mut end: usize) -> bool {
    while matches!(
        bytes.get(start),
        Some(b' ' | b'\t' | b'\n' | b'\r' | b'\x0c')
    ) {
        start += 1;
    }
    while end > start
        && matches!(
            bytes.get(end - 1),
            Some(b' ' | b'\t' | b'\n' | b'\r' | b'\x0c')
        )
    {
        end -= 1;
    }
    let Some(&first) = bytes.get(start) else {
        return false;
    };
    if !(first == b'_' || first.is_ascii_alphabetic() || first >= 0x80) {
        return false;
    }
    let mut index = start + 1;
    while index < end {
        if bytes[index] < 0x80 && !is_ascii_identifier_char(bytes[index]) {
            return false;
        }
        index += 1;
    }
    true
}

fn is_function_parameter_list(bytes: &[u8], paren: usize) -> bool {
    let mut cursor = paren;
    while cursor > 0 && matches!(bytes.get(cursor - 1), Some(b' ' | b'\t' | b'\x0c')) {
        cursor -= 1;
    }
    if cursor > 0 && bytes.get(cursor - 1) == Some(&b']') {
        let mut bracket = cursor;
        let mut level = 0usize;
        while bracket > 0 {
            bracket -= 1;
            match bytes[bracket] {
                b']' => level += 1,
                b'[' => {
                    level = level.saturating_sub(1);
                    if level == 0 {
                        cursor = bracket;
                        break;
                    }
                }
                _ => {}
            }
        }
        while cursor > 0 && matches!(bytes.get(cursor - 1), Some(b' ' | b'\t' | b'\x0c')) {
            cursor -= 1;
        }
    }
    while cursor > 0
        && bytes
            .get(cursor - 1)
            .is_some_and(|byte| *byte >= 0x80 || is_ascii_identifier_char(*byte))
    {
        cursor -= 1;
    }
    while cursor > 0 && matches!(bytes.get(cursor - 1), Some(b' ' | b'\t' | b'\x0c')) {
        cursor -= 1;
    }
    cursor >= 3
        && starts_identifier(bytes, cursor - 3, b"def")
        && cursor
            .checked_sub(4)
            .and_then(|before| bytes.get(before))
            .is_none_or(|byte| !is_ascii_identifier_char(*byte))
}

#[derive(Clone, Copy)]
enum ParameterListKind {
    Function,
    Lambda,
}

fn matching_delimiter(bytes: &[u8], open: usize, close: u8) -> Option<usize> {
    let mut index = open;
    let mut level = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'\'' | b'"' => index = skip_quoted_string(bytes, index),
            b'(' | b'[' | b'{' => {
                level += 1;
                index += 1;
            }
            byte if byte == close => {
                level = level.saturating_sub(1);
                if level == 0 {
                    return Some(index);
                }
                index += 1;
            }
            b')' | b']' | b'}' => {
                level = level.saturating_sub(1);
                index += 1;
            }
            _ => index += 1,
        }
    }
    None
}

fn find_lambda_parameter_end(bytes: &[u8], mut index: usize) -> Option<usize> {
    let mut level = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'#' if level == 0 => return None,
            b'\'' | b'"' => index = skip_quoted_string(bytes, index),
            b'(' | b'[' | b'{' => {
                level += 1;
                index += 1;
            }
            b')' | b']' | b'}' => {
                level = level.saturating_sub(1);
                index += 1;
            }
            b':' if level == 0 => return Some(index),
            _ => index += 1,
        }
    }
    None
}

fn top_level_byte(bytes: &[u8], mut index: usize, end: usize, needle: u8) -> Option<usize> {
    let mut level = 0usize;
    while index < end {
        match bytes[index] {
            b'\'' | b'"' => index = skip_quoted_string(bytes, index),
            byte if level == 0 && byte == needle => return Some(index),
            b'(' | b'[' | b'{' => {
                level += 1;
                index += 1;
            }
            b')' | b']' | b'}' => {
                level = level.saturating_sub(1);
                index += 1;
            }
            _ => index += 1,
        }
    }
    None
}

fn identifier_end(bytes: &[u8], mut index: usize, end: usize) -> usize {
    if !bytes
        .get(index)
        .is_some_and(|byte| *byte >= 0x80 || *byte == b'_' || byte.is_ascii_alphabetic())
    {
        return index;
    }
    index += 1;
    while index < end
        && bytes
            .get(index)
            .is_some_and(|byte| *byte >= 0x80 || is_ascii_identifier_char(*byte))
    {
        index += 1;
    }
    index
}

fn expression_slice_is_tuple(source: &str, start: usize, end: usize) -> bool {
    let bytes = source.as_bytes();
    let (start, end) = trim_target_range(bytes, start, end);
    if start >= end {
        return false;
    }
    let Ok(parsed) = parser::parse(&source[start..end], parser::Mode::Expression.into()) else {
        return false;
    };
    matches!(parsed.into_syntax(), ast::Mod::Expression(expression) if matches!(*expression.body, ast::Expr::Tuple(_)))
}

fn type_param_list_open(bytes: &[u8], open: usize) -> bool {
    let mut cursor = open;
    while cursor > 0 && matches!(bytes.get(cursor - 1), Some(b' ' | b'\t' | b'\x0c')) {
        cursor -= 1;
    }
    while cursor > 0
        && bytes
            .get(cursor - 1)
            .is_some_and(|byte| *byte >= 0x80 || is_ascii_identifier_char(*byte))
    {
        cursor -= 1;
    }
    while cursor > 0 && matches!(bytes.get(cursor - 1), Some(b' ' | b'\t' | b'\x0c')) {
        cursor -= 1;
    }
    (cursor >= 3 && starts_identifier(bytes, cursor - 3, b"def"))
        || (cursor >= 5 && starts_identifier(bytes, cursor - 5, b"class"))
        || (cursor >= 4 && starts_identifier(bytes, cursor - 4, b"type"))
}

fn invalid_type_param_item_error(
    source: &str,
    start: usize,
    end: usize,
) -> Option<(String, usize, usize)> {
    let bytes = source.as_bytes();
    let (start, end) = trim_target_range(bytes, start, end);
    if start >= end || bytes.get(start) != Some(&b'*') {
        return None;
    }
    let is_param_spec = bytes.get(start + 1) == Some(&b'*');
    let name_start = start + if is_param_spec { 2 } else { 1 };
    let name_end = identifier_end(bytes, name_start, end);
    if name_start == name_end {
        return None;
    }
    let colon = next_non_horizontal_whitespace(bytes, name_end);
    if colon >= end || bytes.get(colon) != Some(&b':') {
        return None;
    }
    let has_constraints = expression_slice_is_tuple(source, colon + 1, end);
    let message = match (is_param_spec, has_constraints) {
        (false, false) => "cannot use bound with TypeVarTuple",
        (false, true) => "cannot use constraints with TypeVarTuple",
        (true, false) => "cannot use bound with ParamSpec",
        (true, true) => "cannot use constraints with ParamSpec",
    };
    Some((message.to_owned(), colon, colon + 1))
}

fn invalid_type_param_list_error(
    source: &str,
    open: usize,
    close: usize,
) -> Option<(String, usize, usize)> {
    let bytes = source.as_bytes();
    let mut item_start = open + 1;
    let mut index = item_start;
    let mut level = 0usize;
    while index <= close {
        if index == close || (level == 0 && bytes.get(index) == Some(&b',')) {
            if let Some(error) = invalid_type_param_item_error(source, item_start, index) {
                return Some(error);
            }
            item_start = index + 1;
            index += 1;
            continue;
        }
        match bytes[index] {
            b'\'' | b'"' => index = skip_quoted_string(bytes, index),
            b'(' | b'[' | b'{' => {
                level += 1;
                index += 1;
            }
            b')' | b']' | b'}' => {
                level = level.saturating_sub(1);
                index += 1;
            }
            _ => index += 1,
        }
    }
    None
}

fn invalid_type_param_error(source: &str) -> Option<(String, usize, usize)> {
    let bytes = source.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'#' => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'\'' | b'"' => index = skip_quoted_string(bytes, index),
            b'[' if type_param_list_open(bytes, index) => {
                let Some(close) = matching_delimiter(bytes, index, b']') else {
                    index += 1;
                    continue;
                };
                if let Some(error) = invalid_type_param_list_error(source, index, close) {
                    return Some(error);
                }
                index = close + 1;
            }
            _ => index += 1,
        }
    }
    None
}

fn invalid_comprehension_in_slice(
    bytes: &[u8],
    open: usize,
    close: usize,
) -> Option<(String, usize, usize)> {
    let for_index = find_keyword_at_level(bytes, open + 1, close, b"for")?;
    let item_start = next_non_horizontal_whitespace(bytes, open + 1);
    if item_start >= for_index {
        return None;
    }
    if bytes.get(item_start..item_start + 2) == Some(b"**") && bytes.get(open) == Some(&b'{') {
        return Some((
            "dict unpacking cannot be used in dict comprehension".to_owned(),
            item_start,
            item_start + 2,
        ));
    }
    if bytes.get(item_start..item_start + 2) == Some(b"**") && bytes.get(open) == Some(&b'(') {
        return Some(("invalid syntax".to_owned(), for_index, for_index + 3));
    }
    if bytes.get(item_start) == Some(&b'*') {
        return Some((
            "iterable unpacking cannot be used in comprehension".to_owned(),
            item_start,
            item_start + 1,
        ));
    }
    if !matches!(bytes.get(open), Some(b'[' | b'{')) {
        return None;
    }
    if top_level_colon(bytes, open + 1, for_index).is_none()
        && let Some(comma) = top_level_byte(bytes, open + 1, for_index, b',')
    {
        let (start, _) = trim_target_range(bytes, open + 1, comma);
        return Some((
            "did you forget parentheses around the comprehension target?".to_owned(),
            start,
            comma + 1,
        ));
    }
    None
}

fn invalid_comprehension_error(source: &str) -> Option<(String, usize, usize)> {
    let bytes = source.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'#' => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'\'' | b'"' => index = skip_quoted_string(bytes, index),
            b'(' | b'[' | b'{' => {
                let close_byte = match bytes[index] {
                    b'(' => b')',
                    b'[' => b']',
                    _ => b'}',
                };
                let Some(close) = matching_delimiter(bytes, index, close_byte) else {
                    index += 1;
                    continue;
                };
                if let Some(error) = invalid_comprehension_in_slice(bytes, index, close) {
                    return Some(error);
                }
                index = close + 1;
            }
            _ => index += 1,
        }
    }
    None
}

fn invalid_group_in_slice(
    bytes: &[u8],
    open: usize,
    close: usize,
) -> Option<(String, usize, usize)> {
    let (item_start, item_end) = trim_target_range(bytes, open + 1, close);
    if item_start >= item_end
        || top_level_byte(bytes, item_start, item_end, b',').is_some()
        || top_level_colon(bytes, item_start, item_end).is_some()
        || find_keyword_at_level(bytes, item_start, item_end, b"for").is_some()
    {
        return None;
    }
    if bytes.get(item_start..item_start + 2) == Some(b"**") {
        return Some((
            "cannot use double starred expression here".to_owned(),
            item_start,
            item_start + 2,
        ));
    }
    if bytes.get(item_start) == Some(&b'*') {
        return Some((
            "cannot use starred expression here".to_owned(),
            item_start,
            item_start + 1,
        ));
    }
    None
}

fn invalid_group_error(source: &str) -> Option<(String, usize, usize)> {
    let bytes = source.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'#' => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'\'' | b'"' => index = skip_quoted_string(bytes, index),
            b'(' => {
                let Some(close) = matching_delimiter(bytes, index, b')') else {
                    index += 1;
                    continue;
                };
                if let Some(error) = invalid_group_in_slice(bytes, index, close) {
                    return Some(error);
                }
                index = close + 1;
            }
            _ => index += 1,
        }
    }
    None
}

fn invalid_parameter_star_annotation_error(source: &str) -> Option<(String, usize, usize)> {
    let bytes = source.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'#' => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'\'' | b'"' => index = skip_quoted_string(bytes, index),
            b'(' => {
                let Some(close) = matching_delimiter(bytes, index, b')') else {
                    index += 1;
                    continue;
                };
                let mut param_start = index + 1;
                while param_start < close {
                    let param_end =
                        find_byte_at_level(bytes, param_start, close, b',').unwrap_or(close);
                    if let Some(colon) = top_level_colon(bytes, param_start, param_end) {
                        let value_start = next_non_horizontal_whitespace(bytes, colon + 1);
                        if bytes.get(value_start) == Some(&b'*') {
                            return Some((
                                "invalid syntax".to_owned(),
                                value_start,
                                value_start + 1,
                            ));
                        }
                    }
                    param_start = param_end.saturating_add(1);
                }
                index = close + 1;
            }
            _ => index += 1,
        }
    }
    None
}

fn invalid_def_type_params_error(source: &str) -> Option<(String, usize, usize)> {
    let bytes = source.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'#' => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'\'' | b'"' => index = skip_quoted_string(bytes, index),
            _ if starts_identifier(bytes, index, b"def") => {
                let name_start = skip_horizontal_whitespace(bytes, index + 3);
                let name_end = identifier_end(bytes, name_start, bytes.len());
                let bracket = skip_horizontal_whitespace(bytes, name_end);
                if bytes.get(bracket) == Some(&b'[') {
                    let Some(close) = matching_delimiter(bytes, bracket, b']') else {
                        index = bracket + 1;
                        continue;
                    };
                    let after_close = skip_horizontal_whitespace(bytes, close + 1);
                    if bytes.get(after_close) == Some(&b'(')
                        && type_param_list_is_malformed(bytes, bracket + 1, close)
                    {
                        return Some(("expected '('".to_owned(), bracket, bracket + 1));
                    }
                }
                index = name_end.max(index + 3);
            }
            _ => index += 1,
        }
    }
    None
}

fn type_param_list_is_malformed(bytes: &[u8], start: usize, end: usize) -> bool {
    let mut index = start;
    let mut expect_item = true;
    while index < end {
        index = skip_horizontal_whitespace(bytes, index);
        if index >= end {
            break;
        }
        if bytes[index] == b',' {
            if expect_item {
                return true;
            }
            expect_item = true;
            index += 1;
            continue;
        }
        if !expect_item {
            return true;
        }
        if bytes.get(index..index + 2) == Some(b"**") {
            index += 2;
        } else if bytes.get(index) == Some(&b'*') {
            index += 1;
        }
        let item_start = skip_horizontal_whitespace(bytes, index);
        let item_end = identifier_end(bytes, item_start, end);
        if item_end == item_start {
            return true;
        }
        index = item_end;
        if bytes.get(skip_horizontal_whitespace(bytes, index)) == Some(&b':') {
            index = skip_horizontal_whitespace(bytes, index) + 1;
            while index < end && bytes[index] != b',' {
                index = match bytes[index] {
                    b'\'' | b'"' => skip_quoted_string(bytes, index),
                    _ => index + 1,
                };
            }
        }
        expect_item = false;
    }
    false
}

fn invalid_parameter_list_slice_error(
    source: &str,
    start: usize,
    end: usize,
    kind: ParameterListKind,
) -> Option<(String, usize, usize)> {
    let bytes = source.as_bytes();
    let mut index = start;
    let mut level = 0usize;
    let mut default_seen = false;
    let mut keyword_only = false;
    let mut slash_seen = false;
    let mut var_keyword_seen = false;
    while index < end {
        match bytes[index] {
            b'\'' | b'"' => index = skip_quoted_string(bytes, index),
            _ if level == 0
                && var_keyword_seen
                && bytes.get(index).is_some_and(|byte| {
                    *byte >= 0x80 || *byte == b'_' || byte.is_ascii_alphabetic()
                }) =>
            {
                let name_end = identifier_end(bytes, index, end);
                return Some((
                    "arguments cannot follow var-keyword argument".to_owned(),
                    index,
                    name_end,
                ));
            }
            _ if level == 0
                && !keyword_only
                && bytes.get(index).is_some_and(|byte| {
                    *byte >= 0x80 || *byte == b'_' || byte.is_ascii_alphabetic()
                }) =>
            {
                let param_end = find_byte_at_level(bytes, index, end, b',')
                    .or_else(|| top_level_byte(bytes, index, end, b')'))
                    .or_else(|| {
                        matches!(kind, ParameterListKind::Lambda)
                            .then(|| top_level_byte(bytes, index, end, b':'))
                            .flatten()
                    })
                    .unwrap_or(end);
                let name_end = identifier_end(bytes, index, param_end);
                if top_level_byte(bytes, index, param_end, b'=').is_some() {
                    default_seen = true;
                } else if default_seen {
                    return Some((
                        "parameter without a default follows parameter with a default".to_owned(),
                        index,
                        name_end,
                    ));
                }
                index = name_end;
            }
            b'(' if level == 0 => {
                let close = matching_delimiter(bytes, index, b')')
                    .filter(|close| *close <= end)
                    .unwrap_or(index + 1);
                let message = match kind {
                    ParameterListKind::Function => "Function parameters cannot be parenthesized",
                    ParameterListKind::Lambda => {
                        "Lambda expression parameters cannot be parenthesized"
                    }
                };
                return Some((message.to_owned(), index, close + 1));
            }
            b'(' | b'[' | b'{' => {
                level += 1;
                index += 1;
            }
            b')' | b']' | b'}' => {
                level = level.saturating_sub(1);
                index += 1;
            }
            b'/' if level == 0 => {
                if var_keyword_seen {
                    return Some((
                        "arguments cannot follow var-keyword argument".to_owned(),
                        index,
                        index + 1,
                    ));
                }
                if slash_seen {
                    return Some(("/ may appear only once".to_owned(), index, index + 1));
                }
                slash_seen = true;
                let next = next_non_horizontal_whitespace(bytes, index + 1);
                if bytes.get(next) == Some(&b'*') {
                    return Some(("expected comma between / and *".to_owned(), next, next + 1));
                }
                index += 1;
            }
            b'*' if level == 0 => {
                if var_keyword_seen {
                    return Some((
                        "arguments cannot follow var-keyword argument".to_owned(),
                        index,
                        index + 1,
                    ));
                }
                keyword_only = true;
                let stars = usize::from(bytes.get(index + 1) == Some(&b'*')) + 1;
                let name_start = next_non_horizontal_whitespace(bytes, index + stars);
                for keyword in [b"True".as_slice(), b"False".as_slice(), b"None".as_slice()] {
                    if starts_identifier(bytes, name_start, keyword) {
                        return Some((
                            "invalid syntax".to_owned(),
                            name_start,
                            name_start + keyword.len(),
                        ));
                    }
                }
                let param_end = find_byte_at_level(bytes, name_start, end, b',')
                    .or_else(|| top_level_byte(bytes, name_start, end, b')'))
                    .or_else(|| {
                        matches!(kind, ParameterListKind::Lambda)
                            .then(|| top_level_byte(bytes, name_start, end, b':'))
                            .flatten()
                    })
                    .unwrap_or(end);
                if stars == 1 && matches!(bytes.get(name_start), Some(b')' | b',' | b':')) {
                    return Some((
                        "named arguments must follow bare *".to_owned(),
                        index,
                        index + 1,
                    ));
                }
                if stars == 1 && top_level_byte(bytes, name_start, param_end, b'=').is_some() {
                    return Some((
                        "var-positional argument cannot have default value".to_owned(),
                        index,
                        index + 1,
                    ));
                }
                if stars == 2 && top_level_byte(bytes, name_start, param_end, b'=').is_some() {
                    return Some((
                        "var-keyword argument cannot have default value".to_owned(),
                        index,
                        index + 2,
                    ));
                }
                if stars == 2 {
                    var_keyword_seen = true;
                    index = param_end;
                    continue;
                }
                index += stars;
            }
            b'=' if level == 0 => {
                let value_start = next_non_horizontal_whitespace(bytes, index + 1);
                if value_start >= end || matches!(bytes.get(value_start), Some(b',' | b')' | b':'))
                {
                    if matches!(kind, ParameterListKind::Lambda)
                        && matches!(bytes.get(value_start), Some(b':'))
                    {
                        return Some(("invalid syntax".to_owned(), index, index + 1));
                    }
                    return Some((
                        "expected default value expression".to_owned(),
                        index,
                        index + 1,
                    ));
                }
                index += 1;
            }
            _ => index += 1,
        }
    }
    None
}

fn invalid_parameter_list_error(source: &str) -> Option<(String, usize, usize)> {
    let bytes = source.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'#' => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'\'' | b'"' => index = skip_quoted_string(bytes, index),
            _ if starts_identifier(bytes, index, b"def") => {
                let Some(paren) = top_level_byte(bytes, index + 3, bytes.len(), b'(') else {
                    index += 3;
                    continue;
                };
                let Some(close) = matching_delimiter(bytes, paren, b')') else {
                    index = paren + 1;
                    continue;
                };
                if let Some(error) = invalid_parameter_list_slice_error(
                    source,
                    paren + 1,
                    close,
                    ParameterListKind::Function,
                ) {
                    return Some(error);
                }
                index = close + 1;
            }
            _ if starts_identifier(bytes, index, b"lambda") => {
                let params_start = index + 6;
                let Some(params_end) = find_lambda_parameter_end(bytes, params_start) else {
                    index = params_start;
                    continue;
                };
                if let Some(error) = invalid_parameter_list_slice_error(
                    source,
                    params_start,
                    params_end,
                    ParameterListKind::Lambda,
                ) {
                    return Some(error);
                }
                index = params_end + 1;
            }
            _ => index += 1,
        }
    }
    None
}

#[derive(Clone, Copy)]
struct CallArgFrame {
    level: usize,
    arg_start: Option<usize>,
    in_call: bool,
}

fn next_non_horizontal_whitespace(bytes: &[u8], mut index: usize) -> usize {
    while matches!(bytes.get(index), Some(b' ' | b'\t' | b'\x0c')) {
        index += 1;
    }
    index
}

fn invalid_call_argument_assignment_error(
    source: &str,
    arg_start: usize,
    equal: usize,
) -> Option<(String, usize, usize)> {
    let bytes = source.as_bytes();
    let start = bytes[arg_start..equal]
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(arg_start, |newline| arg_start + newline + 1);
    let (target_start, target_end) = trim_target_range(bytes, start, equal);
    if target_start >= target_end {
        return None;
    }
    let value_start = next_non_horizontal_whitespace(bytes, equal + 1);
    if matches!(bytes.get(value_start), None | Some(b',' | b')')) {
        return Some((
            "expected argument value expression".to_owned(),
            target_start,
            equal + 1,
        ));
    }
    if bytes.get(target_start..target_start + 2) == Some(b"**") {
        return Some((
            "cannot assign to keyword argument unpacking".to_owned(),
            target_start,
            value_start,
        ));
    }
    if bytes.get(target_start) == Some(&b'*') {
        return Some((
            "cannot assign to iterable argument unpacking".to_owned(),
            target_start,
            value_start,
        ));
    }
    for keyword in [b"True".as_slice(), b"False".as_slice(), b"None".as_slice()] {
        if bytes.get(target_start..target_end) == Some(keyword) {
            let keyword = ::core::str::from_utf8(keyword).ok()?;
            return Some((
                format!("cannot assign to {keyword}"),
                target_start,
                target_end,
            ));
        }
    }
    if is_simple_keyword_name(bytes, target_start, target_end) {
        return None;
    }
    Some((
        "expression cannot contain assignment, perhaps you meant \"==\"?".to_owned(),
        target_start,
        equal,
    ))
}

fn invalid_call_star_expression_error(
    bytes: &[u8],
    arg_start: usize,
    index: usize,
) -> Option<(String, usize, usize)> {
    let start = next_non_horizontal_whitespace(bytes, arg_start);
    if start != index || bytes.get(index) != Some(&b'*') {
        return None;
    }
    let after_star = next_non_horizontal_whitespace(bytes, index + 1);
    if matches!(bytes.get(after_star), None | Some(b',' | b')' | b':')) {
        return Some((
            "Invalid star expression".to_owned(),
            index,
            (index + 1).min(bytes.len()),
        ));
    }
    None
}

fn invalid_call_argument_error(source: &str) -> Option<(String, usize, usize)> {
    let bytes = source.as_bytes();
    let mut index = 0usize;
    let mut level = 0usize;
    let mut frames: Vec<CallArgFrame> = Vec::new();
    while index < bytes.len() {
        match bytes[index] {
            b'#' => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'\'' | b'"' => index = skip_quoted_string(bytes, index),
            _ if starts_identifier(bytes, index, b"lambda") => {
                let params_start = index + 6;
                if let Some(params_end) = find_lambda_parameter_end(bytes, params_start) {
                    index = params_end + 1;
                } else {
                    index = params_start;
                }
            }
            b'(' => {
                level += 1;
                let in_call = opening_paren_is_call(bytes, index)
                    || frames.last().is_some_and(|frame| frame.in_call);
                frames.push(CallArgFrame {
                    level,
                    arg_start: (in_call && !is_function_parameter_list(bytes, index))
                        .then_some(index + 1),
                    in_call,
                });
                index += 1;
            }
            b')' => {
                if matches!(frames.last(), Some(frame) if frame.level == level) {
                    frames.pop();
                }
                level = level.saturating_sub(1);
                index += 1;
            }
            b'[' | b'{' => {
                level += 1;
                index += 1;
            }
            b']' | b'}' => {
                level = level.saturating_sub(1);
                index += 1;
            }
            b',' => {
                if let Some(frame) = frames.last_mut()
                    && frame.level == level
                    && frame.arg_start.is_some()
                {
                    frame.arg_start = Some(index + 1);
                }
                index += 1;
            }
            b'*' => {
                if let Some(CallArgFrame {
                    level: frame_level,
                    arg_start: Some(arg_start),
                    in_call: true,
                }) = frames.last().copied()
                    && frame_level == level
                    && let Some(error) = invalid_call_star_expression_error(bytes, arg_start, index)
                {
                    return Some(error);
                }
                index += 1;
            }
            b'=' if is_plain_assignment_operator(bytes, index) => {
                if let Some(CallArgFrame {
                    level: frame_level,
                    arg_start: Some(arg_start),
                    in_call: true,
                }) = frames.last().copied()
                    && frame_level == level
                    && let Some(error) =
                        invalid_call_argument_assignment_error(source, arg_start, index)
                {
                    return Some(error);
                }
                index += 1;
            }
            _ => index += 1,
        }
    }
    None
}

fn top_level_colon(bytes: &[u8], mut index: usize, end: usize) -> Option<usize> {
    let mut level = 0usize;
    while index < end {
        match bytes[index] {
            b'\'' | b'"' => index = skip_quoted_string(bytes, index),
            b'(' | b'[' | b'{' => {
                level += 1;
                index += 1;
            }
            b')' | b']' | b'}' => {
                level = level.saturating_sub(1);
                index += 1;
            }
            b':' if level == 0 => return Some(index),
            _ => index += 1,
        }
    }
    None
}

fn expression_slice_is_valid(source: &str, start: usize, end: usize) -> bool {
    let bytes = source.as_bytes();
    let (start, end) = trim_target_range(bytes, start, end);
    start < end
        && parser::parse(&source[start..end], parser::Mode::Expression.into())
            .is_ok_and(|parsed| matches!(parsed.into_syntax(), ast::Mod::Expression(_)))
}

fn invalid_dict_entry_error(
    source: &str,
    item_start: usize,
    item_end: usize,
    colon: Option<usize>,
    saw_dict_item: bool,
) -> Option<(String, usize, usize)> {
    let bytes = source.as_bytes();
    let (item_start, item_end) = trim_target_range(bytes, item_start, item_end);
    if item_start >= item_end {
        return None;
    }
    if let Some(colon) = colon {
        let value_start = next_non_horizontal_whitespace(bytes, colon + 1);
        if value_start >= item_end {
            return Some((
                "expression expected after dictionary key and ':'".to_owned(),
                colon,
                colon + 1,
            ));
        }
        if bytes.get(value_start) == Some(&b'*') {
            return Some((
                "cannot use a starred expression in a dictionary value".to_owned(),
                value_start,
                value_start + 1,
            ));
        }
        if !expression_slice_is_valid(source, value_start, item_end) {
            return Some(("invalid syntax".to_owned(), value_start, value_start));
        }
    } else if saw_dict_item {
        return Some((
            "':' expected after dictionary key".to_owned(),
            item_end.saturating_sub(1),
            item_end,
        ));
    }
    None
}

fn invalid_dict_literal_error(
    source: &str,
    open: usize,
    close: usize,
) -> Option<(String, usize, usize)> {
    let bytes = source.as_bytes();
    let mut item_start = open + 1;
    let mut index = item_start;
    let mut level = 0usize;
    let mut saw_dict_item = false;
    let mut item_colon = None;
    while index <= close {
        if index == close || (level == 0 && bytes.get(index) == Some(&b',')) {
            if let Some(error) =
                invalid_dict_entry_error(source, item_start, index, item_colon, saw_dict_item)
            {
                return Some(error);
            }
            saw_dict_item |= item_colon.is_some();
            item_start = index + 1;
            item_colon = None;
            index += 1;
            continue;
        }
        match bytes[index] {
            b'\'' | b'"' => index = skip_quoted_string(bytes, index),
            b'(' | b'[' | b'{' => {
                level += 1;
                index += 1;
            }
            b')' | b']' | b'}' => {
                level = level.saturating_sub(1);
                index += 1;
            }
            b':' if level == 0 && item_colon.is_none() => {
                item_colon = Some(index);
                saw_dict_item = true;
                index += 1;
            }
            _ => index += 1,
        }
    }
    None
}

fn invalid_dict_error(source: &str) -> Option<(String, usize, usize)> {
    let bytes = source.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'#' => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'\'' | b'"' => index = skip_quoted_string(bytes, index),
            b'{' => {
                let Some(close) = matching_delimiter(bytes, index, b'}') else {
                    index += 1;
                    continue;
                };
                if top_level_colon(bytes, index + 1, close).is_some()
                    && let Some(error) = invalid_dict_literal_error(source, index, close)
                {
                    return Some(error);
                }
                index = close + 1;
            }
            _ => index += 1,
        }
    }
    None
}

fn collection_open_is_call(bytes: &[u8], open: usize) -> bool {
    if bytes.get(open) != Some(&b'(') {
        return false;
    }
    let mut cursor = open;
    while cursor > 0 && matches!(bytes.get(cursor - 1), Some(b' ' | b'\t' | b'\x0c')) {
        cursor -= 1;
    }
    matches!(
        cursor.checked_sub(1).and_then(|before| bytes.get(before)),
        Some(b')' | b']' | b'_' | b'a'..=b'z' | b'A'..=b'Z' | 0x80..=0xff)
    )
}

fn invalid_collection_assignment_in_slice(
    source: &str,
    bytes: &[u8],
    start: usize,
    end: usize,
) -> Option<(String, usize, usize)> {
    let mut item_start = start;
    let mut index = start;
    let mut level = 0usize;
    while index < end {
        match bytes[index] {
            b'\'' | b'"' => index = skip_quoted_string(bytes, index),
            b'(' | b'[' | b'{' => {
                level += 1;
                index += 1;
            }
            b')' | b']' | b'}' => {
                level = level.saturating_sub(1);
                index += 1;
            }
            b',' if level == 0 => {
                item_start = index + 1;
                index += 1;
            }
            b'=' if level == 0 && is_plain_assignment_operator(bytes, index) => {
                if top_level_colon(bytes, item_start, index).is_none() {
                    let start = next_non_horizontal_whitespace(bytes, item_start);
                    let target_end = trim_end_horizontal_whitespace(bytes, start, index);
                    if start < target_end
                        && let Some((expr_name, expr_start, expr_end, _)) =
                            expression_name_and_range(&source[start..target_end])
                    {
                        if matches!(expr_name, "list" | "tuple") {
                            return None;
                        }
                        if matches!(expr_name, "expression" | "attribute" | "subscript") {
                            return Some((
                                format!(
                                    "cannot assign to {expr_name} here. Maybe you meant '==' instead of '='?"
                                ),
                                start + expr_start,
                                start + expr_end,
                            ));
                        }
                    }
                    return Some((
                        "invalid syntax. Maybe you meant '==' or ':=' instead of '='?".to_owned(),
                        start,
                        index + 1,
                    ));
                }
                index += 1;
            }
            _ => index += 1,
        }
    }
    None
}

fn invalid_collection_assignment_error(source: &str) -> Option<(String, usize, usize)> {
    let bytes = source.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'#' => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'\'' | b'"' => index = skip_quoted_string(bytes, index),
            b'(' | b'[' | b'{' => {
                let close_byte = match bytes[index] {
                    b'(' => b')',
                    b'[' => b']',
                    _ => b'}',
                };
                let Some(close) = matching_delimiter(bytes, index, close_byte) else {
                    index += 1;
                    continue;
                };
                if !collection_open_is_call(bytes, index)
                    && let Some(error) =
                        invalid_collection_assignment_in_slice(source, bytes, index + 1, close)
                {
                    return Some(error);
                }
                index = close + 1;
            }
            _ => index += 1,
        }
    }
    None
}

fn expression_assignment_error(source: &str) -> Option<(String, usize, usize)> {
    let bytes = source.as_bytes();
    let mut index = 0;
    let mut paren_arg_starts: Vec<(Option<usize>, bool)> = Vec::new();
    while index < bytes.len() {
        match bytes[index] {
            b'#' => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'\'' | b'"' => {
                index = skip_quoted_string(bytes, index);
            }
            _ if starts_identifier(bytes, index, b"lambda") => {
                let params_start = index + 6;
                if let Some(params_end) = find_lambda_parameter_end(bytes, params_start) {
                    index = params_end + 1;
                } else {
                    index = params_start;
                }
            }
            b'(' => {
                let in_call_context = opening_paren_is_call(bytes, index)
                    || paren_arg_starts.last().is_some_and(|(_, in_call)| *in_call);
                paren_arg_starts.push((
                    (!is_function_parameter_list(bytes, index)).then_some(index + 1),
                    in_call_context,
                ));
                index += 1;
            }
            b')' => {
                paren_arg_starts.pop();
                index += 1;
            }
            b',' => {
                if let Some((start, _)) = paren_arg_starts.last_mut()
                    && start.is_some()
                {
                    *start = Some(index + 1);
                }
                index += 1;
            }
            b'=' if is_plain_assignment_operator(bytes, index) => {
                if let Some((Some(start), true)) = paren_arg_starts.last().copied()
                    && !is_simple_keyword_name(bytes, start, index)
                {
                    let mut expr_start = start;
                    while matches!(bytes.get(expr_start), Some(b' ' | b'\t' | b'\x0c')) {
                        expr_start += 1;
                    }
                    return Some((
                        "expression cannot contain assignment, perhaps you meant \"==\"?"
                            .to_owned(),
                        expr_start,
                        index,
                    ));
                }
                index += 1;
            }
            _ => index += 1,
        }
    }
    None
}

fn invalid_named_expression_error(source: &str) -> Option<(String, usize, usize)> {
    let bytes = source.as_bytes();
    let mut index = 0;
    while index + 1 < bytes.len() {
        match bytes[index] {
            b'#' => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'\'' | b'"' => index = skip_quoted_string(bytes, index),
            b':' if bytes.get(index + 1) == Some(&b'=') => {
                let target_start = named_expression_target_start(bytes, index);
                let target_end = trim_end_horizontal_whitespace(bytes, target_start, index);
                if target_start < target_end
                    && let Some((expr_name, start, end, is_name)) =
                        expression_name_and_range(&source[target_start..target_end])
                    && !is_name
                {
                    return Some((
                        format!("cannot use assignment expressions with {expr_name}"),
                        target_start + start,
                        target_start + end,
                    ));
                }
                index += 2;
            }
            _ => index += 1,
        }
    }
    None
}

#[derive(Clone, Copy)]
struct AssignmentContext {
    start: usize,
    call: bool,
}

fn invalid_plain_assignment_error(source: &str) -> Option<(String, usize, usize)> {
    let bytes = source.as_bytes();
    let mut stack: Vec<AssignmentContext> = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'#' => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'\'' | b'"' => index = skip_quoted_string(bytes, index),
            b'(' | b'[' | b'{' => {
                stack.push(AssignmentContext {
                    start: index + 1,
                    call: bytes[index] == b'(' && opening_paren_is_call(bytes, index),
                });
                index += 1;
            }
            b')' | b']' | b'}' => {
                stack.pop();
                index += 1;
            }
            b',' => {
                if let Some(context) = stack.last_mut()
                    && !context.call
                {
                    context.start = index + 1;
                }
                index += 1;
            }
            b'=' if is_plain_assignment_operator(bytes, index) => {
                if let Some(context) = stack.last().copied()
                    && !context.call
                {
                    let target_start = skip_horizontal_whitespace(bytes, context.start);
                    let target_end = trim_end_horizontal_whitespace(bytes, target_start, index);
                    if target_start < target_end
                        && let Some((expr_name, start, end, _)) =
                            expression_name_and_range(&source[target_start..target_end])
                        && matches!(expr_name, "expression" | "attribute" | "subscript")
                    {
                        return Some((
                            format!(
                                "cannot assign to {expr_name} here. Maybe you meant '==' instead of '='?"
                            ),
                            target_start + start,
                            target_start + end,
                        ));
                    }
                }
                index += 1;
            }
            _ => index += 1,
        }
    }
    None
}

fn opening_paren_is_call(bytes: &[u8], paren: usize) -> bool {
    let mut cursor = paren;
    while cursor > 0 && matches!(bytes[cursor - 1], b' ' | b'\t' | b'\x0c') {
        cursor -= 1;
    }
    cursor > 0
        && (bytes[cursor - 1] >= 0x80
            || is_ascii_identifier_char(bytes[cursor - 1])
            || matches!(bytes[cursor - 1], b')' | b']'))
}

fn named_expression_target_start(bytes: &[u8], walrus: usize) -> usize {
    let mut index = walrus;
    let mut level = 0usize;
    while index > 0 {
        index -= 1;
        match bytes[index] {
            b')' | b']' | b'}' => level += 1,
            b'(' | b'[' | b'{' if level > 0 => level -= 1,
            b'(' | b'[' | b'{' if level == 0 => return index + 1,
            b',' | b'\n' | b';' if level == 0 => return index + 1,
            _ => {}
        }
    }
    0
}

fn trim_end_horizontal_whitespace(bytes: &[u8], start: usize, mut end: usize) -> usize {
    while end > start && matches!(bytes[end - 1], b' ' | b'\t' | b'\x0c') {
        end -= 1;
    }
    end
}

fn annotation_target_error_for_slice(
    source: &str,
    start: usize,
    colon: usize,
) -> Option<(String, usize, usize)> {
    let bytes = source.as_bytes();
    let (target_start, target_end) = trim_target_range(bytes, start, colon);
    if target_start >= target_end {
        return None;
    }
    let target_text = &source[target_start..target_end];
    let Ok(parsed) = parser::parse(target_text, parser::Mode::Expression.into()) else {
        return None;
    };
    let ast::Mod::Expression(expression) = parsed.into_syntax() else {
        return None;
    };
    match expression.body.as_ref() {
        ast::Expr::Name(_) | ast::Expr::Attribute(_) | ast::Expr::Subscript(_) => None,
        ast::Expr::List(_) => Some((
            "only single target (not list) can be annotated".to_owned(),
            target_start,
            target_end,
        )),
        ast::Expr::Tuple(_) => Some((
            "only single target (not tuple) can be annotated".to_owned(),
            target_start,
            target_end,
        )),
        _ => Some((
            "illegal target for annotation".to_owned(),
            target_start,
            target_end,
        )),
    }
}

fn invalid_annotation_line_start(bytes: &[u8], line_start: usize) -> bool {
    let column = skip_horizontal_whitespace(bytes, line_start);
    for keyword in [
        b"async".as_slice(),
        b"case",
        b"class",
        b"def",
        b"elif",
        b"else",
        b"except",
        b"finally",
        b"for",
        b"if",
        b"match",
        b"try",
        b"while",
        b"with",
    ] {
        if starts_identifier(bytes, column, keyword) {
            return false;
        }
    }
    true
}

fn invalid_annotation_target_error(source: &str) -> Option<(String, usize, usize)> {
    let bytes = source.as_bytes();
    let mut line_start = 0usize;
    for line in source.split_inclusive('\n') {
        let line_end = line_start + line.len();
        if invalid_annotation_line_start(bytes, line_start)
            && let Some(colon) = find_byte_at_level(bytes, line_start, line_end, b':')
            && bytes.get(colon + 1) != Some(&b'=')
            && colon.checked_sub(1).and_then(|before| bytes.get(before)) != Some(&b':')
            && let Some(error) = annotation_target_error_for_slice(source, line_start, colon)
        {
            return Some(error);
        }
        line_start = line_end;
    }
    None
}

fn statement_target_end(bytes: &[u8], mut index: usize) -> usize {
    let mut level = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'#' if level == 0 => return index,
            b'\n' | b';' if level == 0 => return index,
            b'\'' | b'"' => {
                index = skip_quoted_string(bytes, index);
            }
            b'(' | b'[' | b'{' => {
                level += 1;
                index += 1;
            }
            b')' | b']' | b'}' => {
                level = level.saturating_sub(1);
                index += 1;
            }
            _ => index += 1,
        }
    }
    index
}

fn invalid_assignment_target(expression: &ast::Expr) -> Option<&ast::Expr> {
    match expression {
        ast::Expr::List(ast::ExprList { elts, .. })
        | ast::Expr::Tuple(ast::ExprTuple { elts, .. }) => {
            elts.iter().find_map(invalid_assignment_target)
        }
        ast::Expr::Starred(ast::ExprStarred { value, .. }) => invalid_assignment_target(value),
        ast::Expr::Name(_) | ast::Expr::Subscript(_) | ast::Expr::Attribute(_) => None,
        _ => Some(expression),
    }
}

fn invalid_for_target(expression: &ast::Expr) -> Option<&ast::Expr> {
    match expression {
        ast::Expr::List(ast::ExprList { elts, .. })
        | ast::Expr::Tuple(ast::ExprTuple { elts, .. }) => elts.iter().find_map(invalid_for_target),
        ast::Expr::Starred(ast::ExprStarred { value, .. }) => invalid_for_target(value),
        ast::Expr::Compare(ast::ExprCompare { left, ops, .. }) => {
            if matches!(ops.first(), Some(ast::CmpOp::In)) {
                invalid_for_target(left)
            } else {
                None
            }
        }
        ast::Expr::Name(_) | ast::Expr::Subscript(_) | ast::Expr::Attribute(_) => None,
        _ => Some(expression),
    }
}

fn invalid_delete_target(expression: &ast::Expr) -> Option<&ast::Expr> {
    match expression {
        ast::Expr::List(ast::ExprList { elts, .. })
        | ast::Expr::Tuple(ast::ExprTuple { elts, .. }) => {
            elts.iter().find_map(invalid_delete_target)
        }
        ast::Expr::Name(_) | ast::Expr::Subscript(_) | ast::Expr::Attribute(_) => None,
        ast::Expr::Starred(_) => Some(expression),
        ast::Expr::Compare(_) => Some(expression),
        _ => Some(expression),
    }
}

fn delete_target_expr_name(expression: &ast::Expr) -> &'static str {
    match expression {
        ast::Expr::Attribute(_) => "attribute",
        ast::Expr::Subscript(_) => "subscript",
        ast::Expr::Starred(_) => "starred",
        ast::Expr::Name(_) => "name",
        ast::Expr::List(_) => "list",
        ast::Expr::Tuple(_) => "tuple",
        ast::Expr::Lambda(_) => "lambda",
        ast::Expr::Call(_) => "function call",
        ast::Expr::BoolOp(_) | ast::Expr::BinOp(_) | ast::Expr::UnaryOp(_) => "expression",
        ast::Expr::Generator(_) => "generator expression",
        ast::Expr::Yield(_) | ast::Expr::YieldFrom(_) => "yield expression",
        ast::Expr::Await(_) => "await expression",
        ast::Expr::ListComp(_) => "list comprehension",
        ast::Expr::SetComp(_) => "set comprehension",
        ast::Expr::DictComp(_) => "dict comprehension",
        ast::Expr::Dict(_) => "dict literal",
        ast::Expr::Set(_) => "set display",
        ast::Expr::FString(_) => "f-string expression",
        ast::Expr::TString(_) => "t-string expression",
        ast::Expr::NumberLiteral(_) | ast::Expr::StringLiteral(_) | ast::Expr::BytesLiteral(_) => {
            "literal"
        }
        ast::Expr::Constant(expr) => match &expr.value {
            ast::ConstantValue::None => "None",
            ast::ConstantValue::Boolean(true) => "True",
            ast::ConstantValue::Boolean(false) => "False",
            ast::ConstantValue::Ellipsis => "ellipsis",
            ast::ConstantValue::Tuple(_) => "tuple",
            ast::ConstantValue::Frozenset(_) => "literal",
            ast::ConstantValue::Str(_)
            | ast::ConstantValue::Bytes(_)
            | ast::ConstantValue::Integer(_)
            | ast::ConstantValue::Float(_)
            | ast::ConstantValue::Complex { .. } => "literal",
        },
        ast::Expr::BooleanLiteral(boolean) => {
            if boolean.value {
                "True"
            } else {
                "False"
            }
        }
        ast::Expr::NoneLiteral(_) => "None",
        ast::Expr::EllipsisLiteral(_) => "ellipsis",
        ast::Expr::Compare(_) => "comparison",
        ast::Expr::If(_) => "conditional expression",
        ast::Expr::Named(_) => "named expression",
        ast::Expr::Slice(_) | ast::Expr::IpyEscapeCommand(_) => "expression",
    }
}

fn parenthesized_single_starred_delete_target(bytes: &[u8], start: usize, end: usize) -> bool {
    let mut cursor = start;
    while matches!(bytes.get(cursor), Some(b' ' | b'\t' | b'\x0c')) {
        cursor += 1;
    }
    if bytes.get(cursor) != Some(&b'(') {
        return false;
    }
    cursor += 1;
    while matches!(bytes.get(cursor), Some(b' ' | b'\t' | b'\x0c')) {
        cursor += 1;
    }
    if bytes.get(cursor) != Some(&b'*') {
        return false;
    }
    let mut level = 1usize;
    cursor += 1;
    while cursor < end {
        match bytes[cursor] {
            b'\'' | b'"' => {
                cursor = skip_quoted_string(bytes, cursor);
            }
            b'(' | b'[' | b'{' => {
                level += 1;
                cursor += 1;
            }
            b')' => {
                level = level.saturating_sub(1);
                if level == 0 {
                    cursor += 1;
                    while matches!(bytes.get(cursor), Some(b' ' | b'\t' | b'\x0c')) {
                        cursor += 1;
                    }
                    return cursor == end;
                }
                cursor += 1;
            }
            b',' if level == 1 => return false,
            b']' | b'}' => {
                level = level.saturating_sub(1);
                cursor += 1;
            }
            _ => cursor += 1,
        }
    }
    false
}

fn trim_target_range(bytes: &[u8], mut start: usize, mut end: usize) -> (usize, usize) {
    while start < end
        && matches!(
            bytes.get(start),
            Some(b' ' | b'\t' | b'\n' | b'\r' | b'\x0c')
        )
    {
        start += 1;
    }
    while end > start
        && matches!(
            bytes.get(end - 1),
            Some(b' ' | b'\t' | b'\n' | b'\r' | b'\x0c')
        )
    {
        end -= 1;
    }
    (start, end)
}

fn invalid_assignment_message(name: &'static str, top_level_bitwise: bool) -> String {
    if top_level_bitwise {
        format!("cannot assign to {name} here. Maybe you meant '==' instead of '='?")
    } else {
        format!("cannot assign to {name}")
    }
}

fn assignment_target_error_for_slice(
    source: &str,
    start: usize,
    end: usize,
) -> Option<(String, usize, usize)> {
    let bytes = source.as_bytes();
    let (target_start, target_end) = trim_target_range(bytes, start, end);
    if target_start >= target_end {
        return None;
    }
    if starts_identifier(bytes, target_start, b"yield") {
        return Some((
            "assignment to yield expression not possible".to_owned(),
            target_start,
            target_start + 5,
        ));
    }
    let target_text = &source[target_start..target_end];
    let Ok(parsed) = parser::parse(target_text, parser::Mode::Expression.into()) else {
        return None;
    };
    let ast::Mod::Expression(expression) = parsed.into_syntax() else {
        return None;
    };
    let invalid_target = invalid_assignment_target(&expression.body)?;
    let invalid_start = target_start + invalid_target.range().start().to_usize();
    let invalid_end = target_start + invalid_target.range().end().to_usize();
    if matches!(invalid_target, ast::Expr::FString(_)) {
        return Some(("invalid syntax".to_owned(), invalid_start, invalid_end));
    }
    let name = delete_target_expr_name(invalid_target);
    let top_level = invalid_target.range() == expression.body.range();
    let bitwise_like = matches!(
        invalid_target,
        ast::Expr::Call(_)
            | ast::Expr::BoolOp(_)
            | ast::Expr::BinOp(_)
            | ast::Expr::UnaryOp(_)
            | ast::Expr::NumberLiteral(_)
            | ast::Expr::StringLiteral(_)
            | ast::Expr::BytesLiteral(_)
            | ast::Expr::EllipsisLiteral(_)
    );
    Some((
        invalid_assignment_message(name, top_level && bitwise_like),
        invalid_start,
        invalid_end,
    ))
}

fn star_target_error_for_slice(
    source: &str,
    start: usize,
    end: usize,
) -> Option<(String, usize, usize)> {
    invalid_target_error_for_slice(source, start, end, invalid_assignment_target)
}

fn for_target_error_for_slice(
    source: &str,
    start: usize,
    end: usize,
) -> Option<(String, usize, usize)> {
    invalid_target_error_for_slice(source, start, end, invalid_for_target)
}

fn invalid_target_error_for_slice(
    source: &str,
    start: usize,
    end: usize,
    invalid_target: for<'a> fn(&'a ast::Expr) -> Option<&'a ast::Expr>,
) -> Option<(String, usize, usize)> {
    let bytes = source.as_bytes();
    let (target_start, target_end) = trim_target_range(bytes, start, end);
    if target_start >= target_end {
        return None;
    }
    let target_text = &source[target_start..target_end];
    let Ok(parsed) = parser::parse(target_text, parser::Mode::Expression.into()) else {
        return None;
    };
    let ast::Mod::Expression(expression) = parsed.into_syntax() else {
        return None;
    };
    let invalid_target = invalid_target(&expression.body)?;
    let name = delete_target_expr_name(invalid_target);
    let invalid_start = target_start + invalid_target.range().start().to_usize();
    let invalid_end = target_start + invalid_target.range().end().to_usize();
    Some((
        format!("cannot assign to {name}"),
        invalid_start,
        invalid_end,
    ))
}

fn first_compare_operator_at_level(bytes: &[u8], mut index: usize, end: usize) -> Option<usize> {
    let mut level = 0usize;
    while index < end {
        match bytes[index] {
            b'\'' | b'"' => index = skip_quoted_string(bytes, index),
            b'(' | b'[' | b'{' => {
                level += 1;
                index += 1;
            }
            b')' | b']' | b'}' => {
                level = level.saturating_sub(1);
                index += 1;
            }
            b'<' | b'>' if level == 0 => return Some(index),
            b'=' if level == 0 && bytes.get(index + 1) == Some(&b'=') => return Some(index),
            b'!' if level == 0 && bytes.get(index + 1) == Some(&b'=') => return Some(index),
            _ if level == 0 && starts_identifier(bytes, index, b"is") => return Some(index),
            _ if level == 0 && starts_identifier(bytes, index, b"not") => return Some(index),
            _ => index += 1,
        }
    }
    None
}

fn non_in_compare_for_target_error(
    source: &str,
    start: usize,
    end: usize,
) -> Option<(String, usize, usize)> {
    let bytes = source.as_bytes();
    let (target_start, target_end) = trim_target_range(bytes, start, end);
    if target_start >= target_end {
        return None;
    }
    let target_text = &source[target_start..target_end];
    let Ok(parsed) = parser::parse(target_text, parser::Mode::Expression.into()) else {
        return None;
    };
    let ast::Mod::Expression(expression) = parsed.into_syntax() else {
        return None;
    };
    let ast::Expr::Compare(ast::ExprCompare { ops, .. }) = expression.body.as_ref() else {
        return None;
    };
    if matches!(ops.first(), Some(ast::CmpOp::In)) {
        return None;
    }
    let operator = first_compare_operator_at_level(bytes, target_start, target_end)?;
    Some((
        "invalid syntax".to_owned(),
        operator,
        (operator + 1).min(target_end),
    ))
}

fn top_level_plain_assignment_offsets(bytes: &[u8]) -> Vec<usize> {
    let mut offsets = Vec::new();
    let mut index = 0usize;
    let mut level = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'#' if level == 0 => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'\'' | b'"' => index = skip_quoted_string(bytes, index),
            b'(' | b'[' | b'{' => {
                level += 1;
                index += 1;
            }
            b')' | b']' | b'}' => {
                level = level.saturating_sub(1);
                index += 1;
            }
            b'=' if level == 0 && is_plain_assignment_operator(bytes, index) => {
                offsets.push(index);
                index += 1;
            }
            _ => index += 1,
        }
    }
    offsets
}

fn invalid_assignment_target_error(source: &str) -> Option<(String, usize, usize)> {
    let bytes = source.as_bytes();
    let offsets = top_level_plain_assignment_offsets(bytes);
    if offsets.is_empty() {
        return None;
    }
    let mut start = 0usize;
    for offset in offsets {
        if let Some(error) = assignment_target_error_for_slice(source, start, offset) {
            return Some(error);
        }
        start = offset + 1;
    }
    None
}

fn top_level_augassign_offset(bytes: &[u8]) -> Option<(usize, usize)> {
    let mut index = 0usize;
    let mut level = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'#' if level == 0 => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'\'' | b'"' => index = skip_quoted_string(bytes, index),
            b'(' | b'[' | b'{' => {
                level += 1;
                index += 1;
            }
            b')' | b']' | b'}' => {
                level = level.saturating_sub(1);
                index += 1;
            }
            b'+' | b'-' | b'*' | b'@' | b'/' | b'%' | b'&' | b'|' | b'^'
                if level == 0 && bytes.get(index + 1) == Some(&b'=') =>
            {
                return Some((index, 2));
            }
            b'<' | b'>'
                if level == 0
                    && bytes.get(index + 1) == Some(&bytes[index])
                    && bytes.get(index + 2) == Some(&b'=') =>
            {
                return Some((index, 3));
            }
            b'*' if level == 0
                && bytes.get(index + 1) == Some(&b'*')
                && bytes.get(index + 2) == Some(&b'=') =>
            {
                return Some((index, 3));
            }
            b'/' if level == 0
                && bytes.get(index + 1) == Some(&b'/')
                && bytes.get(index + 2) == Some(&b'=') =>
            {
                return Some((index, 3));
            }
            _ => index += 1,
        }
    }
    None
}

fn invalid_augassign_target_error(source: &str) -> Option<(String, usize, usize)> {
    let bytes = source.as_bytes();
    let (operator, _) = top_level_augassign_offset(bytes)?;
    let (target_start, target_end) = trim_target_range(bytes, 0, operator);
    if target_start >= target_end {
        return None;
    }
    let target_text = &source[target_start..target_end];
    let Ok(parsed) = parser::parse(target_text, parser::Mode::Expression.into()) else {
        return None;
    };
    let ast::Mod::Expression(expression) = parsed.into_syntax() else {
        return None;
    };
    let name = delete_target_expr_name(&expression.body);
    Some((
        format!("'{name}' is an illegal expression for augmented assignment"),
        target_start,
        target_end,
    ))
}

fn find_for_target_delimiter(bytes: &[u8], mut index: usize, end: usize) -> Option<usize> {
    let mut level = 0usize;
    while index < end {
        match bytes[index] {
            b'#' if level == 0 => return None,
            b'\'' | b'"' => index = skip_quoted_string(bytes, index),
            b'(' | b'[' | b'{' => {
                level += 1;
                index += 1;
            }
            b')' | b']' | b'}' => {
                level = level.saturating_sub(1);
                index += 1;
            }
            _ if level == 0 && starts_identifier(bytes, index, b"in") => return Some(index),
            b':' if level == 0 => return Some(index),
            _ => index += 1,
        }
    }
    None
}

fn invalid_for_target_error(source: &str) -> Option<(String, usize, usize)> {
    let bytes = source.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'#' => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'\'' | b'"' => index = skip_quoted_string(bytes, index),
            _ if starts_identifier(bytes, index, b"for") => {
                let target_start = skip_horizontal_whitespace(bytes, index + 3);
                let line_end = source[index..]
                    .find('\n')
                    .map_or(bytes.len(), |newline| index + newline);
                if let Some(target_end) = find_for_target_delimiter(bytes, target_start, line_end) {
                    if let Some(error) =
                        for_target_error_for_slice(source, target_start, target_end)
                    {
                        return Some(error);
                    }
                    if let Some(error) =
                        non_in_compare_for_target_error(source, target_start, target_end)
                    {
                        return Some(error);
                    }
                }
                index = target_start.max(index + 3);
            }
            _ => index += 1,
        }
    }
    None
}

fn find_with_target_delimiter(bytes: &[u8], mut index: usize, end: usize) -> Option<usize> {
    let mut level = 0usize;
    while index < end {
        match bytes[index] {
            b'#' if level == 0 => return None,
            b'\'' | b'"' => index = skip_quoted_string(bytes, index),
            b'(' | b'[' | b'{' => {
                level += 1;
                index += 1;
            }
            b')' if level == 0 => return Some(index),
            b')' | b']' | b'}' => {
                level = level.saturating_sub(1);
                index += 1;
            }
            b',' | b':' if level == 0 => return Some(index),
            _ => index += 1,
        }
    }
    None
}

fn invalid_with_target_error(source: &str) -> Option<(String, usize, usize)> {
    let bytes = source.as_bytes();
    let mut line_start = 0usize;
    for line in source.split_inclusive('\n') {
        let line_end = line_start + line.len();
        let mut column = skip_horizontal_whitespace(bytes, line_start);
        if starts_identifier(bytes, column, b"async") {
            column = skip_horizontal_whitespace(bytes, column + 5);
        }
        if !starts_identifier(bytes, column, b"with") {
            line_start = line_end;
            continue;
        }
        let mut index = column + 4;
        while let Some(as_index) = find_keyword_at_level(bytes, index, line_end, b"as") {
            let target_start = skip_horizontal_whitespace(bytes, as_index + 2);
            if let Some(target_end) = find_with_target_delimiter(bytes, target_start, line_end) {
                if let Some(error) = star_target_error_for_slice(source, target_start, target_end) {
                    return Some(error);
                }
                index = target_end.saturating_add(1);
            } else {
                break;
            }
        }
        line_start = line_end;
    }
    None
}

fn find_missing_in_if_keyword(bytes: &[u8], mut index: usize, end: usize) -> Option<usize> {
    let mut level = 0usize;
    while index < end {
        match bytes[index] {
            b'#' if level == 0 => return None,
            b'\'' | b'"' => index = skip_quoted_string(bytes, index),
            b'(' | b'[' | b'{' => {
                level += 1;
                index += 1;
            }
            b')' | b']' | b'}' if level == 0 => return None,
            b')' | b']' | b'}' => {
                level = level.saturating_sub(1);
                index += 1;
            }
            _ if level == 0 && starts_identifier(bytes, index, b"in") => return None,
            _ if level == 0 && starts_identifier(bytes, index, b"if") => return Some(index),
            _ => index += 1,
        }
    }
    None
}

fn invalid_for_if_clause_error(source: &str) -> Option<(String, usize, usize)> {
    let bytes = source.as_bytes();
    let mut index = 0usize;
    let mut level = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'#' if level == 0 => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'\'' | b'"' => index = skip_quoted_string(bytes, index),
            b'(' | b'[' | b'{' => {
                level += 1;
                index += 1;
            }
            b')' | b']' | b'}' => {
                level = level.saturating_sub(1);
                index += 1;
            }
            _ if level > 0 && starts_identifier(bytes, index, b"for") => {
                let target_start = skip_horizontal_whitespace(bytes, index + 3);
                let line_end = source[index..]
                    .find('\n')
                    .map_or(bytes.len(), |newline| index + newline);
                if let Some(if_index) = find_missing_in_if_keyword(bytes, target_start, line_end) {
                    return Some((
                        "'in' expected after for-loop variables".to_owned(),
                        if_index,
                        (if_index + 2).min(line_end),
                    ));
                }
                index = target_start.max(index + 3);
            }
            _ => index += 1,
        }
    }
    None
}

fn invalid_delete_target_error(source: &str) -> Option<(String, usize, usize)> {
    let bytes = source.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'#' => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'\'' | b'"' => {
                index = skip_quoted_string(bytes, index);
            }
            b'd' if starts_identifier(bytes, index, b"del") => {
                let mut target_start = index + 3;
                if !matches!(bytes.get(target_start), Some(b' ' | b'\t' | b'\x0c')) {
                    index += 3;
                    continue;
                }
                while matches!(bytes.get(target_start), Some(b' ' | b'\t' | b'\x0c')) {
                    target_start += 1;
                }
                let mut target_end = statement_target_end(bytes, target_start);
                while target_end > target_start
                    && matches!(bytes.get(target_end - 1), Some(b' ' | b'\t' | b'\x0c'))
                {
                    target_end -= 1;
                }
                if target_start >= target_end {
                    index = target_end.max(index + 3);
                    continue;
                }
                if parenthesized_single_starred_delete_target(bytes, target_start, target_end) {
                    return Some((
                        "cannot use starred expression here".to_owned(),
                        target_start,
                        target_end,
                    ));
                }
                if bytes.get(target_start) == Some(&b'*') {
                    return Some((
                        "cannot delete starred".to_owned(),
                        target_start,
                        (target_start + 1).min(target_end),
                    ));
                }
                let target_text = &source[target_start..target_end];
                let Ok(parsed) = parser::parse(target_text, parser::Mode::Expression.into()) else {
                    index = target_end;
                    continue;
                };
                let ast::Mod::Expression(expression) = parsed.into_syntax() else {
                    index = target_end;
                    continue;
                };
                let Some(invalid_target) = invalid_delete_target(&expression.body) else {
                    index = target_end;
                    continue;
                };
                let start = target_start + invalid_target.range().start().to_usize();
                let end = target_start + invalid_target.range().end().to_usize();
                if matches!(invalid_target, ast::Expr::FString(_)) {
                    return Some(("invalid syntax".to_owned(), start, end));
                }
                let name = delete_target_expr_name(invalid_target);
                return Some((format!("cannot delete {name}"), start, end));
            }
            _ => index += 1,
        }
    }
    None
}

fn skip_horizontal_whitespace(bytes: &[u8], mut index: usize) -> usize {
    while matches!(bytes.get(index), Some(b' ' | b'\t' | b'\x0c')) {
        index += 1;
    }
    index
}

fn find_keyword_at_level(
    bytes: &[u8],
    mut index: usize,
    end: usize,
    keyword: &[u8],
) -> Option<usize> {
    let mut level = 0usize;
    while index < end {
        match bytes[index] {
            b'#' if level == 0 => return None,
            b'\'' | b'"' => {
                index = skip_quoted_string(bytes, index);
            }
            b'(' | b'[' | b'{' => {
                level += 1;
                index += 1;
            }
            b')' | b']' | b'}' => {
                level = level.saturating_sub(1);
                index += 1;
            }
            _ if level == 0 && starts_identifier(bytes, index, keyword) => return Some(index),
            _ => index += 1,
        }
    }
    None
}

fn find_byte_at_level(bytes: &[u8], mut index: usize, end: usize, needle: u8) -> Option<usize> {
    let mut level = 0usize;
    while index < end {
        match bytes[index] {
            b'#' if level == 0 => return None,
            b'\'' | b'"' => {
                index = skip_quoted_string(bytes, index);
            }
            b'(' | b'[' | b'{' => {
                level += 1;
                index += 1;
            }
            b')' | b']' | b'}' => {
                level = level.saturating_sub(1);
                index += 1;
            }
            byte if level == 0 && byte == needle => return Some(index),
            _ => index += 1,
        }
    }
    None
}

fn expression_name_and_range(source: &str) -> Option<(&'static str, usize, usize, bool)> {
    let parsed = parser::parse(source, parser::Mode::Expression.into()).ok()?;
    let ast::Mod::Expression(expression) = parsed.into_syntax() else {
        return None;
    };
    let is_name = matches!(expression.body.as_ref(), ast::Expr::Name(_));
    Some((
        delete_target_expr_name(&expression.body),
        expression.body.range().start().to_usize(),
        expression.body.range().end().to_usize(),
        is_name,
    ))
}

fn invalid_standalone_except_error(source: &str) -> Option<(String, usize, usize)> {
    let bytes = source.as_bytes();
    let mut line_start = 0usize;
    let mut seen_try = false;
    for line in source.split_inclusive('\n') {
        let line_end = line_start + line.len();
        let column = skip_horizontal_whitespace(bytes, line_start);
        if column >= line_end {
            line_start = line_end;
            continue;
        }
        if starts_identifier(bytes, column, b"try") {
            seen_try = true;
        } else if (bytes.get(column..column + 7) == Some(b"except*")
            || starts_identifier(bytes, column, b"except"))
            && !seen_try
        {
            let end = if bytes.get(column..column + 7) == Some(b"except*") {
                column + 7
            } else {
                column + 6
            };
            return Some(("invalid syntax".to_owned(), column, end));
        }
        line_start = line_end;
    }
    None
}

fn invalid_import_statement_error(source: &str) -> Option<(String, usize, usize)> {
    let bytes = source.as_bytes();
    let mut line_start = 0usize;
    for line in source.split_inclusive('\n') {
        let line_end = line_start + line.len();
        let column = skip_horizontal_whitespace(bytes, line_start);
        if column < line_end
            && starts_identifier(bytes, column, b"import")
            && find_keyword_at_level(bytes, column + 6, line_end, b"from").is_some()
        {
            return Some((
                "Did you mean to use 'from ... import ...' instead?".to_owned(),
                column,
                column + 6,
            ));
        }
        line_start = line_end;
    }
    None
}

fn import_as_target_end(bytes: &[u8], mut index: usize) -> usize {
    let mut level = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'#' if level == 0 => return index,
            b'\'' | b'"' => index = skip_quoted_string(bytes, index),
            b'(' | b'[' | b'{' => {
                level += 1;
                index += 1;
            }
            b')' if level == 0 => return index,
            b')' | b']' | b'}' => {
                level = level.saturating_sub(1);
                index += 1;
            }
            b',' | b';' | b'\n' if level == 0 => return index,
            _ => index += 1,
        }
    }
    index
}

fn valid_import_alias_name(bytes: &[u8], mut start: usize, end: usize) -> bool {
    start = skip_horizontal_whitespace(bytes, start);
    let Some(&first) = bytes.get(start) else {
        return false;
    };
    if !(first == b'_' || first.is_ascii_alphabetic() || first >= 0x80) {
        return false;
    }
    let mut index = start + 1;
    while index < end {
        match bytes[index] {
            b' ' | b'\t' | b'\x0c' => break,
            byte if byte >= 0x80 || is_ascii_identifier_char(byte) => index += 1,
            _ => return false,
        }
    }
    let index = skip_horizontal_whitespace(bytes, index);
    matches!(
        bytes.get(index),
        None | Some(b',' | b')' | b';' | b'\n' | b'\r')
    )
}

fn import_target_error_for_slice(
    source: &str,
    start: usize,
    end: usize,
) -> Option<(String, usize, usize)> {
    let bytes = source.as_bytes();
    let (target_start, target_end) = trim_target_range(bytes, start, end);
    if target_start >= target_end || valid_import_alias_name(bytes, target_start, target_end) {
        return None;
    }
    let parsed = parser::parse(
        &source[target_start..target_end],
        parser::Mode::Expression.into(),
    )
    .ok()?;
    let ast::Mod::Expression(expression) = parsed.into_syntax() else {
        return None;
    };
    let name = delete_target_expr_name(&expression.body);
    let start = target_start + expression.body.range().start().to_usize();
    let end = target_start + expression.body.range().end().to_usize();
    Some((format!("cannot use {name} as import target"), start, end))
}

fn statement_starts_import(bytes: &[u8], line_start: usize, line_end: usize) -> bool {
    let column = skip_horizontal_whitespace(bytes, line_start);
    if starts_identifier(bytes, column, b"import") {
        return true;
    }
    starts_identifier(bytes, column, b"from")
        && find_keyword_at_level(bytes, column + 4, line_end, b"import").is_some()
}

fn invalid_import_target_error(source: &str) -> Option<(String, usize, usize)> {
    let bytes = source.as_bytes();
    let mut line_start = 0usize;
    let mut in_parenthesized_from_import = false;
    for line in source.split_inclusive('\n') {
        let line_end = line_start + line.len();
        let starts_import = statement_starts_import(bytes, line_start, line_end);
        if starts_import && bytes[line_start..line_end].contains(&b'(') {
            in_parenthesized_from_import = true;
        }
        if starts_import || in_parenthesized_from_import {
            let mut index = line_start;
            while index < line_end {
                if starts_identifier(bytes, index, b"as") {
                    let target_start = skip_horizontal_whitespace(bytes, index + 2);
                    let target_end = import_as_target_end(bytes, target_start);
                    if let Some(error) =
                        import_target_error_for_slice(source, target_start, target_end)
                    {
                        return Some(error);
                    }
                    index = target_end.max(index + 2);
                } else {
                    index += 1;
                }
            }
        }
        if in_parenthesized_from_import && bytes[line_start..line_end].contains(&b')') {
            in_parenthesized_from_import = false;
        }
        line_start = line_end;
    }
    None
}

fn invalid_except_as_target_error(source: &str) -> Option<(String, usize, usize)> {
    let bytes = source.as_bytes();
    let mut line_start = 0usize;
    let mut seen_try = false;
    for line in source.split_inclusive('\n') {
        let line_end = line_start + line.len();
        let mut column = skip_horizontal_whitespace(bytes, line_start);
        if column >= line_end {
            line_start = line_end;
            continue;
        }
        if starts_identifier(bytes, column, b"try") {
            seen_try = true;
            line_start = line_end;
            continue;
        }
        let (keyword_len, starred) = if bytes.get(column..column + 7) == Some(b"except*") {
            (7, true)
        } else if starts_identifier(bytes, column, b"except") {
            (6, false)
        } else {
            line_start = line_end;
            continue;
        };
        if !seen_try {
            line_start = line_end;
            continue;
        }
        column += keyword_len;
        let Some(as_index) = find_keyword_at_level(bytes, column, line_end, b"as") else {
            line_start = line_end;
            continue;
        };
        let target_start = skip_horizontal_whitespace(bytes, as_index + 2);
        let Some(delimiter) = find_byte_at_level(bytes, target_start, line_end, b':')
            .into_iter()
            .chain(find_byte_at_level(bytes, target_start, line_end, b','))
            .min()
        else {
            line_start = line_end;
            continue;
        };
        let mut target_end = delimiter;
        while target_end > target_start
            && matches!(bytes.get(target_end - 1), Some(b' ' | b'\t' | b'\x0c'))
        {
            target_end -= 1;
        }
        let Some((expr_name, start, end, is_name)) =
            expression_name_and_range(&source[target_start..target_end])
        else {
            line_start = line_end;
            continue;
        };
        if !is_name {
            let statement = if starred { "except*" } else { "except" };
            return Some((
                format!("cannot use {statement} statement with {expr_name}"),
                target_start + start,
                target_start + end,
            ));
        }
        line_start = line_end;
    }
    None
}

fn invalid_match_as_target_error(source: &str) -> Option<(String, usize, usize)> {
    let bytes = source.as_bytes();
    let quoted_ranges = quoted_string_ranges(bytes);
    let mut quoted_range = 0usize;
    let mut line_start = 0usize;
    for line in source.split_inclusive('\n') {
        let line_end = line_start + line.len();
        let mut column = skip_horizontal_whitespace(bytes, line_start);
        if column >= line_end
            || offset_in_ranges(&quoted_ranges, &mut quoted_range, column)
            || !starts_identifier(bytes, column, b"case")
        {
            line_start = line_end;
            continue;
        }
        column += 4;
        let Some(as_index) = find_keyword_at_level(bytes, column, line_end, b"as") else {
            line_start = line_end;
            continue;
        };
        let target_start = skip_horizontal_whitespace(bytes, as_index + 2);
        let Some(delimiter) = find_byte_at_level(bytes, target_start, line_end, b':')
            .into_iter()
            .chain(find_byte_at_level(bytes, target_start, line_end, b','))
            .min()
        else {
            line_start = line_end;
            continue;
        };
        let mut target_end = delimiter;
        while target_end > target_start
            && matches!(bytes.get(target_end - 1), Some(b' ' | b'\t' | b'\x0c'))
        {
            target_end -= 1;
        }
        if source[target_start..target_end].trim() == "_" {
            return Some((
                "cannot use '_' as a target".to_owned(),
                target_start,
                target_end,
            ));
        }
        let Some((expr_name, start, end, is_name)) =
            expression_name_and_range(&source[target_start..target_end])
        else {
            line_start = line_end;
            continue;
        };
        if !is_name {
            if matches!(expr_name, "expression" | "subscript") {
                line_start = line_end;
                continue;
            }
            return Some((
                format!("cannot use {expr_name} as pattern target"),
                target_start + start,
                target_start + end,
            ));
        }
        line_start = line_end;
    }
    None
}

fn quoted_string_ranges(bytes: &[u8]) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'#' => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'\'' | b'"' => {
                let end = skip_quoted_string(bytes, index);
                ranges.push((index, end));
                index = end;
            }
            _ => index += 1,
        }
    }
    ranges
}

fn offset_in_ranges(ranges: &[(usize, usize)], range_index: &mut usize, offset: usize) -> bool {
    while ranges
        .get(*range_index)
        .is_some_and(|(_, end)| *end <= offset)
    {
        *range_index += 1;
    }
    ranges
        .get(*range_index)
        .is_some_and(|(start, end)| *start <= offset && offset < *end)
}

fn invalid_match_mapping_rest_wildcard_error(source: &str) -> Option<(String, usize, usize)> {
    let bytes = source.as_bytes();
    let next_line_end = |line_start: usize| {
        line_start
            + bytes[line_start..]
                .iter()
                .position(|byte| *byte == b'\n')
                .unwrap_or(bytes.len() - line_start)
    };
    let mut index = 0usize;
    let mut line_start = 0usize;
    let mut line_end = next_line_end(line_start);
    while index < bytes.len() {
        match bytes[index] {
            b'#' => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'\'' | b'"' => index = skip_quoted_string(bytes, index),
            b'\n' => {
                index += 1;
                line_start = index;
                line_end = next_line_end(line_start);
            }
            _ => {
                let column = skip_horizontal_whitespace(bytes, line_start);
                if index != column
                    || column >= line_end
                    || !starts_identifier(bytes, column, b"case")
                {
                    index += 1;
                    continue;
                }
                let mut cursor = column + 4;
                while cursor < line_end {
                    match bytes[cursor] {
                        b'#' => break,
                        b'\'' | b'"' => cursor = skip_quoted_string(bytes, cursor),
                        b'{' => {
                            let rest = next_non_horizontal_whitespace(bytes, cursor + 1);
                            if bytes.get(rest..rest + 2) == Some(b"**") {
                                let name_start = next_non_horizontal_whitespace(bytes, rest + 2);
                                let name_end = identifier_end(bytes, name_start, line_end);
                                if source.get(name_start..name_end) == Some("_") {
                                    return Some((
                                        "invalid syntax".to_owned(),
                                        name_start,
                                        name_end,
                                    ));
                                }
                            }
                            cursor += 1;
                        }
                        _ => cursor += 1,
                    }
                }
                index = line_end;
            }
        }
    }
    None
}

fn invalid_if_expression_statement_error(source: &str) -> Option<(String, usize, usize)> {
    let bytes = source.as_bytes();
    let mut line_start = 0usize;
    for line in source.split_inclusive('\n') {
        let line_end = line_start + line.len();
        if let Some(if_index) = find_keyword_at_level(bytes, line_start, line_end, b"if")
            && let Some((start, end)) = statement_before_if_expression(bytes, line_start, if_index)
            && find_keyword_at_level(bytes, if_index + 2, line_end, b"else").is_some()
        {
            return Some((
                "expected expression before 'if', but statement is given".to_owned(),
                start,
                end,
            ));
        }
        if let Some(else_index) = find_keyword_at_level(bytes, line_start, line_end, b"else")
            && find_keyword_at_level(bytes, line_start, else_index, b"if").is_some()
            && let Some((start, end)) =
                statement_after_else_expression(bytes, else_index + 4, line_end)
        {
            return Some((
                "expected expression after 'else', but statement is given".to_owned(),
                start,
                end,
            ));
        }
        line_start = line_end;
    }
    None
}

fn statement_before_if_expression(
    bytes: &[u8],
    line_start: usize,
    if_index: usize,
) -> Option<(usize, usize)> {
    let mut start = if_index;
    while start > line_start && matches!(bytes.get(start - 1), Some(b' ' | b'\t' | b'\x0c')) {
        start -= 1;
    }
    while start > line_start
        && !matches!(
            bytes.get(start - 1),
            Some(b'=' | b':' | b',' | b'(' | b'[' | b'{')
        )
    {
        start -= 1;
    }
    start = skip_horizontal_whitespace(bytes, start);
    for keyword in [b"pass".as_slice(), b"break", b"continue"] {
        if starts_identifier(bytes, start, keyword) {
            return Some((start, start + keyword.len()));
        }
    }
    None
}

fn statement_after_else_expression(
    bytes: &[u8],
    else_end: usize,
    line_end: usize,
) -> Option<(usize, usize)> {
    let start = skip_horizontal_whitespace(bytes, else_end);
    for keyword in [
        b"pass".as_slice(),
        b"return",
        b"raise",
        b"del",
        b"yield",
        b"assert",
        b"break",
        b"continue",
        b"import",
        b"from",
    ] {
        if starts_identifier(bytes, start, keyword) {
            let end = statement_target_end(bytes, start).min(line_end);
            return Some((start, end.max(start + keyword.len())));
        }
    }
    None
}

fn invalid_else_elif_error(source: &str) -> Option<(String, usize, usize)> {
    let bytes = source.as_bytes();
    let mut line_start = 0usize;
    let mut else_indents: Vec<usize> = Vec::new();
    for line in source.split_inclusive('\n') {
        let line_end = line_start + line.len();
        let column = skip_horizontal_whitespace(bytes, line_start);
        let line_column = column.saturating_sub(line_start);
        if column >= line_end {
            line_start = line_end;
            continue;
        }
        while else_indents
            .last()
            .is_some_and(|indent| line_column < *indent)
        {
            else_indents.pop();
        }
        if starts_identifier(bytes, column, b"else")
            && find_byte_at_level(bytes, column + 4, line_end, b':').is_some()
        {
            else_indents.push(line_column);
        } else if starts_identifier(bytes, column, b"elif") && else_indents.contains(&line_column) {
            return Some((
                "'elif' block follows an 'else' block".to_owned(),
                column,
                column + 4,
            ));
        }
        line_start = line_end;
    }
    None
}

fn mixed_except_handlers_error(source: &str) -> Option<(String, usize, usize)> {
    let message = "cannot have both 'except' and 'except*' on the same 'try'".to_owned();
    let mut seen_except = false;
    let mut seen_except_star = false;
    let mut line_start = 0usize;
    for line in source.split_inclusive('\n') {
        let bytes = line.as_bytes();
        let mut column = 0usize;
        while matches!(bytes.get(column), Some(b' ' | b'\t' | b'\x0c')) {
            column += 1;
        }
        let token_start = line_start + column;
        if bytes.get(column..column + 7) == Some(b"except*") {
            if seen_except {
                return Some((message, token_start, token_start + 7));
            }
            seen_except_star = true;
        } else if starts_identifier(bytes, column, b"except") {
            if seen_except_star {
                return Some((message, token_start, token_start + 6));
            }
            seen_except = true;
        }
        line_start += line.len();
    }
    None
}

fn non_printable_character_error(source: &str) -> Option<(String, usize, usize)> {
    let bytes = source.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'#' => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'\'' | b'"' => {
                index = skip_quoted_string(bytes, index);
            }
            byte if byte.is_ascii_control() && !matches!(byte, b'\t' | b'\n' | b'\r' | b'\x0c') => {
                return Some((
                    format!("invalid non-printable character U+{byte:04X}"),
                    index,
                    index + 1,
                ));
            }
            byte if byte >= 0x80 => {
                let ch = source[index..].chars().next()?;
                if ch.is_control() {
                    return Some((
                        format!("invalid non-printable character U+{:04X}", ch as u32),
                        index,
                        index + ch.len_utf8(),
                    ));
                }
                index += ch.len_utf8();
            }
            _ => index += 1,
        }
    }
    None
}

fn unterminated_string_error(source: &str) -> Option<(String, usize, usize)> {
    let bytes = source.as_bytes();
    let mut index = 0;
    let mut line = 1usize;
    while index < bytes.len() {
        match bytes[index] {
            b'#' => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'\n' => {
                line += 1;
                index += 1;
            }
            quote @ (b'\'' | b'"') => {
                let start = index;
                let start_line = line;
                let quote_size = if bytes.get(index + 1) == Some(&quote)
                    && bytes.get(index + 2) == Some(&quote)
                {
                    3
                } else {
                    1
                };
                index += quote_size;
                let mut has_escaped_quote = false;
                let mut closed = false;
                while index < bytes.len() {
                    let c = bytes[index];
                    if c == b'\n' {
                        if quote_size == 1 {
                            return Some((
                                unterminated_string_message(line, false, has_escaped_quote),
                                start,
                                start + 1,
                            ));
                        }
                        line += 1;
                        index += 1;
                    } else if c == quote {
                        if quote_size == 3 {
                            if bytes.get(index + 1) == Some(&quote)
                                && bytes.get(index + 2) == Some(&quote)
                            {
                                index += 3;
                                closed = true;
                                break;
                            }
                            index += 1;
                        } else {
                            index += 1;
                            closed = true;
                            break;
                        }
                    } else if c == b'\\' {
                        if bytes.get(index + 1) == Some(&quote) {
                            has_escaped_quote = true;
                        }
                        index = (index + 2).min(bytes.len());
                    } else {
                        index += 1;
                    }
                }
                if !closed {
                    let detected_line = if quote_size == 3 { line } else { start_line };
                    return Some((
                        unterminated_string_message(
                            detected_line,
                            quote_size == 3,
                            has_escaped_quote,
                        ),
                        start,
                        start + 1,
                    ));
                }
            }
            _ => index += 1,
        }
    }
    None
}

fn invalid_interpolated_string_error(source: &str) -> Option<(String, usize, usize)> {
    let bytes = source.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'#' => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            quote @ (b'\'' | b'"') => {
                let Some(prefix) = interpolated_string_prefix(bytes, index) else {
                    index = skip_quoted_string(bytes, index);
                    continue;
                };
                if let Some(error) =
                    single_quoted_format_spec_newline_error(bytes, index, quote, prefix)
                {
                    return Some(error);
                }
                let Some((content_start, content_end)) =
                    quoted_string_content_range(bytes, index, quote)
                else {
                    index = skip_quoted_string(bytes, index);
                    continue;
                };
                if let Some(error) =
                    invalid_replacement_field_error(bytes, content_start, content_end, prefix)
                {
                    return Some(error);
                }
                index = skip_quoted_string(bytes, index);
            }
            _ => index += 1,
        }
    }
    None
}

fn single_quoted_format_spec_newline_error(
    bytes: &[u8],
    quote_index: usize,
    quote: u8,
    prefix: &str,
) -> Option<(String, usize, usize)> {
    if bytes.get(quote_index + 1) == Some(&quote) && bytes.get(quote_index + 2) == Some(&quote) {
        return None;
    }

    let (content_start, content_end) = quoted_string_content_range(bytes, quote_index, quote)?;
    let mut index = content_start;
    while index < content_end {
        match bytes[index] {
            b'{' if bytes.get(index + 1) == Some(&b'{') => index += 2,
            b'}' if bytes.get(index + 1) == Some(&b'}') => index += 2,
            b'{' => {
                let expr_start = skip_ascii_whitespace(bytes, index + 1, content_end);
                if let Some(separator) = replacement_field_separator(bytes, expr_start, content_end)
                    && bytes[separator] == b':'
                {
                    let format_end =
                        replacement_field_closing_brace(bytes, separator + 1, content_end)
                            .unwrap_or(content_end);
                    if bytes[separator + 1..format_end].contains(&b'\n') {
                        return Some((
                            format!(
                                "{prefix}: newlines are not allowed in format specifiers for single quoted {prefix}s"
                            ),
                            quote_index,
                            quote_index + 1,
                        ));
                    }
                }
                index += 1;
            }
            _ => index += 1,
        }
    }
    None
}

fn interpolated_string_prefix(bytes: &[u8], quote: usize) -> Option<&'static str> {
    let prev = quote.checked_sub(1).and_then(|index| bytes.get(index))?;
    let lower_prev = prev.to_ascii_lowercase();
    let (prefix_start, marker) = if matches!(lower_prev, b'f' | b't') {
        if quote >= 2 && bytes[quote - 2].eq_ignore_ascii_case(&b'r') {
            (quote - 2, lower_prev)
        } else {
            (quote - 1, lower_prev)
        }
    } else if lower_prev == b'r'
        && quote >= 2
        && matches!(bytes[quote - 2].to_ascii_lowercase(), b'f' | b't')
    {
        (quote - 2, bytes[quote - 2].to_ascii_lowercase())
    } else {
        return None;
    };

    if prefix_start > 0 && is_ascii_identifier_char(bytes[prefix_start - 1]) {
        return None;
    }

    Some(if marker == b'f' {
        "f-string"
    } else {
        "t-string"
    })
}

fn quoted_string_content_range(
    bytes: &[u8],
    quote_index: usize,
    quote: u8,
) -> Option<(usize, usize)> {
    let triple =
        bytes.get(quote_index + 1) == Some(&quote) && bytes.get(quote_index + 2) == Some(&quote);
    let quote_len = if triple { 3 } else { 1 };
    let content_start = quote_index + quote_len;
    let mut index = content_start;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index = (index + 2).min(bytes.len());
        } else if (triple
            && bytes.get(index) == Some(&quote)
            && bytes.get(index + 1) == Some(&quote)
            && bytes.get(index + 2) == Some(&quote))
            || (!triple && bytes[index] == quote)
        {
            return Some((content_start, index));
        } else {
            index += 1;
        }
    }
    None
}

fn invalid_replacement_field_error(
    bytes: &[u8],
    start: usize,
    end: usize,
    prefix: &str,
) -> Option<(String, usize, usize)> {
    let mut index = start;
    while index < end {
        match bytes[index] {
            b'{' if bytes.get(index + 1) == Some(&b'{') => index += 2,
            b'}' if bytes.get(index + 1) == Some(&b'}') => index += 2,
            b'{' => {
                if let Some(error) = replacement_field_error(bytes, index, end, prefix) {
                    return Some(error);
                }
                index += 1;
            }
            _ => index += 1,
        }
    }
    None
}

fn replacement_field_error(
    bytes: &[u8],
    open: usize,
    end: usize,
    prefix: &str,
) -> Option<(String, usize, usize)> {
    let expr_start = skip_ascii_whitespace(bytes, open + 1, end);
    if let Some(backslash) = replacement_field_line_continuation(bytes, expr_start, end) {
        return Some((
            "unexpected character after line continuation character".to_owned(),
            backslash + 1,
            (backslash + 2).min(end),
        ));
    }
    if let Some(quote) = unterminated_string_in_replacement_field(bytes, expr_start, end) {
        return Some((
            unterminated_string_message(1, false, false),
            quote,
            quote + 1,
        ));
    }
    match bytes.get(expr_start).copied() {
        Some(marker @ (b'=' | b'!' | b':' | b'}')) => {
            return Some((
                format!(
                    "{prefix}: valid expression required before '{}'",
                    marker as char
                ),
                expr_start,
                expr_start + 1,
            ));
        }
        Some(_) => {}
        None => {
            return Some((
                format!("{prefix}: expecting a valid expression after '{{'"),
                open,
                open + 1,
            ));
        }
    }

    if starts_identifier(bytes, expr_start, b"lambda") {
        return Some((
            format!("{prefix}: lambda expressions are not allowed without parentheses"),
            expr_start,
            expr_start + b"lambda".len(),
        ));
    }

    if invalid_replacement_expression_start(bytes, expr_start, end) {
        return Some((
            format!("{prefix}: expecting a valid expression after '{{'"),
            open,
            open + 1,
        ));
    }

    let Some(separator) = replacement_field_separator(bytes, expr_start, end) else {
        return Some((format!("{prefix}: expecting '}}'"), open, open + 1));
    };

    if bytes[separator] == b':'
        && replacement_expression_has_parse_error(bytes, expr_start, separator)
    {
        return Some(("invalid syntax".to_owned(), expr_start, separator));
    }

    match bytes[separator] {
        b'=' => invalid_debug_expression_error(bytes, separator, end, prefix),
        b'!' => invalid_conversion_error(bytes, separator, end, prefix),
        b':' => invalid_format_spec_error(bytes, separator, end, prefix),
        b'}' => None,
        _ => unreachable!(),
    }
}

fn replacement_field_line_continuation(
    bytes: &[u8],
    mut index: usize,
    end: usize,
) -> Option<usize> {
    let mut level = 0usize;
    while index < end {
        match bytes[index] {
            b'\'' | b'"' => index = skip_quoted_string(bytes, index),
            b'\\' => return Some(index),
            b'(' | b'[' | b'{' => {
                level += 1;
                index += 1;
            }
            b')' | b']' | b'}' if level > 0 => {
                level -= 1;
                index += 1;
            }
            b'=' | b'!' | b':' | b'}' if level == 0 => return None,
            _ => index += 1,
        }
    }
    None
}

fn unterminated_string_in_replacement_field(
    bytes: &[u8],
    mut index: usize,
    end: usize,
) -> Option<usize> {
    while index < end {
        match bytes[index] {
            quote @ (b'\'' | b'"') => {
                let string_end = skip_quoted_string(bytes, index);
                if string_end >= end && !bytes[index + 1..end].contains(&quote) {
                    return Some(index);
                }
                index = string_end;
            }
            _ => index += 1,
        }
    }
    None
}

fn replacement_expression_has_parse_error(bytes: &[u8], start: usize, end: usize) -> bool {
    let Ok(expression) = ::core::str::from_utf8(&bytes[start..end]) else {
        return false;
    };
    parser::parse_expression(expression).is_err()
}

fn invalid_replacement_expression_start(bytes: &[u8], index: usize, end: usize) -> bool {
    if index >= end {
        return true;
    }

    if matches!(
        bytes[index],
        b'.' | b',' | b'*' | b'/' | b'%' | b'&' | b'|' | b'^' | b'<' | b'>' | b'@'
    ) {
        return true;
    }

    if matches!(bytes[index], b'+' | b'-' | b'~') {
        let operand = skip_ascii_whitespace(bytes, index + 1, end);
        return !bytes.get(operand).is_some_and(|byte| {
            *byte >= 0x80
                || *byte == b'_'
                || byte.is_ascii_alphabetic()
                || byte.is_ascii_digit()
                || matches!(*byte, b'\'' | b'"' | b'(' | b'[' | b'{')
        });
    }

    [
        b"and".as_slice(),
        b"as".as_slice(),
        b"else".as_slice(),
        b"for".as_slice(),
        b"if".as_slice(),
        b"in".as_slice(),
        b"is".as_slice(),
        b"or".as_slice(),
    ]
    .iter()
    .any(|keyword| starts_identifier(bytes, index, keyword))
}

fn replacement_field_separator(bytes: &[u8], mut index: usize, end: usize) -> Option<usize> {
    let mut level = 0usize;
    while index < end {
        match bytes[index] {
            b'\'' | b'"' => index = skip_quoted_string(bytes, index),
            b'(' | b'[' | b'{' => {
                level += 1;
                index += 1;
            }
            b')' | b']' | b'}' if level > 0 => {
                level -= 1;
                index += 1;
            }
            b'=' | b'!' | b':' | b'}' if level == 0 => return Some(index),
            _ => index += 1,
        }
    }
    None
}

fn invalid_debug_expression_error(
    bytes: &[u8],
    equals: usize,
    end: usize,
    prefix: &str,
) -> Option<(String, usize, usize)> {
    let next = equals + 1;
    if next >= end || matches!(bytes[next], b'!' | b':' | b'}') {
        return None;
    }
    Some((
        format!("{prefix}: expecting '!', or ':', or '}}'"),
        next,
        next.saturating_add(1).min(end),
    ))
}

fn invalid_conversion_error(
    bytes: &[u8],
    bang: usize,
    end: usize,
    prefix: &str,
) -> Option<(String, usize, usize)> {
    let next = bang + 1;
    if next >= end {
        return Some((format!("{prefix}: expecting '}}'"), bang, bang + 1));
    }

    if bytes[next].is_ascii_whitespace() {
        let following = skip_ascii_whitespace(bytes, next, end);
        let message = if bytes
            .get(following)
            .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
        {
            "conversion type must come right after the exclamation mark"
        } else {
            "missing conversion character"
        };
        return Some((format!("{prefix}: {message}"), next, next + 1));
    }

    if matches!(bytes[next], b':' | b'}') {
        return Some((
            format!("{prefix}: missing conversion character"),
            next,
            next + 1,
        ));
    }

    if !bytes[next].is_ascii_alphabetic() && bytes[next] != b'_' {
        return Some((
            format!("{prefix}: invalid conversion character"),
            next,
            next + 1,
        ));
    }

    let conversion_end = identifier_end(bytes, next, end);
    let conversion = &bytes[next..conversion_end];
    if !matches!(conversion, b"s" | b"r" | b"a") {
        let conversion = ::core::str::from_utf8(conversion).unwrap_or("");
        return Some((
            format!(
                "{prefix}: invalid conversion character '{conversion}': expected 's', 'r', or 'a'"
            ),
            next,
            conversion_end,
        ));
    }

    if conversion_end >= end || matches!(bytes[conversion_end], b':' | b'}') {
        return None;
    }

    Some((
        format!("{prefix}: expecting ':' or '}}'"),
        conversion_end,
        conversion_end + 1,
    ))
}

fn invalid_format_spec_error(
    bytes: &[u8],
    colon: usize,
    end: usize,
    prefix: &str,
) -> Option<(String, usize, usize)> {
    if replacement_field_closing_brace(bytes, colon + 1, end).is_some() {
        return None;
    }
    Some((
        format!("{prefix}: expecting '}}', or format specs"),
        colon,
        colon + 1,
    ))
}

fn replacement_field_closing_brace(bytes: &[u8], mut index: usize, end: usize) -> Option<usize> {
    let mut level = 0usize;
    while index < end {
        match bytes[index] {
            b'\'' | b'"' => index = skip_quoted_string(bytes, index),
            b'{' => {
                level += 1;
                index += 1;
            }
            b'}' if level > 0 => {
                level -= 1;
                index += 1;
            }
            b'}' => return Some(index),
            _ => index += 1,
        }
    }
    None
}

fn skip_ascii_whitespace(bytes: &[u8], mut index: usize, end: usize) -> usize {
    while index < end && matches!(bytes[index], b' ' | b'\t' | b'\r' | b'\n' | 0x0c) {
        index += 1;
    }
    index
}

fn string_literal_end_at(bytes: &[u8], index: usize) -> Option<usize> {
    match bytes.get(index).copied()? {
        b'\'' | b'"' => Some(skip_quoted_string(bytes, index)),
        first if first.is_ascii_alphabetic() => {
            if matches!(bytes.get(index + 1), Some(b'\'' | b'"')) {
                return string_literal_prefix(bytes, index, index + 1)
                    .then(|| skip_quoted_string(bytes, index + 1));
            }
            if matches!(bytes.get(index + 2), Some(b'\'' | b'"')) {
                return string_literal_prefix(bytes, index, index + 2)
                    .then(|| skip_quoted_string(bytes, index + 2));
            }
            None
        }
        _ => None,
    }
}

fn string_literal_prefix(bytes: &[u8], start: usize, quote: usize) -> bool {
    let prefix = &bytes[start..quote];
    let valid = matches!(
        prefix,
        b"b" | b"B"
            | b"r"
            | b"R"
            | b"u"
            | b"U"
            | b"f"
            | b"F"
            | b"t"
            | b"T"
            | b"br"
            | b"bR"
            | b"Br"
            | b"BR"
            | b"rb"
            | b"rB"
            | b"Rb"
            | b"RB"
            | b"fr"
            | b"fR"
            | b"Fr"
            | b"FR"
            | b"rf"
            | b"rF"
            | b"Rf"
            | b"RF"
            | b"tr"
            | b"tR"
            | b"Tr"
            | b"TR"
            | b"rt"
            | b"rT"
            | b"Rt"
            | b"RT"
    );
    valid && (start == 0 || !is_ascii_identifier_char(bytes[start - 1]))
}

fn invalid_expression_error(source: &str) -> Option<(String, usize, usize)> {
    invalid_string_expression_error(source).or_else(|| missing_comma_expression_error(source))
}

fn invalid_string_expression_error(source: &str) -> Option<(String, usize, usize)> {
    let bytes = source.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if let Some(first_string_end) = string_literal_end_at(bytes, index) {
            let expr_start = skip_ascii_whitespace(bytes, first_string_end, bytes.len());
            if expression_atom_start(bytes, expr_start)
                && let Some(expr_end) = adjacent_atom_end(bytes, expr_start)
            {
                let next = skip_ascii_whitespace(bytes, expr_end, bytes.len());
                if string_literal_end_at(bytes, next).is_some() {
                    return Some((
                        "invalid syntax. Is this intended to be part of the string?".to_owned(),
                        expr_start,
                        expr_end,
                    ));
                }
            }
            index = first_string_end;
        } else {
            index += 1;
        }
    }
    None
}

fn missing_comma_expression_error(source: &str) -> Option<(String, usize, usize)> {
    let bytes = source.as_bytes();
    let mut stack: Vec<u8> = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'#' {
            while index < bytes.len() && bytes[index] != b'\n' {
                index += 1;
            }
        } else if let Some(string_end) = string_literal_end_at(bytes, index) {
            index = string_end;
        } else {
            match bytes[index] {
                b'(' | b'[' | b'{' => {
                    if bytes[index] == b'[' && opening_bracket_is_class_type_params(bytes, index) {
                        let Some(close) = matching_delimiter(bytes, index, b']') else {
                            index += 1;
                            continue;
                        };
                        index = close + 1;
                        continue;
                    }
                    stack.push(bytes[index]);
                    index += 1;
                }
                b')' | b']' | b'}' => {
                    stack.pop();
                    index += 1;
                }
                _ if !stack.is_empty() && expression_continuation_keyword(bytes, index) => {
                    index = identifier_end(bytes, index, bytes.len());
                }
                byte if !stack.is_empty() && expression_atom_start_byte(byte) => {
                    let atom_end = adjacent_atom_end(bytes, index).unwrap_or(index + 1);
                    let next = skip_ascii_whitespace(bytes, atom_end, bytes.len());
                    if next > atom_end
                        && expression_atom_start(bytes, next)
                        && !expression_continuation_keyword(bytes, next)
                    {
                        return Some((
                            "invalid syntax. Perhaps you forgot a comma?".to_owned(),
                            index,
                            next + 1,
                        ));
                    }
                    index = atom_end;
                }
                _ => index += 1,
            }
        }
    }
    None
}

fn opening_bracket_is_class_type_params(bytes: &[u8], bracket: usize) -> bool {
    let mut cursor = bracket;
    while cursor > 0 && matches!(bytes[cursor - 1], b' ' | b'\t' | b'\x0c') {
        cursor -= 1;
    }
    while cursor > 0
        && bytes
            .get(cursor - 1)
            .is_some_and(|byte| *byte >= 0x80 || is_ascii_identifier_char(*byte))
    {
        cursor -= 1;
    }
    while cursor > 0 && matches!(bytes[cursor - 1], b' ' | b'\t' | b'\x0c') {
        cursor -= 1;
    }
    cursor >= 5 && starts_identifier(bytes, cursor - 5, b"class")
}

fn expression_continuation_keyword(bytes: &[u8], index: usize) -> bool {
    [
        b"and".as_slice(),
        b"else".as_slice(),
        b"for".as_slice(),
        b"if".as_slice(),
        b"in".as_slice(),
        b"is".as_slice(),
        b"not".as_slice(),
        b"or".as_slice(),
    ]
    .iter()
    .any(|keyword| starts_identifier(bytes, index, keyword))
}

fn expression_atom_start(bytes: &[u8], index: usize) -> bool {
    bytes
        .get(index)
        .is_some_and(|byte| expression_atom_start_byte(*byte))
        || string_literal_end_at(bytes, index).is_some()
}

fn expression_atom_start_byte(byte: u8) -> bool {
    byte >= 0x80
        || byte == b'_'
        || byte.is_ascii_alphabetic()
        || byte.is_ascii_digit()
        || matches!(byte, b'\'' | b'"' | b'(' | b'[' | b'{')
}

fn adjacent_atom_end(bytes: &[u8], index: usize) -> Option<usize> {
    if let Some(string_end) = string_literal_end_at(bytes, index) {
        return Some(string_end);
    }
    match bytes.get(index).copied()? {
        byte if byte >= 0x80 || byte == b'_' || byte.is_ascii_alphabetic() => {
            Some(identifier_end(bytes, index, bytes.len()))
        }
        byte if byte.is_ascii_digit() => {
            let mut end = index + 1;
            while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
                end += 1;
            }
            Some(end)
        }
        b'(' | b'[' | b'{' => Some(index + 1),
        _ => None,
    }
}

fn unterminated_string_message(
    detected_line: usize,
    triple: bool,
    has_escaped_quote: bool,
) -> String {
    if triple {
        format!("unterminated triple-quoted string literal (detected at line {detected_line})")
    } else if has_escaped_quote {
        format!(
            "unterminated string literal (detected at line {detected_line}); perhaps you escaped the end quote?"
        )
    } else {
        format!("unterminated string literal (detected at line {detected_line})")
    }
}

fn expected_opening_bracket(closing: char) -> char {
    match closing {
        ')' => '(',
        ']' => '[',
        '}' => '{',
        _ => unreachable!(),
    }
}

fn bracket_syntax_error(source: &str) -> Option<(String, usize, usize, bool)> {
    let mut stack: Vec<(char, usize, usize)> = Vec::new();
    let mut in_string = false;
    let mut string_quote = '\0';
    let mut triple_quote = false;
    let mut escape_next = false;
    let mut is_raw_string = false;
    let mut line = 1usize;

    let chars: Vec<(usize, char)> = source.char_indices().collect();
    let mut index = 0;
    while index < chars.len() {
        let (byte_offset, ch) = chars[index];

        if ch == '\n' {
            line += 1;
        }

        if escape_next {
            escape_next = false;
            index += 1;
            continue;
        }

        if in_string {
            if ch == '\\' && !is_raw_string {
                escape_next = true;
            } else if triple_quote {
                if ch == string_quote
                    && index + 2 < chars.len()
                    && chars[index + 1].1 == string_quote
                    && chars[index + 2].1 == string_quote
                {
                    in_string = false;
                    index += 3;
                    continue;
                }
            } else if ch == string_quote {
                in_string = false;
            }
            index += 1;
            continue;
        }

        if ch == '#' {
            while index < chars.len() && chars[index].1 != '\n' {
                index += 1;
            }
            continue;
        }

        if ch == '\'' || ch == '"' {
            is_raw_string = false;
            for look_back in 1..=2.min(index) {
                let prev = chars[index - look_back].1;
                if matches!(prev, 'r' | 'R') {
                    is_raw_string = true;
                    break;
                }
                if !matches!(prev, 'b' | 'B' | 'f' | 'F' | 'u' | 'U') {
                    break;
                }
            }
            string_quote = ch;
            if index + 2 < chars.len() && chars[index + 1].1 == ch && chars[index + 2].1 == ch {
                triple_quote = true;
                in_string = true;
                index += 3;
                continue;
            }
            triple_quote = false;
            in_string = true;
            index += 1;
            continue;
        }

        match ch {
            '(' | '[' | '{' => stack.push((ch, byte_offset, line)),
            ')' | ']' | '}' => {
                let expected = expected_opening_bracket(ch);
                let Some(&(opening, _, opening_line)) = stack.last() else {
                    return Some((format!("unmatched '{ch}'"), byte_offset, byte_offset, false));
                };
                if opening == expected {
                    stack.pop();
                } else {
                    let suffix = if opening_line != line {
                        format!(" on line {opening_line}")
                    } else {
                        String::new()
                    };
                    return Some((
                        format!(
                            "closing parenthesis '{ch}' does not match opening parenthesis '{opening}'{suffix}"
                        ),
                        byte_offset,
                        byte_offset,
                        false,
                    ));
                }
            }
            _ => {}
        }

        index += 1;
    }

    stack.last().map(|(opening, byte_offset, _)| {
        (
            format!("'{opening}' was never closed"),
            *byte_offset,
            *byte_offset,
            true,
        )
    })
}

fn is_legacy_statement_expression_start(byte: u8) -> bool {
    byte >= 0x80
        || byte == b'_'
        || byte.is_ascii_alphabetic()
        || byte.is_ascii_digit()
        || matches!(byte, b'\'' | b'"' | b'{' | b'[')
}

fn legacy_statement_container_has_invalid_attribute(bytes: &[u8], start: usize) -> bool {
    let Some(&opening) = bytes.get(start) else {
        return false;
    };
    if !matches!(opening, b'{' | b'[') {
        return false;
    }

    let mut index = start;
    let mut level = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'#' => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'\n' | b';' if level == 0 => return false,
            b'\'' | b'"' => {
                index = skip_quoted_string(bytes, index);
            }
            b'(' | b'[' | b'{' => {
                level += 1;
                index += 1;
            }
            b')' | b']' | b'}' => {
                level = level.saturating_sub(1);
                index += 1;
                if level == 0 {
                    return false;
                }
            }
            b'.' => {
                let mut cursor = index + 1;
                while matches!(bytes.get(cursor), Some(b' ' | b'\t' | b'\x0c')) {
                    cursor += 1;
                }
                if matches!(bytes.get(cursor), Some(b')' | b']' | b'}')) {
                    return true;
                }
                index += 1;
            }
            _ => index += 1,
        }
    }
    false
}

fn invalid_legacy_statement_error(source: &str) -> Option<(String, usize, usize)> {
    let bytes = source.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'#' => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'\'' | b'"' => {
                index = skip_quoted_string(bytes, index);
            }
            b'p' | b'e' => {
                let keyword = if starts_identifier(bytes, index, b"print") {
                    Some("print")
                } else if starts_identifier(bytes, index, b"exec") {
                    Some("exec")
                } else {
                    None
                };
                let Some(keyword) = keyword else {
                    index += 1;
                    continue;
                };
                let after_keyword = index + keyword.len();
                if !matches!(bytes.get(after_keyword), Some(b' ' | b'\t' | b'\x0c')) {
                    index = after_keyword;
                    continue;
                }
                let mut cursor = after_keyword;
                while matches!(bytes.get(cursor), Some(b' ' | b'\t' | b'\x0c')) {
                    cursor += 1;
                }
                if legacy_statement_container_has_invalid_attribute(bytes, cursor) {
                    index = after_keyword;
                    continue;
                }
                if bytes.get(cursor).is_some_and(|byte| {
                    *byte != b'(' && is_legacy_statement_expression_start(*byte)
                }) {
                    return Some((
                        format!(
                            "Missing parentheses in call to '{keyword}'. Did you mean {keyword}(...)?"
                        ),
                        index,
                        after_keyword,
                    ));
                }
                index = after_keyword;
            }
            _ => index += 1,
        }
    }
    None
}

fn long_decimal_integer_literal_error(
    source: &str,
    max_str_digits: usize,
) -> Option<(String, usize, usize)> {
    if max_str_digits == 0 {
        return None;
    }
    let bytes = source.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'#' => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'\'' | b'"' => {
                index = skip_quoted_string(bytes, index);
            }
            byte if byte >= 0x80 || byte == b'_' || byte.is_ascii_alphabetic() => {
                index += 1;
                while index < bytes.len()
                    && (bytes[index] >= 0x80 || is_ascii_identifier_char(bytes[index]))
                {
                    index += 1;
                }
            }
            b'.' => {
                if bytes
                    .get(index + 1)
                    .is_some_and(|byte| byte.is_ascii_digit())
                {
                    let (_, end) = number_literal_end(bytes, index)?;
                    index = end.max(index + 1);
                } else {
                    index += 1;
                }
            }
            b'0'..=b'9' => {
                if bytes.get(index) == Some(&b'0')
                    && matches!(
                        bytes.get(index + 1),
                        Some(b'x' | b'X' | b'o' | b'O' | b'b' | b'B')
                    )
                {
                    let Some((_, end)) = number_literal_end(bytes, index) else {
                        index += 1;
                        continue;
                    };
                    index = end.max(index + 1);
                    continue;
                }

                let start = index;
                let mut digits = 0usize;
                while index < bytes.len() {
                    match bytes[index] {
                        b'0'..=b'9' => {
                            digits += 1;
                            index += 1;
                        }
                        b'_' if bytes
                            .get(index + 1)
                            .is_some_and(|byte| byte.is_ascii_digit()) =>
                        {
                            index += 1;
                        }
                        _ => break,
                    }
                }
                if matches!(bytes.get(index), Some(b'.' | b'e' | b'E' | b'j' | b'J')) {
                    let Some((_, end)) = number_literal_end(bytes, start) else {
                        continue;
                    };
                    index = end.max(index + 1);
                    continue;
                }
                if digits > max_str_digits {
                    return Some((
                        format!(
                            "Exceeds the limit ({max_str_digits} digits) for integer string conversion: value has {digits} digits; use sys.set_int_max_str_digits() to increase the limit - Consider hexadecimal for huge integer literals to avoid decimal conversion limits."
                        ),
                        start,
                        start,
                    ));
                }
            }
            _ => index += 1,
        }
    }
    None
}

fn invalid_parenthesized_import_star_error(source: &str) -> Option<(String, usize, usize)> {
    let bytes = source.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'#' => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'\'' | b'"' => {
                index = skip_quoted_string(bytes, index);
            }
            b'f' if starts_identifier(bytes, index, b"from") => {
                let mut cursor = index + 4;
                while cursor < bytes.len() && !matches!(bytes[cursor], b'\n' | b';') {
                    if starts_identifier(bytes, cursor, b"import") {
                        cursor += 6;
                        while matches!(bytes.get(cursor), Some(b' ' | b'\t' | b'\r')) {
                            cursor += 1;
                        }
                        if bytes.get(cursor) == Some(&b'(') {
                            cursor += 1;
                            while cursor < bytes.len()
                                && !matches!(bytes[cursor], b')' | b'\n' | b';')
                            {
                                if bytes[cursor] == b'*' {
                                    return Some(("invalid syntax".to_owned(), cursor, cursor + 1));
                                }
                                cursor += 1;
                            }
                        }
                        break;
                    }
                    cursor += 1;
                }
                index = cursor;
            }
            _ => index += 1,
        }
    }
    None
}

fn too_many_nested_parentheses_error(source: &str) -> Option<(String, usize, usize)> {
    const MAXLEVEL: usize = 200;

    let bytes = source.as_bytes();
    let mut index = 0;
    let mut level = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'#' => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'\'' | b'"' => {
                index = skip_quoted_string(bytes, index);
            }
            b'(' | b'[' | b'{' => {
                if level >= MAXLEVEL {
                    return Some(("too many nested parentheses".to_owned(), index, index + 1));
                }
                level += 1;
                index += 1;
            }
            b')' | b']' | b'}' => {
                level = level.saturating_sub(1);
                index += 1;
            }
            _ => index += 1,
        }
    }
    None
}

fn invalid_unparenthesized_yield_after_comma_error(source: &str) -> Option<(String, usize, usize)> {
    let bytes = source.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'#' => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'\'' | b'"' => {
                index = skip_quoted_string(bytes, index);
            }
            b',' => {
                let mut cursor = index + 1;
                while matches!(bytes.get(cursor), Some(b' ' | b'\t' | b'\x0c')) {
                    cursor += 1;
                }
                if starts_identifier(bytes, cursor, b"yield") {
                    return Some(("invalid syntax".to_owned(), cursor, cursor + 5));
                }
                index += 1;
            }
            _ => index += 1,
        }
    }
    None
}

fn post_parse_source_error(source_file: &SourceFile, opts: &CompileOpts) -> Option<CompileError> {
    too_many_nested_parentheses_error(source_file.source_text())
        .or_else(|| {
            long_decimal_integer_literal_error(source_file.source_text(), opts.int_max_str_digits)
        })
        .or_else(|| invalid_call_argument_error(source_file.source_text()))
        .or_else(|| invalid_match_mapping_rest_wildcard_error(source_file.source_text()))
        .or_else(|| invalid_match_as_target_error(source_file.source_text()))
        .or_else(|| invalid_unparenthesized_yield_after_comma_error(source_file.source_text()))
        .or_else(|| invalid_parenthesized_import_star_error(source_file.source_text()))
        .map(|(message, start, end)| {
            CompileError::from_source_error(source_file, message, start, end)
        })
}

fn is_compound_stmt(stmt: &ast::Stmt) -> bool {
    matches!(
        stmt,
        ast::Stmt::FunctionDef(_)
            | ast::Stmt::ClassDef(_)
            | ast::Stmt::If(_)
            | ast::Stmt::For(_)
            | ast::Stmt::While(_)
            | ast::Stmt::With(_)
            | ast::Stmt::Try(_)
            | ast::Stmt::Match(_)
    )
}

fn single_mode_body_error(body: &[ast::Stmt], source_file: &SourceFile) -> Option<CompileError> {
    let first = body.first()?;
    let source_code = source_file.to_source_code();
    let first_start = source_code.source_location(first.range().start(), PositionEncoding::Utf8);
    let first_end = source_code.source_location(first.range().end(), PositionEncoding::Utf8);

    if body.iter().skip(1).any(|stmt| {
        source_code
            .source_location(stmt.range().start(), PositionEncoding::Utf8)
            .line
            > first_start.line
    }) {
        return Some(CompileError::from_source_error(
            source_file,
            "multiple statements found while compiling a single statement".to_owned(),
            first.range().end().to_usize(),
            first.range().end().to_usize(),
        ));
    }

    if is_compound_stmt(first)
        && first_start.line == first_end.line
        && !ends_with_line_break(source_file.source_text())
    {
        return Some(CompileError::from_source_error(
            source_file,
            "invalid syntax".to_owned(),
            first.range().start().to_usize(),
            first.range().start().to_usize(),
        ));
    }
    None
}

fn single_mode_source_error(ast: &ast::Mod, source_file: &SourceFile) -> Option<CompileError> {
    let ast::Mod::Module(module) = ast else {
        return None;
    };
    single_mode_body_error(&module.body, source_file)
}

fn ends_with_line_break(source: &str) -> bool {
    source.ends_with('\n') || source.ends_with('\r')
}

fn ends_with_implied_dedent(source: &str) -> bool {
    let mut lexer = parser::lexer::lex(source, parser::Mode::Module);
    let mut last_kind = TokenKind::EndOfFile;
    loop {
        let kind = lexer.next_token();
        if kind.is_eof() {
            break;
        }
        last_kind = kind;
    }
    matches!(last_kind, TokenKind::Dedent)
}

/// Detect input that only parses because Ruff's lexer closes indentation at EOF.
///
/// `PyCF_DONT_IMPLY_DEDENT` is used by `codeop` and interactive compile
/// paths to keep an indented block incomplete until a terminating newline is seen.
#[must_use]
pub fn dont_imply_dedent_source_error(source_file: &SourceFile) -> Option<CompileError> {
    let source = source_file.source_text();
    if ends_with_line_break(source) || !ends_with_implied_dedent(source) {
        return None;
    }
    let eof = source.len();
    Some(CompileError::from_source_error(
        source_file,
        "incomplete input".to_owned(),
        eof,
        eof,
    ))
}

/// Find the last unclosed opening bracket in source code.
/// Returns the bracket character and its byte offset, or None if all brackets are balanced.
fn find_unclosed_bracket(source: &str) -> Option<(char, usize)> {
    let mut stack: Vec<(char, usize)> = Vec::new();
    let mut in_string = false;
    let mut string_quote = '\0';
    let mut triple_quote = false;
    let mut escape_next = false;
    let mut is_raw_string = false;

    let chars: Vec<(usize, char)> = source.char_indices().collect();
    let mut i = 0;

    while i < chars.len() {
        let (byte_offset, ch) = chars[i];

        if escape_next {
            escape_next = false;
            i += 1;
            continue;
        }

        if in_string {
            if ch == '\\' && !is_raw_string {
                escape_next = true;
            } else if triple_quote {
                if ch == string_quote
                    && i + 2 < chars.len()
                    && chars[i + 1].1 == string_quote
                    && chars[i + 2].1 == string_quote
                {
                    in_string = false;
                    i += 3;
                    continue;
                }
            } else if ch == string_quote {
                in_string = false;
            }
            i += 1;
            continue;
        }

        // Check for comments
        if ch == '#' {
            // Skip to end of line
            while i < chars.len() && chars[i].1 != '\n' {
                i += 1;
            }
            continue;
        }

        // Check for string start (with optional prefix like r, b, f, u, rb, br, etc.)
        if ch == '\'' || ch == '"' {
            // Check up to 2 characters before the quote for string prefix
            is_raw_string = false;
            for look_back in 1..=2.min(i) {
                let prev = chars[i - look_back].1;
                if matches!(prev, 'r' | 'R') {
                    is_raw_string = true;
                    break;
                }
                if !matches!(prev, 'b' | 'B' | 'f' | 'F' | 'u' | 'U') {
                    break;
                }
            }
            string_quote = ch;
            if i + 2 < chars.len() && chars[i + 1].1 == ch && chars[i + 2].1 == ch {
                triple_quote = true;
                in_string = true;
                i += 3;
                continue;
            }
            triple_quote = false;
            in_string = true;
            i += 1;
            continue;
        }

        match ch {
            '(' | '[' | '{' => stack.push((ch, byte_offset)),
            ')' | ']' | '}' => {
                let expected = match ch {
                    ')' => '(',
                    ']' => '[',
                    '}' => '{',
                    _ => unreachable!(),
                };
                if stack.last().is_some_and(|&(open, _)| open == expected) {
                    stack.pop();
                }
            }
            _ => {}
        }

        i += 1;
    }

    stack.last().copied()
}

/// Compile a given source code into a bytecode object.
pub fn compile(
    source: &str,
    mode: Mode,
    source_path: &str,
    opts: CompileOpts,
) -> Result<CodeObject, CompileError> {
    // TODO: do this less hacky; ruff's parser should translate a CRLF line
    //       break in a multiline string into just an LF in the parsed value
    #[cfg(windows)]
    let source = source.replace("\r\n", "\n");
    #[cfg(windows)]
    let source = source.as_str();

    let source_file = SourceFileBuilder::new(source_path, source).finish();
    _compile(source_file, mode, opts)
    // let index = LineIndex::from_source_text(source);
    // let source_code = SourceCode::new(source, &index);
    // let mut locator = LinearLocator::new(source);
    // let mut ast = match parser::parse(source, mode.into(), &source_path) {
    //     Ok(x) => x,
    //     Err(e) => return Err(locator.locate_error(e)),
    // };

    // TODO:
    // if opts.optimize > 0 {
    //     ast = ConstantOptimizer::new()
    //         .fold_mod(ast)
    //         .unwrap_or_else(|e| match e {});
    // }
    // let ast = locator.fold_mod(ast).unwrap_or_else(|e| match e {});
}

fn _compile(
    source_file: SourceFile,
    mode: Mode,
    opts: CompileOpts,
) -> Result<CodeObject, CompileError> {
    _compile_with_syntax_warning_handler(source_file, mode, opts, None)
}

fn _compile_with_syntax_warning_handler<'a>(
    source_file: SourceFile,
    mode: Mode,
    opts: CompileOpts,
    syntax_warning_handler: Option<&'a mut compile::SyntaxWarningHandler<'a>>,
) -> Result<CodeObject, CompileError> {
    let parser_mode = match mode {
        Mode::Exec => parser::Mode::Module,
        Mode::Eval => parser::Mode::Expression,
        // ruff does not have an interactive mode, which is fine,
        // since these are only different in terms of compilation
        Mode::Single | Mode::BlockExpr => parser::Mode::Module,
    };
    let parser_options = parser::ParseOptions::from(parser_mode);
    let parsed = parser::parse(source_file.source_text(), parser_options)
        .map_err(|err| CompileError::from_ruff_parse_error(err, &source_file))?;
    if opts.dont_imply_dedent
        && matches!(mode, Mode::Single)
        && let Some(error) = dont_imply_dedent_source_error(&source_file)
    {
        return Err(error);
    }
    if let Some(error) = post_parse_source_error(&source_file, &opts) {
        return Err(error);
    }
    let ast = parsed.into_syntax();
    let single_mode_error = matches!(mode, Mode::Single)
        .then(|| single_mode_source_error(&ast, &source_file))
        .flatten();
    let code = compile::compile_top_with_syntax_warning_handler(
        ast,
        source_file,
        mode,
        opts,
        syntax_warning_handler,
    )
    .map_err(CompileError::from)?;
    if let Some(error) = single_mode_error {
        return Err(error);
    }
    Ok(code)
}

pub fn compile_with_syntax_warning_handler<'a>(
    source: &str,
    mode: Mode,
    source_path: &str,
    opts: CompileOpts,
    syntax_warning_handler: &'a mut compile::SyntaxWarningHandler<'a>,
) -> Result<CodeObject, CompileError> {
    let source = source.replace("\r\n", "\n");
    #[cfg(windows)]
    let source = source.as_str();

    let source_file = SourceFileBuilder::new(source_path, source).finish();
    _compile_with_syntax_warning_handler(source_file, mode, opts, Some(syntax_warning_handler))
}

pub fn compile_symtable(
    source: &str,
    mode: Mode,
    source_path: &str,
) -> Result<symboltable::SymbolTable, CompileError> {
    let source_file = SourceFileBuilder::new(source_path, source).finish();
    _compile_symtable(source_file, mode)
}

pub fn _compile_symtable(
    source_file: SourceFile,
    mode: Mode,
) -> Result<symboltable::SymbolTable, CompileError> {
    let res = match mode {
        Mode::Exec | Mode::Single | Mode::BlockExpr => {
            let ast = ruff_python_parser::parse_module(source_file.source_text())
                .map_err(|e| CompileError::from_ruff_parse_error(e, &source_file))?;
            if let Some(error) = post_parse_source_error(&source_file, &CompileOpts::default()) {
                return Err(error);
            }
            let ast = ast.into_syntax();
            if matches!(mode, Mode::Single)
                && let Some(error) = single_mode_body_error(&ast.body, &source_file)
            {
                return Err(error);
            }
            symboltable::SymbolTable::scan_program(&ast, source_file.clone())
        }
        Mode::Eval => {
            let ast = ruff_python_parser::parse(
                source_file.source_text(),
                parser::Mode::Expression.into(),
            )
            .map_err(|e| CompileError::from_ruff_parse_error(e, &source_file))?;
            if let Some(error) = post_parse_source_error(&source_file, &CompileOpts::default()) {
                return Err(error);
            }
            symboltable::SymbolTable::scan_expr(
                &ast.into_syntax().expect_expression(),
                source_file.clone(),
            )
        }
    };
    res.map_err(|e| e.into_codegen_error(source_file.name().to_owned()).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_compile() {
        let code = "x = 'abc'";
        let compiled = compile(code, Mode::Single, "<>", CompileOpts::default());
        dbg!(compiled.expect("compile error"));
    }

    #[test]
    fn dont_imply_dedent_requires_terminating_newline() {
        let code = "if True:\n    pass";

        let opts = CompileOpts {
            dont_imply_dedent: true,
            ..CompileOpts::default()
        };
        let err = compile(code, Mode::Single, "<>", opts.clone()).expect_err("compile succeeded");
        assert_eq!(err.to_string(), "incomplete input");

        compile("if True:\n    pass\n", Mode::Single, "<>", opts).expect("compile error");
        compile(code, Mode::Single, "<>", CompileOpts::default()).expect("compile error");
    }

    #[test]
    fn compile_phello() {
        let code = r#"
initialized = True
def main():
    print("Hello world!")
if __name__ == '__main__':
    main()
"#;
        let compiled = compile(code, Mode::Exec, "<>", CompileOpts::default());
        dbg!(compiled.expect("compile error"));
    }

    #[test]
    fn compile_if_elif_else() {
        let code = r#"
if False:
    pass
elif False:
    pass
elif False:
    pass
else:
    pass
"#;
        let compiled = compile(code, Mode::Exec, "<>", CompileOpts::default());
        dbg!(compiled.expect("compile error"));
    }

    #[test]
    fn compile_lambda() {
        let code = r#"
lambda: 'a'
"#;
        let compiled = compile(code, Mode::Exec, "<>", CompileOpts::default());
        dbg!(compiled.expect("compile error"));
    }

    #[test]
    fn compile_lambda2() {
        let code = r#"
(lambda x: f'hello, {x}')('world}')
"#;
        let compiled = compile(code, Mode::Exec, "<>", CompileOpts::default());
        dbg!(compiled.expect("compile error"));
    }

    #[test]
    fn compile_lambda3() {
        let code = r#"
def g():
    pass
def f():
    if False:
        return lambda x: g(x)
    elif False:
        return g
    else:
        return g
"#;
        let compiled = compile(code, Mode::Exec, "<>", CompileOpts::default());
        dbg!(compiled.expect("compile error"));
    }

    #[test]
    fn compile_call_arg_lambda_default() {
        let code = "signature((lambda a=10: a))";
        let compiled = compile(code, Mode::Exec, "<>", CompileOpts::default());
        dbg!(compiled.expect("compile error"));
    }

    #[test]
    fn compile_generic_function_parameter_default() {
        let code = "def __repr__[T: str](self, default: T = '') -> str: pass";
        let compiled = compile(code, Mode::Exec, "<>", CompileOpts::default());
        dbg!(compiled.expect("compile error"));
    }

    #[test]
    fn compile_int() {
        let code = r#"
a = 0xFF
"#;
        let compiled = compile(code, Mode::Exec, "<>", CompileOpts::default());
        dbg!(compiled.expect("compile error"));
    }

    #[test]
    fn compile_bigint() {
        let code = r#"
a = 0xFFFFFFFFFFFFFFFFFFFFFFFF
"#;
        let compiled = compile(code, Mode::Exec, "<>", CompileOpts::default());
        dbg!(compiled.expect("compile error"));
    }

    #[test]
    fn compile_fstring() {
        let code1 = r#"
assert f"1" == '1'
    "#;
        let compiled = compile(code1, Mode::Exec, "<>", CompileOpts::default());
        dbg!(compiled.expect("compile error"));

        let code2 = r#"
assert f"{1}" == '1'
    "#;
        let compiled = compile(code2, Mode::Exec, "<>", CompileOpts::default());
        dbg!(compiled.expect("compile error"));
        let code3 = r#"
assert f"{1+1}" == '2'
    "#;
        let compiled = compile(code3, Mode::Exec, "<>", CompileOpts::default());
        dbg!(compiled.expect("compile error"));

        let code4 = r#"
assert f"{{{(lambda: f'{1}')}" == '{1'
    "#;
        let compiled = compile(code4, Mode::Exec, "<>", CompileOpts::default());
        dbg!(compiled.expect("compile error"));

        let code5 = r#"
assert f"a{1}" == 'a1'
    "#;
        let compiled = compile(code5, Mode::Exec, "<>", CompileOpts::default());
        dbg!(compiled.expect("compile error"));

        let code6 = r#"
assert f"{{{(lambda x: f'hello, {x}')('world}')}" == '{hello, world}'
    "#;
        let compiled = compile(code6, Mode::Exec, "<>", CompileOpts::default());
        dbg!(compiled.expect("compile error"));
    }

    #[test]
    fn simple_enum() {
        let code = r#"
import enum
@enum._simple_enum(enum.IntFlag, boundary=enum.KEEP)
class RegexFlag:
    NOFLAG = 0
    DEBUG = 1
print(RegexFlag.NOFLAG & RegexFlag.DEBUG)
"#;
        let compiled = compile(code, Mode::Exec, "<string>", CompileOpts::default());
        dbg!(compiled.expect("compile error"));
    }
}

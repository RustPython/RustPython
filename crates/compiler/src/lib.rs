extern crate alloc;

use alloc::borrow::Cow;
pub use ruff_python_ast::token::{TokenKind, Tokens};
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
    /// Set when a string is still open at EOF and more input could close it.
    pub is_unclosed_string: bool,
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
    pub fn from_ruff_parse_error(
        error: parser::ParseError,
        source_file: &SourceFile,
        mode: Mode,
    ) -> Self {
        let raw_location = error.location;
        let diagnostic = match cpython_parse_diagnostic_override(&error, source_file, mode) {
            Some(diagnostic) => diagnostic,
            None => {
                let is_unclosed_bracket = matches!(
                    error.error,
                    parser::ParseErrorType::Lexical(parser::LexicalErrorType::UnclosedBracket {
                        incomplete: true,
                        ..
                    })
                );
                let is_unclosed_string =
                    unclosed_string_is_continuable(&error, source_file.source_text(), mode);
                let mut diagnostic = default_parse_diagnostic(error, source_file);
                diagnostic.is_unclosed_bracket = is_unclosed_bracket;
                diagnostic.is_unclosed_string = is_unclosed_string;
                diagnostic
            }
        };

        Self::Parse(ParseError {
            error: diagnostic.error,
            raw_location,
            location: diagnostic.location,
            end_location: diagnostic.end_location,
            source_path: source_file.name().to_owned(),
            is_unclosed_bracket: diagnostic.is_unclosed_bracket,
            is_unclosed_string: diagnostic.is_unclosed_string,
        })
    }

    fn from_source_error(source_file: &SourceFile, diagnostic: CpythonDiagnostic) -> Self {
        let (location, end_location) = source_locations(
            source_file,
            diagnostic.range.start(),
            diagnostic.range.end(),
        );
        Self::Parse(ParseError {
            error: parser::ParseErrorType::OtherError(diagnostic.message),
            raw_location: diagnostic.range,
            location,
            end_location,
            source_path: source_file.name().to_owned(),
            is_unclosed_bracket: false,
            is_unclosed_string: false,
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
            Self::Codegen(codegen_error) => codegen_error
                .end_location
                .map(|end| (end.line.get(), end.character_offset.get())),
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

// A syntax error the parser reports counts its columns in characters rather
// than in the bytes the range is measured in. An offset that lands inside a
// character walks back to where that character starts, the way decoding the
// line up to a truncated offset would.
fn source_location(source_file: &SourceFile, offset: TextSize) -> SourceLocation {
    let text = source_file.source_text();
    let mut index = offset.to_usize().min(text.len());
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    source_file
        .to_source_code()
        .source_location(TextSize::new(index as u32), PositionEncoding::Utf32)
}

fn source_locations(
    source_file: &SourceFile,
    start: TextSize,
    end: TextSize,
) -> (SourceLocation, SourceLocation) {
    (
        source_location(source_file, start),
        source_location(source_file, end),
    )
}

struct NormalizedParseDiagnostic {
    error: parser::ParseErrorType,
    location: SourceLocation,
    end_location: SourceLocation,
    is_unclosed_bracket: bool,
    is_unclosed_string: bool,
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
            is_unclosed_string: false,
        }
    }

    const fn with_unclosed_bracket(mut self, is_unclosed_bracket: bool) -> Self {
        self.is_unclosed_bracket = is_unclosed_bracket;
        self
    }
}

/// What CPython would have reported for a piece of source, before it is resolved to a line and
/// column. These are reconstructed by re-scanning after ruff's parse has already failed, so they
/// carry CPython's wording rather than a translation of ruff's own error, and they never reach
/// ruff — `NormalizedParseDiagnostic` and `CompileError` are the only things that consume one.
#[derive(Clone)]
struct CpythonDiagnostic {
    message: String,
    range: ruff_text_size::TextRange,
}

impl CpythonDiagnostic {
    /// The `u32` cast is ruff's invariant rather than one this adds: its `Lexer::new` asserts
    /// that the source fits in a `u32` ("Lexer only supports files with a size up to 4GB") and
    /// relies on that for its own offset arithmetic, and nothing here runs until that lexer has
    /// read the source and the parse built on it has failed.
    fn new(message: String, start: usize, end: usize) -> Self {
        Self {
            message,
            range: ruff_text_size::TextRange::new(
                TextSize::new(start as u32),
                TextSize::new(end as u32),
            ),
        }
    }
}

fn cpython_parse_diagnostic_override(
    error: &parser::ParseError,
    source_file: &SourceFile,
    mode: Mode,
) -> Option<NormalizedParseDiagnostic> {
    let source_text = source_file.source_text();

    let tokenizer_error = matches!(
        &error.error,
        parser::ParseErrorType::Lexical(lexical) if lexical.is_tokenizer_error()
    );
    let line_continuation = matches!(
        &error.error,
        parser::ParseErrorType::Lexical(parser::LexicalErrorType::LineContinuationError)
    );
    if line_continuation {
        // exec input gets an implicit trailing newline, so a final `\` is a
        // continuation that then hits EOF (`E_EOF`). single/eval see `\` at
        // EOF as `E_LINECONT` instead.
        if matches!(mode, Mode::Exec) && error.location.start().to_usize() == source_text.len() {
            let source_with_newline = format!("{source_text}\n");
            let parsed = parser::parse_unchecked(
                &source_with_newline,
                parser::ParseOptions::from(parser::Mode::Module),
            );
            if let Some(unclosed) = parsed.errors().first().filter(|first| {
                matches!(
                    first.error,
                    parser::ParseErrorType::Lexical(
                        parser::LexicalErrorType::UnclosedBracket { .. }
                    )
                )
            }) {
                let mut diagnostic = default_parse_diagnostic(unclosed.clone(), source_file);
                diagnostic.is_unclosed_bracket = matches!(
                    unclosed.error,
                    parser::ParseErrorType::Lexical(parser::LexicalErrorType::UnclosedBracket {
                        incomplete: true,
                        ..
                    })
                );
                return Some(diagnostic);
            }
            let loc = source_line_end_location(source_file, error.location.start());
            return Some(NormalizedParseDiagnostic::new(
                parser::ParseErrorType::OtherError("unexpected EOF while parsing".to_owned()),
                loc,
                loc,
            ));
        }
        return None;
    }

    if matches!(
        &error.error,
        parser::ParseErrorType::Lexical(parser::LexicalErrorType::Eof)
    ) {
        return Some(eof_parse_diagnostic(error, source_file));
    }
    if tokenizer_error
        || matches!(
            &error.error,
            parser::ParseErrorType::ExpectedIndentedBlock { .. }
        )
    {
        return None;
    }

    // CPython's PEG parser collapses a bare "expected an expression" failure
    // into the generic "invalid syntax" message. rustpython-vm's `vm_new.rs`
    // does this same collapse for its own callers; rustpython-compiler has no
    // vm dependency, so mirror it here.
    if matches!(
        &error.error,
        parser::ParseErrorType::ExpectedExpression
            | parser::ParseErrorType::UnexpectedExpressionToken
    ) {
        let (loc, end_loc) = adjusted_error_locations(source_file, error.location);
        return Some(NormalizedParseDiagnostic::new(
            parser::ParseErrorType::OtherError("invalid syntax".into()),
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

/// Returns whether more input could still terminate the string that `error` reports as
/// unterminated.
///
/// A string that reached the end of the source can continue, except a single-quoted f/t-string,
/// a single-quoted string in exec input whose implicit final newline ends it, and a string after
/// an assignment in eval input.
fn unclosed_string_is_continuable(error: &parser::ParseError, source: &str, mode: Mode) -> bool {
    use parser::{InterpolatedStringErrorType, LexicalErrorType};

    let parser::ParseErrorType::Lexical(lexical) = &error.error else {
        return false;
    };
    let start = error.location.start().to_usize();
    let at_eof = match lexical {
        LexicalErrorType::UnclosedStringError {
            triple_quoted: true,
            ..
        }
        | LexicalErrorType::FStringError(
            InterpolatedStringErrorType::UnterminatedTripleQuotedString { .. },
        )
        | LexicalErrorType::TStringError(
            InterpolatedStringErrorType::UnterminatedTripleQuotedString { .. },
        ) => true,
        LexicalErrorType::UnclosedStringError { .. } => {
            // Exec mode appends a newline, which only a final `\` can escape.
            let rest = &source[start..];
            !has_unescaped_newline(rest) && (!matches!(mode, Mode::Exec) || rest.ends_with('\\'))
        }
        _ => false,
    };
    at_eof && !(matches!(mode, Mode::Eval) && eval_has_assignment_before(source.as_bytes(), start))
}

fn has_unescaped_newline(text: &str) -> bool {
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                chars.next();
            }
            '\n' | '\r' => return true,
            _ => {}
        }
    }
    false
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
    } else if range.is_empty() {
        // The parser blames a token, and the narrowest token still covers a
        // character, so an error reported between two of them spans one.
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

fn is_ascii_identifier_char(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphanumeric()
}

fn identifier_continue_before(bytes: &[u8], index: usize) -> bool {
    if index == 0 {
        return false;
    }
    if bytes[index - 1].is_ascii() {
        return is_ascii_identifier_char(bytes[index - 1]);
    }
    let mut start = index - 1;
    while start > 0 && bytes[start] & 0b1100_0000 == 0b1000_0000 {
        start -= 1;
    }
    ::core::str::from_utf8(&bytes[start..index])
        .ok()
        .and_then(|text| text.chars().next_back())
        .is_some_and(|ch| ch == '_' || ch.is_alphanumeric())
}

fn quoted_string_is_closed(bytes: &[u8], start: usize) -> bool {
    let quote = bytes[start];
    let triple = bytes.get(start + 1) == Some(&quote) && bytes.get(start + 2) == Some(&quote);
    let end = skip_quoted_string(bytes, start);
    if triple {
        end >= start + 6
            && bytes[end - 3] == quote
            && bytes[end - 2] == quote
            && bytes[end - 1] == quote
    } else {
        end > start + 1 && bytes[end - 1] == quote
    }
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

fn eval_has_assignment_before(bytes: &[u8], end: usize) -> bool {
    let mut index = 0;
    let mut level = 0usize;
    let mut in_lambda_params = false;
    while index < end {
        match bytes[index] {
            b'#' => {
                while index < end && bytes[index] != b'\n' {
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
            b':' if level == 0 => {
                in_lambda_params = false;
                index += 1;
            }
            b'=' if level == 0
                && !in_lambda_params
                && bytes.get(index + 1) != Some(&b'=')
                && !matches!(
                    bytes.get(index.saturating_sub(1)),
                    Some(b':' | b'!' | b'<' | b'>')
                ) =>
            {
                return true;
            }
            _ if level == 0 && starts_identifier(bytes, index, b"lambda") => {
                in_lambda_params = true;
                index += b"lambda".len();
            }
            _ => index += 1,
        }
    }
    false
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

    if prefix_start > 0 && identifier_continue_before(bytes, prefix_start) {
        return None;
    }

    Some(if marker == b'f' {
        "f-string"
    } else {
        "t-string"
    })
}

const MAXFSTRINGLEVEL: usize = 150;

fn skip_string_token(bytes: &[u8], quote_index: usize) -> usize {
    if interpolated_string_prefix(bytes, quote_index).is_some() {
        interpolated_string_end(bytes, quote_index).unwrap_or(bytes.len())
    } else {
        skip_quoted_string(bytes, quote_index)
    }
}

fn interpolated_string_end(bytes: &[u8], quote_index: usize) -> Option<usize> {
    let quote = bytes[quote_index];
    let triple =
        bytes.get(quote_index + 1) == Some(&quote) && bytes.get(quote_index + 2) == Some(&quote);
    let quote_len = if triple { 3 } else { 1 };
    let mut index = quote_index + quote_len;
    let mut brace_depth = 0usize;
    while index < bytes.len() {
        if brace_depth == 0 {
            if bytes[index] == b'\\' {
                index = (index + 2).min(bytes.len());
                continue;
            }
            if triple
                && bytes.get(index) == Some(&quote)
                && bytes.get(index + 1) == Some(&quote)
                && bytes.get(index + 2) == Some(&quote)
            {
                return Some(index + 3);
            }
            if !triple && bytes[index] == quote {
                return Some(index + 1);
            }
            if bytes[index] == b'{' {
                if bytes.get(index + 1) == Some(&b'{') {
                    index += 2;
                } else {
                    brace_depth = 1;
                    index += 1;
                }
            } else if bytes[index] == b'}' && bytes.get(index + 1) == Some(&b'}') {
                index += 2;
            } else {
                index += 1;
            }
        } else {
            match bytes[index] {
                quote_ch @ (b'\'' | b'"') => {
                    if quoted_string_is_closed(bytes, index) {
                        index = skip_string_token(bytes, index).max(index + 1);
                    } else if !triple && quote_ch == quote {
                        return Some(index + 1);
                    } else {
                        index = skip_string_token(bytes, index).max(index + 1);
                    }
                }
                b'#' => {
                    while index < bytes.len() && bytes[index] != b'\n' {
                        index += 1;
                    }
                }
                b'{' => {
                    brace_depth += 1;
                    index += 1;
                }
                b'}' => {
                    brace_depth -= 1;
                    index += 1;
                }
                b'\\' => index = (index + 2).min(bytes.len()),
                _ => index += 1,
            }
        }
    }
    None
}

fn quote_len_at(bytes: &[u8], quote_index: usize) -> usize {
    let quote = bytes[quote_index];
    if bytes.get(quote_index + 1) == Some(&quote) && bytes.get(quote_index + 2) == Some(&quote) {
        3
    } else {
        1
    }
}

fn interpolated_string_content_range(bytes: &[u8], quote_index: usize) -> Option<(usize, usize)> {
    let end = interpolated_string_end(bytes, quote_index)?;
    let quote_len = quote_len_at(bytes, quote_index);
    Some((quote_index + quote_len, end - quote_len))
}

/// Return the syntax error for a decimal integer literal exceeding the configured limit.
///
/// The parser has already distinguished integer literals from strings, comments, floats, and
/// complex numbers. Inspecting its tokens keeps the limit consistent for every source parsing
/// entry point without reimplementing Python's lexer here.
#[must_use]
pub fn long_decimal_integer_literal_error(
    source_file: &SourceFile,
    tokens: &Tokens,
    max_str_digits: usize,
) -> Option<CompileError> {
    if max_str_digits == 0 {
        return None;
    }
    tokens.iter().find_map(|token| {
        if token.kind() != TokenKind::Int {
            return None;
        }
        let literal = source_file.source_text().slice(token.range());
        if literal
            .as_bytes()
            .get(..2)
            .is_some_and(|prefix| matches!(prefix, b"0x" | b"0X" | b"0o" | b"0O" | b"0b" | b"0B"))
        {
            return None;
        }
        let digits = literal.bytes().filter(u8::is_ascii_digit).count();
        (digits > max_str_digits).then(|| {
            let start = token.range().start().to_usize();
            CompileError::from_source_error(source_file, CpythonDiagnostic::new(format!(
                    "Exceeds the limit ({max_str_digits} digits) for integer string conversion: value has {digits} digits; use sys.set_int_max_str_digits() to increase the limit - Consider hexadecimal for huge integer literals to avoid decimal conversion limits."
                ), start, start))
        })
    })
}

fn too_many_nested_interpolated_strings(source: &str) -> Option<CpythonDiagnostic> {
    too_many_nested_interpolated_strings_in(source.as_bytes(), 0, source.len(), 0)
}

fn too_many_nested_interpolated_strings_in(
    bytes: &[u8],
    mut index: usize,
    end: usize,
    depth: usize,
) -> Option<CpythonDiagnostic> {
    while index < end {
        match bytes[index] {
            b'#' => {
                while index < end && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'\'' | b'"' => {
                if interpolated_string_prefix(bytes, index).is_some() {
                    if depth + 1 >= MAXFSTRINGLEVEL {
                        return Some(CpythonDiagnostic::new(
                            "too many nested f-strings or t-strings".to_owned(),
                            index,
                            index + 1,
                        ));
                    }
                    if let Some((content_start, content_end)) =
                        interpolated_string_content_range(bytes, index)
                        && let Some(error) = too_many_nested_interpolated_strings_in(
                            bytes,
                            content_start,
                            content_end,
                            depth + 1,
                        )
                    {
                        return Some(error);
                    }
                }
                index = skip_string_token(bytes, index).max(index + 1);
            }
            _ => index += 1,
        }
    }
    None
}

/// Horizontal and vertical space the tokenizer skips, not `str::trim()`.
const fn is_ascii_tokenizer_whitespace(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0c')
}

/// True when every line is empty or a comment after tokenizer whitespace.
#[doc(hidden)]
#[must_use]
pub fn is_blank_python_source(source: &str) -> bool {
    source.lines().all(|line| {
        let trimmed = line.trim_matches(is_ascii_tokenizer_whitespace);
        trimmed.is_empty() || trimmed.starts_with('#')
    })
}

fn single_mode_blank_source_error(source_file: &SourceFile) -> Option<CompileError> {
    let source = source_file.source_text();
    if !is_blank_python_source(source) {
        return None;
    }
    let has_indent_only_line = source.lines().any(|line| {
        line.trim_matches(is_ascii_tokenizer_whitespace).is_empty()
            && line.chars().any(|c| matches!(c, ' ' | '\t'))
    });
    if has_indent_only_line {
        let (location, end_location) =
            source_locations(source_file, TextSize::new(0), TextSize::new(0));
        return Some(CompileError::Parse(ParseError {
            error: parser::ParseErrorType::UnexpectedIndentation,
            raw_location: ruff_text_size::TextRange::new(TextSize::new(0), TextSize::new(0)),
            location,
            end_location,
            source_path: source_file.name().to_owned(),
            is_unclosed_bracket: false,
            is_unclosed_string: false,
        }));
    }
    Some(CompileError::from_source_error(
        source_file,
        CpythonDiagnostic::new("invalid syntax".to_owned(), 0, 0),
    ))
}

/// The byte-order mark is only stripped while decoding source bytes, so one
/// that survives into the text is just a non-printable character. The tokenizer
/// rejects it everywhere except at the very start of the text, which is where
/// this covers.
#[doc(hidden)]
#[must_use]
pub fn leading_byte_order_mark_error(source_file: &SourceFile) -> Option<CompileError> {
    source_file.source_text().starts_with('\u{feff}').then(|| {
        CompileError::from_source_error(
            source_file,
            CpythonDiagnostic::new("invalid non-printable character U+FEFF".to_owned(), 0, 0),
        )
    })
}

fn too_many_nested_parentheses_error(source: &str) -> Option<CpythonDiagnostic> {
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
                    return Some(CpythonDiagnostic::new(
                        "too many nested parentheses".to_owned(),
                        index,
                        index,
                    ));
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

/// Source-level errors raised before parsing, as CPython's tokenizer does.
///
/// Checking after the parse would build the tree first, and one nested that
/// deep exhausts the native stack when it is dropped.
pub fn pre_parse_source_error(source_file: &SourceFile) -> Result<(), CompileError> {
    match too_many_nested_parentheses_error(source_file.source_text()) {
        Some(error) => Err(CompileError::from_source_error(source_file, error)),
        None => Ok(()),
    }
}

fn post_parse_source_error(
    source_file: &SourceFile,
    tokens: &Tokens,
    opts: &CompileOpts,
) -> Option<CompileError> {
    if let Some(error) = leading_byte_order_mark_error(source_file) {
        return Some(error);
    }
    if let Some(error) = too_many_nested_interpolated_strings(source_file.source_text()) {
        return Some(CompileError::from_source_error(source_file, error));
    }
    long_decimal_integer_literal_error(source_file, tokens, opts.int_max_str_digits)
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

/// Rejects an AST nested deeper than `limit`.
///
/// The parser grows its own stack, so it returns trees deeper than the passes
/// after it can walk, and each of those recurses without a limit of its own.
/// The symbol table applies the same limit, but `compile(..., PyCF_ONLY_AST)`
/// returns before reaching it.
pub fn too_deeply_nested_error(
    ast: &ast::Mod,
    source_file: &SourceFile,
    limit: usize,
) -> Result<(), CompileError> {
    use ast::visitor::Visitor;

    struct DepthChecker {
        depth: usize,
        limit: usize,
        too_deep: Option<ruff_text_size::TextRange>,
    }

    impl DepthChecker {
        fn descend(&mut self, range: ruff_text_size::TextRange, walk: impl FnOnce(&mut Self)) {
            if self.too_deep.is_some() {
                return;
            }
            if self.depth >= self.limit {
                self.too_deep = Some(range);
                return;
            }
            self.depth += 1;
            walk(self);
            self.depth -= 1;
        }
    }

    impl<'a> Visitor<'a> for DepthChecker {
        fn visit_stmt(&mut self, stmt: &'a ast::Stmt) {
            self.descend(stmt.range(), |checker| {
                ast::visitor::walk_stmt(checker, stmt);
            });
        }

        fn visit_expr(&mut self, expr: &'a ast::Expr) {
            self.descend(expr.range(), |checker| {
                ast::visitor::walk_expr(checker, expr);
            });
        }

        fn visit_pattern(&mut self, pattern: &'a ast::Pattern) {
            self.descend(pattern.range(), |checker| {
                ast::visitor::walk_pattern(checker, pattern);
            });
        }
    }

    let mut checker = DepthChecker {
        depth: 0,
        limit,
        too_deep: None,
    };
    match ast {
        ast::Mod::Module(module) => checker.visit_body(&module.body),
        ast::Mod::Expression(expression) => checker.visit_expr(&expression.body),
    }

    let Some(range) = checker.too_deep else {
        return Ok(());
    };
    let (location, end_location) = source_locations(source_file, range.start(), range.end());
    Err(CompileError::Codegen(codegen::error::CodegenError {
        location: Some(location),
        end_location: Some(end_location),
        error: codegen::error::CodegenErrorType::RecursionError,
        source_path: source_file.name().to_owned(),
    }))
}

/// Source-only rejection performed by CPython's checked_from_import parser action.
#[doc(hidden)]
#[must_use]
pub fn lazy_future_import_error(ast: &ast::Mod, source_file: &SourceFile) -> Option<CompileError> {
    use ast::statement_visitor::StatementVisitor;

    struct Checker<'a> {
        pending: Vec<&'a ast::Stmt>,
    }

    impl<'a> StatementVisitor<'a> for Checker<'a> {
        fn visit_stmt(&mut self, stmt: &'a ast::Stmt) {
            self.pending.push(stmt);
        }
    }

    let ast::Mod::Module(module) = ast else {
        return None;
    };
    let mut checker = Checker {
        pending: module.body.iter().collect(),
    };
    let mut first = None;
    while let Some(stmt) = checker.pending.pop() {
        if let ast::Stmt::ImportFrom(import) = stmt
            && import.is_lazy
            && import.level == 0
            && import
                .module
                .as_ref()
                .is_some_and(|name| name.as_str() == "__future__")
        {
            let start = import.range.start().to_usize();
            first = Some(first.map_or(start, |previous: usize| previous.min(start)));
        }
        ast::statement_visitor::walk_stmt(&mut checker, stmt);
    }
    first.map(|start| {
        CompileError::from_source_error(
            source_file,
            CpythonDiagnostic::new(
                "lazy from __future__ import is not allowed".to_owned(),
                start,
                start + 4,
            ),
        )
    })
}

/// Syntax the reference grammar has no rule for, but that this parser accepts.
///
/// A bare generator expression is one: `f(x for x in y)` is the `primary
/// genexp` alternative of a call, and a class header only takes `arguments`,
/// which has no such alternative. A format spec nested more than two deep is
/// the other; the tokenizer runs out of nesting levels for it.
#[doc(hidden)]
#[must_use]
pub fn unsupported_grammar_error(ast: &ast::Mod, source_file: &SourceFile) -> Option<CompileError> {
    use ast::visitor::Visitor;

    /// The deepest chain of format specs one string literal may hold.
    const MAX_FORMAT_SPEC_DEPTH: usize = 2;

    struct Checker<'a> {
        source_file: &'a SourceFile,
        error: Option<CompileError>,
    }

    impl Checker<'_> {
        fn fail(&mut self, message: &str, range: ruff_text_size::TextRange) {
            self.error = Some(CompileError::from_source_error(
                self.source_file,
                CpythonDiagnostic::new(
                    message.to_owned(),
                    range.start().to_usize(),
                    range.end().to_usize(),
                ),
            ));
        }

        fn check_format_specs(
            &mut self,
            kind: &str,
            elements: &ast::InterpolatedStringElements,
            depth: usize,
        ) {
            for element in elements.interpolations() {
                let Some(format_spec) = &element.format_spec else {
                    continue;
                };
                if depth == MAX_FORMAT_SPEC_DEPTH {
                    self.fail(
                        &alloc::format!("{kind}: expressions nested too deeply"),
                        format_spec.range,
                    );
                    return;
                }
                self.check_format_specs(kind, &format_spec.elements, depth + 1);
                if self.error.is_some() {
                    return;
                }
            }
        }
    }

    impl<'a> Visitor<'a> for Checker<'_> {
        fn visit_stmt(&mut self, stmt: &'a ast::Stmt) {
            if self.error.is_some() {
                return;
            }
            if let ast::Stmt::ClassDef(class_def) = stmt
                && let Some(arguments) = &class_def.arguments
                && let [ast::Expr::Generator(generator)] = &arguments.args[..]
                && !generator.parenthesized
            {
                let range = generator
                    .generators
                    .first()
                    .map_or(generator.range, |comprehension| comprehension.range);
                self.fail("invalid syntax", range);
                return;
            }
            ast::visitor::walk_stmt(self, stmt);
        }

        fn visit_expr(&mut self, expr: &'a ast::Expr) {
            if self.error.is_some() {
                return;
            }
            // Each literal counts its own nesting: one written inside a format
            // spec starts over.
            match expr {
                ast::Expr::FString(fstring) => {
                    for part in &fstring.value {
                        if let ast::FStringPartRef::FString(part) = part {
                            self.check_format_specs("f-string", &part.elements, 0);
                        }
                    }
                }
                ast::Expr::TString(tstring) => {
                    for part in &tstring.value {
                        self.check_format_specs("t-string", &part.elements, 0);
                    }
                }
                _ => {}
            }
            if self.error.is_some() {
                return;
            }
            ast::visitor::walk_expr(self, expr);
        }
    }

    let mut checker = Checker {
        source_file,
        error: None,
    };
    match ast {
        ast::Mod::Module(module) => checker.visit_body(&module.body),
        ast::Mod::Expression(expression) => checker.visit_expr(&expression.body),
    }
    checker.error
}

fn single_mode_body_error(body: &[ast::Stmt], source_file: &SourceFile) -> Option<CompileError> {
    let first = body.first()?;
    let source_code = source_file.to_source_code();
    let first_start = source_code.source_location(first.range().start(), PositionEncoding::Utf8);
    let first_end = source_code.source_location(first.range().end(), PositionEncoding::Utf8);

    if is_compound_stmt(first)
        && first_start.line == first_end.line
        && !ends_with_line_break(source_file.source_text())
    {
        return Some(CompileError::from_source_error(
            source_file,
            CpythonDiagnostic::new(
                "invalid syntax".to_owned(),
                first.range().start().to_usize(),
                first.range().start().to_usize(),
            ),
        ));
    }
    None
}

/// Converts the first parse error. A tokenizer error that follows an earlier syntax error is
/// only found by the pass over the rest of the source, so it never asks for more input.
fn first_parse_error(
    parsed: parser::Parsed<ast::Mod>,
    source_file: &SourceFile,
    mode: Mode,
) -> Result<parser::Parsed<ast::Mod>, CompileError> {
    let Some((first, rest)) = parsed.errors().split_first() else {
        return Ok(parsed);
    };
    let after_syntax_error = rest.iter().any(|error| {
        !matches!(error.error, ParseErrorType::Lexical(_))
            && error.location.start() < first.location.start()
    });
    let mut error = CompileError::from_ruff_parse_error(first.clone(), source_file, mode);
    if after_syntax_error && let CompileError::Parse(error) = &mut error {
        error.is_unclosed_string = false;
    }
    Err(error)
}

/// Single mode reads only the first statement when it is a simple statement, so any code after
/// its line is reported as multiple statements, ahead of errors the rest of the source has.
#[must_use]
pub fn single_mode_multiple_statements_error(
    source_file: &SourceFile,
    parsed: &parser::Parsed<ast::Mod>,
) -> Option<CompileError> {
    let ast::Mod::Module(module) = parsed.syntax() else {
        return None;
    };
    if is_compound_stmt(module.body.first()?) {
        return None;
    }
    let tokens = parsed.tokens();
    let newline_index = tokens
        .iter()
        .position(|token| token.kind() == TokenKind::Newline)?;
    let newline = &tokens[newline_index];
    if parsed
        .errors()
        .iter()
        .any(|error| error.location.start() < newline.end())
    {
        return None;
    }
    let mut rest = &source_file.source_text()[newline.end().to_usize()..];
    loop {
        rest = rest.trim_start_matches([' ', '\t', '\n', '\r', '\x0c']);
        match rest.strip_prefix('#') {
            Some(comment) => rest = comment.find('\n').map_or("", |end| &comment[end..]),
            None if rest.is_empty() => return None,
            None => break,
        }
    }
    // The error is at the newline token, which starts at a trailing comment.
    let position = match newline_index.checked_sub(1).map(|index| &tokens[index]) {
        Some(comment) if comment.kind() == TokenKind::Comment => comment.start(),
        _ => newline.start(),
    }
    .to_usize();
    Some(CompileError::from_source_error(
        source_file,
        CpythonDiagnostic::new(
            "multiple statements found while compiling a single statement".to_owned(),
            position,
            position,
        ),
    ))
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
        CpythonDiagnostic::new("incomplete input".to_owned(), eof, eof),
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
    let barry_source = prepare_barry_as_flufl_source(
        source_file.source_text(),
        parser_options.clone(),
        opts.future_features
            .contains(core::bytecode::CodeFlags::FUTURE_BARRY_AS_BDFL),
    );
    pre_parse_source_error(&source_file)?;
    let parsed = parser::parse_unchecked(barry_source.source(), parser_options);
    if parsed.errors().is_empty()
        && let Some(error) = lazy_future_import_error(parsed.syntax(), &source_file)
    {
        return Err(error);
    }
    if matches!(mode, Mode::Single)
        && let Some(error) = single_mode_multiple_statements_error(&source_file, &parsed)
    {
        return Err(error);
    }
    if let Some(error) = barry_source.diagnostic(parsed.errors().first(), &source_file) {
        return Err(error);
    }
    let parsed = first_parse_error(parsed, &source_file, mode)?;
    if matches!(mode, Mode::Single)
        && let Some(error) = single_mode_blank_source_error(&source_file)
    {
        return Err(error);
    }
    if opts.dont_imply_dedent
        && matches!(mode, Mode::Single)
        && let Some(error) = dont_imply_dedent_source_error(&source_file)
    {
        return Err(error);
    }
    if let Some(error) = post_parse_source_error(&source_file, parsed.tokens(), &opts) {
        return Err(error);
    }
    let ast = parsed.into_syntax();
    too_deeply_nested_error(&ast, &source_file, opts.recursion_limit)?;
    if let Some(error) = unsupported_grammar_error(&ast, &source_file) {
        return Err(error);
    }
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

#[doc(hidden)]
pub struct BarrySource<'a> {
    source: Cow<'a, str>,
    not_equal: Option<ruff_text_size::TextRange>,
    legacy_not_equal: Vec<ruff_text_size::TextRange>,
}

impl BarrySource<'_> {
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    #[must_use]
    pub fn not_equal_before(
        &self,
        parse_error: Option<&parser::ParseError>,
    ) -> Option<ruff_text_size::TextRange> {
        self.not_equal.filter(|range| {
            parse_error.is_none_or(|error| {
                let diagnostic_start = if matches!(
                    &error.error,
                    parser::ParseErrorType::Lexical(parser::LexicalErrorType::Eof)
                ) {
                    find_unclosed_bracket(&self.source).map_or_else(
                        || error.location.start(),
                        |(_, offset)| TextSize::new(offset as u32),
                    )
                } else {
                    error.location.start()
                };
                range.start() <= diagnostic_start
            })
        })
    }

    /// The obsolete `<>` operator the parse error points at, if any. In Barry
    /// mode the operator was rewritten to `!=`, so the error lands on its
    /// start; outside Barry mode ruff lexes `<` and then an unexpected `>`, so
    /// the error lands one character in. Either way the whole operator is one
    /// token to the tokenizer, so report it as one.
    #[must_use]
    pub fn invalid_legacy_operator(
        &self,
        parse_error: &parser::ParseError,
    ) -> Option<ruff_text_size::TextRange> {
        let location = parse_error.location.start();
        self.legacy_not_equal
            .iter()
            .copied()
            .find(|range| range.contains(location) || range.start() == location)
            .filter(|range| self.outranks_unclosed_bracket(*range))
    }

    /// Whether `range` outranks an unclosed bracket. The bracket is reported
    /// at itself, so it wins over anything that starts after it.
    fn outranks_unclosed_bracket(&self, range: ruff_text_size::TextRange) -> bool {
        find_unclosed_bracket(&self.source)
            .is_none_or(|(_, offset)| range.start() <= TextSize::new(offset as u32))
    }

    /// The diagnostic for this source, if any: an obsolete `<>` the parse
    /// error points at, or -- in Barry mode only -- the first `!=`. A `<>`
    /// takes precedence over a `!=` reported later in the source.
    #[must_use]
    pub fn diagnostic(
        &self,
        parse_error: Option<&parser::ParseError>,
        source_file: &SourceFile,
    ) -> Option<CompileError> {
        if let Some(error) = parse_error
            && let Some(range) = self.invalid_legacy_operator(error)
        {
            if error.location.start() > range.start() {
                return Some(CompileError::from_source_error(
                    source_file,
                    CpythonDiagnostic::new(
                        "invalid syntax.  Maybe you meant '!=' instead of '<>'?".to_owned(),
                        range.start().to_usize(),
                        range.end().to_usize(),
                    ),
                ));
            }
            // At expression start, '<' is already invalid: the comparison
            // production (and its obsolete-operator hint) was never entered.
            // Barry mode instead lexed the rewritten '!=' as one token.
            let range = if self.source.as_bytes()[range.start().to_usize()] == b'<' {
                ruff_text_size::TextRange::at(range.start(), TextSize::new(1))
            } else {
                range
            };
            return Some(barry_as_flufl_invalid_legacy_operator_error(
                source_file,
                range,
            ));
        }
        self.not_equal_before(parse_error)
            .map(|range| barry_as_flufl_not_equal_error(source_file, range))
    }
}

#[doc(hidden)]
#[must_use]
pub fn barry_as_flufl_not_equal_error(
    source_file: &SourceFile,
    range: ruff_text_size::TextRange,
) -> CompileError {
    CompileError::from_source_error(
        source_file,
        CpythonDiagnostic::new(
            "with Barry as BDFL, use '<>' instead of '!='".to_owned(),
            range.start().to_usize(),
            range.end().to_usize(),
        ),
    )
}

#[doc(hidden)]
#[must_use]
pub fn barry_as_flufl_invalid_legacy_operator_error(
    source_file: &SourceFile,
    range: ruff_text_size::TextRange,
) -> CompileError {
    CompileError::from_source_error(
        source_file,
        CpythonDiagnostic::new(
            "invalid syntax".to_owned(),
            range.start().to_usize(),
            range.end().to_usize(),
        ),
    )
}

/// Every `<>` in `source`, located by plain text search. Only used where the
/// operator is not rewritten, so an occurrence inside a string or a comment
/// costs nothing: it can never coincide with the location of a parse error.
fn textual_legacy_not_equal(source: &str) -> Vec<ruff_text_size::TextRange> {
    source
        .match_indices("<>")
        .map(|(offset, matched)| {
            ruff_text_size::TextRange::at(
                TextSize::new(offset as u32),
                TextSize::new(matched.len() as u32),
            )
        })
        .collect()
}

#[doc(hidden)]
pub fn prepare_barry_as_flufl_source(
    source: &str,
    parser_options: parser::ParseOptions,
    inherited: bool,
) -> BarrySource<'_> {
    let scanned = (inherited || source.contains("barry_as_FLUFL"))
        .then(|| parser::parse_unchecked(source, parser_options));
    let enabled = scanned.as_ref().is_some_and(|scanned| {
        inherited
            || codegen::preprocess::future_features(scanned.syntax())
                .contains(core::bytecode::CodeFlags::FUTURE_BARRY_AS_BDFL)
    });
    let Some(scanned) = scanned.filter(|_| enabled) else {
        return BarrySource {
            source: Cow::Borrowed(source),
            not_equal: None,
            legacy_not_equal: textual_legacy_not_equal(source),
        };
    };

    let not_equal = scanned
        .tokens()
        .iter()
        .find(|token| token.kind() == TokenKind::NotEqual)
        .map(Ranged::range);
    let replacements = scanned
        .tokens()
        .windows(2)
        .filter_map(|tokens| {
            let [less, greater] = tokens else {
                return None;
            };
            (less.kind() == TokenKind::Less
                && greater.kind() == TokenKind::Greater
                && less.end() == greater.start())
            .then(|| ruff_text_size::TextRange::new(less.start(), greater.end()))
        })
        .collect::<Vec<_>>();

    let source = if replacements.is_empty() {
        Cow::Borrowed(source)
    } else {
        let mut rewritten = source.to_owned();
        for range in replacements.iter().rev() {
            rewritten.replace_range(range.start().to_usize()..range.end().to_usize(), "!=");
        }
        Cow::Owned(rewritten)
    };
    BarrySource {
        source,
        not_equal,
        legacy_not_equal: replacements,
    }
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

/// Bring a module into the shape the symbol table is built from.
fn symtable_preprocess_module(
    module: &mut ast::ModModule,
    source_file: &SourceFile,
) -> Result<(), CompileError> {
    let future_features = codegen::preprocess::checked_future_features_in_body(&module.body)
        .map_err(|error| future_feature_error(error, source_file))?;
    let future_annotations =
        future_features.contains(core::bytecode::CodeFlags::FUTURE_ANNOTATIONS);
    // Constant folding is left out; the symbol table is built from the parsed
    // program as written.
    codegen::preprocess::preprocess_statements(&mut module.body, 0, future_annotations, true);
    Ok(())
}

fn future_feature_error(
    error: codegen::preprocess::FutureFeatureError,
    source_file: &SourceFile,
) -> CompileError {
    let source_code = source_file.to_source_code();
    let location = source_code.source_location(error.range.start(), PositionEncoding::Utf8);
    let end_location = source_code.source_location(error.range.end(), PositionEncoding::Utf8);
    let error = match error.kind {
        codegen::preprocess::FutureFeatureErrorKind::InvalidFeature(feature) => {
            codegen::error::CodegenErrorType::InvalidFutureFeature(feature)
        }
        codegen::preprocess::FutureFeatureErrorKind::InvalidBraces => {
            codegen::error::CodegenErrorType::InvalidFutureBraces
        }
    };
    codegen::error::CodegenError {
        location: Some(location),
        end_location: Some(end_location),
        error,
        source_path: source_file.name().to_owned(),
    }
    .into()
}

pub fn _compile_symtable(
    source_file: SourceFile,
    mode: Mode,
) -> Result<symboltable::SymbolTable, CompileError> {
    let parser_mode = match mode {
        Mode::Exec | Mode::Single | Mode::BlockExpr => parser::Mode::Module,
        Mode::Eval => parser::Mode::Expression,
    };
    let parser_options = parser::ParseOptions::from(parser_mode);
    let barry_source =
        prepare_barry_as_flufl_source(source_file.source_text(), parser_options.clone(), false);
    let res = match mode {
        Mode::Exec | Mode::Single | Mode::BlockExpr => {
            pre_parse_source_error(&source_file)?;
            let parsed = ruff_python_parser::parse_unchecked(barry_source.source(), parser_options);
            if parsed.errors().is_empty()
                && let Some(error) = lazy_future_import_error(parsed.syntax(), &source_file)
            {
                return Err(error);
            }
            if matches!(mode, Mode::Single)
                && let Some(error) = single_mode_multiple_statements_error(&source_file, &parsed)
            {
                return Err(error);
            }
            if let Some(error) = barry_source.diagnostic(parsed.errors().first(), &source_file) {
                return Err(error);
            }
            let ast = first_parse_error(parsed, &source_file, mode)?;
            if let Some(error) =
                post_parse_source_error(&source_file, ast.tokens(), &CompileOpts::default())
            {
                return Err(error);
            }
            let ast = ast.into_syntax();
            too_deeply_nested_error(&ast, &source_file, CompileOpts::default().recursion_limit)?;
            if let Some(error) = unsupported_grammar_error(&ast, &source_file) {
                return Err(error);
            }
            let ast = ast.expect_module();
            if matches!(mode, Mode::Single)
                && let Some(error) = single_mode_body_error(&ast.body, &source_file)
            {
                return Err(error);
            }
            let mut ast = ast;
            symtable_preprocess_module(&mut ast, &source_file)?;
            symboltable::SymbolTable::scan_program(&ast, source_file.clone())
        }
        Mode::Eval => {
            pre_parse_source_error(&source_file)?;
            let parsed = ruff_python_parser::parse_unchecked(barry_source.source(), parser_options);
            if let Some(error) = barry_source.diagnostic(parsed.errors().first(), &source_file) {
                return Err(error);
            }
            let ast = first_parse_error(parsed, &source_file, mode)?;
            if let Some(error) =
                post_parse_source_error(&source_file, ast.tokens(), &CompileOpts::default())
            {
                return Err(error);
            }
            let ast = ast.into_syntax();
            too_deeply_nested_error(&ast, &source_file, CompileOpts::default().recursion_limit)?;
            if let Some(error) = unsupported_grammar_error(&ast, &source_file) {
                return Err(error);
            }
            let mut ast = ast;
            codegen::preprocess::preprocess_mod(&mut ast, 0, false, true);
            symboltable::SymbolTable::scan_expr(&ast.expect_expression(), source_file.clone())
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
    fn empty_parameter_default_is_reported_before_later_params() {
        let err = compile(
            "def foo(a=1,d=,c):\n    pass\n",
            Mode::Exec,
            "<params>",
            CompileOpts::default(),
        )
        .expect_err("empty default should fail");
        assert!(
            err.to_string()
                .contains("expected default value expression"),
            "got {err}"
        );
    }

    #[test]
    fn too_many_nested_fstrings_match_tokenizer_limit() {
        fn nested(n: usize) -> String {
            if n == 0 {
                return "1+1".to_owned();
            }
            format!("f\"{{{}}}\"", nested(n - 1))
        }

        compile(&nested(149), Mode::Eval, "<nest>", CompileOpts::default())
            .expect("149 nested f-strings should compile");
        let err = compile(&nested(150), Mode::Eval, "<nest>", CompileOpts::default())
            .expect_err("150 nested f-strings should fail");
        assert!(
            err.to_string()
                .contains("too many nested f-strings or t-strings"),
            "got {err}"
        );
    }

    #[test]
    fn interpolated_string_diagnostics_match_cpython() {
        for (source, expected) in [
            ("f'{'", "f-string: expecting '}'"),
            ("t'{'", "t-string: expecting '}'"),
            (
                "f'{1=}{;'",
                "f-string: expecting a valid expression after '{'",
            ),
            (
                "t'{x;y}'",
                "t-string: expecting '=', or '!', or ':', or '}'",
            ),
            ("t'{x!s:'", "t-string: expecting '}', or format specs"),
            ("t'{x=!}'", "t-string: missing conversion character"),
            (
                "t'{x:{;}}'",
                "t-string: expecting a valid expression after '{'",
            ),
            ("f'{1#}'", "'{' was never closed"),
            ("t'{", "'{' was never closed"),
            // A field is only "never closed" while the tokenizer is reading its expression.
            // Past the field's own `:` it emits literal text again, so the quote is what is
            // missing — and a `:` in a slice, a display or a lambda is not the field's own.
            ("f'{a", "'{' was never closed"),
            ("f'{a!r", "'{' was never closed"),
            ("f'{a=", "'{' was never closed"),
            ("f'{ {1:2}", "'{' was never closed"),
            ("f'{d[1:2]", "'{' was never closed"),
            ("f'{(lambda x: x)", "'{' was never closed"),
            ("f'{a:{b:{c", "'{' was never closed"),
            ("f'{a:", "unterminated f-string literal"),
            ("f'{a:>5", "unterminated f-string literal"),
            ("f'{a!r:", "unterminated f-string literal"),
            ("f'{a}{b:", "unterminated f-string literal"),
            ("f'{a:{b}c", "unterminated f-string literal"),
            ("t'{a:>5", "unterminated t-string literal"),
            ("f'''{a:>5", "unterminated triple-quoted f-string literal"),
            // CPython names the innermost unclosed delimiter, not the field around it.
            ("f'{a[", "'[' was never closed"),
            ("f'{(a", "'(' was never closed"),
            ("f'{)#}'", "f-string: unmatched ')'"),
            (
                "f'{a[4)}'",
                "closing parenthesis ')' does not match opening parenthesis '['",
            ),
            ("t'", "unterminated t-string literal (detected at line 1)"),
            (
                "t'''",
                "unterminated triple-quoted t-string literal (detected at line 1)",
            ),
            (
                "t\"x\" b\"y\"",
                "cannot mix t-string literals with string or bytes literals",
            ),
            (
                "b\"x\" t\"y\"",
                "cannot mix t-string literals with string or bytes literals",
            ),
            // The literals ahead of the first t-string already mix, so CPython's first pass
            // raises from `_PyPegen_concatenate_strings` before the t-string rule is reached.
            (
                "\"a\" b\"b\" t\"c\"",
                "cannot mix bytes and nonbytes literals",
            ),
            // A dangling operator ends the expression before a separator could follow.
            (
                "f'{a==}'",
                "f-string: expecting '=', or '!', or ':', or '}'",
            ),
            (
                "f'{a and}'",
                "f-string: expecting '=', or '!', or ':', or '}'",
            ),
            (
                "f'{a is not}'",
                "f-string: expecting '=', or '!', or ':', or '}'",
            ),
            (
                "f'{a.b.}'",
                "f-string: expecting '=', or '!', or ':', or '}'",
            ),
            ("t'{a~}'", "t-string: expecting '=', or '!', or ':', or '}'"),
            // A doubled `=` or `!` opens no debug specifier and no conversion, so the field has
            // no expression at all rather than an empty one before a marker.
            (
                "f'{==a}'",
                "f-string: expecting a valid expression after '{'",
            ),
            (
                "f'{!=a}'",
                "f-string: expecting a valid expression after '{'",
            ),
            (
                "t'{==a}'",
                "t-string: expecting a valid expression after '{'",
            ),
            ("f'{=a}'", "f-string: valid expression required before '='"),
            ("f'{!}'", "f-string: valid expression required before '!'"),
            (
                "f'{lambda x:x}'",
                "f-string: lambda expressions are not allowed without parentheses",
            ),
            (
                "f'{1, lambda:x}'",
                "f-string: lambda expressions are not allowed without parentheses",
            ),
            (
                "f'{+ lambda:None}'",
                "f-string: expecting a valid expression after '{'",
            ),
            ("fu''", "'u' and 'f' prefixes are incompatible"),
            ("fb''", "'b' and 'f' prefixes are incompatible"),
            ("ufr''", "'u' and 'r' prefixes are incompatible"),
            (
                "(]\nbu'x'",
                "closing parenthesis ']' does not match opening parenthesis '('",
            ),
            ("0x\nbu'x'", "invalid hexadecimal literal"),
            ("(0x", "invalid hexadecimal literal"),
            ("print x; 0x", "invalid hexadecimal literal"),
            ("exec x; 0x", "invalid hexadecimal literal"),
            (
                "print x; 0x1",
                "Missing parentheses in call to 'print'. Did you mean print(...)?",
            ),
            (
                "print x; (",
                "Missing parentheses in call to 'print'. Did you mean print(...)?",
            ),
            ("print x; )", "unmatched ')'"),
            (
                "( '\\N'",
                "(unicode error) 'unicodeescape' codec can't decode bytes in position 0-1: malformed \\N character escape",
            ),
            ("(print x", "'(' was never closed"),
            (
                "(print x; '\\N'",
                "Missing parentheses in call to 'print'. Did you mean print(...)?",
            ),
            ("f'{x'; 0x", "f-string: expecting '}'"),
            (
                "print x; f'{x'; 0x",
                "Missing parentheses in call to 'print'. Did you mean print(...)?",
            ),
            (
                concat!("f'{1:", "d\n}'"),
                "f-string: newlines are not allowed in format specifiers",
            ),
            ("f'{\n}'", "f-string: valid expression required before '}'"),
            (
                "f'''\n{\n# only a comment\n}'''",
                "f-string: valid expression required before '}'",
            ),
            ("{\\'a\\'}", "unexpected character after line continuation"),
            (
                "\"\\\n\"(1 for c in I,\\\n\\",
                "unexpected character after line continuation character",
            ),
            (
                r"'\N'",
                "(unicode error) 'unicodeescape' codec can't decode bytes in position 0-1: malformed \\N character escape",
            ),
            (
                r"f'\N{'",
                "(unicode error) 'unicodeescape' codec can't decode bytes in position 0-2: malformed \\N character escape",
            ),
        ] {
            let err = compile(source, Mode::Eval, "<interp>", CompileOpts::default())
                .expect_err("should not compile");
            assert!(
                err.to_string().contains(expected),
                "{source:?}: expected {expected:?}, got {err}"
            );
        }
    }

    #[test]
    fn missing_indent_outranks_print_missing_parentheses() {
        for (source, expected) in [
            (
                "if True:\nprint \"No indent\"",
                "expected an indented block after 'if' statement on line 1",
            ),
            (
                "print \"old style\"",
                "Missing parentheses in call to 'print'. Did you mean print(...)?",
            ),
        ] {
            let err = compile(source, Mode::Exec, "<fragment>", CompileOpts::default())
                .expect_err("should not compile");
            assert!(
                err.to_string().contains(expected),
                "{source:?}: expected {expected:?}, got {err}"
            );
        }
    }

    #[test]
    fn unclosed_fstring_field_keeps_the_unclosed_bracket_flag() {
        let err = compile("f'{", Mode::Eval, "<interp>", CompileOpts::default())
            .expect_err("should not compile");
        let crate::CompileError::Parse(parse) = err else {
            panic!("expected a parse error, got {err}");
        };
        assert!(
            parse.is_unclosed_bracket,
            "unclosed f-string field must stay incomplete, got {parse}"
        );
        assert!(
            parse.to_string().contains("'{' was never closed"),
            "got {parse}"
        );
    }

    #[test]
    fn interpolated_literals_do_not_take_the_escaped_quote_hint() {
        // Parser/lexer/lexer.c offers "perhaps you escaped the end quote?" from its plain-string
        // branch only; the interpolated branch has just the triple-quoted and plain forms. The
        // assertion has to be exact: the hint is a suffix, so `contains` would pass either way.
        for (source, expected) in [
            (
                r"f'\'",
                "unterminated f-string literal (detected at line 1)",
            ),
            (
                r"t'\'",
                "unterminated t-string literal (detected at line 1)",
            ),
        ] {
            let err = compile(source, Mode::Eval, "<escaped>", CompileOpts::default())
                .expect_err("should not compile");
            assert_eq!(err.to_string(), expected, "{source:?}");
        }
        // A literal without a prefix still gets it.
        let err = compile(r"'\'", Mode::Eval, "<escaped>", CompileOpts::default())
            .expect_err("should not compile");
        assert_eq!(
            err.to_string(),
            "unterminated string literal (detected at line 1); perhaps you escaped the end quote?"
        );
    }

    #[test]
    fn a_field_bracket_mismatch_names_the_opening_line() {
        // CPython compares `parenlinenostack[level]` against the current `lineno`, so it names
        // the opening line only when the two brackets are not on the same one.
        for (source, expected) in [
            (
                "x = f\"\"\"{a[\n4)}\"\"\"\n",
                "closing parenthesis ')' does not match opening parenthesis '[' on line 1",
            ),
            (
                "x = f\"\"\"{a(\n\n4]}\"\"\"\n",
                "closing parenthesis ']' does not match opening parenthesis '(' on line 1",
            ),
            (
                "x = f\"{a[4)}\"\n",
                "closing parenthesis ')' does not match opening parenthesis '['",
            ),
        ] {
            let err = compile(source, Mode::Exec, "<paren>", CompileOpts::default())
                .expect_err("should not compile");
            assert_eq!(err.to_string(), expected, "{source:?}");
        }
    }

    #[test]
    fn a_dangling_operator_needs_the_field_to_close() {
        // A stray character is a finished token, so CPython's grammar rejects it on lookahead
        // and names the separators even when the field never closes. A dangling operator makes
        // the parser ask for one more token, and producing it hits the closing quote, where
        // lexer.c answers "expecting '}'" from the tokenizer instead.
        for (source, expected) in [
            ("f'{a;'", "f-string: expecting '=', or '!', or ':', or '}'"),
            ("f'{a$'", "f-string: expecting '=', or '!', or ':', or '}'"),
            ("f'{a?'", "f-string: expecting '=', or '!', or ':', or '}'"),
            ("f'{a and'", "f-string: expecting '}'"),
            ("f'{a+'", "f-string: expecting '}'"),
            ("f'{a=='", "f-string: expecting '}'"),
            ("f'{a.b.'", "f-string: expecting '}'"),
            ("f'{a is not'", "f-string: expecting '}'"),
            ("t'{a and'", "t-string: expecting '}'"),
            // With the field closed, the operator is what gets named.
            (
                "f'{a and}'",
                "f-string: expecting '=', or '!', or ':', or '}'",
            ),
            (
                "f'{a==}'",
                "f-string: expecting '=', or '!', or ':', or '}'",
            ),
        ] {
            let err = compile(source, Mode::Eval, "<dangling>", CompileOpts::default())
                .expect_err("should not compile");
            assert!(
                err.to_string().contains(expected),
                "{source:?}: expected {expected:?}, got {err}"
            );
        }
    }

    #[test]
    fn deeply_nested_format_specs_stay_linear() {
        // Each enclosing spec used to re-scan every field nested inside it, which made this
        // O(2^depth): a 110-byte source took six seconds. The scan now resumes past a nested
        // field once it has checked it, so this has to finish immediately.
        for depth in [32usize, 200] {
            let source = format!("x = f\"{}{}\"\n$", "{1:".repeat(depth), "}".repeat(depth));
            compile(&source, Mode::Exec, "<nested>", CompileOpts::default())
                .expect_err("the trailing `$` is a syntax error");
        }
    }

    #[test]
    #[expect(
        clippy::literal_string_with_formatting_args,
        reason = "these are Python format specs, not Rust format args"
    )]
    fn valid_interpolated_literals_do_not_shadow_a_later_syntax_error() {
        // A format spec is text rather than code, and a `#` in a replacement field starts a
        // comment. Mistaking either for expression syntax would blame a perfectly good literal
        // for the `$` further down, so the reported line must stay on the `$`.
        for source in [
            "x = f\"{1:(}\"\n$",
            "x = f\"{1:[}\"\n$",
            "x = f\"{1:#x}\"\n$",
            "x = f\"{1!r:#>5}\"\n$",
            "x = t\"{1:(}\"\n$",
            "x = f\"\"\"{1 # (\n}\"\"\"\n$",
            "x = f\"\"\"{1 # {\n}\"\"\"\n$",
            "x = f\"\"\"{1 # ]\n}\"\"\"\n$",
            "x = f\"\"\"{1 # a\\ b\n}\"\"\"\n$",
            // A comment tail is not the end of the expression, whatever it looks like.
            "x = f\"\"\"{a # +\n}\"\"\"\n$",
            "x = f\"\"\"{a # and\n}\"\"\"\n$",
            "x = f\"\"\"{a # ==\n}\"\"\"\n$",
            "x = t\"\"\"{a # .\n}\"\"\"\n$",
            "x = f\"\"\"{1:{a # +\n}}\"\"\"\n$",
            "x = f\"\"\"{a # +\n + b}\"\"\"\n$",
            // A signed leading-dot float is an operand.
            "x = f\"{-.5}\"\n$",
            "x = f\"{+.5}\"\n$",
            // Ellipsis and a leading-dot float start real expressions.
            "x = f\"{...}\"\n$",
            "x = f\"{.5}\"\n$",
            "x = t\"{...}\"\n$",
            // A dangling operator is reported, but a complete one is not.
            "x = f\"{a+b}\"\n$",
            "x = f\"{a.b.c}\"\n$",
            "x = f\"{1.}\"\n$",
            // A comparison operator is not a debug or conversion marker.
            "x = f\"{a==b}\"\n$",
            "x = f\"{a!=b}\"\n$",
            "x = f\"{a<=b}\"\n$",
            "x = f\"{a>=b}\"\n$",
            "x = t\"{a==b}\"\n$",
            "x = f\"{a:{b==c}}\"\n$",
        ] {
            let err = compile(source, Mode::Exec, "<interp>", CompileOpts::default())
                .expect_err("the trailing `$` is a syntax error");
            assert_eq!(
                err.python_location().0,
                source.lines().count(),
                "{source:?} reported the wrong line: {err}"
            );
        }
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
    fn barry_as_flufl_rewrites_legacy_not_equal_after_future_import() {
        let code = compile(
            "from __future__ import barry_as_FLUFL\nresult = 2 <> 3\n",
            Mode::Exec,
            "<barry>",
            CompileOpts::default(),
        )
        .expect("Barry comparison should compile");
        assert!(
            code.flags
                .contains(core::bytecode::CodeFlags::FUTURE_BARRY_AS_BDFL)
        );
    }

    #[test]
    fn inherited_barry_as_flufl_rewrites_legacy_not_equal() {
        let opts = CompileOpts {
            future_features: core::bytecode::CodeFlags::FUTURE_BARRY_AS_BDFL,
            ..CompileOpts::default()
        };
        compile("2 <> 3", Mode::Single, "<barry>", opts)
            .expect("inherited Barry comparison should compile");
    }

    #[test]
    fn barry_as_flufl_rejects_modern_not_equal() {
        let err = compile(
            "from __future__ import barry_as_FLUFL\n2 != 3\n",
            Mode::Exec,
            "<barry>",
            CompileOpts::default(),
        )
        .expect_err("Barry mode should reject !=");
        assert_eq!(
            err.to_string(),
            "with Barry as BDFL, use '<>' instead of '!='"
        );
        assert_eq!(err.python_location(), (2, 3));
    }

    #[test]
    fn fstring_adjacent_atoms_are_a_missing_comma() {
        let err = compile("f'{6 0}'", Mode::Exec, "<fragment>", CompileOpts::default())
            .expect_err("adjacent atoms in an f-string field are a syntax error");
        assert_eq!(
            err.to_string(),
            "invalid syntax. Perhaps you forgot a comma?"
        );
        assert_eq!(err.python_location(), (1, 4));
        assert_eq!(err.python_end_location(), Some((1, 7)));

        compile(
            "f'{not x}'",
            Mode::Exec,
            "<fragment>",
            CompileOpts::default(),
        )
        .expect("unary not is a prefix, not two atoms");
        let err = compile(
            "f'{a and}'",
            Mode::Exec,
            "<fragment>",
            CompileOpts::default(),
        )
        .expect_err("a dangling 'and' is an f-string separator error");
        assert_eq!(
            err.to_string(),
            "f-string: expecting '=', or '!', or ':', or '}'"
        );
    }

    #[test]
    fn missing_comma_diagnostic_spans_the_whole_second_atom() {
        // `(start, end)` reported as one-based character columns, matching
        // `SyntaxError.offset` / `.end_offset`.
        let span = |source: &str| {
            let err = compile(source, Mode::Eval, "<comma>", CompileOpts::default())
                .expect_err("two adjacent atoms are a syntax error");
            assert_eq!(
                err.to_string(),
                "invalid syntax. Perhaps you forgot a comma?"
            );
            (
                err.python_location().1,
                err.python_end_location().unwrap().1,
            )
        };

        // A one-character second atom is the case that already worked.
        assert_eq!(span("(a b)"), (2, 5));
        // A longer one ends where it ends, not one byte in.
        assert_eq!(span("(a bb)"), (2, 6));
        assert_eq!(span("(a bbb)"), (2, 7));
        assert_eq!(span("(1 22)"), (2, 6));
        // A non-ASCII atom is one character but several bytes, so counting
        // bytes here used to stop inside it and round back off the boundary.
        assert_eq!(span("(a \u{3b2})"), (2, 5));
        assert_eq!(span("(a \u{3b2}\u{3b2})"), (2, 6));
        assert_eq!(span("(\u{3b1}\u{3b1} \u{3b2})"), (2, 6));
        // The first atom's width was never the problem; pin it anyway.
        assert_eq!(span("(\u{3b1} b)"), (2, 5));
        // Other bracket kinds take the same path.
        assert_eq!(span("[\u{3b1} \u{3b2}]"), (2, 5));
    }

    #[test]
    fn parenthesized_yield_assignment_uses_invalid_target_message() {
        let err = compile(
            "def f(): (yield bar) = y\n",
            Mode::Exec,
            "<yield>",
            CompileOpts::default(),
        )
        .expect_err("parenthesized yield is not an assignment target");
        assert_eq!(
            err.to_string(),
            "cannot assign to yield expression here. Maybe you meant '==' instead of '='?"
        );
    }

    #[test]
    fn parenthesized_yield_augassign_uses_illegal_expression_message() {
        let err = compile(
            "def f(): (yield bar) += y\n",
            Mode::Exec,
            "<yield>",
            CompileOpts::default(),
        )
        .expect_err("parenthesized yield is not an augmented assignment target");
        assert_eq!(
            err.to_string(),
            "'yield expression' is an illegal expression for augmented assignment"
        );
    }

    #[test]
    fn kwarg_unparenthesized_genexp_uses_eq_or_walrus_message() {
        let err = compile(
            "dict(a = i for i in range(10))\n",
            Mode::Exec,
            "<kwarg>",
            CompileOpts::default(),
        )
        .expect_err("unparenthesized genexp after '=' is invalid");
        assert_eq!(
            err.to_string(),
            "invalid syntax. Maybe you meant '==' or ':=' instead of '='?"
        );
    }

    #[test]
    fn eval_fstring_assignment_keeps_invalid_syntax() {
        for source in ["f'' = 3", "f'{0}' = x", "f'{x}' = x"] {
            let err = compile(source, Mode::Eval, "<eval>", CompileOpts::default())
                .expect_err("assignment is invalid in eval");
            assert_eq!(err.to_string(), "invalid syntax", "{source}");
        }
        let err = compile("f'' = 3", Mode::Exec, "<exec>", CompileOpts::default())
            .expect_err("f-string is not an assignment target");
        assert_eq!(
            err.to_string(),
            "cannot assign to f-string expression here. Maybe you meant '==' instead of '='?"
        );
    }

    #[test]
    fn if_assignment_uses_eq_or_walrus_message() {
        let err = compile(
            "if x = 3: pass\n",
            Mode::Exec,
            "<if>",
            CompileOpts::default(),
        )
        .expect_err("assignment in if condition is invalid");
        assert_eq!(
            err.to_string(),
            "invalid syntax. Maybe you meant '==' or ':=' instead of '='?"
        );
    }

    #[test]
    fn parenthesized_if_assignment_uses_eq_or_walrus_message() {
        for source in [
            "if (x = 3): pass\n",
            "if ((x = 3)): pass\n",
            "if (x = 3) and y: pass\n",
        ] {
            let err = compile(source, Mode::Exec, "<if>", CompileOpts::default())
                .expect_err("parenthesized assignment in if condition is invalid");
            assert_eq!(
                err.to_string(),
                "invalid syntax. Maybe you meant '==' or ':=' instead of '='?"
            );
        }
    }

    #[test]
    fn earlier_syntax_error_is_not_replaced_by_later_condition() {
        let err = compile(
            "@@@\nif x = 3: pass\n",
            Mode::Exec,
            "<if>",
            CompileOpts::default(),
        )
        .expect_err("the first invalid token is the syntax error");
        assert_eq!(err.to_string(), "invalid syntax");
        assert_eq!(err.python_location().0, 1);
    }

    #[test]
    fn if_attribute_assignment_uses_invalid_target_hint() {
        let err = compile(
            "if x.a = 3: pass\n",
            Mode::Exec,
            "<if>",
            CompileOpts::default(),
        )
        .expect_err("attribute assignment in if condition is invalid");
        assert_eq!(
            err.to_string(),
            "cannot assign to attribute here. Maybe you meant '==' instead of '='?"
        );
    }

    #[test]
    fn parenthesized_yield_from_assignment_uses_invalid_target_message() {
        let err = compile(
            "def f(): (yield from value) = target\n",
            Mode::Exec,
            "<yield>",
            CompileOpts::default(),
        )
        .expect_err("parenthesized yield from is not an assignment target");
        assert_eq!(
            err.to_string(),
            "cannot assign to yield expression here. Maybe you meant '==' instead of '='?"
        );
    }

    #[test]
    fn set_display_assignment_uses_invalid_target_hint() {
        let err = compile(
            "{1, 2, 3} = 42\n",
            Mode::Exec,
            "<set>",
            CompileOpts::default(),
        )
        .expect_err("set display is not an assignment target");
        assert_eq!(
            err.to_string(),
            "cannot assign to set display here. Maybe you meant '==' instead of '='?"
        );
    }

    #[test]
    fn obsolete_not_equal_diagnostic_spans_the_whole_operator() {
        let err = compile("2 <> 3\n", Mode::Exec, "<obsolete>", CompileOpts::default())
            .expect_err("'<>' outside Barry mode is a syntax error");
        assert_eq!(
            err.to_string(),
            "invalid syntax.  Maybe you meant '!=' instead of '<>'?"
        );
        assert_eq!(err.python_location(), (1, 3));
        assert_eq!(err.python_end_location(), Some((1, 5)));

        // Only `<>` spans two characters; any other token that cannot start an
        // expression keeps its own location.
        let err = compile("2 <;\n", Mode::Exec, "<obsolete>", CompileOpts::default())
            .expect_err("'<;' is a syntax error");
        assert_eq!(err.to_string(), "invalid syntax");
        assert_eq!(err.python_location(), (1, 4));
        assert_eq!(err.python_end_location(), Some((1, 5)));

        // A `<>` that starts a statement is reported at the `<` too, where the
        // parser stops; only that first character is highlighted.
        let err = compile("<>\n", Mode::Exec, "<obsolete>", CompileOpts::default())
            .expect_err("a bare '<>' is a syntax error");
        assert_eq!(err.to_string(), "invalid syntax");
        assert_eq!(err.python_location(), (1, 1));
        assert_eq!(err.python_end_location(), Some((1, 2)));

        // A bracket left open earlier in the source outranks the operator.
        let err = compile(
            "(\n2 <> 3",
            Mode::Exec,
            "<obsolete>",
            CompileOpts::default(),
        )
        .expect_err("the bracket is never closed");
        assert_eq!(err.to_string(), "'(' was never closed");
        assert_eq!(err.python_location(), (1, 1));
    }

    #[test]
    fn barry_as_flufl_does_not_rewrite_strings_or_comments() {
        compile(
            "from __future__ import barry_as_FLUFL\nx = '<>'\n# <>\n",
            Mode::Exec,
            "<barry>",
            CompileOpts::default(),
        )
        .expect("Barry markers in strings and comments should stay untouched");
    }

    #[test]
    fn syntax_error_before_barry_not_equal_takes_precedence() {
        let err = compile(
            "from __future__ import barry_as_FLUFL\n<>\n2 != 3\n",
            Mode::Exec,
            "<barry>",
            CompileOpts::default(),
        )
        .expect_err("the earlier invalid comparison should fail");
        assert_eq!(err.to_string(), "invalid syntax");
        assert_eq!(err.python_location(), (2, 1));
    }

    #[test]
    fn unclosed_bracket_before_barry_not_equal_takes_precedence() {
        let err = compile(
            "from __future__ import barry_as_FLUFL\n(\n2 != 3",
            Mode::Exec,
            "<barry>",
            CompileOpts::default(),
        )
        .expect_err("the earlier unclosed bracket should fail");
        assert_eq!(err.to_string(), "'(' was never closed");
        assert_eq!(err.python_location(), (2, 1));
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

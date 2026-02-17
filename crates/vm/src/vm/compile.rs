//! Python code compilation functions.
//!
//! For code execution functions, see python_run.rs

use crate::{
    PyRef, VirtualMachine,
    builtins::PyCode,
    compiler::{self, CompileError, CompileOpts},
};

impl VirtualMachine {
    pub fn compile(
        &self,
        source: &str,
        mode: compiler::Mode,
        source_path: String,
    ) -> Result<PyRef<PyCode>, CompileError> {
        self.compile_with_opts(source, mode, source_path, self.compile_opts())
    }

    pub fn compile_with_opts(
        &self,
        source: &str,
        mode: compiler::Mode,
        source_path: String,
        opts: CompileOpts,
    ) -> Result<PyRef<PyCode>, CompileError> {
        let code =
            compiler::compile(source, mode, &source_path, opts).map(|code| self.ctx.new_code(code));
        #[cfg(feature = "parser")]
        if code.is_ok() {
            self.emit_string_escape_warnings(source, &source_path);
        }
        code
    }
}

/// Scan source for invalid escape sequences in all string literals and emit
/// SyntaxWarning.
///
/// Corresponds to:
/// - `warn_invalid_escape_sequence()` in `Parser/string_parser.c`
/// - `_PyTokenizer_warn_invalid_escape_sequence()` in `Parser/tokenizer/helpers.c`
#[cfg(feature = "parser")]
mod escape_warnings {
    use super::*;
    use crate::warn;
    use ruff_python_ast::{self as ast, visitor::Visitor};
    use ruff_text_size::TextRange;

    /// Calculate 1-indexed line number at byte offset in source.
    fn line_number_at(source: &str, offset: usize) -> usize {
        source[..offset.min(source.len())]
            .bytes()
            .filter(|&b| b == b'\n')
            .count()
            + 1
    }

    /// Get content bounds (start, end byte offsets) of a quoted string literal,
    /// excluding prefix characters and quote delimiters.
    fn content_bounds(source: &str, range: TextRange) -> Option<(usize, usize)> {
        let s = range.start().to_usize();
        let e = range.end().to_usize();
        if s >= e || e > source.len() {
            return None;
        }
        let bytes = &source.as_bytes()[s..e];
        // Skip prefix (u, b, r, etc.) to find the first quote character.
        let qi = bytes.iter().position(|&c| c == b'\'' || c == b'"')?;
        let qc = bytes[qi];
        let ql = if bytes.get(qi + 1) == Some(&qc) && bytes.get(qi + 2) == Some(&qc) {
            3
        } else {
            1
        };
        let cs = s + qi + ql;
        let ce = e.checked_sub(ql)?;
        if cs <= ce { Some((cs, ce)) } else { None }
    }

    /// Scan `source[start..end]` for the first invalid escape sequence.
    /// Returns `Some((invalid_char, byte_offset_in_source))` for the first
    /// invalid escape found, or `None` if all escapes are valid.
    ///
    /// When `is_bytes` is true, `\u`, `\U`, and `\N` are treated as invalid
    /// (bytes literals only support byte-oriented escapes).
    ///
    /// Only reports the **first** invalid escape per string literal, matching
    /// `_PyUnicode_DecodeUnicodeEscapeInternal2` which stores only the first
    /// `first_invalid_escape_char`.
    fn first_invalid_escape(
        source: &str,
        start: usize,
        end: usize,
        is_bytes: bool,
    ) -> Option<(char, usize)> {
        let raw = &source[start..end];
        let mut chars = raw.char_indices().peekable();
        while let Some((i, ch)) = chars.next() {
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
                    if matches!(chars.peek(), Some(&(_, '\n'))) {
                        chars.next();
                    }
                    true
                }
                '0'..='7' => {
                    for _ in 0..2 {
                        if matches!(chars.peek(), Some(&(_, '0'..='7'))) {
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    true
                }
                'x' | 'u' | 'U' => {
                    // \u and \U are only valid in string literals, not bytes
                    if is_bytes && next != 'x' {
                        false
                    } else {
                        let count = match next {
                            'x' => 2,
                            'u' => 4,
                            'U' => 8,
                            _ => unreachable!(),
                        };
                        for _ in 0..count {
                            if chars.peek().is_some_and(|&(_, c)| c.is_ascii_hexdigit()) {
                                chars.next();
                            } else {
                                break;
                            }
                        }
                        true
                    }
                }
                'N' => {
                    // \N{name} is only valid in string literals, not bytes
                    if is_bytes {
                        false
                    } else {
                        if matches!(chars.peek(), Some(&(_, '{'))) {
                            chars.next();
                            for (_, c) in chars.by_ref() {
                                if c == '}' {
                                    break;
                                }
                            }
                        }
                        true
                    }
                }
                _ => false,
            };
            if !valid {
                return Some((next, start + i));
            }
        }
        None
    }

    /// Emit `SyntaxWarning` for an invalid escape sequence.
    ///
    /// `warn_invalid_escape_sequence()` in `Parser/string_parser.c`
    fn warn_invalid_escape_sequence(
        source: &str,
        ch: char,
        offset: usize,
        filename: &str,
        vm: &VirtualMachine,
    ) {
        let lineno = line_number_at(source, offset);
        let message = vm.ctx.new_str(format!(
            "\"\\{ch}\" is an invalid escape sequence. \
             Such sequences will not work in the future. \
             Did you mean \"\\\\{ch}\"? A raw string is also an option."
        ));
        let fname = vm.ctx.new_str(filename);
        let _ = warn::warn_explicit(
            Some(vm.ctx.exceptions.syntax_warning.to_owned()),
            message.into(),
            fname,
            lineno,
            None,
            vm.ctx.none(),
            None,
            None,
            vm,
        );
    }

    struct EscapeWarningVisitor<'a> {
        source: &'a str,
        filename: &'a str,
        vm: &'a VirtualMachine,
    }

    impl<'a> EscapeWarningVisitor<'a> {
        /// Check a quoted string/bytes literal for invalid escapes.
        /// The range must include the prefix and quote delimiters.
        fn check_quoted_literal(&self, range: TextRange, is_bytes: bool) {
            if let Some((start, end)) = content_bounds(self.source, range)
                && let Some((ch, offset)) = first_invalid_escape(self.source, start, end, is_bytes)
            {
                warn_invalid_escape_sequence(self.source, ch, offset, self.filename, self.vm);
            }
        }

        /// Check an f-string literal element for invalid escapes.
        /// The range covers content only (no prefix/quotes).
        ///
        /// Also handles `\{` / `\}` at the literal–interpolation boundary,
        /// equivalent to `_PyTokenizer_warn_invalid_escape_sequence` handling
        /// `FSTRING_MIDDLE` / `FSTRING_END` tokens.
        fn check_fstring_literal(&self, range: TextRange) {
            let start = range.start().to_usize();
            let end = range.end().to_usize();
            if start >= end || end > self.source.len() {
                return;
            }
            if let Some((ch, offset)) = first_invalid_escape(self.source, start, end, false) {
                warn_invalid_escape_sequence(self.source, ch, offset, self.filename, self.vm);
                return;
            }
            // In CPython, _PyTokenizer_warn_invalid_escape_sequence handles
            // `\{` and `\}` for FSTRING_MIDDLE/FSTRING_END tokens.  Ruff
            // splits the literal element before the interpolation delimiter,
            // so the `\` sits at the end of the literal range and the `{`/`}`
            // sits just after it.  Only warn when the number of trailing
            // backslashes is odd (an even count means they are all escaped).
            let trailing_bs = self.source.as_bytes()[start..end]
                .iter()
                .rev()
                .take_while(|&&b| b == b'\\')
                .count();
            if trailing_bs % 2 == 1
                && let Some(&after) = self.source.as_bytes().get(end)
                && (after == b'{' || after == b'}')
            {
                warn_invalid_escape_sequence(
                    self.source,
                    after as char,
                    end - 1,
                    self.filename,
                    self.vm,
                );
            }
        }

        /// Visit f-string elements, checking literals and recursing into
        /// interpolation expressions and format specs.
        fn visit_fstring_elements(&mut self, elements: &'a ast::InterpolatedStringElements) {
            for element in elements {
                match element {
                    ast::InterpolatedStringElement::Literal(lit) => {
                        self.check_fstring_literal(lit.range);
                    }
                    ast::InterpolatedStringElement::Interpolation(interp) => {
                        self.visit_expr(&interp.expression);
                        if let Some(spec) = &interp.format_spec {
                            self.visit_fstring_elements(&spec.elements);
                        }
                    }
                }
            }
        }
    }

    impl<'a> Visitor<'a> for EscapeWarningVisitor<'a> {
        fn visit_expr(&mut self, expr: &'a ast::Expr) {
            match expr {
                // Regular string literals — decode_unicode_with_escapes path
                ast::Expr::StringLiteral(string) => {
                    for part in string.value.as_slice() {
                        if !matches!(
                            part.flags.prefix(),
                            ast::str_prefix::StringLiteralPrefix::Raw { .. }
                        ) {
                            self.check_quoted_literal(part.range, false);
                        }
                    }
                }
                // Byte string literals — decode_bytes_with_escapes path
                ast::Expr::BytesLiteral(bytes) => {
                    for part in bytes.value.as_slice() {
                        if !matches!(
                            part.flags.prefix(),
                            ast::str_prefix::ByteStringPrefix::Raw { .. }
                        ) {
                            self.check_quoted_literal(part.range, true);
                        }
                    }
                }
                // F-string literals — tokenizer + string_parser paths
                ast::Expr::FString(fstring_expr) => {
                    for part in fstring_expr.value.as_slice() {
                        match part {
                            ast::FStringPart::Literal(string_lit) => {
                                // Plain string part in f-string concatenation
                                if !matches!(
                                    string_lit.flags.prefix(),
                                    ast::str_prefix::StringLiteralPrefix::Raw { .. }
                                ) {
                                    self.check_quoted_literal(string_lit.range, false);
                                }
                            }
                            ast::FStringPart::FString(fstring) => {
                                if matches!(
                                    fstring.flags.prefix(),
                                    ast::str_prefix::FStringPrefix::Raw { .. }
                                ) {
                                    continue;
                                }
                                self.visit_fstring_elements(&fstring.elements);
                            }
                        }
                    }
                }
                _ => ast::visitor::walk_expr(self, expr),
            }
        }
    }

    impl VirtualMachine {
        /// Walk all string literals in `source` and emit `SyntaxWarning` for
        /// each that contains an invalid escape sequence.
        pub(super) fn emit_string_escape_warnings(&self, source: &str, filename: &str) {
            let Ok(parsed) =
                ruff_python_parser::parse(source, ruff_python_parser::Mode::Module.into())
            else {
                return;
            };
            let ast = parsed.into_syntax();
            let mut visitor = EscapeWarningVisitor {
                source,
                filename,
                vm: self,
            };
            match ast {
                ast::Mod::Module(module) => {
                    for stmt in &module.body {
                        visitor.visit_stmt(stmt);
                    }
                }
                ast::Mod::Expression(expr) => {
                    visitor.visit_expr(&expr.body);
                }
            }
        }
    }
}

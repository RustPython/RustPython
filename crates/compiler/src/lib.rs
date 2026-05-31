pub use ruff_python_ast::token::TokenKind;
use ruff_python_parser::{LexicalErrorType, ParseErrorType};
use ruff_source_file::{PositionEncoding, SourceFile, SourceFileBuilder, SourceLocation};
use ruff_text_size::TextSlice;
use thiserror::Error;

use rustpython_codegen::{compile, symboltable};

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
        let source_code = source_file.to_source_code();
        let source_text = source_file.source_text();

        // For EOF errors (unclosed brackets), find the unclosed bracket position
        // and adjust both the error location and message
        let mut is_unclosed_bracket = false;
        let (error_type, location, end_location) = match &error.error {
            ParseErrorType::Lexical(LexicalErrorType::Eof) => {
                if let Some((bracket_char, bracket_offset)) = find_unclosed_bracket(source_text) {
                    let bracket_text_size = ruff_text_size::TextSize::new(bracket_offset as u32);
                    let loc =
                        source_code.source_location(bracket_text_size, PositionEncoding::Utf8);
                    let end_loc = SourceLocation {
                        line: loc.line,
                        character_offset: loc.character_offset.saturating_add(1),
                    };
                    let msg = format!("'{bracket_char}' was never closed");
                    is_unclosed_bracket = true;
                    (ParseErrorType::OtherError(msg), loc, end_loc)
                } else {
                    let loc =
                        source_code.source_location(error.location.start(), PositionEncoding::Utf8);
                    let end_loc =
                        source_code.source_location(error.location.end(), PositionEncoding::Utf8);
                    (error.error, loc, end_loc)
                }
            }

            ParseErrorType::Lexical(LexicalErrorType::IndentationError) => {
                // For IndentationError, point the offset to the end of the line content
                // instead of the beginning
                let loc =
                    source_code.source_location(error.location.start(), PositionEncoding::Utf8);
                let line_idx = loc.line.to_zero_indexed();
                let line = source_text.split('\n').nth(line_idx).unwrap_or("");
                let line_end_col = line.chars().count() + 1; // 1-indexed, past last char
                let end_loc = SourceLocation {
                    line: loc.line,
                    character_offset: ruff_source_file::OneIndexed::new(line_end_col)
                        .unwrap_or(loc.character_offset),
                };
                (error.error, end_loc, end_loc)
            }
            ParseErrorType::ExpectedToken { expected, found }
                if matches!((expected, found), (TokenKind::Comma, TokenKind::Int)) =>
            {
                let loc =
                    source_code.source_location(error.location.start(), PositionEncoding::Utf8);
                let mut end_loc =
                    source_code.source_location(error.location.end(), PositionEncoding::Utf8);

                // If the error range ends at the start of a new line (column 1),
                // adjust it to the end of the previous line
                if end_loc.character_offset.get() == 1 && end_loc.line > loc.line {
                    let prev_line_end = error.location.end() - ruff_text_size::TextSize::from(1);
                    end_loc = source_code.source_location(prev_line_end, PositionEncoding::Utf8);
                    end_loc.character_offset = end_loc.character_offset.saturating_add(1);
                }
                let msg = "invalid syntax. Perhaps you forgot a comma?".into();
                (ParseErrorType::OtherError(msg), loc, end_loc)
            }

            ParseErrorType::InvalidAssignmentTarget => {
                let (loc, end_loc) = adjusted_locations(&source_code, error.location);
                let expr_str = source_file.source_text().slice(error.location);
                let followed_by_eq = next_nonspace_after(source_text, error.location) == Some('=');

                let msg = parser::parse_expression(expr_str).map_or_else(
                    |_| match expr_str {
                        "yield" => "assignment to yield expression not possible".into(),
                        _ => format!("cannot assign to {expr_str}"),
                    },
                    |parsed| {
                        // A comparison / bool-op `for` target (`for i < (): ...`)
                        // is plain "invalid syntax" in CPython, not a
                        // "cannot assign to comparison" message.
                        if is_for_loop_target(source_text, error.location)
                            && matches!(*parsed.syntax().body, ast::Expr::Compare(_))
                        {
                            "invalid syntax".to_owned()
                        } else {
                            assign_target_message(&parsed.syntax().body, expr_str, followed_by_eq)
                        }
                    },
                );

                (ParseErrorType::OtherError(msg), loc, end_loc)
            }

            ParseErrorType::InvalidAugmentedAssignmentTarget => {
                let (loc, end_loc) = adjusted_locations(&source_code, error.location);
                let expr_str = source_file.source_text().slice(error.location);

                let kind = parser::parse_expression(expr_str)
                    .ok()
                    .map_or("expression", |parsed| expr_kind_name(&parsed.syntax().body));
                let msg = format!("'{kind}' is an illegal expression for augmented assignment");
                (ParseErrorType::OtherError(msg), loc, end_loc)
            }

            ParseErrorType::InvalidDeleteTarget => {
                let (loc, end_loc) = adjusted_locations(&source_code, error.location);
                let expr_str = source_file.source_text().slice(error.location);

                // A bare starred target (`del *x,`) doesn't parse as a
                // standalone expression, so fall back to detecting the `*`.
                let kind = parser::parse_expression(expr_str).ok().map_or_else(
                    || {
                        if expr_str.trim_start().starts_with('*') {
                            "starred"
                        } else {
                            "expression"
                        }
                    },
                    |parsed| expr_kind_name(&parsed.syntax().body),
                );
                let msg = format!("cannot delete {kind}");
                (ParseErrorType::OtherError(msg), loc, end_loc)
            }

            ParseErrorType::InvalidNamedAssignmentTarget => {
                let (loc, end_loc) = adjusted_locations(&source_code, error.location);
                let target = source_file.source_text().slice(error.location);
                // CPython names the target kind (`tuple`, `attribute`, `True`, …)
                // rather than echoing the raw source; fall back to the raw text
                // if it doesn't parse as an expression.
                let kind = parser::parse_expression(target).ok().map_or_else(
                    || target.to_owned(),
                    |p| expr_kind_name(&p.syntax().body).to_owned(),
                );
                let msg = format!("cannot use assignment expressions with {kind}");
                (ParseErrorType::OtherError(msg), loc, end_loc)
            }

            // Convert "Expected an indented block after `X` <kind>" (ruff)
            // into "expected an indented block after 'X' <kind> on line N"
            // (CPython). Replaces backticks with single quotes and appends
            // the originating statement's line number.
            ParseErrorType::OtherError(s) if s.starts_with("Expected an indented block after") => {
                let (loc, end_loc) = adjusted_locations(&source_code, error.location);
                let mut msg = normalize_indented_block_message(s);
                // Find the keyword in backticks to compute the line number.
                let lineno = find_indented_block_keyword_line(source_text, error.location, s)
                    .unwrap_or_else(|| loc.line.get());
                // For `except`, CPython distinguishes `except*` from `except`.
                if msg.contains("'except' statement")
                    && source_uses_except_star(source_text, lineno)
                {
                    msg = msg.replace("'except' statement", "'except*' statement");
                }
                // Keep the leading "Expected" capital so vm_new.rs still
                // detects the message and picks `IndentationError`.
                msg.push_str(&format!(" on line {lineno}"));
                (ParseErrorType::OtherError(msg), loc, end_loc)
            }

            // `def f(.../*,...)` / `lambda .../*,...:`: missing comma between
            // `/` and `*` markers.
            ParseErrorType::ExpectedToken {
                expected: TokenKind::Comma,
                found: TokenKind::Star,
            } if is_slash_star_in_params(source_text, error.location) => {
                let (loc, end_loc) = adjusted_locations(&source_code, error.location);
                let msg = "expected comma between / and *".to_owned();
                (ParseErrorType::OtherError(msg), loc, end_loc)
            }

            // `f(**kwargs=...)`: assignment to a `**` unpacking.
            ParseErrorType::ExpectedToken {
                expected: TokenKind::Comma,
                found: TokenKind::Equal,
            } if is_kwarg_unpacking_assignment(source_text, error.location) => {
                let (loc, end_loc) = adjusted_locations(&source_code, error.location);
                let msg = "cannot assign to keyword argument unpacking".to_owned();
                (ParseErrorType::OtherError(msg), loc, end_loc)
            }

            // Comprehension with comma'd unparenthesised target:
            //   [x,y for x,y in range(100)] / {x,y for x,y in range(100)}.
            ParseErrorType::ExpectedToken {
                expected: TokenKind::Rsqb | TokenKind::Rbrace,
                found: TokenKind::For,
            } if is_unparen_comprehension_target(source_text, error.location) => {
                let (loc, end_loc) = adjusted_locations(&source_code, error.location);
                let msg = "did you forget parentheses around the comprehension target?".to_owned();
                (ParseErrorType::OtherError(msg), loc, end_loc)
            }

            // `raise X, Y` (old Python-2 syntax): ruff parses `X, Y` as an
            // unparenthesised tuple and complains; CPython just says
            // "invalid syntax".
            ParseErrorType::UnparenthesizedTupleExpression
                if source_starts_with_raise(source_text, error.location) =>
            {
                let (loc, end_loc) = adjusted_locations(&source_code, error.location);
                let msg = "invalid syntax".to_owned();
                (ParseErrorType::OtherError(msg), loc, end_loc)
            }

            // Bare `*` in a function call argument list:
            //   f(x, *), f(**x, *), f(x = 5, *), f(x, *:) — CPython says
            //   "Invalid star expression".
            ParseErrorType::ExpectedExpression | ParseErrorType::ExpectedToken { .. }
                if is_bare_star_in_call(source_text, error.location) =>
            {
                let (loc, end_loc) = adjusted_locations(&source_code, error.location);
                let msg = "Invalid star expression".to_owned();
                (ParseErrorType::OtherError(msg), loc, end_loc)
            }

            // `except[*] T as <bad>`: ruff rejects with either
            // "Expected name after `as`" or "invalid syntax". CPython
            // distinguishes the kind of the bad target.
            ParseErrorType::OtherError(s)
                if s == "Expected name after `as`"
                    && let Some(msg) =
                        except_as_bad_target_message(source_text, error.location) =>
            {
                let (loc, end_loc) = adjusted_locations(&source_code, error.location);
                (ParseErrorType::OtherError(msg), loc, end_loc)
            }
            ParseErrorType::ExpectedToken { .. } | ParseErrorType::OtherError(_)
                if let Some(msg) = except_as_bad_target_message(source_text, error.location) =>
            {
                let (loc, end_loc) = adjusted_locations(&source_code, error.location);
                (ParseErrorType::OtherError(msg), loc, end_loc)
            }

            // Comprehension with `if` after the for-target instead of `in`:
            //   [x for x if range(1)] / (x for x if y) / {... if ...}.
            // Ruff treats `x if y` as a ternary expression and complains about
            // a missing `else`; CPython points out the missing `in`.
            ParseErrorType::ExpectedToken {
                expected: TokenKind::Else,
                ..
            } if is_in_comprehension_if(source_text, error.location) => {
                let (loc, end_loc) = adjusted_locations(&source_code, error.location);
                let msg = "'in' expected after for-loop variables".to_owned();
                (ParseErrorType::OtherError(msg), loc, end_loc)
            }

            // Subscript with bare or partial starred expression:
            //   A[*], A[*:], A[*(1:2)], A[*(1:2)] = 1, del A[*(1:2)].
            // CPython reports "Invalid star expression".
            ParseErrorType::ExpectedExpression | ParseErrorType::ExpectedToken { .. }
                if is_invalid_star_in_subscript(source_text, error.location) =>
            {
                let (loc, end_loc) = adjusted_locations(&source_code, error.location);
                let msg = "Invalid star expression".to_owned();
                (ParseErrorType::OtherError(msg), loc, end_loc)
            }

            // Subscript with parenthesised starred expression as slice arg:
            //   A[(*b):], A[:(*b)] — "cannot use starred expression here".
            // Only fires when the starred expression is parenthesised (the
            // unparenthesised `A[:*b]` case stays "invalid syntax" per CPython).
            ParseErrorType::InvalidStarredExpressionUsage
                if is_paren_starred_in_subscript(source_text, error.location) =>
            {
                let (loc, end_loc) = adjusted_locations(&source_code, error.location);
                let msg = "cannot use starred expression here".to_owned();
                (ParseErrorType::OtherError(msg), loc, end_loc)
            }

            // Bare `*` as the leading element of a set/dict display `{*}` /
            // `{*, 1}` or a non-call parenthesised group `(*)` / `(*,)`. CPython
            // reports "Invalid star expression". A non-leading star (`{1, *}`),
            // a double star (`{**}`), or a dict value (`{1: *}`) is excluded.
            ParseErrorType::ExpectedExpression
                if is_bare_star_first_in_group(source_text, error.location) =>
            {
                let (loc, end_loc) = adjusted_locations(&source_code, error.location);
                let msg = "Invalid star expression".to_owned();
                (ParseErrorType::OtherError(msg), loc, end_loc)
            }

            // Dict literal: `{1:}` / `{1: 2, 3: 4, 5: }` — missing value.
            ParseErrorType::ExpectedExpression
                if is_dict_value_position(source_text, error.location) =>
            {
                let (loc, end_loc) = adjusted_locations(&source_code, error.location);
                let msg = "expression expected after dictionary key and ':'".to_owned();
                (ParseErrorType::OtherError(msg), loc, end_loc)
            }

            // Dict literal: `{1: 2, 3: 4, 5}` — missing `:` after last key.
            // (Not inside a match `case {…}` mapping pattern → "invalid syntax".)
            ParseErrorType::ExpectedToken {
                expected: TokenKind::Colon,
                found: TokenKind::Rbrace,
            } if !is_in_case_pattern(source_text, error.location) => {
                let (loc, end_loc) = adjusted_locations(&source_code, error.location);
                let msg = "':' expected after dictionary key".to_owned();
                (ParseErrorType::OtherError(msg), loc, end_loc)
            }

            // Dict literal: `{1: *12+1}` — starred expression as value.
            ParseErrorType::InvalidStarredExpressionUsage
                if is_dict_value_position(source_text, error.location) =>
            {
                let (loc, end_loc) = adjusted_locations(&source_code, error.location);
                let msg = "cannot use a starred expression in a dictionary value".to_owned();
                (ParseErrorType::OtherError(msg), loc, end_loc)
            }

            // Detect missing default-value expression (`def f(a=, b): ...`)
            // and missing argument-value expression (`f(a=)`).
            _ if let Some(msg) = missing_default_or_argument_value(source_text, error.location) => {
                let (loc, end_loc) = adjusted_locations(&source_code, error.location);
                (ParseErrorType::OtherError(msg), loc, end_loc)
            }

            // Detect incompatible string prefixes (`ub''`, `bf""`, etc.).
            // CPython reports "'X' and 'Y' prefixes are incompatible".
            _ if let Some(msg) =
                incompatible_string_prefix_message(source_text, error.location) =>
            {
                let (loc, end_loc) = adjusted_locations(&source_code, error.location);
                (ParseErrorType::OtherError(msg), loc, end_loc)
            }

            // Detect parenthesized parameters in `def f(x, (y, z), w): ...`
            // or `lambda x, (y, z), w: None` — CPython rejects with
            // "Function parameters cannot be parenthesized" / "Lambda expression
            // parameters cannot be parenthesized".
            ParseErrorType::ExpectedToken { .. }
            | ParseErrorType::OtherError(_)
            | ParseErrorType::ExpectedExpression
                if let Some(msg) = parenthesized_param_message(source_text, error.location) =>
            {
                let (loc, end_loc) = adjusted_locations(&source_code, error.location);
                (ParseErrorType::OtherError(msg), loc, end_loc)
            }

            // Detect bad keyword-argument LHS in a function call:
            //   f(x()=2), f(a or b=1), f(x.y=1), f((x)=2),
            //   f(True=1), f(False=1), f(None=1),
            //   f(*args=[0]), f(**kwargs={...}).
            // Ruff emits "Expected a parameter name". CPython varies the
            // message by the LHS shape.
            ParseErrorType::OtherError(s)
                if s == "Expected a parameter name"
                    && is_call_keyword_assignment(source_text, error.location) =>
            {
                let (loc, end_loc) = adjusted_locations(&source_code, error.location);
                let lhs = call_keyword_lhs(source_text, error.location);
                let trimmed = lhs.trim();
                let msg = if trimmed.starts_with("**") {
                    "cannot assign to keyword argument unpacking".to_owned()
                } else if trimmed.starts_with('*') {
                    "cannot assign to iterable argument unpacking".to_owned()
                } else {
                    match trimmed {
                        "True" => "cannot assign to True".to_owned(),
                        "False" => "cannot assign to False".to_owned(),
                        "None" => "cannot assign to None".to_owned(),
                        "__debug__" => "cannot assign to __debug__".to_owned(),
                        _ => r#"expression cannot contain assignment, perhaps you meant "=="?"#
                            .to_owned(),
                    }
                };
                (ParseErrorType::OtherError(msg), loc, end_loc)
            }

            // Detect `X=Y` in a tuple/list/set literal (not a call):
            //   (x, y, z=3), [a=1], {a=1}.
            _ if let Some(msg) = collection_kwarg_message(source_text, error.location) => {
                let (loc, end_loc) = adjusted_locations(&source_code, error.location);
                (ParseErrorType::OtherError(msg), loc, end_loc)
            }

            // Detect `pass` (or other statement keywords) used in expression
            // position within a ternary `<expr> if <cond> else <expr>` —
            // CPython suggests "expected expression after/before '...'".
            ParseErrorType::ExpectedExpression
            | ParseErrorType::ExpectedToken { .. }
            | ParseErrorType::InvalidYieldExpressionUsage
            | ParseErrorType::InvalidStarredExpressionUsage
            | ParseErrorType::OtherError(_)
                if let Some(msg) =
                    ternary_statement_keyword_message(source_text, error.location) =>
            {
                let (loc, end_loc) = adjusted_locations(&source_code, error.location);
                (ParseErrorType::OtherError(msg), loc, end_loc)
            }

            // Detect `if X = Y:` / `while X = Y:` where `=` should be `==` or `:=`.
            // Ruff emits `ExpectedToken { Colon, Equal }`. CPython varies the
            // message by the LHS expression kind.
            ParseErrorType::ExpectedToken {
                expected: TokenKind::Colon,
                found: TokenKind::Equal,
            } if precedes_if_or_while(source_text, error.location) => {
                let (loc, end_loc) = adjusted_locations(&source_code, error.location);
                let start: usize = error.location.start().into();
                let line_start = source_text[..start].rfind('\n').map_or(0, |i| i + 1);
                // Pull out the LHS expression between the keyword and `=`.
                let lhs_slice = lhs_after_keyword(&source_text[line_start..start]);
                let msg = match parser::parse_expression(lhs_slice) {
                    Ok(parsed) => match *parsed.syntax().body {
                        ast::Expr::Attribute(_) => {
                            "cannot assign to attribute here. Maybe you meant '==' instead of '='?"
                                .to_owned()
                        }
                        ast::Expr::Subscript(_) => {
                            "cannot assign to subscript here. Maybe you meant '==' instead of '='?"
                                .to_owned()
                        }
                        _ => "invalid syntax. Maybe you meant '==' or ':=' instead of '='?"
                            .to_owned(),
                    },
                    Err(_) => {
                        "invalid syntax. Maybe you meant '==' or ':=' instead of '='?".to_owned()
                    }
                };
                (ParseErrorType::OtherError(msg), loc, end_loc)
            }

            // Detect `import X from Y` — CPython suggests the correct syntax.
            ParseErrorType::SimpleStatementsOnSameLine
            | ParseErrorType::ExpectedToken { .. }
            | ParseErrorType::OtherError(_)
                if is_old_import_from(source_text, error.location) =>
            {
                let (loc, end_loc) = adjusted_locations(&source_code, error.location);
                let msg = "Did you mean to use 'from ... import ...' instead?".to_owned();
                (ParseErrorType::OtherError(msg), loc, end_loc)
            }

            // Detect `import X as Y.Z` / `import X as Y[Z]` / `import X as Y()`
            // where the parser successfully parsed the asname as a Name but
            // the following token is `.`, `[`, or `(`. CPython distinguishes
            // these as "cannot use attribute/subscript/function call as import target".
            ParseErrorType::ExpectedToken { .. } | ParseErrorType::OtherError(_)
                if is_import_target_continuation(source_text, error.location) =>
            {
                let (loc, end_loc) = adjusted_locations(&source_code, error.location);
                let start: usize = error.location.start().into();
                let first = source_text[start..].chars().next();
                let kind = match first {
                    Some('.') => "attribute",
                    Some('[') => "subscript",
                    Some('(') => "function call",
                    _ => "expression",
                };
                let msg = format!("cannot use {kind} as import target");
                (ParseErrorType::OtherError(msg), loc, end_loc)
            }

            ParseErrorType::OtherError(s) if s == "Expected symbol after `as`" => {
                let (loc, end_loc) = adjusted_locations(&source_code, error.location);
                let start: usize = error.location.start().into();
                let after = source_text[start..].trim_start();
                let kind = if let Some(first) = after.chars().next() {
                    match first {
                        '(' => Some("tuple"),
                        '[' => Some("list"),
                        '0'..='9' | '\'' | '"' | '.' => Some("literal"),
                        _ => None,
                    }
                } else {
                    None
                };
                let msg = match kind {
                    Some(k) => format!("cannot use {k} as import target"),
                    None => "Expected symbol after `as`".to_owned(),
                };
                (ParseErrorType::OtherError(msg), loc, end_loc)
            }

            ParseErrorType::VarParameterWithDefault => {
                let (loc, end_loc) = adjusted_locations(&source_code, error.location);
                // The error location is just the current token (e.g. `=` or the
                // default value), not the whole `*X=Y` / `**X=Y`. Find the most
                // recent `**` / `*` before the error to know which one applies.
                let start: usize = error.location.start().into();
                let prefix = &source_text[..start];
                let last_star = prefix.rfind('*');
                let is_kwarg =
                    last_star.is_some_and(|idx| idx > 0 && prefix.as_bytes()[idx - 1] == b'*');
                let msg = if is_kwarg {
                    "var-keyword argument cannot have default value"
                } else {
                    "var-positional argument cannot have default value"
                }
                .to_owned();
                (ParseErrorType::OtherError(msg), loc, end_loc)
            }

            // A non-ASCII unrecognized character → CPython's
            // "invalid character 'X' (U+XXXX)". ASCII junk (`$`, `?`, `@`) stays
            // "invalid syntax" via the default path.
            ParseErrorType::Lexical(LexicalErrorType::UnrecognizedToken { tok })
                if !tok.is_ascii() =>
            {
                let (loc, end_loc) = adjusted_locations(&source_code, error.location);
                let msg = format!("invalid character '{tok}' (U+{:04X})", *tok as u32);
                (ParseErrorType::OtherError(msg), loc, end_loc)
            }

            // Ruff `OtherError` strings that CPython collapses to "invalid syntax":
            //   `match x:\n y=3` (no case), `case {**rest, ...}` (pattern after
            //   double-star), `… raise from None` (missing exception), `foo(,)`,
            //   `{1:2,, 3}` / `[1,, 2]` (double comma in dict/set/list display).
            ParseErrorType::OtherError(s)
                if matches!(
                    s.as_str(),
                    "Expected `case` block"
                        | "Pattern cannot follow a double star pattern"
                        | "Exception missing in `raise` statement with cause"
                        | "Expected an expression or a ')'"
                        | "Expected an expression or a '}'"
                        | "Expected an expression or a ']'"
                ) =>
            {
                let (loc, end_loc) = adjusted_locations(&source_code, error.location);
                (
                    ParseErrorType::OtherError("invalid syntax".to_owned()),
                    loc,
                    end_loc,
                )
            }

            // `if …: else: … elif …:` — an `elif` after the `else` block.
            ParseErrorType::OtherError(s)
                if s == "Expected a statement"
                    && source_text.get(
                        usize::from(error.location.start())..usize::from(error.location.end()),
                    ) == Some("elif") =>
            {
                let (loc, end_loc) = adjusted_locations(&source_code, error.location);
                (
                    ParseErrorType::OtherError("'elif' block follows an 'else' block".to_owned()),
                    loc,
                    end_loc,
                )
            }

            // Any other "Expected a statement" (e.g. a bare `x := 0`, `@`,
            // `else: pass`) is plain "invalid syntax" in CPython.
            ParseErrorType::OtherError(s) if s == "Expected a statement" => {
                let (loc, end_loc) = adjusted_locations(&source_code, error.location);
                (
                    ParseErrorType::OtherError("invalid syntax".to_owned()),
                    loc,
                    end_loc,
                )
            }

            // `(mat x)` / `(a b)` — a name where `)` (i.e. a comma) was expected.
            // (Not inside a match `case (a b)` sequence pattern, where CPython
            // reports plain "invalid syntax".)
            ParseErrorType::ExpectedToken {
                expected: TokenKind::Rpar,
                found: TokenKind::Name,
            } if !is_in_case_pattern(source_text, error.location) => {
                let (loc, end_loc) = adjusted_locations(&source_code, error.location);
                (
                    ParseErrorType::OtherError(
                        "invalid syntax. Perhaps you forgot a comma?".to_owned(),
                    ),
                    loc,
                    end_loc,
                )
            }

            // `def f(*None): …` — bare `*` followed by a keyword (not a name).
            ParseErrorType::ExpectedKeywordParam
                if next_word_is_keyword(source_text, error.location) =>
            {
                let (loc, end_loc) = adjusted_locations(&source_code, error.location);
                (
                    ParseErrorType::OtherError("invalid syntax".to_owned()),
                    loc,
                    end_loc,
                )
            }

            // `a, b += 1, 2` — unparenthesized tuple as an augmented-assignment
            // target (ruff wants a comma but found an augmented-assign operator).
            ParseErrorType::ExpectedToken {
                expected: TokenKind::Comma,
                ..
            } if source_text
                .get(usize::from(error.location.start())..usize::from(error.location.end()))
                .is_some_and(is_augassign_op) =>
            {
                let (loc, end_loc) = adjusted_locations(&source_code, error.location);
                (
                    ParseErrorType::OtherError(
                        "'tuple' is an illegal expression for augmented assignment".to_owned(),
                    ),
                    loc,
                    end_loc,
                )
            }

            // `from a import b, c as d[e]` — a subscript as an import target.
            ParseErrorType::SimpleStatementsOnSameLine
                if is_import_subscript_target(source_text, error.location) =>
            {
                let (loc, end_loc) = adjusted_locations(&source_code, error.location);
                (
                    ParseErrorType::OtherError("cannot use subscript as import target".to_owned()),
                    loc,
                    end_loc,
                )
            }

            // `"a "b" c"` — a string literal immediately followed by a bare word,
            // i.e. an unintended split string.
            ParseErrorType::SimpleStatementsOnSameLine
                if looks_like_split_string(source_text, error.location) =>
            {
                let (loc, end_loc) = adjusted_locations(&source_code, error.location);
                (
                    ParseErrorType::OtherError(
                        "invalid syntax. Is this intended to be part of the string?".to_owned(),
                    ),
                    loc,
                    end_loc,
                )
            }

            // `case 42 as a.b` / `as (a, b)` / `as a()` etc. — an invalid
            // capture target in a match `case`. CPython reports
            // "cannot use {kind} as pattern target".
            _ if let Some(msg) = match_pattern_target_message(source_text, error.location) => {
                let (loc, end_loc) = adjusted_locations(&source_code, error.location);
                (ParseErrorType::OtherError(msg), loc, end_loc)
            }

            _ => {
                let (loc, end_loc) = adjusted_locations(&source_code, error.location);
                (error.error, loc, end_loc)
            }
        };

        Self::Parse(ParseError {
            error: error_type,
            raw_location: error.location,
            location,
            end_location,
            source_path: source_file.name().to_owned(),
            is_unclosed_bracket,
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

/// Compute the `(start, end)` `SourceLocation` for `range`, adjusting an
/// end that lands at column 1 of the following line back to the end of the
/// previous line. Used to keep the caret on the offending token rather than
/// wrapping to the next line.
fn adjusted_locations(
    source_code: &ruff_source_file::SourceCode<'_, '_>,
    range: ruff_text_size::TextRange,
) -> (SourceLocation, SourceLocation) {
    let loc = source_code.source_location(range.start(), PositionEncoding::Utf8);
    let mut end_loc = source_code.source_location(range.end(), PositionEncoding::Utf8);
    if end_loc.character_offset.get() == 1 && end_loc.line > loc.line {
        let prev_line_end = range.end() - ruff_text_size::TextSize::from(1);
        end_loc = source_code.source_location(prev_line_end, PositionEncoding::Utf8);
        end_loc.character_offset = end_loc.character_offset.saturating_add(1);
    }
    (loc, end_loc)
}

/// Convert ruff's wording (`'else' clause`, `'class' definition`, …) into
/// CPython's wording for indented-block errors.
fn normalize_indented_block_message(s: &str) -> String {
    let mut msg = s.replace('`', "'");
    // Clause keywords that CPython calls "statement" instead of "clause"/"block".
    for kw in &["else", "elif", "except", "finally"] {
        msg = msg.replace(&format!("'{kw}' clause"), &format!("'{kw}' statement"));
    }
    // ruff says "`case` block"; CPython says "'case' statement".
    msg = msg.replace("'case' block", "'case' statement");
    // CPython prints "class definition" (without quotes), not "'class' definition".
    msg = msg.replace("'class' definition", "class definition");
    msg
}

/// Detect whether the statement containing `lineno` uses `except*` rather
/// than `except`.
fn source_uses_except_star(source: &str, lineno: usize) -> bool {
    let line = source
        .split('\n')
        .nth(lineno.saturating_sub(1))
        .unwrap_or("");
    line.trim_start().starts_with("except*")
}

/// Find the line number of the statement keyword referenced by an
/// "Expected an indented block after `X` <kind>" message.
fn find_indented_block_keyword_line(
    source: &str,
    range: ruff_text_size::TextRange,
    msg: &str,
) -> Option<usize> {
    // Extract the keyword (the token between the backticks); when the
    // message has no backticks (e.g. "after function definition"), look up
    // the matching source keyword.
    let keyword_owned;
    let keyword: &str = if let Some(kw_start) = msg.find('`') {
        let kw_start = kw_start + 1;
        let kw_end = kw_start + msg[kw_start..].find('`')?;
        &msg[kw_start..kw_end]
    } else if msg.contains("function definition") {
        keyword_owned = "def".to_string();
        &keyword_owned
    } else if msg.contains("class definition") {
        keyword_owned = "class".to_string();
        &keyword_owned
    } else {
        return None;
    };
    let start: usize = range.start().into();
    // Search backward in the source for `<keyword>` (whole word, possibly
    // followed by whitespace, `:` or `(`).
    let prefix = &source[..start];
    let needle = keyword.to_string();
    let mut search_from = prefix.len();
    while let Some(idx) = prefix[..search_from].rfind(&needle) {
        // Ensure it is a whole-word match.
        let before_ok = idx == 0
            || prefix
                .as_bytes()
                .get(idx.wrapping_sub(1))
                .is_none_or(|b| !(b.is_ascii_alphanumeric() || *b == b'_'));
        let after_ok = prefix
            .as_bytes()
            .get(idx + needle.len())
            .is_none_or(|b| !(b.is_ascii_alphanumeric() || *b == b'_'));
        if before_ok && after_ok {
            // Count newlines before this index (1-indexed line number).
            let lineno = source[..idx].chars().filter(|c| *c == '\n').count() + 1;
            return Some(lineno);
        }
        if idx == 0 {
            break;
        }
        search_from = idx;
    }
    None
}

/// Detect a missing default value (`def f(a=, ...)`) or argument value
/// (`f(a=)`). Returns the matching CPython message if applicable.
fn missing_default_or_argument_value(
    source: &str,
    range: ruff_text_size::TextRange,
) -> Option<String> {
    let start: usize = range.start().into();
    let first = source.get(start..)?.chars().next()?;
    if !matches!(first, ',' | ')' | ':') {
        return None;
    }
    // Look backward, skipping whitespace, for `=` (and ensure it's not `==`).
    let prefix_bytes = &source.as_bytes()[..start];
    let mut idx = prefix_bytes.len();
    while idx > 0 && prefix_bytes[idx - 1].is_ascii_whitespace() {
        idx -= 1;
    }
    if idx == 0 || prefix_bytes[idx - 1] != b'=' {
        return None;
    }
    // The character immediately before `=` must not be another `=` (would be
    // a `==` token).
    if idx >= 2 && prefix_bytes[idx - 2] == b'=' {
        return None;
    }
    // Distinguish: in a function call `f(...)`, the closest unclosed `(` is
    // preceded by an identifier (the callee). In a def/lambda parameter list,
    // the `(` is preceded by `def IDENT` or the `lambda` keyword (no leading
    // `(` for lambda).
    let prefix = &source[..start];
    // Find the relevant opening token by tracking depth.
    let mut depth = 0i32;
    let mut open_idx = None;
    for (i, c) in prefix.char_indices().rev() {
        match c {
            ')' | ']' | '}' => depth += 1,
            '(' | '[' | '{' => {
                if depth == 0 {
                    open_idx = Some((i, c));
                    break;
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    let in_def_or_lambda = match open_idx {
        Some((i, '(')) => {
            let before = prefix[..i].trim_end();
            // `lambda` (no parens) or `def IDENT(...`.
            if before.ends_with("lambda") {
                true
            } else {
                let last_word = before
                    .rsplit_once(|c: char| c.is_whitespace())
                    .map_or(before, |(_, w)| w);
                let before_word = before.trim_end_matches(last_word).trim_end();
                before_word.ends_with("def") || before_word.ends_with("async def")
            }
        }
        // `lambda x=, y: ...` has no surrounding `(`.
        None => prefix.contains("lambda"),
        _ => false,
    };
    // `lambda x=: x` — an empty default immediately before the lambda body `:`
    // is plain "invalid syntax" in CPython (only a `,`-terminated empty default,
    // `lambda x=, y: …`, gets the "expected default value expression" hint).
    if first == ':' {
        return None;
    }
    Some(if in_def_or_lambda {
        "expected default value expression".to_owned()
    } else {
        "expected argument value expression".to_owned()
    })
}

/// Detect string literals with incompatible prefixes (e.g. `ub''`, `bf""`,
/// `tfu"…"`). CPython rejects these with "'X' and 'Y' prefixes are
/// incompatible". Returns `None` if there is no string literal at `range`.
fn incompatible_string_prefix_message(
    source: &str,
    range: ruff_text_size::TextRange,
) -> Option<String> {
    let start: usize = range.start().into();
    let first = source.get(start..)?.chars().next()?;
    if first != '\'' && first != '"' {
        return None;
    }
    // Walk back collecting `[a-zA-Z]+` characters as the prefix.
    let prefix_bytes = source.as_bytes()[..start]
        .iter()
        .rev()
        .take_while(|b| b.is_ascii_alphabetic())
        .copied()
        .collect::<Vec<u8>>();
    if prefix_bytes.is_empty() {
        return None;
    }
    // Reverse back to get source order.
    let prefix: Vec<char> = prefix_bytes
        .into_iter()
        .rev()
        .map(|b| (b as char).to_ascii_lowercase())
        .collect();
    // Single prefix is fine.
    if prefix.len() < 2 {
        return None;
    }
    // A real string prefix uses only {b,r,f,u,t}; otherwise we are looking at
    // a bare identifier (e.g. `data`, `about`) juxtaposed with a string. (No
    // length cap: CPython still diagnoses long invalid runs like `turf"…"`.)
    if !prefix
        .iter()
        .all(|c| matches!(c, 'b' | 'r' | 'f' | 'u' | 't'))
    {
        return None;
    }
    // The character preceding the prefix must not be a name continuation —
    // otherwise we are looking at an identifier ending in prefix letters glued
    // to a string. Inspect the full char (not a single byte) so multibyte
    // identifier chars are handled too; `is_alphanumeric` mirrors Python rules.
    let pre_idx = start - prefix.len();
    if source[..pre_idx]
        .chars()
        .next_back()
        .is_some_and(|pc| pc.is_alphanumeric() || pc == '_')
    {
        return None;
    }

    // Match CPython's algorithm in `Parser/lexer/lexer.c` —
    // `maybe_raise_syntax_error_for_string_prefixes`. The checks fire in this
    // fixed order; the first matching pair is the message.
    let saw_u = prefix.contains(&'u');
    let saw_b = prefix.contains(&'b');
    let saw_r = prefix.contains(&'r');
    let saw_f = prefix.contains(&'f');
    let saw_t = prefix.contains(&'t');
    let pair = if saw_u && saw_b {
        Some(("u", "b"))
    } else if saw_u && saw_r {
        Some(("u", "r"))
    } else if saw_u && saw_f {
        Some(("u", "f"))
    } else if saw_u && saw_t {
        Some(("u", "t"))
    } else if saw_b && saw_f {
        Some(("b", "f"))
    } else if saw_b && saw_t {
        Some(("b", "t"))
    } else if saw_f && saw_t {
        Some(("f", "t"))
    } else {
        // CPython treats `rr"…"` / `bb"…"` as a name-then-string error
        // ("invalid syntax"), so fall through rather than fabricate a message.
        None
    };
    pair.map(|(a, b)| format!("'{a}' and '{b}' prefixes are incompatible"))
}

/// Detect parenthesized parameters in a `def`/`lambda` parameter list
/// (e.g. `def f(x, (y, z), w)` or `lambda (x, y): None`).
fn parenthesized_param_message(source: &str, range: ruff_text_size::TextRange) -> Option<String> {
    let start: usize = range.start().into();
    // Only fires when the error points at `(`.
    if source.get(start..)?.chars().next()? != '(' {
        return None;
    }
    let prefix = &source[..start];

    // Lambda parameter context: the nearest preceding `lambda` (possibly on an
    // earlier line) with no `:` body-separator before the error → in its params.
    if let Some(idx) = prefix.rfind("lambda") {
        let after_lambda = &prefix[idx + "lambda".len()..];
        if !after_lambda.contains(':') {
            return Some("Lambda expression parameters cannot be parenthesized".to_owned());
        }
    }
    // Function parameter context: the nearest preceding `def `; if the error
    // `(` is still inside its parameter list (paren depth never returns to 0,
    // possibly across multiple lines) → a parenthesized parameter.
    if let Some(def_idx) = prefix.rfind("def ")
        && let Some(open) = prefix[def_idx..].find('(')
    {
        let inner = &prefix[def_idx + open + 1..];
        let mut depth = 1i32;
        for c in inner.chars() {
            match c {
                '(' => depth += 1,
                ')' => depth -= 1,
                _ => {}
            }
            if depth == 0 {
                return None;
            }
        }
        if depth >= 1 {
            return Some("Function parameters cannot be parenthesized".to_owned());
        }
    }
    None
}

/// Detect whether we are inside a subscript `[...]` whose current "argument"
/// starts with `*` (i.e. CPython's "Invalid star expression").
fn is_invalid_star_in_subscript(source: &str, range: ruff_text_size::TextRange) -> bool {
    if !is_inside_subscript(source, range) {
        return false;
    }
    let start: usize = range.start().into();
    let prefix = &source[..start];
    // Walk back to the start of the current subscript "slot" (i.e. the most
    // recent enclosing `[` or `,` at depth 0).
    let mut depth = 0i32;
    for (i, c) in prefix.char_indices().rev() {
        match c {
            ')' | ']' | '}' => depth += 1,
            '(' | '{' => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            '[' if depth == 0 => {
                let slot = prefix[i + 1..].trim_start();
                return slot.starts_with('*');
            }
            '[' => depth -= 1,
            ',' if depth == 0 => {
                let slot = prefix[i + 1..].trim_start();
                return slot.starts_with('*');
            }
            _ => {}
        }
    }
    false
}

/// Detect `def f(.../*,...): ...` or `lambda .../*,...: None`: a `*` token
/// that follows `/` with no separating comma.
fn is_slash_star_in_params(source: &str, range: ruff_text_size::TextRange) -> bool {
    let start: usize = range.start().into();
    let prefix = &source[..start];
    // The token immediately before the `*` should be `/`.
    if !prefix.trim_end().ends_with('/') {
        return false;
    }
    // We must be inside a def/lambda parameter list.
    let line_start = prefix.rfind('\n').map_or(0, |i| i + 1);
    let line = &prefix[line_start..];
    line.contains("def ") || line.contains("lambda")
}

/// Detect `f(**kwargs=...)`: an `=` token where ruff expected a comma, with
/// `**IDENT` immediately preceding.
fn is_kwarg_unpacking_assignment(source: &str, range: ruff_text_size::TextRange) -> bool {
    let start: usize = range.start().into();
    let prefix = &source[..start];
    // Strip a trailing identifier and look for `**`.
    let trimmed = prefix.trim_end_matches(|c: char| c.is_ascii_alphanumeric() || c == '_');
    trimmed.trim_end().ends_with("**")
}

/// Detect `[a, b for a, b in ...]` / `{a, b for a, b in ...}` — the comma
/// before `for` makes ruff stop at `Rsqb`/`Rbrace`. CPython's hint is
/// "did you forget parentheses around the comprehension target?".
fn is_unparen_comprehension_target(source: &str, range: ruff_text_size::TextRange) -> bool {
    let start: usize = range.start().into();
    let prefix = &source[..start];
    // Look back to the enclosing `[` / `{` and check there is a `,` at
    // depth 0 between it and the error position.
    let mut depth = 0i32;
    let mut open_idx = None;
    for (i, c) in prefix.char_indices().rev() {
        match c {
            ')' | ']' | '}' => depth += 1,
            '(' | '[' | '{' => {
                if depth == 0 {
                    open_idx = Some(i);
                    break;
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    let Some(open_idx) = open_idx else {
        return false;
    };
    if !matches!(prefix.as_bytes()[open_idx], b'[' | b'{') {
        return false;
    }
    let inside = &prefix[open_idx + 1..];
    let mut d = 0i32;
    for c in inside.chars() {
        match c {
            '(' | '[' | '{' => d += 1,
            ')' | ']' | '}' => d -= 1,
            ',' if d == 0 => return true,
            _ => {}
        }
    }
    false
}

/// Detect that the current logical line begins with `raise `.
fn source_starts_with_raise(source: &str, range: ruff_text_size::TextRange) -> bool {
    let start: usize = range.start().into();
    let prefix = &source[..start];
    let line_start = prefix.rfind('\n').map_or(0, |i| i + 1);
    prefix[line_start..].trim_start().starts_with("raise ")
}

/// Detect a bare `*` (followed by `)`, `:`, or another argument) in a
/// function-call argument list.
fn is_bare_star_in_call(source: &str, range: ruff_text_size::TextRange) -> bool {
    let start: usize = range.start().into();
    let prefix = &source[..start];
    // Closest unclosed `(` should be preceded by an identifier (callee).
    let mut depth = 0i32;
    let mut open_idx = None;
    for (i, c) in prefix.char_indices().rev() {
        match c {
            ')' | ']' | '}' => depth += 1,
            '(' => {
                if depth == 0 {
                    open_idx = Some(i);
                    break;
                }
                depth -= 1;
            }
            '[' | '{' => {
                if depth == 0 {
                    return false;
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    let Some(open_idx) = open_idx else {
        return false;
    };
    let before_paren = prefix[..open_idx]
        .chars()
        .rev()
        .find(|c| !c.is_whitespace());
    if !matches!(before_paren, Some(c) if c.is_ascii_alphanumeric() || c == '_' || c == ')' || c == ']')
    {
        return false;
    }
    // The token immediately before the error must be `*`.
    prefix.trim_end().ends_with('*')
}

/// Detect a bare single `*` as the leading element of a set/dict display
/// `{ ... }` or a non-call parenthesised group/tuple `( ... )` — CPython's
/// "Invalid star expression" (`{*}`, `{*, 1}`, `(*)`, `(*,)`, `(*, 1)`). A
/// non-leading star (`{1, *}`, `(1, *)`), a dict value (`{1: *}`), a double
/// star (`{**}`), a subscript/list (`[*]`, handled elsewhere), or a call
/// (`f(*)`, handled elsewhere) is intentionally excluded.
fn is_bare_star_first_in_group(source: &str, range: ruff_text_size::TextRange) -> bool {
    let start: usize = range.start().into();
    let prefix = &source[..start];
    let trimmed = prefix.trim_end();
    // The token immediately before the error must be a single `*` (not `**`).
    if !trimmed.ends_with('*') || trimmed.ends_with("**") {
        return false;
    }
    // Walk back to the nearest depth-0 opener. Bail at a depth-0 `,` (the star
    // is not the leading element), `:` (a dict value), or `[` (subscript/list,
    // handled by `is_invalid_star_in_subscript`).
    let mut depth = 0i32;
    for (i, c) in prefix.char_indices().rev() {
        match c {
            ')' | ']' | '}' => depth += 1,
            ',' | ':' if depth == 0 => return false,
            '[' if depth == 0 => return false,
            '{' if depth == 0 => return true,
            '(' if depth == 0 => {
                // Only a non-call group: the token before `(` must not be a
                // callee (identifier / `)` / `]`).
                let before = prefix[..i].chars().rev().find(|c| !c.is_whitespace());
                return !matches!(
                    before,
                    Some(c) if c.is_ascii_alphanumeric() || c == '_' || c == ')' || c == ']'
                );
            }
            '(' | '{' | '[' => depth -= 1,
            _ => {}
        }
    }
    false
}

/// Detect bad target in `except[*] T as <bad>:`. Returns the matching CPython
/// message (e.g. "cannot use except statement with attribute") or `None`.
fn except_as_bad_target_message(source: &str, range: ruff_text_size::TextRange) -> Option<String> {
    let start: usize = range.start().into();
    let prefix = &source[..start];
    // Find the most recent `except` / `except*` keyword on the logical line.
    let line_start = prefix.rfind('\n').map_or(0, |i| i + 1);
    let logical = &prefix[line_start..];
    let kw = if logical.contains("except*") {
        "except*"
    } else if logical.contains("except ") || logical.trim_start() == "except" {
        "except"
    } else {
        return None;
    };
    // Require an `as ` before the error position.
    let as_idx = prefix.rfind(" as ")?;
    // Pull out everything between `as` and the error.
    let after_as = prefix[as_idx + 4..].trim();
    // The first character at error.location signals the kind of bad target.
    let at_error = source.get(start..)?.chars().next()?;
    let kind = if !after_as.is_empty() {
        // ruff parsed `as IDENT`, then saw `.` / `[` (attribute / subscript).
        match at_error {
            '.' => "attribute",
            '[' => "subscript",
            _ => return None,
        }
    } else {
        // ruff didn't even reach a name — bad target was `(`, `[`, literal, …
        match at_error {
            '(' => "tuple",
            '[' => "list",
            '0'..='9' | '\'' | '"' => "literal",
            _ => return None,
        }
    };
    Some(format!("cannot use {kw} statement with {kind}"))
}

/// Detect `[x for x if ...]` / `(x for x if ...)` / `{... if ...}` — i.e.
/// a comprehension where `if` was used in place of `in` after the for-target.
fn is_in_comprehension_if(source: &str, range: ruff_text_size::TextRange) -> bool {
    let start: usize = range.start().into();
    let prefix = &source[..start];
    // Walk back to find the *outermost* unclosed bracket that contains the
    // error position. Any of `(`, `[`, `{` qualifies for a comprehension.
    let mut depth = 0i32;
    let mut open_idx = None;
    for (i, c) in prefix.char_indices().rev() {
        match c {
            ')' | ']' | '}' => depth += 1,
            '(' | '[' | '{' => {
                if depth == 0 {
                    open_idx = Some(i);
                    break;
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    let Some(open_idx) = open_idx else {
        return false;
    };
    let inside = &prefix[open_idx + 1..];
    inside.contains(" for ") && inside.matches(" for ").count() == inside.matches(" if ").count()
}

/// Detect a "tight" parenthesised starred expression (`(*x)`) used inside
/// subscript brackets, e.g. `A[:(*b)]` or `A[(*b):]`.
/// More complex constructs like `A[(*b:*b)]` are intentionally NOT matched —
/// they keep CPython's "invalid syntax" message.
fn is_paren_starred_in_subscript(source: &str, range: ruff_text_size::TextRange) -> bool {
    if !is_inside_subscript(source, range) {
        return false;
    }
    let start: usize = range.start().into();
    // The character immediately before `*` should be `(`.
    if !source[..start].trim_end().ends_with('(') {
        return false;
    }
    // The character immediately after the starred name should be `)`,
    // ensuring the parens tightly wrap `*X`.
    let end: usize = range.end().into();
    let rest = source.get(end..).unwrap_or("");
    rest.trim_start().starts_with(')')
}

/// Detect whether `range` sits inside subscript brackets `[ ... ]` at any
/// level of enclosing nesting.
fn is_inside_subscript(source: &str, range: ruff_text_size::TextRange) -> bool {
    let start: usize = range.start().into();
    let prefix = &source[..start];
    let mut depth = 0i32;
    for c in prefix.chars().rev() {
        match c {
            ')' | ']' | '}' => depth += 1,
            '(' | '{' => {
                if depth > 0 {
                    depth -= 1;
                }
                // depth == 0 ⇒ unmatched opening of a *different* kind; keep
                // walking to see if `[` further out encloses us.
            }
            '[' => {
                if depth == 0 {
                    return true;
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    false
}

/// Detect whether `range` points at a position that follows a `:` inside a
/// `{ ... }` dict literal (i.e. a missing or starred dict value).
fn is_dict_value_position(source: &str, range: ruff_text_size::TextRange) -> bool {
    let start: usize = range.start().into();
    let prefix = &source[..start];
    // Find the enclosing `{`.
    let mut depth = 0i32;
    let mut found_brace = false;
    for c in prefix.chars().rev() {
        match c {
            '}' | ')' | ']' => depth += 1,
            '{' => {
                if depth == 0 {
                    found_brace = true;
                    break;
                }
                depth -= 1;
            }
            '(' | '[' => {
                if depth == 0 {
                    return false;
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    if !found_brace {
        return false;
    }
    // The most recent `:` (at depth 0 inside the `{ ... }`) must appear
    // before any `,` at the same depth, meaning we are in a `key:_value`.
    let mut d = 0i32;
    let mut last_relevant = None;
    for c in prefix.chars().rev() {
        match c {
            ')' | ']' | '}' => d += 1,
            '(' | '[' => d -= 1,
            '{' if d == 0 => break,
            '{' => d -= 1,
            ',' | ':' if d == 0 => {
                last_relevant = Some(c);
                break;
            }
            _ => {}
        }
    }
    matches!(last_relevant, Some(':'))
}

/// Detect `X=Y` in tuple/list/set literals (i.e. not a function call).
/// CPython suggests "invalid syntax. Maybe you meant '==' or ':=' instead of
/// '='?".
fn collection_kwarg_message(source: &str, range: ruff_text_size::TextRange) -> Option<String> {
    let start: usize = range.start().into();
    let end: usize = range.end().into();
    // Find the surrounding `(`, `[`, or `{`.
    let prefix = &source[..start];
    let mut depth = 0i32;
    let mut opening = None;
    for (i, c) in prefix.char_indices().rev() {
        match c {
            ')' | ']' | '}' => depth += 1,
            '(' | '[' | '{' => {
                if depth == 0 {
                    opening = Some((i, c));
                    break;
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    let (open_idx, open_char) = opening?;
    // Excluding function-call parens: the char before `(` is a name/`)`/`]`.
    if open_char == '(' {
        let prev = prefix[..open_idx]
            .chars()
            .rev()
            .find(|c| !c.is_whitespace());
        if let Some(prev) = prev
            && (prev.is_ascii_alphanumeric() || prev == '_' || prev == ')' || prev == ']')
        {
            return None;
        }
    }
    // Look for a `IDENT = VALUE` chunk inside the display. The `=` must be a
    // bare assignment, NOT part of `:=` / `==` / `!=` / `<=` / `>=` — otherwise
    // a parenthesised walrus like `(b := 2)` would be misread as a kwarg.
    let seg = &source[open_idx + 1..end.min(source.len())];
    seg.split(',')
        .find_map(|chunk| bare_assignment_message(chunk.trim()))
}

/// If `chunk` is a bare `LHS = …` (a single `=`, not `:=`/`==`/`!=`/`<=`/`>=`)
/// inside a tuple/list/set display, return CPython's message for that misplaced
/// `=`, classified by the LHS kind; otherwise `None`.
fn bare_assignment_message(chunk: &str) -> Option<String> {
    let bytes = chunk.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b != b'=' {
            continue;
        }
        if matches!(
            i.checked_sub(1).map(|p| bytes[p]),
            Some(b':' | b'=' | b'!' | b'<' | b'>')
        ) {
            continue;
        }
        if bytes.get(i + 1) == Some(&b'=') {
            continue;
        }
        let lhs = chunk[..i].trim();
        let mut lhs_chars = lhs.chars();
        let msg = match lhs_chars.next() {
            // A real identifier (Unicode, e.g. `α`): the `==`/`:=` hint — unless
            // it's a keyword literal, which CPython reports as plain "invalid
            // syntax".
            Some(first)
                if (first.is_alphabetic() || first == '_')
                    && lhs_chars.all(|c| c.is_alphanumeric() || c == '_') =>
            {
                if matches!(lhs, "True" | "False" | "None") {
                    Some("invalid syntax".to_owned())
                } else {
                    Some("invalid syntax. Maybe you meant '==' or ':=' instead of '='?".to_owned())
                }
            }
            // A number/string literal target → "cannot assign to literal …".
            Some(first) if first.is_ascii_digit() || first == '"' || first == '\'' => Some(
                "cannot assign to literal here. Maybe you meant '==' instead of '='?".to_owned(),
            ),
            _ => None,
        };
        if msg.is_some() {
            return msg;
        }
    }
    None
}

/// Detect whether `range` covers the LHS expression of a bad
/// `f(<expr>=<value>)` keyword argument.
fn is_call_keyword_assignment(source: &str, range: ruff_text_size::TextRange) -> bool {
    let start: usize = range.start().into();
    let end: usize = range.end().into();
    // The error must sit inside a call: there must be an unclosed `(` before it.
    let prefix = &source[..start];
    let depth = prefix.chars().fold(0i32, |d, c| match c {
        '(' => d + 1,
        ')' => d - 1,
        _ => d,
    });
    if depth <= 0 {
        return false;
    }
    // The character immediately after the bad LHS (skipping whitespace) must
    // be `=` (but not `==` — that's a comparison, not an assignment).
    let mut chars = source[end..].chars();
    while let Some(c) = chars.clone().next() {
        if c.is_whitespace() {
            chars.next();
        } else {
            break;
        }
    }
    let next = chars.next();
    let next2 = chars.next();
    matches!(next, Some('=')) && !matches!(next2, Some('='))
}

/// Pull out the LHS expression of a bad keyword-argument assignment using
/// the error range as the LHS span.
fn call_keyword_lhs(source: &str, range: ruff_text_size::TextRange) -> &str {
    let start: usize = range.start().into();
    let end: usize = range.end().into();
    source[start..end].trim()
}

/// If `range` covers a statement keyword (`pass`, `break`, `continue`, etc.)
/// that appears in expression position before or after `if`/`else` in a
/// ternary expression, return the matching CPython error message.
fn ternary_statement_keyword_message(
    source: &str,
    range: ruff_text_size::TextRange,
) -> Option<String> {
    const STMT_KEYWORDS: &[&str] = &[
        "pass", "break", "continue", "return", "raise", "import", "from", "global", "nonlocal",
        "del", "assert", "with", "try", "while", "for", "if", "elif", "else", "class", "def",
        "yield",
    ];
    let start: usize = range.start().into();
    let end: usize = range.end().into();
    let slice = source[start..end].trim();
    // Accept either an exact match (`pass`) or a keyword + arguments
    // (`return 2`, `yield 2`, `raise Exception('a')`, `import ast`).
    let first_word = slice
        .split(|c: char| c.is_whitespace() || c == '(')
        .next()
        .unwrap_or("");
    if !STMT_KEYWORDS.contains(&first_word) {
        return None;
    }
    // Look backward to find `if ` or `else ` on the same logical statement.
    let before = &source[..start];
    let trailing = before.trim_end();
    if trailing.ends_with(" else") || trailing.ends_with("else") {
        // After `else` in a ternary: `<expr> if <cond> else pass`.
        // CPython: "expected expression after 'else', but statement is given".
        return Some("expected expression after 'else', but statement is given".to_owned());
    }
    // Look forward to see whether `if` follows the keyword on the same line.
    let after = source[end..].split('\n').next().unwrap_or("").trim_start();
    if after.starts_with("if ") || after == "if" {
        // Before `if` in a ternary: `<expr> = pass if <cond> else <expr>`.
        // CPython only emits this hint for the bare simple statements
        // pass/break/continue; other keywords (`return if …`, `import if …`)
        // fail earlier and are plain "invalid syntax".
        if matches!(first_word, "pass" | "break" | "continue") {
            return Some("expected expression before 'if', but statement is given".to_owned());
        }
    }
    None
}

/// Detect whether the `=` at `range` is the head of an `if X = Y:` /
/// `while X = Y:` / `elif X = Y:` clause that CPython points out with the
/// "Maybe you meant '==' or ':=' instead of '='?" message.
fn precedes_if_or_while(source: &str, range: ruff_text_size::TextRange) -> bool {
    let start: usize = range.start().into();
    let line_start = source[..start].rfind('\n').map_or(0, |i| i + 1);
    let line_before = &source[line_start..start];
    let trimmed = line_before.trim_start();
    trimmed.starts_with("if ") || trimmed.starts_with("elif ") || trimmed.starts_with("while ")
}

/// Strip the leading `if `/`elif `/`while ` keyword from `prefix` and return
/// the candidate LHS expression up to (but not including) `=`.
fn lhs_after_keyword(prefix: &str) -> &str {
    let trimmed = prefix.trim_start();
    for kw in ["elif ", "while ", "if "] {
        if let Some(rest) = trimmed.strip_prefix(kw) {
            return rest.trim();
        }
    }
    trimmed.trim()
}

/// Detect the legacy `import X from Y` form where the user likely meant
/// `from Y import X`.
fn is_old_import_from(source: &str, range: ruff_text_size::TextRange) -> bool {
    let start: usize = range.start().into();
    let line_start = source[..start].rfind('\n').map_or(0, |i| i + 1);
    let line_before = &source[line_start..start];
    let trimmed = line_before.trim_start();
    if !trimmed.starts_with("import ") {
        return false;
    }
    // The error must be on or before the `from` keyword in the rest of the line.
    let after = source[start..]
        .split('\n')
        .next()
        .unwrap_or("")
        .trim_start();
    after.starts_with("from ") || source[start..].starts_with("from")
}

/// Detect whether `range` points to a `.`, `[`, or `(` that continues an
/// `import X as Y` / `from X import Y as Z` clause. CPython rejects these
/// with "cannot use {attribute,subscript,function call} as import target".
fn is_import_target_continuation(source: &str, range: ruff_text_size::TextRange) -> bool {
    let start: usize = range.start().into();
    let Some(first) = source[start..].chars().next() else {
        return false;
    };
    if !matches!(first, '.' | '[' | '(') {
        return false;
    }
    // Walk back to the start of the current physical line.
    let line_start = source[..start].rfind('\n').map_or(0, |i| i + 1);
    let prefix = &source[line_start..start];

    // For a multi-line `from a import (\n  b as f()\n)` form the `from` /
    // `import` keyword lives on a previous physical line. Allow that case too
    // by also searching the whole text before the error.
    let in_import_line = prefix.contains("import ")
        || prefix.starts_with("import")
        || prefix.trim_start().starts_with("import");
    let in_from_block = source[..start]
        .rfind("from ")
        .is_some_and(|idx| source[idx..start].contains("import"));
    if !(in_import_line || in_from_block) {
        return false;
    }

    // Require an `as <ident>` sequence immediately before the error.
    let trimmed = prefix.trim_end();
    // strip the trailing identifier
    let after_ident = trimmed.trim_end_matches(|c: char| c.is_alphanumeric() || c == '_');
    after_ident.trim_end().ends_with(" as") || after_ident.trim_end().ends_with("\tas")
}

/// Return the next non-whitespace character after `range` in `source`, if any.
fn next_nonspace_after(source: &str, range: ruff_text_size::TextRange) -> Option<char> {
    let end: usize = range.end().into();
    source[end..]
        .chars()
        .find(|c| !c.is_whitespace() && *c != '\\')
}

/// Whether the logical line containing `range` is a `for` / `async for` header
/// (so an invalid target there should read "invalid syntax", not "cannot
/// assign to …").
fn is_for_loop_target(source: &str, range: ruff_text_size::TextRange) -> bool {
    let start: usize = range.start().into();
    let line_start = source[..start].rfind('\n').map_or(0, |i| i + 1);
    let line = source[line_start..start].trim_start();
    line.starts_with("for ") || line.starts_with("async for ")
}

/// Whether the word immediately following `range` (skipping `*`/whitespace) is
/// a Python keyword — used to tell `def f(*None)` (→ "invalid syntax") from a
/// genuine bare `*` (→ "named arguments must follow bare *").
fn next_word_is_keyword(source: &str, range: ruff_text_size::TextRange) -> bool {
    let end: usize = range.end().into();
    let rest = source[end..].trim_start_matches(|c: char| c.is_whitespace() || c == '*');
    let word: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    matches!(
        word.as_str(),
        "None"
            | "True"
            | "False"
            | "and"
            | "or"
            | "not"
            | "if"
            | "else"
            | "lambda"
            | "yield"
            | "await"
            | "import"
            | "from"
            | "class"
            | "def"
            | "return"
            | "pass"
            | "in"
            | "is"
    )
}

/// Whether `s` is an augmented-assignment operator (`+=`, `**=`, `<<=`, …) as
/// opposed to a comparison/assignment (`==`, `!=`, `<=`, `>=`, `=`).
fn is_augassign_op(s: &str) -> bool {
    s.len() >= 2 && s.ends_with('=') && !matches!(s, "==" | "!=" | "<=" | ">=" | ":=")
}

/// Detect `from a import b, c as d[e]` — a subscript (`[`) right after an
/// `as NAME` inside an `import` statement.
fn is_import_subscript_target(source: &str, range: ruff_text_size::TextRange) -> bool {
    let start: usize = range.start().into();
    if !source[start..].trim_start().starts_with('[') {
        return false;
    }
    let line_start = source[..start].rfind('\n').map_or(0, |i| i + 1);
    let line = &source[line_start..start];
    if !(line.trim_start().starts_with("import ") || line.contains(" import ")) {
        return false;
    }
    // The token before `[` must be `as <ident>`.
    let before = line.trim_end();
    let after_ident = before.trim_end_matches(|c: char| c.is_alphanumeric() || c == '_');
    after_ident.trim_end().ends_with(" as")
}

/// Detect the `STRING WORD STRING` shape of an unintended split string, e.g.
/// `"a "b" c"` — a closing quote immediately before `range` AND another quote
/// later on the line. (A plain `STRING WORD` adjacency stays "invalid syntax".)
fn looks_like_split_string(source: &str, range: ruff_text_size::TextRange) -> bool {
    let start: usize = range.start().into();
    let preceded = matches!(
        source[..start].trim_end().chars().next_back(),
        Some('"' | '\'')
    );
    if !preceded {
        return false;
    }
    let end: usize = range.end().into();
    let rest_of_line = source[end..].split('\n').next().unwrap_or("");
    rest_of_line.contains('"') || rest_of_line.contains('\'')
}

/// Whether `range` sits inside a match `case <pattern>` header. Used to keep
/// parse errors there as plain "invalid syntax" instead of borrowing
/// expression-context messages ("forgot a comma", "':' expected after
/// dictionary key"). Requires a `case ` line (soft keyword, not `case = …`)
/// with an enclosing lower-indent `match ` block, so module-level `case (a b)`
/// (a call) is left untouched.
fn is_in_case_pattern(source: &str, range: ruff_text_size::TextRange) -> bool {
    let start: usize = range.start().into();
    let prefix = &source[..start];
    // Find the nearest `case ` keyword starting a current/preceding line (the
    // pattern may span multiple lines inside an unclosed bracket).
    let mut search = prefix.len();
    loop {
        let line_start = prefix[..search].rfind('\n').map_or(0, |i| i + 1);
        let line = &prefix[line_start..];
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("case ") {
            // `case = …` is an assignment to a var named `case`, not a pattern.
            if rest.trim_start().starts_with('=') {
                return false;
            }
            // The error must still be in the pattern: no top-level `:` (which
            // ends the header) between this `case` and the error. Otherwise the
            // error is in the case BODY (a normal expression context).
            let case_kw = line_start + (line.len() - trimmed.len());
            let mut depth = 0i32;
            for c in source[case_kw..start].chars() {
                match c {
                    '(' | '[' | '{' => depth += 1,
                    ')' | ']' | '}' => depth -= 1,
                    ':' if depth == 0 => return false,
                    _ => {}
                }
            }
            // Require an enclosing lower-indent `match ` block (so module-level
            // `case (a b)` — a call — is left untouched).
            let case_indent = line.chars().take_while(|c| c.is_whitespace()).count();
            return source[..line_start].split('\n').any(|prev| {
                let indent = prev.chars().take_while(|c| c.is_whitespace()).count();
                indent < case_indent && prev.trim_start().starts_with("match ")
            });
        }
        if line_start == 0 {
            return false;
        }
        search = line_start - 1;
    }
}

/// Detect an invalid capture target in a match `case … as <target>` (e.g.
/// `case 42 as a.b`, `as (a, b)`, `as a()`), returning CPython's
/// "cannot use {kind} as pattern target". Only fires when the text after the
/// last ` as ` parses as a non-`Name` expression, so a valid `case P as name:`
/// (a plain identifier) is left untouched.
fn match_pattern_target_message(source: &str, range: ruff_text_size::TextRange) -> Option<String> {
    let start: usize = range.start().into();
    let prefix = &source[..start];
    // Require a `case … as` header: the last ` as ` must be preceded by a
    // `case ` with no intervening `:` (which would end the case header).
    let as_pos = prefix.rfind(" as ")?;
    let case_pos = prefix[..as_pos].rfind("case ")?;
    if prefix[case_pos..as_pos].contains(':') {
        return None;
    }
    // Extract the target text after ` as `, up to the case `:` / `|` / `,` at
    // bracket-depth 0, a bracket that closes an enclosing group, or newline.
    let after = &source[as_pos + 4..];
    let mut depth = 0i32;
    let mut end = after.len();
    for (i, c) in after.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                if depth == 0 {
                    end = i;
                    break;
                }
                depth -= 1;
            }
            ':' | '|' | ',' if depth == 0 => {
                end = i;
                break;
            }
            '\n' => {
                end = i;
                break;
            }
            _ => {}
        }
    }
    let target = after[..end].trim();
    if target.is_empty() {
        return None;
    }
    // A plain name is a valid capture target; only non-`Name` exprs are errors.
    match *parser::parse_expression(target).ok()?.syntax().body {
        ast::Expr::Name(_) => None,
        ref e => Some(format!(
            "cannot use {} as pattern target",
            expr_kind_name(e)
        )),
    }
}

/// Map an [`ast::Expr`] to the human-readable name CPython uses in syntax
/// error messages (mirrors CPython's expression-kind names in
/// `Parser/action_helpers.c`).
fn expr_kind_name(expr: &ast::Expr) -> &'static str {
    match expr {
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
        ast::Expr::Compare(_) => "comparison",
        ast::Expr::If(_) => "conditional expression",
        ast::Expr::Attribute(_) => "attribute",
        ast::Expr::Named(_) => "named expression",
        ast::Expr::NoneLiteral(_) => "None",
        ast::Expr::EllipsisLiteral(_) => "ellipsis",
        ast::Expr::BooleanLiteral(ast::ExprBooleanLiteral { value, .. }) => {
            if *value {
                "True"
            } else {
                "False"
            }
        }
        ast::Expr::NumberLiteral(_) | ast::Expr::StringLiteral(_) | ast::Expr::BytesLiteral(_) => {
            "literal"
        }
        ast::Expr::Slice(_) => "slice",
        ast::Expr::IpyEscapeCommand(_) => "expression",
    }
}

/// CPython's message for `<target> = <value>` where `<target>` is an invalid
/// assignment target. Whether the helpful `here. Maybe you meant '==' instead
/// of '='?` suffix applies depends on both the kind of target and whether the
/// target is the whole LHS (followed by `=`) or a sub-element of a tuple/list
/// target (followed by `,` / `)` / `]` / etc.).
fn assign_target_message(expr: &ast::Expr, expr_str: &str, followed_by_eq: bool) -> String {
    let here_suffix = " here. Maybe you meant '==' instead of '='?";
    let with_suffix = |base: &str| -> String {
        if followed_by_eq {
            format!("{base}{here_suffix}")
        } else {
            base.to_owned()
        }
    };
    match expr {
        ast::Expr::Call(_) => with_suffix("cannot assign to function call"),
        ast::Expr::BinOp(_) | ast::Expr::UnaryOp(_) => with_suffix("cannot assign to expression"),
        // CPython always names a bool-op target "expression" (no `=` gating).
        ast::Expr::BoolOp(_) => "cannot assign to expression".to_owned(),
        ast::Expr::Compare(_) if followed_by_eq => "cannot assign to comparison".to_owned(),
        ast::Expr::If(_) => "cannot assign to conditional expression".to_owned(),
        ast::Expr::Generator(_) => "cannot assign to generator expression".to_owned(),
        ast::Expr::Lambda(_) => "cannot assign to lambda".to_owned(),
        ast::Expr::Yield(_) | ast::Expr::YieldFrom(_) => {
            "assignment to yield expression not possible".to_owned()
        }
        ast::Expr::Await(_) => "cannot assign to await expression".to_owned(),
        ast::Expr::ListComp(_) => "cannot assign to list comprehension".to_owned(),
        ast::Expr::SetComp(_) => "cannot assign to set comprehension".to_owned(),
        ast::Expr::DictComp(_) => "cannot assign to dict comprehension".to_owned(),
        ast::Expr::Set(_) => with_suffix("cannot assign to set display"),
        ast::Expr::Dict(_) => with_suffix("cannot assign to dict literal"),
        ast::Expr::FString(_) => with_suffix("cannot assign to f-string expression"),
        ast::Expr::TString(_) => with_suffix("cannot assign to t-string expression"),
        ast::Expr::StringLiteral(_) | ast::Expr::BytesLiteral(_) | ast::Expr::NumberLiteral(_) => {
            with_suffix("cannot assign to literal")
        }
        ast::Expr::EllipsisLiteral(_) => with_suffix("cannot assign to ellipsis"),
        ast::Expr::NoneLiteral(_) => "cannot assign to None".to_owned(),
        ast::Expr::BooleanLiteral(ast::ExprBooleanLiteral { value, .. }) => {
            if *value {
                "cannot assign to True".to_owned()
            } else {
                "cannot assign to False".to_owned()
            }
        }
        ast::Expr::Attribute(_) => with_suffix("cannot assign to attribute"),
        _ => format!("cannot assign to {expr_str}"),
    }
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
    let parser_mode = match mode {
        Mode::Exec => parser::Mode::Module,
        Mode::Eval => parser::Mode::Expression,
        // ruff does not have an interactive mode, which is fine,
        // since these are only different in terms of compilation
        Mode::Single | Mode::BlockExpr => parser::Mode::Module,
    };
    let parsed = parser::parse(source_file.source_text(), parser_mode.into())
        .map_err(|err| CompileError::from_ruff_parse_error(err, &source_file))?;
    let ast = parsed.into_syntax();
    compile::compile_top(ast, source_file, mode, opts).map_err(|e| e.into())
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
            symboltable::SymbolTable::scan_program(&ast.into_syntax(), source_file.clone())
        }
        Mode::Eval => {
            let ast = ruff_python_parser::parse(
                source_file.source_text(),
                parser::Mode::Expression.into(),
            )
            .map_err(|e| CompileError::from_ruff_parse_error(e, &source_file))?;
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

//! The invalid standalone-case production. This is diagnostic-only speculation:
//! the caller always parses the original statement again for syntax and tokens.

use ruff_python_ast::token::TokenKind;
use ruff_python_ast::{Expr, Pattern};
use ruff_text_size::TextRange;

use super::Parser;
use super::expression::ExpressionContext;
use crate::{BlockClause, ParseError, ParseErrorType};

impl Parser<'_> {
    pub(super) fn standalone_case_diagnostic(&mut self) -> Option<ParseError> {
        if !self.at(TokenKind::Case)
            || !self.errors.is_empty()
            || self
                .token_before(self.node_start())
                .is_some_and(|(kind, _)| {
                    !matches!(
                        kind,
                        TokenKind::Newline | TokenKind::Indent | TokenKind::Dedent
                    )
                })
        {
            return None;
        }

        let checkpoint = self.checkpoint();
        // These recovery fields are not covered by the normal parser checkpoint.
        let unclosed_bracket = self.first_unclosed_bracket_recovery;
        let escape_error = self.interpolated_string_escape_error.clone();
        let replacement_field = self.replacement_field;
        let diagnostic = self.parse_invalid_standalone_case();
        self.rewind(checkpoint);
        self.first_unclosed_bracket_recovery = unclosed_bracket;
        self.interpolated_string_escape_error = escape_error;
        self.replacement_field = replacement_field;
        diagnostic
    }

    fn parse_invalid_standalone_case(&mut self) -> Option<ParseError> {
        let start = self.node_start();
        self.bump(TokenKind::Case);
        let pattern = self.parse_match_patterns();
        if !self.errors.is_empty() {
            return self.standalone_case_inner_error();
        }
        if !pattern_has_valid_value_names(&pattern) {
            return None;
        }

        if self.eat(TokenKind::If) {
            if !self.at_expr() {
                return None;
            }
            self.parse_named_expression_or_higher(ExpressionContext::default());
            if !self.errors.is_empty() {
                return self.standalone_case_inner_error();
            }
        }
        if !self.at(TokenKind::Colon) {
            return None;
        }
        let header_range = TextRange::new(start, self.current_token_range().end());
        self.bump(TokenKind::Colon);
        self.parse_body(BlockClause::StandaloneCase, start);

        if self
            .tokens
            .has_tokenizer_error_in(TextRange::new(start, self.current_token_range().end()))
        {
            return None;
        }
        if self.errors.is_empty() {
            Some(ParseError {
                error: ParseErrorType::CaseOutsideMatch,
                location: header_range,
            })
        } else {
            self.standalone_case_inner_error()
        }
    }

    /// A malformed production does not establish a standalone case. An invalid
    /// rule reached while parsing its pattern, guard, or body can still raise
    /// its own diagnostic before the enclosing standalone-case rule fires.
    fn standalone_case_inner_error(&self) -> Option<ParseError> {
        self.errors
            .first()
            .filter(|error| {
                matches!(
                    error.error,
                    ParseErrorType::CaseOutsideMatch
                        | ParseErrorType::ExpectedIndentedBlock { .. }
                        | ParseErrorType::InvalidMatchPatternTarget
                        | ParseErrorType::InvalidPatternTarget(_)
                        | ParseErrorType::MissingRaiseException
                        | ParseErrorType::MissingRaiseCause
                        | ParseErrorType::NamedExpressionWithoutParentheses
                        | ParseErrorType::MappingRestPatternNotLast
                        | ParseErrorType::TrailingCommaInWith
                        | ParseErrorType::MisplacedLazyImport
                        | ParseErrorType::LazyFutureImport
                        | ParseErrorType::InvalidAssignmentTarget { .. }
                        | ParseErrorType::InvalidAnnotatedAssignmentTarget
                        | ParseErrorType::InvalidAugmentedAssignmentTarget(_)
                        | ParseErrorType::InvalidDeleteTarget(_)
                )
            })
            .cloned()
    }
}

/// The recovery parser permits an underscore at the root of a value or class
/// pattern. In the grammar it commits to the wildcard instead, so `_.x` cannot
/// complete the pattern even when the recovered AST is an attribute.
fn pattern_has_valid_value_names(pattern: &Pattern) -> bool {
    match pattern {
        Pattern::MatchValue(pattern) => value_has_valid_root(&pattern.value),
        Pattern::MatchSingleton(_) | Pattern::MatchStar(_) => true,
        Pattern::MatchSequence(pattern) => {
            pattern.patterns.iter().all(pattern_has_valid_value_names)
        }
        Pattern::MatchMapping(pattern) => {
            pattern.patterns.iter().all(pattern_has_valid_value_names)
        }
        Pattern::MatchClass(pattern) => {
            value_has_valid_root(&pattern.cls)
                && pattern
                    .arguments
                    .patterns
                    .iter()
                    .all(pattern_has_valid_value_names)
                && pattern
                    .arguments
                    .keywords
                    .iter()
                    .all(|keyword| pattern_has_valid_value_names(&keyword.pattern))
        }
        Pattern::MatchAs(pattern) => pattern
            .pattern
            .as_deref()
            .is_none_or(pattern_has_valid_value_names),
        Pattern::MatchOr(pattern) => pattern.patterns.iter().all(pattern_has_valid_value_names),
    }
}

fn value_has_valid_root(mut value: &Expr) -> bool {
    while let Expr::Attribute(attribute) = value {
        value = &attribute.value;
    }
    match value {
        Expr::Name(name) => name.id != "_",
        _ => true,
    }
}

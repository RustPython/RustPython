//! Original lexer-read provenance for Python 3.15 diagnostic composition.
//!
//! This is observation metadata, not a second lexer or a warning dispatcher.
//! Parser-recovery reads remain labelled and cannot silently become first-pass
//! evidence. The VM must not infer a CPython recovery trace from recovered ASTs.

use ruff_python_ast::token::{Token, TokenKind};
use ruff_text_size::{Ranged, TextRange, TextSize};

use crate::error::{LexicalErrorType, InterpolatedStringErrorType};

#[derive(Debug, Clone, Copy, PartialEq, Eq, get_size2::GetSize)]
pub enum DiagnosticReadOrigin {
    FirstPass,
    ParserRecovery,
    /// Independent original-lexer continuation; never a source of parser actions.
    TokenizerContinuation,
}

/// Explicit effect of parser-requested lexer recovery, not a range heuristic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, get_size2::GetSize)]
pub enum LexerRecoveryEffect {
    Unchanged,
    /// State changed without producing a replacement read (for example nesting).
    StateChanged,
    /// The lexer moved and formed a new token from a rewritten state.
    Retokenized,
    /// An existing token/cursor was replaced directly.
    TokenRewritten,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, get_size2::GetSize)]
pub struct LexicalDivergence {
    /// Reads strictly before this ordinal retain original lexical lineage.
    /// State-only recovery can set this boundary without appending a read.
    pub next_read: usize,
    pub effect: LexerRecoveryEffect,
}

/// The continuation API deliberately supports one original lexer stream only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, get_size2::GetSize)]
pub enum DiagnosticContinuity {
    SingleStream { divergence: Option<LexicalDivergence> },
    /// Cell concatenation combines independent lexers, not one tokenizer suffix.
    IndependentStreams,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, get_size2::GetSize)]
pub enum LexicalDisposition {
    /// CPython returns an ordinary token such as OP, which the parser rejects.
    ParserToken,
    /// A tokenizer ERRORTOKEN without PyErr; retain the original reason.
    BareErrorToken,
    /// The original tokenizer path sets an exception before ERRORTOKEN.
    RaisedErrorToken,
    /// No original tokenizer-stop interpretation has been established.
    DecoderOrUnclassified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, get_size2::GetSize)]
pub struct OpenBracketDiagnostic {
    pub kind: char,
    pub offset: TextSize,
    // Derive a line from the original source only if a consumer needs it.
    // Keeping this snapshot offset-only avoids prefix scans on successful closes.
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, get_size2::GetSize)]
pub struct LexicalContext {
    /// Any active f/t mode, including literal and format-spec scanning.
    pub in_interpolated_string: bool,
    pub nesting: u32,
    pub implicit_nesting: u32,
    pub open_bracket: Option<OpenBracketDiagnostic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, get_size2::GetSize)]
pub enum LexicalWarningKind {
    Numeric(crate::error::NumberLiteralKind),
    InterpolatedBrace(char),
}

#[derive(Debug, Clone, PartialEq, Eq, get_size2::GetSize)]
pub struct LexicalWarningDiscovery {
    pub kind: LexicalWarningKind,
    /// Numeric literal start or original backslash, independent of emission.
    pub offset: TextSize,
    /// Numeric last character or observed brace for existing emitter helpers.
    pub last_offset: TextSize,
    /// Canonical observation cursor, including an observed brace lookahead.
    pub cursor: TextSize,
    pub context: LexicalContext,
}

#[derive(Debug, Clone, PartialEq, Eq, get_size2::GetSize)]
pub struct LexicalDiscovery {
    pub error: LexicalErrorType,
    pub diagnostic: TextRange,
    pub cursor: TextSize,
    /// None when failure occurred while skipping whitespace before a token.
    pub token_range: Option<TextRange>,
    pub context: LexicalContext,
    pub disposition: LexicalDisposition,
    /// EOF cleanup can synthesize several diagnostics after its first failure.
    pub eof_cleanup: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, get_size2::GetSize)]
pub enum TypeCommentKind {
    Regular,
    Ignore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, get_size2::GetSize)]
pub struct TypeCommentToken {
    pub kind: TypeCommentKind,
    pub token: TextRange,
    /// The canonical token payload starts after '#', optional spaces/tabs,
    /// 'type:', and optional spaces/tabs (or after 'ignore' for TYPE_IGNORE).
    pub payload: TextRange,
}

#[derive(Debug, Clone, PartialEq, Eq, get_size2::GetSize)]
pub struct DiagnosticTokenRead {
    /// Monotonic within the committed branch, independent of final token slots.
    pub ordinal: usize,
    /// Historical token-buffer slot. Recovery may later reuse it.
    pub token_slot: usize,
    pub token: Token,
    /// None for a failure before a new token was formed.
    pub read_range: Option<TextRange>,
    pub origin: DiagnosticReadOrigin,
    pub discoveries: Vec<LexicalDiscovery>,
    pub warnings: Vec<LexicalWarningDiscovery>,
    pub type_comment: Option<TypeCommentToken>,
    /// Explicit even though Parsed.tokens() omits the final EOF token.
    pub end_of_input: bool,
}

/// Actual Ruff parser observation point, not a canonical PEG failure frontier.
#[derive(Debug, Clone, PartialEq, Eq, get_size2::GetSize)]
pub struct DiagnosticParserPoint {
    pub token: Token,
    /// None when no eligible header had activated the lazy read journal yet.
    pub read: Option<usize>,
    /// No original lexical discovery or parser-requested lexer rewrite preceded
    /// this point. A ParserToken discovery also prevents a clean prefix.
    pub original_lexical_prefix: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, get_size2::GetSize)]
pub enum DiagnosticParserFailureKind {
    Error(crate::ParseErrorType),
    /// Unclosed-bracket recovery can rewrite the lexer before adding an error.
    LexicalRecovery,
}

#[derive(Debug, Clone, PartialEq, Eq, get_size2::GetSize)]
pub struct DiagnosticParserFailure {
    pub point: DiagnosticParserPoint,
    pub kind: DiagnosticParserFailureKind,
}

#[derive(Debug, Clone, PartialEq, Eq, get_size2::GetSize)]
pub struct DiagnosticNewlineConsumption {
    pub point: DiagnosticParserPoint,
    pub parser_errors_absent: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, get_size2::GetSize)]
pub enum DiagnosticStatementKind { Simple, Compound }

#[derive(Debug, Clone, PartialEq, Eq, get_size2::GetSize)]
pub struct DiagnosticStatementReturn {
    pub kind: DiagnosticStatementKind,
    pub statement: TextRange,
    pub point: DiagnosticParserPoint,
    pub parser_errors_absent: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, get_size2::GetSize)]
pub enum TypeCommentOwner { Assignment, ForHeader, WithHeader, FunctionHeader, FunctionBody, Parameter }

/// A positive grammar-owned acceptance, not absence of validator rejection.
#[derive(Debug, Clone, PartialEq, Eq, get_size2::GetSize)]
pub struct TypeCommentAcceptance {
    pub comment: TypeCommentToken,
    pub owner: TypeCommentOwner,
    pub preceding: Token,
    pub following: Token,
    pub point: DiagnosticParserPoint,
}

/// Actual FunctionDef named-parameter production before consuming a comma.
/// Canonical param TYPE_COMMENT? &')' fails at this following comma.
#[derive(Debug, Clone, PartialEq, Eq, get_size2::GetSize)]
pub struct ParameterTypeCommentBarrier {
    pub comment: TypeCommentToken,
    pub preceding: Token,
    pub point: DiagnosticParserPoint,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, get_size2::GetSize)]
pub struct DiagnosticParserProgress {
    pub parameter_type_comment_barrier: Option<ParameterTypeCommentBarrier>,
    pub first_failure: Option<DiagnosticParserFailure>,
    /// Actual first logical NEWLINE consumption, before requesting another token.
    /// Only the existing compiler's Simple classification can turn this into a
    /// single_input completion; a compound header newline is not completion.
    pub first_logical_newline: Option<DiagnosticNewlineConsumption>,
    /// Actual return from the first module-level statement, before list recovery.
    /// Only Compound is a Single-input boundary; semicolon Simple returns are not.
    pub first_statement_return: Option<DiagnosticStatementReturn>,
}

/// The actual invalid-import-alias expression production. CPython only
/// decodes this expression in invalid_import_from_as_name on pass two, where
/// string_parser.c suppresses decoder warnings. Tokenizer warnings are distinct.
#[derive(Debug, Clone, PartialEq, Eq, get_size2::GetSize)]
pub struct InvalidImportAliasDecoder {
    pub imported_name: TextRange,
    pub as_keyword: TextRange,
    pub first: DiagnosticParserPoint,
    pub expression: TextRange,
}

/// Canonical token-backed classification from CPython lexer.c's type prefix.
/// The caller must supply a Comment token; string contents are never scanned.
pub fn classify_type_comment(source: &str, token: &Token) -> Option<TypeCommentToken> {
    if token.kind() != TokenKind::Comment {
        return None;
    }
    let text = &source[token.range()];
    let rest = text.strip_prefix('#')?.trim_start_matches([' ', '\t']);
    let rest = rest.strip_prefix("type:")?.trim_start_matches([' ', '\t']);
    let payload_start = token.end() - TextSize::try_from(rest.len()).unwrap();
    let ignore = rest.strip_prefix("ignore").filter(|suffix| {
        suffix.as_bytes().first().is_none_or(|byte| byte.is_ascii() && !byte.is_ascii_alphanumeric())
    });
    Some(TypeCommentToken {
        kind: if ignore.is_some() { TypeCommentKind::Ignore } else { TypeCommentKind::Regular },
        token: token.range(),
        payload: TextRange::new(
            payload_start + if ignore.is_some() { TextSize::new(6) } else { TextSize::new(0) },
            token.end(),
        ),
    })
}

/// Only call for an error recorded by an original lexer production site.
/// Parser-created Lexical variants and synthesized/reordered errors are excluded.
pub(crate) fn original_lexer_disposition(error: &LexicalErrorType) -> LexicalDisposition {
    use LexicalDisposition::{BareErrorToken, DecoderOrUnclassified, ParserToken, RaisedErrorToken};
    use LexicalErrorType as E;
    match error {
        E::IndentationError | E::TabError | E::TooDeepIndentation
        | E::LineContinuationError | E::Eof | E::UnclosedBracket { .. } => BareErrorToken,
        E::UnclosedStringError { .. } | E::UnmatchedBracket { .. }
        | E::MismatchedBracket { .. } | E::TooDeeplyNestedBrackets
        | E::InvalidNumberLiteral { .. } | E::InvalidDigit { .. }
        | E::LeadingZerosInDecimalInteger | E::IncompatibleStringPrefixes { .. } => RaisedErrorToken,
        E::UnrecognizedToken { tok } if tok.is_ascii() && !tok.is_ascii_control() => ParserToken,
        E::UnrecognizedToken { .. } => RaisedErrorToken,
        E::FStringError(error) | E::TStringError(error) => match error {
            InterpolatedStringErrorType::SingleRbrace
            | InterpolatedStringErrorType::UnterminatedString { .. }
            | InterpolatedStringErrorType::UnterminatedTripleQuotedString { .. }
            | InterpolatedStringErrorType::NewlineInFormatSpec
            | InterpolatedStringErrorType::UnclosedLbrace
            | InterpolatedStringErrorType::UnclosedLbraceOnLine { .. }
            | InterpolatedStringErrorType::UnmatchedBracket(_) => RaisedErrorToken,
            _ => DecoderOrUnclassified,
        },
        E::StringError | E::UnicodeEscapeError { .. } | E::BytesEscapeError { .. }
        | E::InvalidByteLiteral | E::OtherError(_) => DecoderOrUnclassified,
    }
}

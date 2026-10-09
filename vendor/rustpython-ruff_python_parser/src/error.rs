use std::fmt::{self, Display};

use ruff_python_ast::token::TokenKind;
use ruff_python_ast::{ConstantValue, Expr, PythonVersion};
use ruff_text_size::{Ranged, TextRange, TextSize};

use crate::string::InterpolatedStringKind;

/// Represents represent errors that occur during parsing and are
/// returned by the `parse_*` functions.
#[derive(Debug, PartialEq, Eq, Clone, get_size2::GetSize)]
pub struct ParseError {
    pub error: ParseErrorType,
    pub location: TextRange,
}

impl std::ops::Deref for ParseError {
    type Target = ParseErrorType;

    fn deref(&self) -> &Self::Target {
        &self.error
    }
}

impl std::error::Error for ParseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{} at byte range {:?}", self.error, self.location)
    }
}

impl From<LexicalError> for ParseError {
    fn from(error: LexicalError) -> Self {
        ParseError {
            location: error.location(),
            error: ParseErrorType::Lexical(error.into_error()),
        }
    }
}

impl Ranged for ParseError {
    fn range(&self) -> TextRange {
        self.location
    }
}

impl ParseError {
    pub fn error(self) -> ParseErrorType {
        self.error
    }
}

/// The reason an escape sequence in a string literal cannot be decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, get_size2::GetSize)]
pub enum UnicodeEscapeErrorKind {
    /// `\x` is not followed by two hexadecimal digits.
    TruncatedHexByte,
    /// `\u` is not followed by four hexadecimal digits.
    TruncatedShortUnicode,
    /// `\U` is not followed by eight hexadecimal digits.
    TruncatedLongUnicode,
    /// `\U` names a code point above `U+10FFFF`.
    IllegalCharacter,
    /// `\N` is not followed by a non-empty name in braces.
    MalformedName,
    /// `\N{...}` names no Unicode character.
    UnknownName,
}

impl Display for UnicodeEscapeErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::TruncatedHexByte => "truncated \\xXX escape",
            Self::TruncatedShortUnicode => "truncated \\uXXXX escape",
            Self::TruncatedLongUnicode => "truncated \\UXXXXXXXX escape",
            Self::IllegalCharacter => "illegal Unicode character",
            Self::MalformedName => "malformed \\N character escape",
            Self::UnknownName => "unknown Unicode character name",
        })
    }
}

/// Represents the different types of errors that can occur during parsing of an f-string or t-string.
#[derive(Debug, Clone, PartialEq, Eq, get_size2::GetSize)]
pub enum InterpolatedStringErrorType {
    /// Expected a right brace after an opened left brace.
    UnclosedLbrace,
    /// Expected a right brace for a replacement field opened on an earlier line.
    UnclosedLbraceOnLine { opening_line: u32 },
    /// An invalid conversion flag was encountered.
    InvalidConversionFlag,
    /// A single right brace was encountered.
    SingleRbrace,
    /// Unterminated string, detected at the given one-based line.
    UnterminatedString { detected_line: u32 },
    /// Unterminated triple-quoted string, detected at the given one-based line.
    UnterminatedTripleQuotedString { detected_line: u32 },
    /// A lambda expression without parentheses was encountered.
    LambdaWithoutParentheses,
    /// Conversion flag does not immediately follow exclamation.
    ConversionFlagNotImmediatelyAfterExclamation,
    /// Newline inside of a format spec for a single quoted f- or t-string.
    NewlineInFormatSpec,
    /// A replacement field has no expression before the given separator.
    ExpressionRequiredBefore(char),
    /// A replacement field does not start with an expression.
    ExpectedExpressionAfterLbrace,
    /// The expression of a replacement field is not followed by `=`, `!`, `:` or `}`.
    ExpectedSeparatorAfterExpression,
    /// The `=` of a replacement field is not followed by `!`, `:` or `}`.
    ExpectedSeparatorAfterDebug,
    /// The conversion of a replacement field is not followed by `:` or `}`.
    ExpectedSeparatorAfterConversion,
    /// The format spec of a replacement field is not followed by `}`.
    UnclosedFormatSpec,
    /// A `!` is directly followed by `:` or `}`.
    MissingConversionFlag,
    /// A closing bracket closes the brace that opens a replacement field.
    UnmatchedBracket(char),
}

impl std::fmt::Display for InterpolatedStringErrorType {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::UnclosedLbrace => f.write_str("expecting '}'"),
            Self::UnclosedLbraceOnLine { opening_line } => {
                write!(f, "expecting '}}' to close '{{' on line {opening_line}")
            }
            Self::InvalidConversionFlag => write!(f, "invalid conversion character"),
            Self::SingleRbrace => f.write_str("single '}' is not allowed"),
            Self::UnterminatedString { .. } => write!(f, "unterminated string"),
            Self::UnterminatedTripleQuotedString { .. } => {
                write!(f, "unterminated triple-quoted string")
            }
            Self::LambdaWithoutParentheses => {
                write!(f, "lambda expressions are not allowed without parentheses")
            }
            Self::ConversionFlagNotImmediatelyAfterExclamation => write!(
                f,
                "conversion type must come right after the exclamation mark"
            ),
            Self::NewlineInFormatSpec => {
                write!(
                    f,
                    "newlines are not allowed in format specifiers when using single quotes"
                )
            }
            Self::ExpressionRequiredBefore(separator) => {
                write!(f, "valid expression required before '{separator}'")
            }
            Self::ExpectedExpressionAfterLbrace => {
                f.write_str("expecting a valid expression after '{'")
            }
            Self::ExpectedSeparatorAfterExpression => {
                f.write_str("expecting '=', or '!', or ':', or '}'")
            }
            Self::ExpectedSeparatorAfterDebug => f.write_str("expecting '!', or ':', or '}'"),
            Self::ExpectedSeparatorAfterConversion => f.write_str("expecting ':' or '}'"),
            Self::UnclosedFormatSpec => f.write_str("expecting '}', or format specs"),
            Self::MissingConversionFlag => f.write_str("missing conversion character"),
            Self::UnmatchedBracket(closing) => write!(f, "unmatched '{closing}'"),
        }
    }
}

fn write_interpolated_string_error(
    f: &mut std::fmt::Formatter,
    kind: InterpolatedStringKind,
    error: &InterpolatedStringErrorType,
) -> std::fmt::Result {
    match error {
        InterpolatedStringErrorType::UnterminatedString { detected_line } => {
            write!(
                f,
                "unterminated {kind} literal (detected at line {detected_line})"
            )
        }
        InterpolatedStringErrorType::UnterminatedTripleQuotedString { detected_line } => {
            write!(
                f,
                "unterminated triple-quoted {kind} literal (detected at line {detected_line})"
            )
        }
        InterpolatedStringErrorType::NewlineInFormatSpec => write!(
            f,
            "{kind}: newlines are not allowed in format specifiers for single quoted {kind}s"
        ),
        _ => write!(f, "{kind}: {error}"),
    }
}

/// Represents the different types of errors that can occur during parsing.
#[derive(Debug, PartialEq, Eq, Clone, get_size2::GetSize)]
pub enum ParseErrorType {
    /// An unexpected error occurred.
    OtherError(String),

    /// A missing-comma hint whose concatenated literal is displayed on its final line.
    /// The original expression range controls error precedence; `diagnostic_start` can be
    /// after the displayed range's end, which a `TextRange` cannot represent.
    MissingCommaAfterConcatenatedLiteral {
        expression_range: TextRange,
        diagnostic_start: TextSize,
    },

    /// An error specific to stringified annotations occurred.
    StringAnnotationError(&'static str),

    /// A `raise from` statement has no exception expression.
    MissingRaiseException,
    /// A `raise` statement ends immediately after `from`.
    MissingRaiseCause,
    /// An unparenthesized `with` item list ends with a comma.
    TrailingCommaInWith,
    /// `lazy` follows the module name instead of preceding `from`.
    MisplacedLazyImport,
    /// An absolute future import is marked lazy.
    LazyFutureImport,

    /// An empty slice was found during parsing, e.g `data[]`.
    EmptySlice,
    /// An empty global names list was found during parsing.
    EmptyGlobalNames,
    /// An empty nonlocal names list was found during parsing.
    EmptyNonlocalNames,
    /// An empty delete targets list was found during parsing.
    EmptyDeleteTargets,
    /// An empty import names list was found during parsing.
    EmptyImportNames,
    /// An empty type parameter list was found during parsing.
    EmptyTypeParams,

    /// An unparenthesized named expression was found where it is not allowed.
    UnparenthesizedNamedExpression,
    /// An unparenthesized tuple expression was found where it is not allowed.
    UnparenthesizedTupleExpression,
    /// An unparenthesized generator expression was found where it is not allowed.
    UnparenthesizedGeneratorExpression,

    /// An invalid usage of a lambda expression was found.
    InvalidLambdaExpressionUsage,
    /// An invalid usage of a yield expression was found.
    InvalidYieldExpressionUsage,
    /// An invalid usage of a starred expression was found.
    InvalidStarredExpressionUsage,
    /// An unpacked conditional expression is missing parentheses.
    InvalidConditionalUnpacking { double_starred: bool },
    /// Only the else branch of a conditional expression is unpacked.
    InvalidConditionalBranchUnpacking { double_starred: bool },
    /// Dictionary unpacking is used in a list or generator comprehension.
    InvalidComprehensionDictUnpacking { generator: bool },
    /// An unpacked expression is used as a dictionary key.
    InvalidDictKeyUnpacking { double_starred: bool },
    /// An unpacked expression is used as a dictionary value.
    InvalidDictValueUnpacking { double_starred: bool },
    /// Dictionary unpacking is used outside a dictionary or call argument.
    InvalidDictUnpacking,
    /// A star pattern was found outside a sequence pattern.
    InvalidStarPatternUsage,
    /// An underscore was used as a binding target in a match pattern.
    InvalidMatchPatternTarget,
    /// A complete case clause was found outside a match statement.
    CaseOutsideMatch,

    /// A parameter was found after a vararg.
    ParamAfterVarKeywordParam,
    /// A non-default parameter follows a default parameter.
    NonDefaultParamAfterDefaultParam,
    /// A default value was found for a `*` or `**` parameter.
    VarParameterWithDefault,
    /// A default value was found for a `**` parameter.
    VarKeywordParameterWithDefault,
    /// A dictionary key after the first item is not followed by a `:`. The error range is the
    /// last character of the key.
    ExpectedColonAfterDictionaryKey,

    /// An invalid expression was found in the assignment target. `maybe_comparison` is set when
    /// the `=` after it may have been meant as `==`.
    InvalidAssignmentTarget {
        kind: ExpressionKind,
        maybe_comparison: bool,
    },
    /// A named expression is used in an assertion without its required parentheses.
    NamedExpressionWithoutParentheses,
    /// A mapping pattern contains an entry after its double-star rest pattern.
    MappingRestPatternNotLast,
    /// A name is assigned to where `==` or `:=` may have been meant.
    AssignmentInsteadOfComparison,
    /// A `yield` expression is assigned to.
    AssignmentToYield,
    /// An invalid expression was found in the named assignment target.
    InvalidNamedAssignmentTarget(ExpressionKind),
    /// An invalid expression was found in the annotated assignment target.
    InvalidAnnotatedAssignmentTarget,
    /// An invalid expression was found in the augmented assignment target.
    InvalidAugmentedAssignmentTarget(ExpressionKind),
    /// An invalid expression was found in the delete target.
    InvalidDeleteTarget(ExpressionKind),
    /// An invalid expression was found after `as` in an import.
    InvalidImportTarget(ExpressionKind),
    /// An invalid expression was found after `as` in a pattern.
    InvalidPatternTarget(ExpressionKind),
    /// An invalid expression was found after `as` in an `except` or `except*` clause.
    InvalidExceptTarget { kind: ExpressionKind, star: bool },

    /// A positional argument was found after a keyword argument.
    PositionalAfterKeywordArgument,
    /// A positional argument was found after a keyword argument unpacking.
    PositionalAfterKeywordUnpacking,
    /// An iterable argument unpacking was found after keyword argument unpacking.
    InvalidArgumentUnpackingOrder,
    /// An invalid usage of iterable unpacking in a comprehension was found.
    IterableUnpackingInComprehension,

    /// Multiple simple statements were found in the same line without a `;` separating them.
    SimpleStatementsOnSameLine,
    /// A simple statement and a compound statement was found in the same line.
    SimpleAndCompoundStatementOnSameLine,

    /// Expected one or more keyword parameter after `*` separator.
    ExpectedKeywordParam,
    /// Expected a real number for a complex literal pattern.
    ExpectedRealNumber,
    /// Expected an imaginary number for a complex literal pattern.
    ExpectedImaginaryNumber,
    /// Expected an expression at the current parser location.
    ExpectedExpression,
    /// Expected an identifier at the current parser location.
    ExpectedIdentifier,
    /// The parser expected a specific token that was not found.
    ExpectedToken {
        expected: TokenKind,
        found: TokenKind,
    },

    /// A compound statement header is not followed by an indented block. `line` is the line of
    /// the header's keyword.
    ExpectedIndentedBlock { clause: BlockClause, line: u32 },
    /// An unexpected indentation was found during parsing.
    UnexpectedIndentation,
    /// The statement being parsed cannot be `async`.
    UnexpectedTokenAfterAsync(TokenKind),
    /// Ipython escape command was found
    UnexpectedIpythonEscapeCommand,
    /// An unexpected token was found at the end of an expression parsing
    UnexpectedExpressionToken,

    /// An f-string error containing the [`InterpolatedStringErrorType`].
    FStringError(InterpolatedStringErrorType),
    /// A t-string error containing the [`InterpolatedStringErrorType`].
    TStringError(InterpolatedStringErrorType),
    /// Parser encountered an error during lexing.
    Lexical(LexicalErrorType),
}

/// The kind of an expression, as syntax errors name it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, get_size2::GetSize)]
pub enum ExpressionKind {
    Attribute,
    Subscript,
    Starred,
    Name,
    List,
    Tuple,
    Lambda,
    FunctionCall,
    Expression,
    GeneratorExpression,
    YieldExpression,
    AwaitExpression,
    ListComprehension,
    SetComprehension,
    DictComprehension,
    DictLiteral,
    SetDisplay,
    FStringExpression,
    TStringExpression,
    None,
    False,
    True,
    Ellipsis,
    Literal,
    Comparison,
    ConditionalExpression,
    NamedExpression,
}

impl ExpressionKind {
    pub fn of(expr: &Expr) -> Self {
        match expr {
            Expr::Attribute(_) => Self::Attribute,
            Expr::Subscript(_) => Self::Subscript,
            Expr::Starred(_) => Self::Starred,
            Expr::Name(_) => Self::Name,
            Expr::List(_) => Self::List,
            Expr::Tuple(_) => Self::Tuple,
            Expr::Lambda(_) => Self::Lambda,
            Expr::Call(_) => Self::FunctionCall,
            Expr::BoolOp(_)
            | Expr::BinOp(_)
            | Expr::UnaryOp(_)
            | Expr::Slice(_)
            | Expr::IpyEscapeCommand(_) => Self::Expression,
            Expr::Generator(_) => Self::GeneratorExpression,
            Expr::Yield(_) | Expr::YieldFrom(_) => Self::YieldExpression,
            Expr::Await(_) => Self::AwaitExpression,
            Expr::ListComp(_) => Self::ListComprehension,
            Expr::SetComp(_) => Self::SetComprehension,
            Expr::DictComp(_) => Self::DictComprehension,
            Expr::Dict(_) => Self::DictLiteral,
            Expr::Set(_) => Self::SetDisplay,
            Expr::FString(_) => Self::FStringExpression,
            Expr::TString(_) => Self::TStringExpression,
            Expr::NoneLiteral(_) => Self::None,
            Expr::BooleanLiteral(literal) if literal.value => Self::True,
            Expr::BooleanLiteral(_) => Self::False,
            Expr::EllipsisLiteral(_) => Self::Ellipsis,
            Expr::StringLiteral(_) | Expr::BytesLiteral(_) | Expr::NumberLiteral(_) => {
                Self::Literal
            }
            Expr::Compare(_) => Self::Comparison,
            Expr::If(_) => Self::ConditionalExpression,
            Expr::Named(_) => Self::NamedExpression,
            Expr::Constant(constant) => match constant.value {
                ConstantValue::None => Self::None,
                ConstantValue::Boolean(true) => Self::True,
                ConstantValue::Boolean(false) => Self::False,
                ConstantValue::Ellipsis => Self::Ellipsis,
                _ => Self::Literal,
            },
        }
    }
}

impl std::fmt::Display for ExpressionKind {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(match self {
            Self::Attribute => "attribute",
            Self::Subscript => "subscript",
            Self::Starred => "starred",
            Self::Name => "name",
            Self::List => "list",
            Self::Tuple => "tuple",
            Self::Lambda => "lambda",
            Self::FunctionCall => "function call",
            Self::Expression => "expression",
            Self::GeneratorExpression => "generator expression",
            Self::YieldExpression => "yield expression",
            Self::AwaitExpression => "await expression",
            Self::ListComprehension => "list comprehension",
            Self::SetComprehension => "set comprehension",
            Self::DictComprehension => "dict comprehension",
            Self::DictLiteral => "dict literal",
            Self::SetDisplay => "set display",
            Self::FStringExpression => "f-string expression",
            Self::TStringExpression => "t-string expression",
            Self::None => "None",
            Self::False => "False",
            Self::True => "True",
            Self::Ellipsis => "ellipsis",
            Self::Literal => "literal",
            Self::Comparison => "comparison",
            Self::ConditionalExpression => "conditional expression",
            Self::NamedExpression => "named expression",
        })
    }
}

/// The compound statement clause whose header precedes an indented block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, get_size2::GetSize)]
pub enum BlockClause {
    If,
    Elif,
    Else,
    For,
    With,
    While,
    Try,
    Except,
    ExceptStar,
    Finally,
    Match,
    Case,
    /// A standalone case uses the generic invalid-block diagnostic.
    StandaloneCase,
    Class,
    FunctionDef,
}

impl std::fmt::Display for BlockClause {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(match self {
            Self::If => "'if' statement",
            Self::Elif => "'elif' statement",
            Self::Else => "'else' statement",
            Self::For => "'for' statement",
            Self::With => "'with' statement",
            Self::While => "'while' statement",
            Self::Try => "'try' statement",
            Self::Except => "'except' statement",
            Self::ExceptStar => "'except*' statement",
            Self::Finally => "'finally' statement",
            Self::Match => "'match' statement",
            Self::Case | Self::StandaloneCase => "'case' statement",
            Self::Class => "class definition",
            Self::FunctionDef => "function definition",
        })
    }
}

impl ParseErrorType {
    pub(crate) fn from_interpolated_string_error(
        error: InterpolatedStringErrorType,
        string_kind: InterpolatedStringKind,
    ) -> Self {
        match string_kind {
            InterpolatedStringKind::FString => Self::FStringError(error),
            InterpolatedStringKind::TString => Self::TStringError(error),
        }
    }
}

impl std::error::Error for ParseErrorType {}

impl std::fmt::Display for ParseErrorType {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            ParseErrorType::OtherError(msg) => f.write_str(msg),
            ParseErrorType::MissingCommaAfterConcatenatedLiteral { .. } => {
                f.write_str("invalid syntax. Perhaps you forgot a comma?")
            }
            ParseErrorType::ExpectedIndentedBlock {
                clause: BlockClause::StandaloneCase,
                ..
            } => f.write_str("expected an indented block"),
            ParseErrorType::ExpectedIndentedBlock { clause, line } => {
                write!(
                    f,
                    "expected an indented block after {clause} on line {line}"
                )
            }
            ParseErrorType::StringAnnotationError(msg) => f.write_str(msg),
            ParseErrorType::ExpectedToken { found, expected } => match (*expected, *found) {
                (TokenKind::Colon, TokenKind::Newline) => f.write_str("expected ':'"),
                (TokenKind::Lpar, _) => f.write_str("expected '('"),
                (TokenKind::Else, found) if found != TokenKind::Colon => {
                    f.write_str("expected 'else' after 'if' expression")
                }
                _ => f.write_str("invalid syntax"),
            },
            ParseErrorType::Lexical(lex_error) => write!(f, "{lex_error}"),
            ParseErrorType::SimpleStatementsOnSameLine => f.write_str("invalid syntax"),
            ParseErrorType::SimpleAndCompoundStatementOnSameLine => f.write_str("invalid syntax"),
            ParseErrorType::UnexpectedTokenAfterAsync(kind) => {
                write!(
                    f,
                    "expected `def`, `with` or `for` to follow `async`, found {kind}",
                )
            }
            ParseErrorType::InvalidArgumentUnpackingOrder => {
                f.write_str("iterable argument unpacking follows keyword argument unpacking")
            }
            ParseErrorType::IterableUnpackingInComprehension => {
                f.write_str("iterable unpacking cannot be used in a comprehension")
            }
            ParseErrorType::UnparenthesizedNamedExpression => {
                f.write_str("unparenthesized named expression cannot be used here")
            }
            ParseErrorType::UnparenthesizedTupleExpression => {
                f.write_str("unparenthesized tuple expression cannot be used here")
            }
            ParseErrorType::UnparenthesizedGeneratorExpression => {
                f.write_str("Generator expression must be parenthesized")
            }
            ParseErrorType::InvalidYieldExpressionUsage => {
                f.write_str("yield expression cannot be used here")
            }
            ParseErrorType::InvalidLambdaExpressionUsage => {
                f.write_str("lambda expression cannot be used here")
            }
            ParseErrorType::InvalidConditionalUnpacking { double_starred } => {
                let kind = if *double_starred {
                    "double starred"
                } else {
                    "starred"
                };
                write!(
                    f,
                    "invalid {kind} expression. Did you forget to wrap the conditional expression in parentheses?"
                )
            }
            ParseErrorType::InvalidConditionalBranchUnpacking { double_starred } => {
                if *double_starred {
                    f.write_str(
                        "cannot use dict unpacking on only part of a conditional expression",
                    )
                } else {
                    f.write_str("cannot unpack only part of a conditional expression")
                }
            }
            ParseErrorType::InvalidComprehensionDictUnpacking { generator } => {
                let kind = if *generator {
                    "generator expression"
                } else {
                    "list comprehension"
                };
                write!(f, "cannot use dict unpacking in {kind}")
            }
            ParseErrorType::InvalidDictKeyUnpacking { double_starred }
            | ParseErrorType::InvalidDictValueUnpacking { double_starred } => {
                let kind = if *double_starred {
                    "dict unpacking"
                } else {
                    "a starred expression"
                };
                let position = if matches!(self, ParseErrorType::InvalidDictKeyUnpacking { .. }) {
                    "key"
                } else {
                    "value"
                };
                write!(f, "cannot use {kind} in a dictionary {position}")
            }
            ParseErrorType::InvalidDictUnpacking => f.write_str("cannot use dict unpacking here"),
            ParseErrorType::InvalidStarredExpressionUsage => f.write_str("invalid syntax"),
            ParseErrorType::PositionalAfterKeywordArgument => {
                f.write_str("positional argument follows keyword argument")
            }
            ParseErrorType::PositionalAfterKeywordUnpacking => {
                f.write_str("positional argument follows keyword argument unpacking")
            }
            ParseErrorType::MissingRaiseException => {
                f.write_str("did you forget an expression between 'raise' and 'from'?")
            }
            ParseErrorType::MissingRaiseCause => {
                f.write_str("did you forget an expression after 'from'?")
            }
            ParseErrorType::TrailingCommaInWith => {
                f.write_str("the last 'with' item has a trailing comma")
            }
            ParseErrorType::MisplacedLazyImport => {
                f.write_str("use 'lazy from ... ' instead of 'from ... lazy import'")
            }
            ParseErrorType::LazyFutureImport => {
                f.write_str("lazy from __future__ import is not allowed")
            }
            ParseErrorType::EmptySlice => f.write_str("expected index or slice expression"),
            ParseErrorType::EmptyGlobalNames => {
                f.write_str("global statement must have at least one name")
            }
            ParseErrorType::EmptyNonlocalNames => {
                f.write_str("nonlocal statement must have at least one name")
            }
            ParseErrorType::EmptyDeleteTargets => {
                f.write_str("delete statement must have at least one target")
            }
            ParseErrorType::EmptyImportNames => {
                f.write_str("Expected one or more names after 'import'")
            }
            ParseErrorType::EmptyTypeParams => f.write_str("Type parameter list cannot be empty"),
            ParseErrorType::ParamAfterVarKeywordParam => {
                f.write_str(if cfg!(feature = "python315-diagnostics") {
                    "parameters cannot follow var-keyword parameter"
                } else {
                    "arguments cannot follow var-keyword argument"
                })
            }
            ParseErrorType::NonDefaultParamAfterDefaultParam => {
                f.write_str("parameter without a default follows parameter with a default")
            }
            ParseErrorType::ExpectedKeywordParam => {
                f.write_str(if cfg!(feature = "python315-diagnostics") {
                    "named parameters must follow bare *"
                } else {
                    "named arguments must follow bare *"
                })
            }
            ParseErrorType::VarKeywordParameterWithDefault => {
                f.write_str(if cfg!(feature = "python315-diagnostics") {
                    "var-keyword parameter cannot have default value"
                } else {
                    "var-keyword argument cannot have default value"
                })
            }
            ParseErrorType::ExpectedColonAfterDictionaryKey => {
                f.write_str("':' expected after dictionary key")
            }
            ParseErrorType::VarParameterWithDefault => {
                f.write_str(if cfg!(feature = "python315-diagnostics") {
                    "var-positional parameter cannot have default value"
                } else {
                    "var-positional argument cannot have default value"
                })
            }
            ParseErrorType::InvalidStarPatternUsage => {
                f.write_str("cannot use starred expression here")
            }
            ParseErrorType::InvalidMatchPatternTarget => f.write_str("cannot use '_' as a target"),
            ParseErrorType::CaseOutsideMatch => {
                f.write_str("case statement must be inside match statement")
            }
            ParseErrorType::ExpectedRealNumber => {
                f.write_str("expected a real number in complex literal pattern")
            }
            ParseErrorType::ExpectedImaginaryNumber => {
                f.write_str("expected an imaginary number in complex literal pattern")
            }
            ParseErrorType::ExpectedExpression | ParseErrorType::ExpectedIdentifier => {
                f.write_str("invalid syntax")
            }
            ParseErrorType::UnexpectedIndentation => f.write_str("unexpected indent"),
            ParseErrorType::InvalidAssignmentTarget {
                kind,
                maybe_comparison: false,
            } => write!(f, "cannot assign to {kind}"),
            ParseErrorType::InvalidAssignmentTarget {
                kind,
                maybe_comparison: true,
            } => write!(
                f,
                "cannot assign to {kind} here. Maybe you meant '==' instead of '='?"
            ),
            ParseErrorType::NamedExpressionWithoutParentheses => {
                f.write_str("cannot use named expression without parentheses here")
            }
            ParseErrorType::MappingRestPatternNotLast => f.write_str(
                "double star pattern must be the last (right-most) subpattern in the mapping pattern",
            ),
            ParseErrorType::AssignmentInsteadOfComparison => {
                f.write_str("invalid syntax. Maybe you meant '==' or ':=' instead of '='?")
            }
            ParseErrorType::AssignmentToYield => {
                f.write_str("assignment to yield expression not possible")
            }
            ParseErrorType::InvalidAnnotatedAssignmentTarget => {
                f.write_str("illegal target for annotation")
            }
            ParseErrorType::InvalidNamedAssignmentTarget(kind) => {
                write!(f, "cannot use assignment expressions with {kind}")
            }
            ParseErrorType::InvalidAugmentedAssignmentTarget(kind) => {
                write!(
                    f,
                    "'{kind}' is an illegal expression for augmented assignment"
                )
            }
            ParseErrorType::InvalidDeleteTarget(kind) => write!(f, "cannot delete {kind}"),
            ParseErrorType::InvalidImportTarget(kind) => {
                write!(f, "cannot use {kind} as import target")
            }
            ParseErrorType::InvalidPatternTarget(kind) => {
                write!(f, "cannot use {kind} as pattern target")
            }
            ParseErrorType::InvalidExceptTarget { kind, star } => {
                let clause = if *star { "except*" } else { "except" };
                write!(f, "cannot use {clause} statement with {kind}")
            }
            ParseErrorType::UnexpectedIpythonEscapeCommand => {
                f.write_str("IPython escape commands are only allowed in `Mode::Ipython`")
            }
            ParseErrorType::FStringError(error) => {
                write_interpolated_string_error(f, InterpolatedStringKind::FString, error)
            }
            ParseErrorType::TStringError(error) => {
                write_interpolated_string_error(f, InterpolatedStringKind::TString, error)
            }
            ParseErrorType::UnexpectedExpressionToken => f.write_str("invalid syntax"),
        }
    }
}

/// Represents an error that occur during lexing and are
/// returned by the `parse_*` functions in the iterator in the
/// [lexer] implementation.
///
/// [lexer]: crate::lexer
#[derive(Debug, Clone, PartialEq)]
pub struct LexicalError {
    /// The type of error that occurred.
    error: LexicalErrorType,
    /// The location of the error.
    location: TextRange,
}

impl LexicalError {
    /// Creates a new `LexicalError` with the given error type and location.
    pub fn new(error: LexicalErrorType, location: TextRange) -> Self {
        Self { error, location }
    }

    pub fn error(&self) -> &LexicalErrorType {
        &self.error
    }

    pub fn into_error(self) -> LexicalErrorType {
        self.error
    }

    pub fn location(&self) -> TextRange {
        self.location
    }
}

impl std::ops::Deref for LexicalError {
    type Target = LexicalErrorType;

    fn deref(&self) -> &Self::Target {
        self.error()
    }
}

impl std::error::Error for LexicalError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.error())
    }
}

impl std::fmt::Display for LexicalError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            f,
            "{} at byte offset {}",
            self.error(),
            u32::from(self.location().start())
        )
    }
}

/// Represents the different types of errors that can occur during lexing.
#[derive(Debug, Clone, PartialEq, Eq, get_size2::GetSize)]
pub enum LexicalErrorType {
    // TODO: Can probably be removed, the places it is used seem to be able
    // to use the `UnicodeError` variant instead.
    #[doc(hidden)]
    StringError,
    /// A string literal without the closing quote, detected at the given one-based line.
    UnclosedStringError {
        triple_quoted: bool,
        /// The literal contains a backslash followed by its quote character.
        escaped_end_quote: bool,
        detected_line: u32,
    },
    /// An escape sequence in a string literal cannot be decoded. `start` and `end` are the
    /// inclusive positions of the escape sequence in the string content, where a non-ASCII
    /// character counts as its ten-character `\U` escape.
    UnicodeEscapeError {
        kind: UnicodeEscapeErrorKind,
        start: u32,
        end: u32,
    },
    /// A `\x` escape in a bytes literal at the given position of its content is not followed by
    /// two hexadecimal digits.
    BytesEscapeError { position: u32 },
    /// A dedent does not match any outer indentation level.
    IndentationError,
    /// Tabs and spaces are mixed in a way that makes the indentation depend on the tab size.
    TabError,
    /// The indentation is nested too deeply.
    TooDeepIndentation,
    /// An unrecognized token was encountered.
    UnrecognizedToken { tok: char },
    /// An f-string error containing the [`InterpolatedStringErrorType`].
    FStringError(InterpolatedStringErrorType),
    /// A t-string error containing the [`InterpolatedStringErrorType`].
    TStringError(InterpolatedStringErrorType),
    /// Invalid character encountered in a byte literal.
    InvalidByteLiteral,
    /// An unexpected character was encountered after a line continuation.
    LineContinuationError,
    /// An unexpected end of file was encountered.
    Eof,
    /// A closing bracket without an opening bracket.
    UnmatchedBracket { closing: char },
    /// A closing bracket that does not match the innermost opening bracket. `opening_line` is the
    /// line of the opening bracket if it is on another line.
    MismatchedBracket {
        closing: char,
        opening: char,
        opening_line: Option<u32>,
    },
    /// An opening bracket that is not closed by the end of the source.
    ///
    /// `incomplete` is whether more input could still close it: the parser read to the end of the
    /// source with the bracket open. Otherwise, the error is reported in place of an earlier syntax
    /// error.
    UnclosedBracket { opening: char, incomplete: bool },
    /// Brackets are nested too deeply.
    TooDeeplyNestedBrackets,
    /// A malformed number literal, or one directly followed by a name.
    InvalidNumberLiteral { kind: NumberLiteralKind },
    /// A digit that is not valid in the octal or binary literal it appears in.
    InvalidDigit {
        digit: char,
        kind: NumberLiteralKind,
    },
    /// A decimal integer literal with leading zeros.
    LeadingZerosInDecimalInteger,
    /// A string prefix that combines incompatible prefixes, such as `ub`.
    IncompatibleStringPrefixes { first: char, second: char },
    /// An unexpected error occurred.
    OtherError(Box<str>),
}

impl std::error::Error for LexicalErrorType {}

/// The kind of a number literal, as named in error messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, get_size2::GetSize)]
pub enum NumberLiteralKind {
    Decimal,
    Hexadecimal,
    Octal,
    Binary,
    Imaginary,
}

impl std::fmt::Display for NumberLiteralKind {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(match self {
            Self::Decimal => "decimal",
            Self::Hexadecimal => "hexadecimal",
            Self::Octal => "octal",
            Self::Binary => "binary",
            Self::Imaginary => "imaginary",
        })
    }
}

/// Whether `c` is printable: not a control, format, surrogate, private use, unassigned or
/// separator character, other than the ASCII space.
fn is_printable(c: char) -> bool {
    use icu_properties::props::{EnumeratedProperty, GeneralCategory};

    c == ' '
        || !matches!(
            GeneralCategory::for_char(c),
            GeneralCategory::SpaceSeparator
                | GeneralCategory::LineSeparator
                | GeneralCategory::ParagraphSeparator
                | GeneralCategory::Control
                | GeneralCategory::Format
                | GeneralCategory::Surrogate
                | GeneralCategory::PrivateUse
                | GeneralCategory::Unassigned
        )
}

impl LexicalErrorType {
    /// Returns `true` if the error reports a string literal without its closing quotes.
    pub(crate) fn is_unclosed_string_error(&self) -> bool {
        matches!(
            self,
            Self::UnclosedStringError { .. }
                | Self::FStringError(
                    InterpolatedStringErrorType::UnclosedLbrace
                        | InterpolatedStringErrorType::UnclosedLbraceOnLine { .. }
                )
                | Self::TStringError(
                    InterpolatedStringErrorType::UnclosedLbrace
                        | InterpolatedStringErrorType::UnclosedLbraceOnLine { .. }
                )
        )
    }

    /// Returns `true` if the error stops tokenization, as opposed to an error found while
    /// decoding the content of a token.
    pub fn is_tokenizer_error(&self) -> bool {
        match self {
            Self::StringError
            | Self::UnicodeEscapeError { .. }
            | Self::BytesEscapeError { .. }
            | Self::InvalidByteLiteral
            | Self::OtherError(_) => false,
            // An ASCII punctuation character is a token that the parser rejects.
            Self::UnrecognizedToken { tok } => !tok.is_ascii() || !is_printable(*tok),
            Self::FStringError(error) | Self::TStringError(error) => matches!(
                error,
                InterpolatedStringErrorType::UnterminatedString { .. }
                    | InterpolatedStringErrorType::UnterminatedTripleQuotedString { .. }
                    | InterpolatedStringErrorType::SingleRbrace
                    | InterpolatedStringErrorType::NewlineInFormatSpec
                    | InterpolatedStringErrorType::UnclosedLbrace
                    | InterpolatedStringErrorType::UnclosedLbraceOnLine { .. }
                    | InterpolatedStringErrorType::UnmatchedBracket(_)
            ),
            Self::UnclosedStringError { .. }
            | Self::IndentationError
            | Self::TabError
            | Self::TooDeepIndentation
            | Self::LineContinuationError
            | Self::Eof
            | Self::UnmatchedBracket { .. }
            | Self::MismatchedBracket { .. }
            | Self::UnclosedBracket { .. }
            | Self::TooDeeplyNestedBrackets
            | Self::InvalidNumberLiteral { .. }
            | Self::InvalidDigit { .. }
            | Self::LeadingZerosInDecimalInteger
            | Self::IncompatibleStringPrefixes { .. } => true,
        }
    }

    pub(crate) fn from_interpolated_string_error(
        error: InterpolatedStringErrorType,
        string_kind: InterpolatedStringKind,
    ) -> Self {
        match string_kind {
            InterpolatedStringKind::FString => Self::FStringError(error),
            InterpolatedStringKind::TString => Self::TStringError(error),
        }
    }
}

impl std::fmt::Display for LexicalErrorType {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::StringError => write!(f, "got unexpected string"),
            Self::FStringError(error) => {
                write_interpolated_string_error(f, InterpolatedStringKind::FString, error)
            }
            Self::TStringError(error) => {
                write_interpolated_string_error(f, InterpolatedStringKind::TString, error)
            }
            Self::InvalidByteLiteral => {
                write!(f, "bytes can only contain ASCII literal characters")
            }
            Self::UnicodeEscapeError { kind, start, end } => write!(
                f,
                "(unicode error) 'unicodeescape' codec can't decode bytes in position {start}-{end}: {kind}"
            ),
            Self::BytesEscapeError { position } => {
                write!(f, "(value error) invalid \\x escape at position {position}")
            }
            Self::IndentationError => {
                write!(f, "unindent does not match any outer indentation level")
            }
            Self::TabError => write!(f, "inconsistent use of tabs and spaces in indentation"),
            Self::TooDeepIndentation => write!(f, "too many levels of indentation"),
            Self::UnrecognizedToken { tok } if !is_printable(*tok) => {
                write!(
                    f,
                    "invalid non-printable character U+{:04X}",
                    u32::from(*tok)
                )
            }
            Self::UnrecognizedToken { tok } if !tok.is_ascii() => {
                write!(f, "invalid character '{tok}' (U+{:04X})", u32::from(*tok))
            }
            Self::UnrecognizedToken { .. } => f.write_str("invalid syntax"),
            Self::InvalidNumberLiteral { kind } => write!(f, "invalid {kind} literal"),
            Self::InvalidDigit { digit, kind } => {
                write!(f, "invalid digit '{digit}' in {kind} literal")
            }
            Self::LeadingZerosInDecimalInteger => f.write_str(
                "leading zeros in decimal integer literals are not permitted; use an 0o prefix for octal integers",
            ),
            Self::IncompatibleStringPrefixes { first, second } => {
                write!(f, "'{first}' and '{second}' prefixes are incompatible")
            }
            Self::LineContinuationError => {
                write!(f, "unexpected character after line continuation character")
            }
            Self::Eof => write!(f, "unexpected EOF while parsing"),
            Self::UnmatchedBracket { closing } => write!(f, "unmatched '{closing}'"),
            Self::MismatchedBracket {
                closing,
                opening,
                opening_line,
            } => {
                write!(
                    f,
                    "closing parenthesis '{closing}' does not match opening parenthesis '{opening}'"
                )?;
                if let Some(line) = opening_line {
                    write!(f, " on line {line}")?;
                }
                Ok(())
            }
            Self::UnclosedBracket { opening, .. } => write!(f, "'{opening}' was never closed"),
            Self::TooDeeplyNestedBrackets => f.write_str("too many nested parentheses"),
            Self::OtherError(msg) => write!(f, "{msg}"),
            Self::UnclosedStringError {
                triple_quoted: true,
                detected_line,
                ..
            } => write!(
                f,
                "unterminated triple-quoted string literal (detected at line {detected_line})"
            ),
            Self::UnclosedStringError {
                escaped_end_quote,
                detected_line,
                ..
            } => {
                write!(
                    f,
                    "unterminated string literal (detected at line {detected_line})"
                )?;
                if *escaped_end_quote {
                    f.write_str("; perhaps you escaped the end quote?")?;
                }
                Ok(())
            }
        }
    }
}

/// Represents a version-related syntax error detected during parsing.
///
/// An example of a version-related error is the use of a `match` statement before Python 3.10, when
/// it was first introduced. See [`UnsupportedSyntaxErrorKind`] for other kinds of errors.
#[derive(Debug, PartialEq, Clone, get_size2::GetSize)]
pub struct UnsupportedSyntaxError {
    pub kind: UnsupportedSyntaxErrorKind,
    pub range: TextRange,
    /// The target [`PythonVersion`] for which this error was detected.
    pub target_version: PythonVersion,
}

impl Ranged for UnsupportedSyntaxError {
    fn range(&self) -> TextRange {
        self.range
    }
}

/// The type of tuple unpacking for [`UnsupportedSyntaxErrorKind::StarTuple`].
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy, get_size2::GetSize)]
pub enum StarTupleKind {
    Return,
    Yield,
}

/// The type of PEP 701 f-string error for [`UnsupportedSyntaxErrorKind::Pep701FString`].
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy, get_size2::GetSize)]
pub enum FStringKind {
    Backslash,
    Comment,
    LineBreak,
    NestedQuote,
}

/// The type of PEP 798 unpacking-comprehension error for
/// [`UnsupportedSyntaxErrorKind::UnpackingInComprehension`].
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy, get_size2::GetSize)]
pub enum ComprehensionUnpackingKind {
    IterableInList,
    IterableInSet,
    IterableInGenerator,
    DictInDict,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy, get_size2::GetSize)]
pub enum UnparenthesizedNamedExprKind {
    SequenceIndex,
    SetLiteral,
    SetComprehension,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy, get_size2::GetSize)]
pub enum UnsupportedSyntaxErrorKind {
    Match,
    Walrus,
    ExceptStar,
    /// Represents the use of an unparenthesized named expression (`:=`) in a set literal, set
    /// comprehension, or sequence index before Python 3.10.
    ///
    /// ## Examples
    ///
    /// These are allowed on Python 3.10:
    ///
    /// ```python
    /// {x := 1, 2, 3}                 # set literal
    /// {last := x for x in range(3)}  # set comprehension
    /// lst[x := 1]                    # sequence index
    /// ```
    ///
    /// But on Python 3.9 the named expression needs to be parenthesized:
    ///
    /// ```python
    /// {(x := 1), 2, 3}                 # set literal
    /// {(last := x) for x in range(3)}  # set comprehension
    /// lst[(x := 1)]                    # sequence index
    /// ```
    ///
    /// However, unparenthesized named expressions are never allowed in slices:
    ///
    /// ```python
    /// lst[x:=1:-1]      # syntax error
    /// lst[1:x:=1]       # syntax error
    /// lst[1:3:x:=1]     # syntax error
    ///
    /// lst[(x:=1):-1]    # ok
    /// lst[1:(x:=1)]     # ok
    /// lst[1:3:(x:=1)]   # ok
    /// ```
    ///
    /// ## References
    ///
    /// - [Python 3.10 Other Language Changes](https://docs.python.org/3/whatsnew/3.10.html#other-language-changes)
    UnparenthesizedNamedExpr(UnparenthesizedNamedExprKind),

    /// Represents the use of a parenthesized keyword argument name after Python 3.8.
    ///
    /// ## Example
    ///
    /// From [BPO 34641] it sounds like this was only accidentally supported and was removed when
    /// noticed. Code like this used to be valid:
    ///
    /// ```python
    /// f((a)=1)
    /// ```
    ///
    /// After Python 3.8, you have to omit the parentheses around `a`:
    ///
    /// ```python
    /// f(a=1)
    /// ```
    ///
    /// [BPO 34641]: https://github.com/python/cpython/issues/78822
    ParenthesizedKeywordArgumentName,

    /// Represents the use of unparenthesized tuple unpacking in a `return` statement or `yield`
    /// expression before Python 3.8.
    ///
    /// ## Examples
    ///
    /// Before Python 3.8, this syntax was allowed:
    ///
    /// ```python
    /// rest = (4, 5, 6)
    ///
    /// def f():
    ///     t = 1, 2, 3, *rest
    ///     return t
    ///
    /// def g():
    ///     t = 1, 2, 3, *rest
    ///     yield t
    /// ```
    ///
    /// But this was not:
    ///
    /// ```python
    /// rest = (4, 5, 6)
    ///
    /// def f():
    ///     return 1, 2, 3, *rest
    ///
    /// def g():
    ///     yield 1, 2, 3, *rest
    /// ```
    ///
    /// Instead, parentheses were required in the `return` and `yield` cases:
    ///
    /// ```python
    /// rest = (4, 5, 6)
    ///
    /// def f():
    ///     return (1, 2, 3, *rest)
    ///
    /// def g():
    ///     yield (1, 2, 3, *rest)
    /// ```
    ///
    /// This was reported in [BPO 32117] and updated in Python 3.8 to allow the unparenthesized
    /// form.
    ///
    /// [BPO 32117]: https://github.com/python/cpython/issues/76298
    StarTuple(StarTupleKind),

    /// Represents the use of a "relaxed" [PEP 614] decorator before Python 3.9.
    ///
    /// ## Examples
    ///
    /// Prior to Python 3.9, decorators were defined to be [`dotted_name`]s, optionally followed by
    /// an argument list. For example:
    ///
    /// ```python
    /// @buttons.clicked.connect
    /// def foo(): ...
    ///
    /// @buttons.clicked.connect(1, 2, 3)
    /// def foo(): ...
    /// ```
    ///
    /// As pointed out in the PEP, this prevented reasonable extensions like subscripts:
    ///
    /// ```python
    /// buttons = [QPushButton(f'Button {i}') for i in range(10)]
    ///
    /// @buttons[0].clicked.connect
    /// def spam(): ...
    /// ```
    ///
    /// Python 3.9 removed these restrictions and expanded the [decorator grammar] to include any
    /// assignment expression and include cases like the example above.
    ///
    /// [PEP 614]: https://peps.python.org/pep-0614/
    /// [`dotted_name`]: https://docs.python.org/3.8/reference/compound_stmts.html#grammar-token-dotted-name
    /// [decorator grammar]: https://docs.python.org/3/reference/compound_stmts.html#grammar-token-python-grammar-decorator
    RelaxedDecorator(RelaxedDecoratorError),

    /// Represents the use of a [PEP 570] positional-only parameter before Python 3.8.
    ///
    /// ## Examples
    ///
    /// Python 3.8 added the `/` syntax for marking preceding parameters as positional-only:
    ///
    /// ```python
    /// def foo(a, b, /, c): ...
    /// ```
    ///
    /// This means `a` and `b` in this case can only be provided by position, not by name. In other
    /// words, this code results in a `TypeError` at runtime:
    ///
    /// ```pycon
    /// >>> def foo(a, b, /, c): ...
    /// ...
    /// >>> foo(a=1, b=2, c=3)
    /// Traceback (most recent call last):
    ///   File "<python-input-3>", line 1, in <module>
    ///     foo(a=1, b=2, c=3)
    ///     ~~~^^^^^^^^^^^^^^^
    /// TypeError: foo() got some positional-only arguments passed as keyword arguments: 'a, b'
    /// ```
    ///
    /// [PEP 570]: https://peps.python.org/pep-0570/
    PositionalOnlyParameter,

    /// Represents the use of a [type parameter list] before Python 3.12.
    ///
    /// ## Examples
    ///
    /// Before Python 3.12, generic parameters had to be declared separately using a class like
    /// [`typing.TypeVar`], which could then be used in a function or class definition:
    ///
    /// ```python
    /// from typing import Generic, TypeVar
    ///
    /// T = TypeVar("T")
    ///
    /// def f(t: T): ...
    /// class C(Generic[T]): ...
    /// ```
    ///
    /// [PEP 695], included in Python 3.12, introduced the new type parameter syntax, which allows
    /// these to be written more compactly and without a separate type variable:
    ///
    /// ```python
    /// def f[T](t: T): ...
    /// class C[T]: ...
    /// ```
    ///
    /// [type parameter list]: https://docs.python.org/3/reference/compound_stmts.html#type-parameter-lists
    /// [PEP 695]: https://peps.python.org/pep-0695/
    /// [`typing.TypeVar`]: https://docs.python.org/3/library/typing.html#typevar
    TypeParameterList,
    LazyImportStatement,
    TypeAliasStatement,
    TypeParamDefault,

    /// Represents the use of a [PEP 701] f-string before Python 3.12.
    ///
    /// ## Examples
    ///
    /// As described in the PEP, each of these cases were invalid before Python 3.12:
    ///
    /// ```python
    /// # nested quotes
    /// f'Magic wand: { bag['wand'] }'
    ///
    /// # escape characters
    /// f"{'\n'.join(a)}"
    ///
    /// # comments
    /// f'''A complex trick: {
    ///     bag['bag']  # recursive bags!
    /// }'''
    ///
    /// # line breaks in a non-triple-quoted replacement field
    /// f"{
    ///     1
    /// }"
    ///
    /// # arbitrary nesting
    /// f"{f"{f"{f"{f"{f"{1+1}"}"}"}"}"}"
    /// ```
    ///
    /// These restrictions were lifted in Python 3.12, meaning that all of these examples are now
    /// valid.
    ///
    /// [PEP 701]: https://peps.python.org/pep-0701/
    Pep701FString(FStringKind),

    /// Represents the use of a parenthesized `with` item before Python 3.9.
    ///
    /// ## Examples
    ///
    /// As described in [BPO 12782], `with` uses like this were not allowed on Python 3.8:
    ///
    /// ```python
    /// with (open("a_really_long_foo") as foo,
    ///       open("a_really_long_bar") as bar):
    ///     pass
    /// ```
    ///
    /// because parentheses were not allowed within the `with` statement itself (see [this comment]
    /// in particular). However, parenthesized expressions were still allowed, including the cases
    /// below, so the issue can be pretty subtle and relates specifically to parenthesized items
    /// with `as` bindings.
    ///
    /// ```python
    /// with (foo, bar): ...  # okay
    /// with (
    ///   open('foo.txt')) as foo: ...  # also okay
    /// with (
    ///   foo,
    ///   bar,
    ///   baz,
    /// ): ...  # also okay, just a tuple
    /// with (
    ///   foo,
    ///   bar,
    ///   baz,
    /// ) as tup: ...  # also okay, binding the tuple
    /// ```
    ///
    /// This restriction was lifted in 3.9 but formally included in the [release notes] for 3.10.
    ///
    /// [BPO 12782]: https://github.com/python/cpython/issues/56991
    /// [this comment]: https://github.com/python/cpython/issues/56991#issuecomment-1093555141
    /// [release notes]: https://docs.python.org/3/whatsnew/3.10.html#summary-release-highlights
    ParenthesizedContextManager,

    /// Represents the use of a [PEP 646] star expression in an index.
    ///
    /// ## Examples
    ///
    /// Before Python 3.11, star expressions were not allowed in index/subscript operations (within
    /// square brackets). This restriction was lifted in [PEP 646] to allow for star-unpacking of
    /// `typing.TypeVarTuple`s, also added in Python 3.11. As such, this is the primary motivating
    /// example from the PEP:
    ///
    /// ```python
    /// from typing import TypeVar, TypeVarTuple
    ///
    /// DType = TypeVar('DType')
    /// Shape = TypeVarTuple('Shape')
    ///
    /// class Array(Generic[DType, *Shape]): ...
    /// ```
    ///
    /// But it applies to simple indexing as well:
    ///
    /// ```python
    /// vector[*x]
    /// array[a, *b]
    /// ```
    ///
    /// [PEP 646]: https://peps.python.org/pep-0646/#change-1-star-expressions-in-indexes
    StarExpressionInIndex,

    /// Represents the use of a [PEP 646] star annotations in a function definition.
    ///
    /// ## Examples
    ///
    /// Before Python 3.11, star annotations were not allowed in function definitions. This
    /// restriction was lifted in [PEP 646] to allow type annotations for `typing.TypeVarTuple`,
    /// also added in Python 3.11:
    ///
    /// ```python
    /// from typing import TypeVarTuple
    ///
    /// Ts = TypeVarTuple('Ts')
    ///
    /// def foo(*args: *Ts): ...
    /// ```
    ///
    /// Unlike [`UnsupportedSyntaxErrorKind::StarExpressionInIndex`], this does not include any
    /// other annotation positions:
    ///
    /// ```python
    /// x: *Ts                # Syntax error
    /// def foo(x: *Ts): ...  # Syntax error
    /// ```
    ///
    /// [PEP 646]: https://peps.python.org/pep-0646/#change-2-args-as-a-typevartuple
    StarAnnotation,

    /// Represents the use of iterable or dictionary unpacking inside a comprehension before Python
    /// 3.15.
    ///
    /// ## Examples
    ///
    /// Before Python 3.15, comprehensions could not use iterable or dictionary unpacking in their
    /// element expression:
    ///
    /// ```python
    /// [*x for x in y]  # SyntaxError
    /// {*x for x in y}  # SyntaxError
    /// (*x for x in y)  # SyntaxError
    /// {**d for d in dicts}  # SyntaxError
    /// ```
    ///
    /// Starting with Python 3.15, [PEP 798] allows unpacking within comprehensions:
    ///
    /// ```python
    /// [*x for x in y]
    /// {*x for x in y}
    /// (*x for x in y)
    /// {**d for d in dicts}
    /// ```
    ///
    /// [PEP 798]: https://peps.python.org/pep-0798/
    UnpackingInComprehension(ComprehensionUnpackingKind),

    /// Represents the use of tuple unpacking in a `for` statement iterator clause before Python
    /// 3.9.
    ///
    /// ## Examples
    ///
    /// Like [`UnsupportedSyntaxErrorKind::StarTuple`] in `return` and `yield` statements, prior to
    /// Python 3.9, tuple unpacking in the iterator clause of a `for` statement required
    /// parentheses:
    ///
    /// ```python
    /// # valid on Python 3.8 and earlier
    /// for i in (*a, *b): ...
    /// ```
    ///
    /// Omitting the parentheses was invalid:
    ///
    /// ```python
    /// for i in *a, *b: ...  # SyntaxError
    /// ```
    ///
    /// This was changed as part of the [PEG parser rewrite] included in Python 3.9 but not
    /// documented directly until the [Python 3.11 release].
    ///
    /// [PEG parser rewrite]: https://peps.python.org/pep-0617/
    /// [Python 3.11 release]: https://docs.python.org/3/whatsnew/3.11.html#other-language-changes
    UnparenthesizedUnpackInFor,
    /// Represents the use of multiple exception names in an except clause without an `as` binding, before Python 3.14.
    ///
    /// ## Examples
    /// Before Python 3.14, catching multiple exceptions required
    /// parentheses like so:
    ///
    /// ```python
    /// try:
    ///     ...
    /// except (ExceptionA, ExceptionB, ExceptionC):
    ///     ...
    /// ```
    ///
    /// Starting with Python 3.14, thanks to [PEP 758], it was permitted
    /// to omit the parentheses:
    ///
    /// ```python
    /// try:
    ///     ...
    /// except ExceptionA, ExceptionB, ExceptionC:
    ///     ...
    /// ```
    ///
    /// However, parentheses are still required in the presence of an `as`:
    ///
    /// ```python
    /// try:
    ///     ...
    /// except (ExceptionA, ExceptionB, ExceptionC) as e:
    ///     ...
    /// ```
    ///
    ///
    /// [PEP 758]: https://peps.python.org/pep-0758/
    UnparenthesizedExceptionTypes,
    /// Represents the use of a template string (t-string)
    /// literal prior to the implementation of [PEP 750]
    /// in Python 3.14.
    ///
    /// [PEP 750]: https://peps.python.org/pep-0750/
    TemplateStrings,

    /// Represents the use of a unary plus in a `match` literal pattern before Python 3.15.
    ///
    /// Before 3.15, unary minus was allowed but not plus:
    ///
    /// ```python
    /// match foo:
    ///     case -1: ...  # okay
    ///     case +1: ...  # error before 3.15
    /// ```
    UnaryPlusMatchPattern,
}

impl Display for UnsupportedSyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match self.kind {
            UnsupportedSyntaxErrorKind::Match => "cannot use `match` statement",
            UnsupportedSyntaxErrorKind::Walrus => "cannot use named assignment expression (`:=`)",
            UnsupportedSyntaxErrorKind::ExceptStar => "cannot use `except*`",
            UnsupportedSyntaxErrorKind::UnparenthesizedNamedExpr(
                UnparenthesizedNamedExprKind::SequenceIndex,
            ) => "cannot use unparenthesized assignment expression in a sequence index",
            UnsupportedSyntaxErrorKind::UnparenthesizedNamedExpr(
                UnparenthesizedNamedExprKind::SetLiteral,
            ) => "cannot use unparenthesized assignment expression as an element in a set literal",
            UnsupportedSyntaxErrorKind::UnparenthesizedNamedExpr(
                UnparenthesizedNamedExprKind::SetComprehension,
            ) => {
                "cannot use unparenthesized assignment expression as an element in a set comprehension"
            }
            UnsupportedSyntaxErrorKind::ParenthesizedKeywordArgumentName => {
                "cannot use parenthesized keyword argument name"
            }
            UnsupportedSyntaxErrorKind::StarTuple(StarTupleKind::Return) => {
                "cannot use iterable unpacking in return statements"
            }
            UnsupportedSyntaxErrorKind::StarTuple(StarTupleKind::Yield) => {
                "cannot use iterable unpacking in yield expressions"
            }
            UnsupportedSyntaxErrorKind::RelaxedDecorator(relaxed_decorator_error) => {
                return match relaxed_decorator_error {
                    RelaxedDecoratorError::CallExpression => {
                        write!(
                            f,
                            "cannot use a call expression in a decorator on Python {} \
                            unless it is the top-level expression or it occurs \
                            in the argument list of a top-level call expression \
                            (relaxed decorator syntax was {changed})",
                            self.target_version,
                            changed = self.kind.changed_version(),
                        )
                    }
                    RelaxedDecoratorError::Other(description) => write!(
                        f,
                        "cannot use {description} outside function call arguments in a decorator on Python {} \
                        (syntax was {changed})",
                        self.target_version,
                        changed = self.kind.changed_version(),
                    ),
                };
            }
            UnsupportedSyntaxErrorKind::PositionalOnlyParameter => {
                "cannot use positional-only parameter separator"
            }
            UnsupportedSyntaxErrorKind::TypeParameterList => "cannot use type parameter lists",
            UnsupportedSyntaxErrorKind::LazyImportStatement => "cannot use `lazy` import statement",
            UnsupportedSyntaxErrorKind::TypeAliasStatement => "cannot use `type` alias statement",
            UnsupportedSyntaxErrorKind::TypeParamDefault => {
                "cannot set default type for a type parameter"
            }
            UnsupportedSyntaxErrorKind::Pep701FString(FStringKind::Backslash) => {
                "cannot use an escape sequence (backslash) in f-strings"
            }
            UnsupportedSyntaxErrorKind::Pep701FString(FStringKind::Comment) => {
                "cannot use comments in f-strings"
            }
            UnsupportedSyntaxErrorKind::Pep701FString(FStringKind::LineBreak) => {
                "cannot use line breaks in non-triple-quoted f-string replacement fields"
            }
            UnsupportedSyntaxErrorKind::Pep701FString(FStringKind::NestedQuote) => {
                "cannot reuse outer quote character in f-strings"
            }
            UnsupportedSyntaxErrorKind::ParenthesizedContextManager => {
                "cannot use parentheses within a `with` statement"
            }
            UnsupportedSyntaxErrorKind::StarExpressionInIndex => {
                "cannot use star expression in index"
            }
            UnsupportedSyntaxErrorKind::StarAnnotation => "cannot use star annotation",
            UnsupportedSyntaxErrorKind::UnpackingInComprehension(
                ComprehensionUnpackingKind::IterableInList,
            ) => "cannot use iterable unpacking in a list comprehension",
            UnsupportedSyntaxErrorKind::UnpackingInComprehension(
                ComprehensionUnpackingKind::IterableInSet,
            ) => "cannot use iterable unpacking in a set comprehension",
            UnsupportedSyntaxErrorKind::UnpackingInComprehension(
                ComprehensionUnpackingKind::IterableInGenerator,
            ) => "cannot use iterable unpacking in a generator expression",
            UnsupportedSyntaxErrorKind::UnpackingInComprehension(
                ComprehensionUnpackingKind::DictInDict,
            ) => "cannot use dictionary unpacking in a dict comprehension",
            UnsupportedSyntaxErrorKind::UnparenthesizedUnpackInFor => {
                "cannot use iterable unpacking in `for` statements"
            }
            UnsupportedSyntaxErrorKind::UnparenthesizedExceptionTypes => {
                "multiple exception types must be parenthesized"
            }
            UnsupportedSyntaxErrorKind::TemplateStrings => "cannot use t-strings",
            UnsupportedSyntaxErrorKind::UnaryPlusMatchPattern => {
                "unary '+' is not allowed in a literal pattern"
            }
        };

        write!(
            f,
            "{kind} on Python {} (syntax was {changed})",
            self.target_version,
            changed = self.kind.changed_version(),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, get_size2::GetSize)]
pub enum RelaxedDecoratorError {
    CallExpression,
    Other(&'static str),
}

/// Represents the kind of change in Python syntax between versions.
enum Change {
    Added(PythonVersion),
    Removed(PythonVersion),
}

impl Display for Change {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Change::Added(version) => write!(f, "added in Python {version}"),
            Change::Removed(version) => write!(f, "removed in Python {version}"),
        }
    }
}

impl UnsupportedSyntaxErrorKind {
    /// Returns the Python version when the syntax associated with this error was changed, and the
    /// type of [`Change`] (added or removed).
    const fn changed_version(self) -> Change {
        match self {
            UnsupportedSyntaxErrorKind::Match => Change::Added(PythonVersion::PY310),
            UnsupportedSyntaxErrorKind::Walrus => Change::Added(PythonVersion::PY38),
            UnsupportedSyntaxErrorKind::ExceptStar => Change::Added(PythonVersion::PY311),
            UnsupportedSyntaxErrorKind::UnparenthesizedNamedExpr(_) => {
                Change::Added(PythonVersion::PY39)
            }
            UnsupportedSyntaxErrorKind::StarTuple(_) => Change::Added(PythonVersion::PY38),
            UnsupportedSyntaxErrorKind::RelaxedDecorator { .. } => {
                Change::Added(PythonVersion::PY39)
            }
            UnsupportedSyntaxErrorKind::PositionalOnlyParameter => {
                Change::Added(PythonVersion::PY38)
            }
            UnsupportedSyntaxErrorKind::ParenthesizedKeywordArgumentName => {
                Change::Removed(PythonVersion::PY38)
            }
            UnsupportedSyntaxErrorKind::TypeParameterList => Change::Added(PythonVersion::PY312),
            UnsupportedSyntaxErrorKind::LazyImportStatement => Change::Added(PythonVersion::PY315),
            UnsupportedSyntaxErrorKind::TypeAliasStatement => Change::Added(PythonVersion::PY312),
            UnsupportedSyntaxErrorKind::TypeParamDefault => Change::Added(PythonVersion::PY313),
            UnsupportedSyntaxErrorKind::Pep701FString(_) => Change::Added(PythonVersion::PY312),
            UnsupportedSyntaxErrorKind::ParenthesizedContextManager => {
                Change::Added(PythonVersion::PY39)
            }
            UnsupportedSyntaxErrorKind::StarExpressionInIndex => {
                Change::Added(PythonVersion::PY311)
            }
            UnsupportedSyntaxErrorKind::StarAnnotation => Change::Added(PythonVersion::PY311),
            UnsupportedSyntaxErrorKind::UnpackingInComprehension(_) => {
                Change::Added(PythonVersion::PY315)
            }
            UnsupportedSyntaxErrorKind::UnparenthesizedUnpackInFor => {
                Change::Added(PythonVersion::PY39)
            }
            UnsupportedSyntaxErrorKind::UnparenthesizedExceptionTypes => {
                Change::Added(PythonVersion::PY314)
            }
            UnsupportedSyntaxErrorKind::TemplateStrings => Change::Added(PythonVersion::PY314),
            UnsupportedSyntaxErrorKind::UnaryPlusMatchPattern => {
                Change::Added(PythonVersion::PY315)
            }
        }
    }

    /// Returns whether or not this kind of syntax is unsupported on `target_version`.
    pub(crate) fn is_unsupported(self, target_version: PythonVersion) -> bool {
        match self.changed_version() {
            Change::Added(version) => target_version < version,
            Change::Removed(version) => target_version >= version,
        }
    }

    /// Returns `true` if this kind of syntax is supported on `target_version`.
    pub(crate) fn is_supported(self, target_version: PythonVersion) -> bool {
        !self.is_unsupported(target_version)
    }
}

#[cfg(target_pointer_width = "64")]
mod sizes {
    use crate::error::{LexicalError, LexicalErrorType};
    use static_assertions::assert_eq_size;

    assert_eq_size!(LexicalErrorType, [u8; 24]);
    assert_eq_size!(LexicalError, [u8; 32]);
}

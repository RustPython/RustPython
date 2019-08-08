//! Implement abstract syntax tree nodes for the python language.
//!
//! Roughly equivalent to this: https://docs.python.org/3/library/ast.html

pub use crate::location::Location;
use num_bigint::BigInt;

/*
#[derive(Debug)]

#[derive(Debug)]
pub struct Node {
    pub location: Location,
}
*/

#[allow(clippy::large_enum_variant)]
#[derive(Debug, PartialEq)]
pub enum Top {
    Program(Program),
    Statement(Vec<Statement>),
    Expression(Expression),
}

#[derive(Debug, PartialEq)]
pub struct Program {
    pub statements: Vec<Statement>,
}

#[derive(Debug, PartialEq)]
pub struct ImportSymbol {
    pub symbol: String,
    pub alias: Option<String>,
}

#[derive(Debug, PartialEq)]
pub struct Located<T> {
    pub location: Location,
    pub node: T,
}

pub type Statement = Located<StatementType>;

/// Abstract syntax tree nodes for python statements.
#[derive(Debug, PartialEq)]
pub enum StatementType {
    Break,
    Continue,
    Return {
        value: Option<Expression>,
    },
    Import {
        names: Vec<ImportSymbol>,
    },
    ImportFrom {
        level: usize,
        module: Option<String>,
        names: Vec<ImportSymbol>,
    },
    Pass,
    Assert {
        test: Expression,
        msg: Option<Expression>,
    },
    Delete {
        targets: Vec<Expression>,
    },
    Assign {
        targets: Vec<Expression>,
        value: Expression,
    },
    AugAssign {
        target: Box<Expression>,
        op: Operator,
        value: Box<Expression>,
    },
    Expression {
        expression: Expression,
    },
    Global {
        names: Vec<String>,
    },
    Nonlocal {
        names: Vec<String>,
    },
    If {
        test: Expression,
        body: Vec<Statement>,
        orelse: Option<Vec<Statement>>,
    },
    While {
        test: Expression,
        body: Vec<Statement>,
        orelse: Option<Vec<Statement>>,
    },
    With {
        is_async: bool,
        items: Vec<WithItem>,
        body: Vec<Statement>,
    },
    For {
        is_async: bool,
        target: Box<Expression>,
        iter: Box<Expression>,
        body: Vec<Statement>,
        orelse: Option<Vec<Statement>>,
    },
    Raise {
        exception: Option<Expression>,
        cause: Option<Expression>,
    },
    Try {
        body: Vec<Statement>,
        handlers: Vec<ExceptHandler>,
        orelse: Option<Vec<Statement>>,
        finalbody: Option<Vec<Statement>>,
    },
    ClassDef {
        name: String,
        body: Vec<Statement>,
        bases: Vec<Expression>,
        keywords: Vec<Keyword>,
        decorator_list: Vec<Expression>,
    },
    FunctionDef {
        is_async: bool,
        name: String,
        args: Box<Parameters>,
        body: Vec<Statement>,
        decorator_list: Vec<Expression>,
        returns: Option<Expression>,
    },
}

#[derive(Debug, PartialEq)]
pub struct WithItem {
    pub context_expr: Expression,
    pub optional_vars: Option<Expression>,
}

pub type Expression = Located<ExpressionType>;

#[derive(Debug, PartialEq)]
pub enum ExpressionType {
    BoolOp {
        a: Box<Expression>,
        op: BooleanOperator,
        b: Box<Expression>,
    },
    Binop {
        a: Box<Expression>,
        op: Operator,
        b: Box<Expression>,
    },
    Subscript {
        a: Box<Expression>,
        b: Box<Expression>,
    },
    Unop {
        op: UnaryOperator,
        a: Box<Expression>,
    },
    Await {
        value: Box<Expression>,
    },
    Yield {
        value: Option<Box<Expression>>,
    },
    YieldFrom {
        value: Box<Expression>,
    },
    Compare {
        vals: Vec<Expression>,
        ops: Vec<Comparison>,
    },
    Attribute {
        value: Box<Expression>,
        name: String,
    },
    Call {
        function: Box<Expression>,
        args: Vec<Expression>,
        keywords: Vec<Keyword>,
    },
    Number {
        value: Number,
    },
    List {
        elements: Vec<Expression>,
    },
    Tuple {
        elements: Vec<Expression>,
    },
    Dict {
        elements: Vec<(Option<Expression>, Expression)>,
    },
    Set {
        elements: Vec<Expression>,
    },
    Comprehension {
        kind: Box<ComprehensionKind>,
        generators: Vec<Comprehension>,
    },
    Starred {
        value: Box<Expression>,
    },
    Slice {
        elements: Vec<Expression>,
    },
    String {
        value: StringGroup,
    },
    Bytes {
        value: Vec<u8>,
    },
    Identifier {
        name: String,
    },
    Lambda {
        args: Box<Parameters>,
        body: Box<Expression>,
    },
    IfExpression {
        test: Box<Expression>,
        body: Box<Expression>,
        orelse: Box<Expression>,
    },
    True,
    False,
    None,
    Ellipsis,
}

impl Expression {
    /// Returns a short name for the node suitable for use in error messages.
    pub fn name(&self) -> &'static str {
        use self::ExpressionType::*;
        use self::StringGroup::*;

        match &self.node {
            BoolOp { .. } | Binop { .. } | Unop { .. } => "operator",
            Subscript { .. } => "subscript",
            Await { .. } => "await expression",
            Yield { .. } | YieldFrom { .. } => "yield expression",
            Compare { .. } => "comparison",
            Attribute { .. } => "attribute",
            Call { .. } => "function call",
            Number { .. }
            | String {
                value: Constant { .. },
            }
            | Bytes { .. } => "literal",
            List { .. } => "list",
            Tuple { .. } => "tuple",
            Dict { .. } => "dict display",
            Set { .. } => "set display",
            Comprehension { kind, .. } => match **kind {
                ComprehensionKind::List { .. } => "list comprehension",
                ComprehensionKind::Dict { .. } => "dict comprehension",
                ComprehensionKind::Set { .. } => "set comprehension",
                ComprehensionKind::GeneratorExpression { .. } => "generator expression",
            },
            Starred { .. } => "starred",
            Slice { .. } => "slice",
            String {
                value: Joined { .. },
            }
            | String {
                value: FormattedValue { .. },
            } => "f-string expression",
            Identifier { .. } => "named expression",
            Lambda { .. } => "lambda",
            IfExpression { .. } => "conditional expression",
            True | False | None => "keyword",
            Ellipsis => "ellipsis",
        }
    }
}

/*
 * In cpython this is called arguments, but we choose parameters to
 * distinguish between function parameters and actual call arguments.
 */
#[derive(Debug, PartialEq, Default)]
pub struct Parameters {
    pub args: Vec<Parameter>,
    pub kwonlyargs: Vec<Parameter>,
    pub vararg: Varargs, // Optionally we handle optionally named '*args' or '*'
    pub kwarg: Varargs,
    pub defaults: Vec<Expression>,
    pub kw_defaults: Vec<Option<Expression>>,
}

#[derive(Debug, PartialEq, Default)]
pub struct Parameter {
    pub location: Location,
    pub arg: String,
    pub annotation: Option<Box<Expression>>,
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, PartialEq)]
pub enum ComprehensionKind {
    GeneratorExpression { element: Expression },
    List { element: Expression },
    Set { element: Expression },
    Dict { key: Expression, value: Expression },
}

#[derive(Debug, PartialEq)]
pub struct Comprehension {
    pub location: Location,
    pub target: Expression,
    pub iter: Expression,
    pub ifs: Vec<Expression>,
}

#[derive(Debug, PartialEq)]
pub struct Keyword {
    pub name: Option<String>,
    pub value: Expression,
}

#[derive(Debug, PartialEq)]
pub struct ExceptHandler {
    pub location: Location,
    pub typ: Option<Expression>,
    pub name: Option<String>,
    pub body: Vec<Statement>,
}

#[derive(Debug, PartialEq)]
pub enum Operator {
    Add,
    Sub,
    Mult,
    MatMult,
    Div,
    Mod,
    Pow,
    LShift,
    RShift,
    BitOr,
    BitXor,
    BitAnd,
    FloorDiv,
}

#[derive(Debug, PartialEq)]
pub enum BooleanOperator {
    And,
    Or,
}

#[derive(Debug, PartialEq)]
pub enum UnaryOperator {
    Pos,
    Neg,
    Not,
    Inv,
}

#[derive(Debug, PartialEq)]
pub enum Comparison {
    Equal,
    NotEqual,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
    In,
    NotIn,
    Is,
    IsNot,
}

#[derive(Debug, PartialEq)]
pub enum Number {
    Integer { value: BigInt },
    Float { value: f64 },
    Complex { real: f64, imag: f64 },
}

/// Transforms a value prior to formatting it.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum ConversionFlag {
    /// Converts by calling `str(<value>)`.
    Str,
    /// Converts by calling `ascii(<value>)`.
    Ascii,
    /// Converts by calling `repr(<value>)`.
    Repr,
}

#[derive(Debug, PartialEq)]
pub enum StringGroup {
    Constant {
        value: String,
    },
    FormattedValue {
        value: Box<Expression>,
        conversion: Option<ConversionFlag>,
        spec: String,
    },
    Joined {
        values: Vec<StringGroup>,
    },
}

#[derive(Debug, PartialEq)]
pub enum Varargs {
    None,
    Unnamed,
    Named(Parameter),
}

impl Default for Varargs {
    fn default() -> Varargs {
        Varargs::None
    }
}

impl From<Option<Option<Parameter>>> for Varargs {
    fn from(opt: Option<Option<Parameter>>) -> Varargs {
        match opt {
            Some(inner_opt) => match inner_opt {
                Some(param) => Varargs::Named(param),
                None => Varargs::Unnamed,
            },
            None => Varargs::None,
        }
    }
}

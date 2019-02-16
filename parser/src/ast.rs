//! Implement abstract syntax tree nodes for the python language.
//!
//! Roughly equivalent to this: https://docs.python.org/3/library/ast.html

pub use super::lexer::Location;
use num_bigint::BigInt;
/*
#[derive(Debug)]

#[derive(Debug)]
pub struct Node {
    pub location: Location,
}
*/

#[derive(Debug, PartialEq)]
pub enum Top {
    Program(Program),
    Statement(LocatedStatement),
    Expression(Expression),
}

#[derive(Debug, PartialEq)]
pub struct Program {
    pub statements: Vec<LocatedStatement>,
}

#[derive(Debug, PartialEq)]
pub struct SingleImport {
    pub module: String,
    // (symbol name in module, name it should be assigned locally)
    pub symbol: Option<String>,
    pub alias: Option<String>,
}

#[derive(Debug, PartialEq)]
pub struct Located<T> {
    pub location: Location,
    pub node: T,
}

pub type LocatedStatement = Located<Statement>;

/// Abstract syntax tree nodes for python statements.
#[derive(Debug, PartialEq)]
pub enum Statement {
    Break,
    Continue,
    Return {
        value: Option<Vec<Expression>>,
    },
    Import {
        import_parts: Vec<SingleImport>,
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
        body: Vec<LocatedStatement>,
        orelse: Option<Vec<LocatedStatement>>,
    },
    While {
        test: Expression,
        body: Vec<LocatedStatement>,
        orelse: Option<Vec<LocatedStatement>>,
    },
    With {
        items: Vec<WithItem>,
        body: Vec<LocatedStatement>,
    },
    For {
        target: Expression,
        iter: Vec<Expression>,
        body: Vec<LocatedStatement>,
        orelse: Option<Vec<LocatedStatement>>,
    },
    Raise {
        exception: Option<Expression>,
        cause: Option<Expression>,
    },
    Try {
        body: Vec<LocatedStatement>,
        handlers: Vec<ExceptHandler>,
        orelse: Option<Vec<LocatedStatement>>,
        finalbody: Option<Vec<LocatedStatement>>,
    },
    ClassDef {
        name: String,
        body: Vec<LocatedStatement>,
        bases: Vec<Expression>,
        keywords: Vec<Keyword>,
        decorator_list: Vec<Expression>,
        // TODO: docstring: String,
    },
    FunctionDef {
        name: String,
        args: Parameters,
        // docstring: String,
        body: Vec<LocatedStatement>,
        decorator_list: Vec<Expression>,
    },
}

#[derive(Debug, PartialEq)]
pub struct WithItem {
    pub context_expr: Expression,
    pub optional_vars: Option<Expression>,
}

#[derive(Debug, PartialEq)]
pub enum Expression {
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
    Yield {
        value: Option<Box<Expression>>,
    },
    YieldFrom {
        value: Box<Expression>,
    },
    Compare {
        a: Box<Expression>,
        op: Comparison,
        b: Box<Expression>,
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
        elements: Vec<(Expression, Expression)>,
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
        args: Parameters,
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
}

/*
 * In cpython this is called arguments, but we choose parameters to
 * distinguish between function parameters and actual call arguments.
 */
#[derive(Debug, PartialEq, Default)]
pub struct Parameters {
    pub args: Vec<String>,
    pub kwonlyargs: Vec<String>,
    pub vararg: Option<Option<String>>, // Optionally we handle optionally named '*args' or '*'
    pub kwarg: Option<Option<String>>,
    pub defaults: Vec<Expression>,
    pub kw_defaults: Vec<Option<Expression>>,
}

#[derive(Debug, PartialEq)]
pub enum ComprehensionKind {
    GeneratorExpression { element: Expression },
    List { element: Expression },
    Set { element: Expression },
    Dict { key: Expression, value: Expression },
}

#[derive(Debug, PartialEq)]
pub struct Comprehension {
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
    pub typ: Option<Expression>,
    pub name: Option<String>,
    pub body: Vec<LocatedStatement>,
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

#[derive(Debug, PartialEq)]
pub enum StringGroup {
    Constant {
        value: String,
    },
    FormattedValue {
        value: Box<Expression>,
        spec: String,
    },
    Joined {
        values: Vec<StringGroup>,
    },
}

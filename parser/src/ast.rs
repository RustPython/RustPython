/*
 * Implement abstract syntax tree nodes for the python language.
 */

/*
#[derive(Debug)]
pub struct Location {
    pub row: i32,
    pub column: i32,
}

#[derive(Debug)]
pub struct Node {
    pub location: Location,
}
*/

#[derive(Debug, PartialEq)]
pub struct Program {
    pub statements: Vec<Statement>,
}

#[derive(Debug, PartialEq)]
pub enum Statement {
    Break,
    Continue,
    Return {
        value: Option<Vec<Expression>>,
    },
    Import {
        module: String,
        symbol: Option<String>,
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
        target: Expression,
        op: Operator,
        value: Expression,
    },
    Expression {
        expression: Expression,
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
        items: Expression,
        body: Vec<Statement>,
    },
    For {
        target: Vec<Expression>,
        iter: Vec<Expression>,
        body: Vec<Statement>,
        orelse: Option<Vec<Statement>>,
    },
    ClassDef {
        name: String,
        body: Vec<Statement>,
        // TODO: docstring: String,
    },
    FunctionDef {
        name: String,
        args: Vec<String>,
        // docstring: String,
        body: Vec<Statement>,
    },
}

#[derive(Debug, PartialEq, Clone)]
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
    },
    Number {
        value: i32,
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
    Slice {
        elements: Vec<Expression>,
    },
    String {
        value: String,
    },
    Identifier {
        name: String,
    },
    Lambda {
        args: Vec<String>,
        body: Box<Expression>,
    },
    True,
    False,
    None,
}

#[derive(Debug, PartialEq, Clone)]
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

#[derive(Debug, PartialEq, Clone)]
pub enum BooleanOperator {
    And,
    Or,
}

#[derive(Debug, PartialEq, Clone)]
pub enum UnaryOperator {
    Neg,
}

#[derive(Debug, PartialEq, Clone)]
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

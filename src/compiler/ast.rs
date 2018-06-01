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
        name: String,
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
    Expression {
        expression: Expression,
    },
    If {
        test: Expression,
        body: Vec<Statement>,
    },
    While {
        test: Expression,
        body: Vec<Statement>,
    },
    With {
        items: Expression,
        body: Vec<Statement>,
    },
    For {
        target: Vec<Expression>,
        iter: Vec<Expression>,
        body: Vec<Statement>,
        or_else: Option<Vec<Statement>>,
    },
    ClassDef {
        name: String,
        // TODO: docstring: String,
    },
    FunctionDef {
        name: String,
        // docstring: String,
        body: Vec<Statement>,
    },
}

#[derive(Debug, PartialEq)]
pub enum Expression {
    Binop {
        a: Box<Expression>,
        op: Operator,
        b: Box<Expression>,
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
    String {
        value: String,
    },
    Identifier {
        name: String,
    },
    True,
    False,
    None,
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

//! Implement abstract syntax tree (AST) nodes for the python language.
//!
//! Roughly equivalent to [the python AST](https://docs.python.org/3/library/ast.html)
//! Many AST nodes have a location attribute, to determine the sourcecode
//! location of the node.

pub use crate::location::Location;
use num_bigint::BigInt;

#[allow(clippy::large_enum_variant)]
#[derive(Debug, PartialEq)]
pub enum Top {
    Program(Program),
    Statement(Vec<Statement>),
    Expression(Expression),
}

#[derive(Debug, PartialEq)]
/// A full python program, it's a sequence of statements.
pub struct Program {
    pub statements: Suite,
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
pub type Suite = Vec<Statement>;

/// Abstract syntax tree nodes for python statements.
#[derive(Debug, PartialEq)]
pub enum StatementType {
    /// A [`break`](https://docs.python.org/3/reference/simple_stmts.html#the-break-statement) statement.
    Break,

    /// A [`continue`](https://docs.python.org/3/reference/simple_stmts.html#the-continue-statement) statement.
    Continue,

    /// A [`return`](https://docs.python.org/3/reference/simple_stmts.html#the-return-statement) statement.
    /// This is used to return from a function.
    Return { value: Option<Expression> },

    /// An [`import`](https://docs.python.org/3/reference/simple_stmts.html#the-import-statement) statement.
    Import { names: Vec<ImportSymbol> },

    /// An [`import` `from`](https://docs.python.org/3/reference/simple_stmts.html#the-import-statement) statement.
    ImportFrom {
        level: usize,
        module: Option<String>,
        names: Vec<ImportSymbol>,
    },

    /// A [`pass`](https://docs.python.org/3/reference/simple_stmts.html#pass) statement.
    Pass,

    /// An [`assert`](https://docs.python.org/3/reference/simple_stmts.html#the-assert-statement) statement.
    Assert {
        test: Expression,
        msg: Option<Expression>,
    },

    /// A `del` statement, to delete some variables.
    Delete { targets: Vec<Expression> },

    /// Variable assignment. Note that we can assign to multiple targets.
    Assign {
        targets: Vec<Expression>,
        value: Expression,
    },

    /// Augmented assignment.
    AugAssign {
        target: Box<Expression>,
        op: Operator,
        value: Box<Expression>,
    },

    /// A type annotated assignment.
    AnnAssign {
        target: Box<Expression>,
        annotation: Box<Expression>,
        value: Option<Expression>,
    },

    /// An expression used as a statement.
    Expression { expression: Expression },

    /// The [`global`](https://docs.python.org/3/reference/simple_stmts.html#the-global-statement) statement,
    /// to declare names as global variables.
    Global { names: Vec<String> },

    /// A [`nonlocal`](https://docs.python.org/3/reference/simple_stmts.html#the-nonlocal-statement) statement,
    /// to declare names a non-local variables.
    Nonlocal { names: Vec<String> },

    /// An [`if`](https://docs.python.org/3/reference/compound_stmts.html#the-if-statement) statement.
    If {
        test: Expression,
        body: Suite,
        orelse: Option<Suite>,
    },

    /// A [`while`](https://docs.python.org/3/reference/compound_stmts.html#the-while-statement) statement.
    While {
        test: Expression,
        body: Suite,
        orelse: Option<Suite>,
    },

    /// The [`with`](https://docs.python.org/3/reference/compound_stmts.html#the-with-statement) statement.
    With {
        is_async: bool,
        items: Vec<WithItem>,
        body: Suite,
    },

    /// A [`for`](https://docs.python.org/3/reference/compound_stmts.html#the-for-statement) statement.
    /// Contains the body of the loop, and the `else` clause.
    For {
        is_async: bool,
        target: Box<Expression>,
        iter: Box<Expression>,
        body: Suite,
        orelse: Option<Suite>,
    },

    /// A `raise` statement.
    Raise {
        exception: Option<Expression>,
        cause: Option<Expression>,
    },

    /// A [`try`](https://docs.python.org/3/reference/compound_stmts.html#the-try-statement) statement.
    Try {
        body: Suite,
        handlers: Vec<ExceptHandler>,
        orelse: Option<Suite>,
        finalbody: Option<Suite>,
    },

    /// A [class definition](https://docs.python.org/3/reference/compound_stmts.html#class-definitions).
    ClassDef {
        name: String,
        body: Suite,
        bases: Vec<Expression>,
        keywords: Vec<Keyword>,
        decorator_list: Vec<Expression>,
    },

    /// A [function definition](https://docs.python.org/3/reference/compound_stmts.html#function-definitions).
    /// Contains the name of the function, it's body
    /// some decorators and formal parameters to the function.
    FunctionDef {
        is_async: bool,
        name: String,
        args: Box<Parameters>,
        body: Suite,
        decorator_list: Vec<Expression>,
        returns: Option<Expression>,
    },
}

#[derive(Debug, PartialEq)]
pub struct WithItem {
    pub context_expr: Expression,
    pub optional_vars: Option<Expression>,
}

/// An expression at a given location in the sourcecode.
pub type Expression = Located<ExpressionType>;

/// A certain type of expression.
#[derive(Debug, PartialEq)]
pub enum ExpressionType {
    BoolOp {
        op: BooleanOperator,
        values: Vec<Expression>,
    },

    /// A binary operation on two operands.
    Binop {
        a: Box<Expression>,
        op: Operator,
        b: Box<Expression>,
    },

    /// Subscript operation.
    Subscript {
        a: Box<Expression>,
        b: Box<Expression>,
    },

    /// An unary operation.
    Unop {
        op: UnaryOperator,
        a: Box<Expression>,
    },

    /// An await expression.
    Await {
        value: Box<Expression>,
    },

    /// A yield expression.
    Yield {
        value: Option<Box<Expression>>,
    },

    // A yield from expression.
    YieldFrom {
        value: Box<Expression>,
    },

    /// A chained comparison. Note that in python you can use
    /// `1 < a < 10` for example.
    Compare {
        vals: Vec<Expression>,
        ops: Vec<Comparison>,
    },

    /// Attribute access in the form of `value.name`.
    Attribute {
        value: Box<Expression>,
        name: String,
    },

    /// A call expression.
    Call {
        function: Box<Expression>,
        args: Vec<Expression>,
        keywords: Vec<Keyword>,
    },

    /// A numeric literal.
    Number {
        value: Number,
    },

    /// A `list` literal value.
    List {
        elements: Vec<Expression>,
    },

    /// A `tuple` literal value.
    Tuple {
        elements: Vec<Expression>,
    },

    /// A `dict` literal value.
    /// For example: `{2: 'two', 3: 'three'}`
    Dict {
        elements: Vec<(Option<Expression>, Expression)>,
    },

    /// A `set` literal.
    Set {
        elements: Vec<Expression>,
    },

    Comprehension {
        kind: Box<ComprehensionKind>,
        generators: Vec<Comprehension>,
    },

    /// A starred expression.
    Starred {
        value: Box<Expression>,
    },

    /// A slice expression.
    Slice {
        elements: Vec<Expression>,
    },

    /// A string literal.
    String {
        value: StringGroup,
    },

    /// A bytes literal.
    Bytes {
        value: Vec<u8>,
    },

    /// An identifier, designating a certain variable or type.
    Identifier {
        name: String,
    },

    /// A `lambda` function expression.
    Lambda {
        args: Box<Parameters>,
        body: Box<Expression>,
    },

    /// An if-expression.
    IfExpression {
        test: Box<Expression>,
        body: Box<Expression>,
        orelse: Box<Expression>,
    },

    // A named expression
    NamedExpression {
        left: Box<Expression>,
        right: Box<Expression>,
    },

    /// The literal 'True'.
    True,

    /// The literal 'False'.
    False,

    // The literal `None`.
    None,

    /// The ellipsis literal `...`.
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
            NamedExpression { .. } => "named expression",
        }
    }
}

/// Formal parameters to a function.
///
/// In cpython this is called arguments, but we choose parameters to
/// distinguish between function parameters and actual call arguments.
#[derive(Debug, PartialEq, Default)]
pub struct Parameters {
    pub posonlyargs_count: usize,
    pub args: Vec<Parameter>,
    pub kwonlyargs: Vec<Parameter>,
    pub vararg: Varargs, // Optionally we handle optionally named '*args' or '*'
    pub kwarg: Varargs,
    pub defaults: Vec<Expression>,
    pub kw_defaults: Vec<Option<Expression>>,
}

/// A single formal parameter to a function.
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

/// A list/set/dict/generator compression.
#[derive(Debug, PartialEq)]
pub struct Comprehension {
    pub location: Location,
    pub target: Expression,
    pub iter: Expression,
    pub ifs: Vec<Expression>,
    pub is_async: bool,
}

#[derive(Debug, PartialEq)]
pub struct ArgumentList {
    pub args: Vec<Expression>,
    pub keywords: Vec<Keyword>,
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
    pub body: Suite,
}

/// An operator for a binary operation (an operation with two operands).
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

/// A boolean operation.
#[derive(Debug, PartialEq)]
pub enum BooleanOperator {
    And,
    Or,
}

/// An unary operator. This is an operation with only a single operand.
#[derive(Debug, PartialEq)]
pub enum UnaryOperator {
    Pos,
    Neg,
    Not,
    Inv,
}

/// A comparison operation.
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

/// A numeric literal.
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
        spec: Option<Box<StringGroup>>,
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

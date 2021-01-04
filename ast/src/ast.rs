//! Implement abstract syntax tree (AST) nodes for the python language.
//!
//! Roughly equivalent to [the python AST](https://docs.python.org/3/library/ast.html)
//! Many AST nodes have a location attribute, to determine the sourcecode
//! location of the node.

pub use crate::location::Location;
use num_bigint::BigInt;

#[allow(clippy::large_enum_variant)]
#[derive(Debug, PartialEq)]
pub enum Top<U = ()> {
    Program(Program<U>),
    Statement(Vec<Statement<U>>),
    Expression(Expression<U>),
}

#[derive(Debug, PartialEq)]
/// A full python program, it's a sequence of statements.
pub struct Program<U = ()> {
    pub statements: Suite<U>,
}

#[derive(Debug, PartialEq)]
pub struct ImportSymbol {
    pub symbol: String,
    pub alias: Option<String>,
}

#[derive(Debug, PartialEq)]
pub struct Located<T, U = ()> {
    pub location: Location,
    pub node: T,
    pub custom: U,
}

pub type Statement<U = ()> = Located<StatementType<U>, U>;
pub type Suite<U = ()> = Vec<Statement<U>>;

/// Abstract syntax tree nodes for python statements.
#[derive(Debug, PartialEq)]
pub enum StatementType<U = ()> {
    /// A [`break`](https://docs.python.org/3/reference/simple_stmts.html#the-break-statement) statement.
    Break,

    /// A [`continue`](https://docs.python.org/3/reference/simple_stmts.html#the-continue-statement) statement.
    Continue,

    /// A [`return`](https://docs.python.org/3/reference/simple_stmts.html#the-return-statement) statement.
    /// This is used to return from a function.
    Return { value: Option<Expression<U>> },

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
        test: Expression<U>,
        msg: Option<Expression<U>>,
    },

    /// A `del` statement, to delete some variables.
    Delete { targets: Vec<Expression<U>> },

    /// Variable assignment. Note that we can assign to multiple targets.
    Assign {
        targets: Vec<Expression<U>>,
        value: Expression<U>,
    },

    /// Augmented assignment.
    AugAssign {
        target: Box<Expression<U>>,
        op: Operator,
        value: Box<Expression<U>>,
    },

    /// A type annotated assignment.
    AnnAssign {
        target: Box<Expression<U>>,
        annotation: Box<Expression<U>>,
        value: Option<Expression<U>>,
    },

    /// An expression used as a statement.
    Expression { expression: Expression<U> },

    /// The [`global`](https://docs.python.org/3/reference/simple_stmts.html#the-global-statement) statement,
    /// to declare names as global variables.
    Global { names: Vec<String> },

    /// A [`nonlocal`](https://docs.python.org/3/reference/simple_stmts.html#the-nonlocal-statement) statement,
    /// to declare names a non-local variables.
    Nonlocal { names: Vec<String> },

    /// An [`if`](https://docs.python.org/3/reference/compound_stmts.html#the-if-statement) statement.
    If {
        test: Expression<U>,
        body: Suite<U>,
        orelse: Option<Suite<U>>,
    },

    /// A [`while`](https://docs.python.org/3/reference/compound_stmts.html#the-while-statement) statement.
    While {
        test: Expression<U>,
        body: Suite<U>,
        orelse: Option<Suite<U>>,
    },

    /// The [`with`](https://docs.python.org/3/reference/compound_stmts.html#the-with-statement) statement.
    With {
        is_async: bool,
        items: Vec<WithItem<U>>,
        body: Suite<U>,
    },

    /// A [`for`](https://docs.python.org/3/reference/compound_stmts.html#the-for-statement) statement.
    /// Contains the body of the loop, and the `else` clause.
    For {
        is_async: bool,
        target: Box<Expression<U>>,
        iter: Box<Expression<U>>,
        body: Suite<U>,
        orelse: Option<Suite<U>>,
    },

    /// A `raise` statement.
    Raise {
        exception: Option<Expression<U>>,
        cause: Option<Expression<U>>,
    },

    /// A [`try`](https://docs.python.org/3/reference/compound_stmts.html#the-try-statement) statement.
    Try {
        body: Suite<U>,
        handlers: Vec<ExceptHandler<U>>,
        orelse: Option<Suite<U>>,
        finalbody: Option<Suite<U>>,
    },

    /// A [class definition](https://docs.python.org/3/reference/compound_stmts.html#class-definitions).
    ClassDef {
        name: String,
        body: Suite<U>,
        bases: Vec<Expression<U>>,
        keywords: Vec<Keyword<U>>,
        decorator_list: Vec<Expression<U>>,
    },

    /// A [function definition](https://docs.python.org/3/reference/compound_stmts.html#function-definitions).
    /// Contains the name of the function, it's body
    /// some decorators and formal parameters to the function.
    FunctionDef {
        is_async: bool,
        name: String,
        args: Box<Parameters<U>>,
        body: Suite<U>,
        decorator_list: Vec<Expression<U>>,
        returns: Option<Expression<U>>,
    },
}

#[derive(Debug, PartialEq)]
pub struct WithItem<U = ()> {
    pub context_expr: Expression<U>,
    pub optional_vars: Option<Expression<U>>,
}

/// An expression at a given location in the sourcecode.
pub type Expression<U = ()> = Located<ExpressionType<U>, U>;

/// A certain type of expression.
#[derive(Debug, PartialEq)]
pub enum ExpressionType<U = ()> {
    BoolOp {
        op: BooleanOperator,
        values: Vec<Expression<U>>,
    },

    /// A binary operation on two operands.
    Binop {
        a: Box<Expression<U>>,
        op: Operator,
        b: Box<Expression<U>>,
    },

    /// Subscript operation.
    Subscript {
        a: Box<Expression<U>>,
        b: Box<Expression<U>>,
    },

    /// An unary operation.
    Unop {
        op: UnaryOperator,
        a: Box<Expression<U>>,
    },

    /// An await expression.
    Await {
        value: Box<Expression<U>>,
    },

    /// A yield expression.
    Yield {
        value: Option<Box<Expression<U>>>,
    },

    // A yield from expression.
    YieldFrom {
        value: Box<Expression<U>>,
    },

    /// A chained comparison. Note that in python you can use
    /// `1 < a < 10` for example.
    Compare {
        vals: Vec<Expression<U>>,
        ops: Vec<Comparison>,
    },

    /// Attribute access in the form of `value.name`.
    Attribute {
        value: Box<Expression<U>>,
        name: String,
    },

    /// A call expression.
    Call {
        function: Box<Expression<U>>,
        args: Vec<Expression<U>>,
        keywords: Vec<Keyword<U>>,
    },

    /// A numeric literal.
    Number {
        value: Number,
    },

    /// A `list` literal value.
    List {
        elements: Vec<Expression<U>>,
    },

    /// A `tuple` literal value.
    Tuple {
        elements: Vec<Expression<U>>,
    },

    /// A `dict` literal value.
    /// For example: `{2: 'two', 3: 'three'}`
    Dict {
        elements: Vec<(Option<Expression<U>>, Expression<U>)>,
    },

    /// A `set` literal.
    Set {
        elements: Vec<Expression<U>>,
    },

    Comprehension {
        kind: Box<ComprehensionKind<U>>,
        generators: Vec<Comprehension<U>>,
    },

    /// A starred expression.
    Starred {
        value: Box<Expression<U>>,
    },

    /// A slice expression.
    Slice {
        elements: Vec<Expression<U>>,
    },

    /// A string literal.
    String {
        value: StringGroup<U>,
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
        args: Box<Parameters<U>>,
        body: Box<Expression<U>>,
    },

    /// An if-expression.
    IfExpression {
        test: Box<Expression<U>>,
        body: Box<Expression<U>>,
        orelse: Box<Expression<U>>,
    },

    // A named expression
    NamedExpression {
        left: Box<Expression<U>>,
        right: Box<Expression<U>>,
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

impl<U> Expression<U> {
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
pub struct Parameters<U = ()> {
    pub posonlyargs_count: usize,
    pub args: Vec<Parameter<U>>,
    pub kwonlyargs: Vec<Parameter<U>>,
    pub vararg: Varargs<U>, // Optionally we handle optionally named '*args' or '*'
    pub kwarg: Varargs<U>,
    pub defaults: Vec<Expression<U>>,
    pub kw_defaults: Vec<Option<Expression<U>>>,
}

/// A single formal parameter to a function.
#[derive(Debug, PartialEq, Default)]
pub struct Parameter<U = ()> {
    pub location: Location,
    pub arg: String,
    pub annotation: Option<Box<Expression<U>>>,
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, PartialEq)]
pub enum ComprehensionKind<U = ()> {
    GeneratorExpression { element: Expression<U> },
    List { element: Expression<U> },
    Set { element: Expression<U> },
    Dict { key: Expression<U>, value: Expression<U> },
}

/// A list/set/dict/generator compression.
#[derive(Debug, PartialEq)]
pub struct Comprehension<U = ()> {
    pub location: Location,
    pub target: Expression<U>,
    pub iter: Expression<U>,
    pub ifs: Vec<Expression<U>>,
    pub is_async: bool,
}

#[derive(Debug, PartialEq)]
pub struct ArgumentList<U = ()> {
    pub args: Vec<Expression<U>>,
    pub keywords: Vec<Keyword<U>>,
}

#[derive(Debug, PartialEq)]
pub struct Keyword<U = ()> {
    pub name: Option<String>,
    pub value: Expression<U>,
}

#[derive(Debug, PartialEq)]
pub struct ExceptHandler<U = ()> {
    pub location: Location,
    pub typ: Option<Expression<U>>,
    pub name: Option<String>,
    pub body: Suite<U>,
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
pub enum StringGroup<U = ()> {
    Constant {
        value: String,
    },
    FormattedValue {
        value: Box<Expression<U>>,
        conversion: Option<ConversionFlag>,
        spec: Option<Box<StringGroup<U>>>,
    },
    Joined {
        values: Vec<StringGroup<U>>,
    },
}

#[derive(Debug, PartialEq)]
pub enum Varargs<U = ()> {
    None,
    Unnamed,
    Named(Parameter<U>),
}

impl<U> Default for Varargs<U> {
    fn default() -> Varargs<U> {
        Varargs::None
    }
}

impl<U> From<Option<Option<Parameter<U>>>> for Varargs<U> {
    fn from(opt: Option<Option<Parameter<U>>>) -> Varargs<U> {
        match opt {
            Some(inner_opt) => match inner_opt {
                Some(param) => Varargs::Named(param),
                None => Varargs::Unnamed,
            },
            None => Varargs::None,
        }
    }
}

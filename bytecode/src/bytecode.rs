//! Implement python as a virtual machine with bytecodes. This module
//! implements bytecode structure.

use bitflags::bitflags;
use num_bigint::BigInt;
use num_complex::Complex64;
use rustpython_parser::ast;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fmt;

/// Primary container of a single code object. Each python function has
/// a codeobject. Also a module has a codeobject.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct CodeObject {
    pub instructions: Vec<Instruction>,
    /// Jump targets.
    pub label_map: HashMap<Label, usize>,
    pub locations: Vec<ast::Location>,
    pub arg_names: Vec<String>, // Names of positional arguments
    pub varargs: Varargs,       // *args or *
    pub kwonlyarg_names: Vec<String>,
    pub varkeywords: Varargs, // **kwargs or **
    pub source_path: String,
    pub first_line_number: usize,
    pub obj_name: String, // Name of the object that created this code object
    pub is_generator: bool,
}

bitflags! {
    #[derive(Serialize, Deserialize)]
    pub struct FunctionOpArg: u8 {
        const HAS_DEFAULTS = 0x01;
        const HAS_KW_ONLY_DEFAULTS = 0x02;
        const HAS_ANNOTATIONS = 0x04;
    }
}

pub type Label = usize;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum NameScope {
    Local,
    NonLocal,
    Global,
}

/// A Single bytecode instruction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Instruction {
    Import {
        name: String,
        symbols: Vec<String>,
        level: usize,
    },
    ImportStar {
        name: String,
        level: usize,
    },
    LoadName {
        name: String,
        scope: NameScope,
    },
    StoreName {
        name: String,
        scope: NameScope,
    },
    DeleteName {
        name: String,
    },
    StoreSubscript,
    DeleteSubscript,
    StoreAttr {
        name: String,
    },
    DeleteAttr {
        name: String,
    },
    LoadConst {
        value: Constant,
    },
    UnaryOperation {
        op: UnaryOperator,
    },
    BinaryOperation {
        op: BinaryOperator,
        inplace: bool,
    },
    LoadAttr {
        name: String,
    },
    CompareOperation {
        op: ComparisonOperator,
    },
    Pop,
    Rotate {
        amount: usize,
    },
    Duplicate,
    GetIter,
    Pass,
    Continue,
    Break,
    Jump {
        target: Label,
    },
    JumpIf {
        target: Label,
    },
    JumpIfFalse {
        target: Label,
    },
    MakeFunction {
        flags: FunctionOpArg,
    },
    CallFunction {
        typ: CallType,
    },
    ForIter {
        target: Label,
    },
    ReturnValue,
    YieldValue,
    YieldFrom,
    SetupLoop {
        start: Label,
        end: Label,
    },
    SetupExcept {
        handler: Label,
    },
    SetupWith {
        end: Label,
    },
    CleanupWith {
        end: Label,
    },
    PopBlock,
    Raise {
        argc: usize,
    },
    BuildString {
        size: usize,
    },
    BuildTuple {
        size: usize,
        unpack: bool,
    },
    BuildList {
        size: usize,
        unpack: bool,
    },
    BuildSet {
        size: usize,
        unpack: bool,
    },
    BuildMap {
        size: usize,
        unpack: bool,
    },
    BuildSlice {
        size: usize,
    },
    ListAppend {
        i: usize,
    },
    SetAdd {
        i: usize,
    },
    MapAdd {
        i: usize,
    },
    PrintExpr,
    LoadBuildClass,
    UnpackSequence {
        size: usize,
    },
    UnpackEx {
        before: usize,
        after: usize,
    },
    Unpack,
    FormatValue {
        conversion: Option<ast::ConversionFlag>,
        spec: String,
    },
    PopException,
}

use self::Instruction::*;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CallType {
    Positional(usize),
    Keyword(usize),
    Ex(bool),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Constant {
    Integer { value: BigInt },
    Float { value: f64 },
    Complex { value: Complex64 },
    Boolean { value: bool },
    String { value: String },
    Bytes { value: Vec<u8> },
    Code { code: Box<CodeObject> },
    Tuple { elements: Vec<Constant> },
    None,
    Ellipsis,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ComparisonOperator {
    Greater,
    GreaterOrEqual,
    Less,
    LessOrEqual,
    Equal,
    NotEqual,
    In,
    NotIn,
    Is,
    IsNot,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum BinaryOperator {
    Power,
    Multiply,
    MatrixMultiply,
    Divide,
    FloorDivide,
    Modulo,
    Add,
    Subtract,
    Subscript,
    Lshift,
    Rshift,
    And,
    Xor,
    Or,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum UnaryOperator {
    Not,
    Invert,
    Minus,
    Plus,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Varargs {
    None,
    Unnamed,
    Named(String),
}

/*
Maintain a stack of blocks on the VM.
pub enum BlockType {
    Loop,
    Except,
}
*/

impl CodeObject {
    pub fn new(
        arg_names: Vec<String>,
        varargs: Varargs,
        kwonlyarg_names: Vec<String>,
        varkeywords: Varargs,
        source_path: String,
        first_line_number: usize,
        obj_name: String,
    ) -> CodeObject {
        CodeObject {
            instructions: Vec::new(),
            label_map: HashMap::new(),
            locations: Vec::new(),
            arg_names,
            varargs,
            kwonlyarg_names,
            varkeywords,
            source_path,
            first_line_number,
            obj_name,
            is_generator: false,
        }
    }

    pub fn get_constants(&self) -> impl Iterator<Item = &Constant> {
        self.instructions.iter().filter_map(|x| {
            if let Instruction::LoadConst { value } = x {
                Some(value)
            } else {
                None
            }
        })
    }
}

impl fmt::Display for CodeObject {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let label_targets: HashSet<&usize> = self.label_map.values().collect();
        for (offset, instruction) in self.instructions.iter().enumerate() {
            let arrow = if label_targets.contains(&offset) {
                ">>"
            } else {
                "  "
            };
            write!(f, "          {} {:5} ", arrow, offset)?;
            instruction.fmt_dis(f, &self.label_map)?;
        }
        Ok(())
    }
}

impl Instruction {
    fn fmt_dis(&self, f: &mut fmt::Formatter, label_map: &HashMap<Label, usize>) -> fmt::Result {
        macro_rules! w {
            ($variant:ident) => {
                write!(f, "{:20}\n", stringify!($variant))
            };
            ($variant:ident, $var:expr) => {
                write!(f, "{:20} ({})\n", stringify!($variant), $var)
            };
            ($variant:ident, $var1:expr, $var2:expr) => {
                write!(f, "{:20} ({}, {})\n", stringify!($variant), $var1, $var2)
            };
            ($variant:ident, $var1:expr, $var2:expr, $var3:expr) => {
                write!(
                    f,
                    "{:20} ({}, {}, {})\n",
                    stringify!($variant),
                    $var1,
                    $var2,
                    $var3
                )
            };
        }

        match self {
            Import {
                name,
                symbols,
                level,
            } => w!(Import, name, format!("{:?}", symbols), level),
            ImportStar { name, level } => w!(ImportStar, name, level),
            LoadName { name, scope } => w!(LoadName, name, format!("{:?}", scope)),
            StoreName { name, scope } => w!(StoreName, name, format!("{:?}", scope)),
            DeleteName { name } => w!(DeleteName, name),
            StoreSubscript => w!(StoreSubscript),
            DeleteSubscript => w!(DeleteSubscript),
            StoreAttr { name } => w!(StoreAttr, name),
            DeleteAttr { name } => w!(DeleteAttr, name),
            LoadConst { value } => w!(LoadConst, value),
            UnaryOperation { op } => w!(UnaryOperation, format!("{:?}", op)),
            BinaryOperation { op, inplace } => w!(BinaryOperation, format!("{:?}", op), inplace),
            LoadAttr { name } => w!(LoadAttr, name),
            CompareOperation { op } => w!(CompareOperation, format!("{:?}", op)),
            Pop => w!(Pop),
            Rotate { amount } => w!(Rotate, amount),
            Duplicate => w!(Duplicate),
            GetIter => w!(GetIter),
            Pass => w!(Pass),
            Continue => w!(Continue),
            Break => w!(Break),
            Jump { target } => w!(Jump, label_map[target]),
            JumpIf { target } => w!(JumpIf, label_map[target]),
            JumpIfFalse { target } => w!(JumpIfFalse, label_map[target]),
            MakeFunction { flags } => w!(MakeFunction, format!("{:?}", flags)),
            CallFunction { typ } => w!(CallFunction, format!("{:?}", typ)),
            ForIter { target } => w!(ForIter, label_map[target]),
            ReturnValue => w!(ReturnValue),
            YieldValue => w!(YieldValue),
            YieldFrom => w!(YieldFrom),
            SetupLoop { start, end } => w!(SetupLoop, label_map[start], label_map[end]),
            SetupExcept { handler } => w!(SetupExcept, handler),
            SetupWith { end } => w!(SetupWith, end),
            CleanupWith { end } => w!(CleanupWith, end),
            PopBlock => w!(PopBlock),
            Raise { argc } => w!(Raise, argc),
            BuildString { size } => w!(BuildString, size),
            BuildTuple { size, unpack } => w!(BuildTuple, size, unpack),
            BuildList { size, unpack } => w!(BuildList, size, unpack),
            BuildSet { size, unpack } => w!(BuildSet, size, unpack),
            BuildMap { size, unpack } => w!(BuildMap, size, unpack),
            BuildSlice { size } => w!(BuildSlice, size),
            ListAppend { i } => w!(ListAppend, i),
            SetAdd { i } => w!(SetAdd, i),
            MapAdd { i } => w!(MapAdd, i),
            PrintExpr => w!(PrintExpr),
            LoadBuildClass => w!(LoadBuildClass),
            UnpackSequence { size } => w!(UnpackSequence, size),
            UnpackEx { before, after } => w!(UnpackEx, before, after),
            Unpack => w!(Unpack),
            FormatValue { spec, .. } => w!(FormatValue, spec), // TODO: write conversion
            PopException => w!(PopException),
        }
    }
}

impl fmt::Display for Constant {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Constant::Integer { value } => write!(f, "{}", value),
            Constant::Float { value } => write!(f, "{}", value),
            Constant::Complex { value } => write!(f, "{}", value),
            Constant::Boolean { value } => write!(f, "{}", value),
            Constant::String { value } => write!(f, "{:?}", value),
            Constant::Bytes { value } => write!(f, "{:?}", value),
            Constant::Code { code } => write!(f, "{:?}", code),
            Constant::Tuple { elements } => write!(
                f,
                "({})",
                elements
                    .iter()
                    .map(|e| format!("{}", e))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Constant::None => write!(f, "None"),
            Constant::Ellipsis => write!(f, "Ellipsis"),
        }
    }
}

impl fmt::Debug for CodeObject {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "<code object {} at ??? file {:?}, line {}>",
            self.obj_name, self.source_path, self.first_line_number
        )
    }
}

impl From<ast::Varargs> for Varargs {
    fn from(varargs: ast::Varargs) -> Varargs {
        match varargs {
            ast::Varargs::None => Varargs::None,
            ast::Varargs::Unnamed => Varargs::Unnamed,
            ast::Varargs::Named(param) => Varargs::Named(param.arg),
        }
    }
}

impl<'a> From<&'a ast::Varargs> for Varargs {
    fn from(varargs: &'a ast::Varargs) -> Varargs {
        match varargs {
            ast::Varargs::None => Varargs::None,
            ast::Varargs::Unnamed => Varargs::Unnamed,
            ast::Varargs::Named(ref param) => Varargs::Named(param.arg.clone()),
        }
    }
}

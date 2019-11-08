//! Implement python as a virtual machine with bytecodes. This module
//! implements bytecode structure.

use bitflags::bitflags;
use num_bigint::BigInt;
use num_complex::Complex64;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fmt;

/// Sourcode location.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Location {
    row: usize,
    column: usize,
}

impl Location {
    pub fn new(row: usize, column: usize) -> Self {
        Location { row, column }
    }

    pub fn row(&self) -> usize {
        self.row
    }

    pub fn column(&self) -> usize {
        self.column
    }
}

/// Primary container of a single code object. Each python function has
/// a codeobject. Also a module has a codeobject.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct CodeObject {
    pub instructions: Vec<Instruction>,
    /// Jump targets.
    pub label_map: HashMap<Label, usize>,
    pub locations: Vec<Location>,
    pub flags: CodeFlags,
    pub arg_names: Vec<String>, // Names of positional arguments
    pub varargs: Varargs,       // *args or *
    pub kwonlyarg_names: Vec<String>,
    pub varkeywords: Varargs, // **kwargs or **
    pub source_path: String,
    pub first_line_number: usize,
    pub obj_name: String, // Name of the object that created this code object
}

bitflags! {
    #[derive(Serialize, Deserialize)]
    pub struct CodeFlags: u8 {
        const HAS_DEFAULTS = 0x01;
        const HAS_KW_ONLY_DEFAULTS = 0x02;
        const HAS_ANNOTATIONS = 0x04;
        const NEW_LOCALS = 0x08;
        const IS_GENERATOR = 0x10;
        const IS_COROUTINE = 0x20;
    }
}

impl Default for CodeFlags {
    fn default() -> Self {
        Self::NEW_LOCALS
    }
}

#[derive(Serialize, Debug, Deserialize, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Label(usize);

impl Label {
    pub fn new(label: usize) -> Self {
        Label(label)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// An indication where the name must be accessed.
pub enum NameScope {
    /// The name will be in the local scope.
    Local,

    /// The name will be located in scope surrounding the current scope.
    NonLocal,

    /// The name will be in global scope.
    Global,

    /// The name will be located in any scope between the current scope and the top scope.
    Free,
}

/// Transforms a value prior to formatting it.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ConversionFlag {
    /// Converts by calling `str(<value>)`.
    Str,
    /// Converts by calling `ascii(<value>)`.
    Ascii,
    /// Converts by calling `repr(<value>)`.
    Repr,
}

/// A Single bytecode instruction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Instruction {
    Import {
        name: Option<String>,
        symbols: Vec<String>,
        level: usize,
    },
    ImportStar,
    ImportFrom {
        name: String,
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
    Subscript,
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
    Continue,
    Break,
    Jump {
        target: Label,
    },
    /// Pop the top of the stack, and jump if this value is true.
    JumpIfTrue {
        target: Label,
    },
    /// Pop the top of the stack, and jump if this value is false.
    JumpIfFalse {
        target: Label,
    },
    /// Peek at the top of the stack, and jump if this value is true.
    /// Otherwise, pop top of stack.
    JumpIfTrueOrPop {
        target: Label,
    },
    /// Peek at the top of the stack, and jump if this value is false.
    /// Otherwise, pop top of stack.
    JumpIfFalseOrPop {
        target: Label,
    },
    MakeFunction,
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

    /// Setup a finally handler, which will be called whenever one of this events occurs:
    /// - the block is popped
    /// - the function returns
    /// - an exception is returned
    SetupFinally {
        handler: Label,
    },

    /// Enter a finally block, without returning, excepting, just because we are there.
    EnterFinally,

    /// Marker bytecode for the end of a finally sequence.
    /// When this bytecode is executed, the eval loop does one of those things:
    /// - Continue at a certain bytecode position
    /// - Propagate the exception
    /// - Return from a function
    /// - Do nothing at all, just continue
    EndFinally,

    SetupExcept {
        handler: Label,
    },
    SetupWith {
        end: Label,
    },
    WithCleanupStart,
    WithCleanupFinish,
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
        for_call: bool,
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
    FormatValue {
        conversion: Option<ConversionFlag>,
        spec: String,
    },
    PopException,
    Reverse {
        amount: usize,
    },
    GetAwaitable,
    BeforeAsyncWith,
    SetupAsyncWith {
        end: Label,
    },
    GetAIter,
    GetANext,
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
    ExceptionMatch,
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
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        flags: CodeFlags,
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
            flags,
            arg_names,
            varargs,
            kwonlyarg_names,
            varkeywords,
            source_path,
            first_line_number,
            obj_name,
        }
    }

    /// Load a code object from bytes
    pub fn from_bytes(data: &[u8]) -> Result<Self, Box<dyn std::error::Error>> {
        let data = lz4_compress::decompress(data)?;
        bincode::deserialize::<Self>(&data).map_err(|e| e.into())
    }

    /// Serialize this bytecode to bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let data = bincode::serialize(&self).expect("Code object must be serializable");
        lz4_compress::compress(&data)
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

    fn display_inner(
        &self,
        f: &mut fmt::Formatter,
        expand_codeobjects: bool,
        level: usize,
    ) -> fmt::Result {
        let label_targets: HashSet<&usize> = self.label_map.values().collect();
        for (offset, instruction) in self.instructions.iter().enumerate() {
            let arrow = if label_targets.contains(&offset) {
                ">>"
            } else {
                "  "
            };
            for _ in 0..level {
                write!(f, "          ")?;
            }
            write!(f, "{} {:5} ", arrow, offset)?;
            instruction.fmt_dis(f, &self.label_map, expand_codeobjects, level)?;
        }
        Ok(())
    }

    pub fn display_expand_codeobjects<'a>(&'a self) -> impl fmt::Display + 'a {
        struct Display<'a>(&'a CodeObject);
        impl fmt::Display for Display<'_> {
            fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
                self.0.display_inner(f, true, 1)
            }
        }
        Display(self)
    }
}

impl fmt::Display for CodeObject {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.display_inner(f, false, 1)
    }
}

impl Instruction {
    fn fmt_dis(
        &self,
        f: &mut fmt::Formatter,
        label_map: &HashMap<Label, usize>,
        expand_codeobjects: bool,
        level: usize,
    ) -> fmt::Result {
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
            } => w!(
                Import,
                format!("{:?}", name),
                format!("{:?}", symbols),
                level
            ),
            ImportStar => w!(ImportStar),
            ImportFrom { name } => w!(ImportFrom, name),
            LoadName { name, scope } => w!(LoadName, name, format!("{:?}", scope)),
            StoreName { name, scope } => w!(StoreName, name, format!("{:?}", scope)),
            DeleteName { name } => w!(DeleteName, name),
            Subscript => w!(Subscript),
            StoreSubscript => w!(StoreSubscript),
            DeleteSubscript => w!(DeleteSubscript),
            StoreAttr { name } => w!(StoreAttr, name),
            DeleteAttr { name } => w!(DeleteAttr, name),
            LoadConst { value } => match value {
                Constant::Code { code } if expand_codeobjects => {
                    writeln!(f, "LoadConst ({:?}):", code)?;
                    code.display_inner(f, true, level + 1)?;
                    Ok(())
                }
                _ => w!(LoadConst, value),
            },
            UnaryOperation { op } => w!(UnaryOperation, format!("{:?}", op)),
            BinaryOperation { op, inplace } => w!(BinaryOperation, format!("{:?}", op), inplace),
            LoadAttr { name } => w!(LoadAttr, name),
            CompareOperation { op } => w!(CompareOperation, format!("{:?}", op)),
            Pop => w!(Pop),
            Rotate { amount } => w!(Rotate, amount),
            Duplicate => w!(Duplicate),
            GetIter => w!(GetIter),
            Continue => w!(Continue),
            Break => w!(Break),
            Jump { target } => w!(Jump, label_map[target]),
            JumpIfTrue { target } => w!(JumpIfTrue, label_map[target]),
            JumpIfFalse { target } => w!(JumpIfFalse, label_map[target]),
            JumpIfTrueOrPop { target } => w!(JumpIfTrueOrPop, label_map[target]),
            JumpIfFalseOrPop { target } => w!(JumpIfFalseOrPop, label_map[target]),
            MakeFunction => w!(MakeFunction),
            CallFunction { typ } => w!(CallFunction, format!("{:?}", typ)),
            ForIter { target } => w!(ForIter, label_map[target]),
            ReturnValue => w!(ReturnValue),
            YieldValue => w!(YieldValue),
            YieldFrom => w!(YieldFrom),
            SetupLoop { start, end } => w!(SetupLoop, label_map[start], label_map[end]),
            SetupExcept { handler } => w!(SetupExcept, label_map[handler]),
            SetupFinally { handler } => w!(SetupFinally, label_map[handler]),
            EnterFinally => w!(EnterFinally),
            EndFinally => w!(EndFinally),
            SetupWith { end } => w!(SetupWith, label_map[end]),
            WithCleanupStart => w!(WithCleanupStart),
            WithCleanupFinish => w!(WithCleanupFinish),
            BeforeAsyncWith => w!(BeforeAsyncWith),
            SetupAsyncWith { end } => w!(SetupAsyncWith, label_map[end]),
            PopBlock => w!(PopBlock),
            Raise { argc } => w!(Raise, argc),
            BuildString { size } => w!(BuildString, size),
            BuildTuple { size, unpack } => w!(BuildTuple, size, unpack),
            BuildList { size, unpack } => w!(BuildList, size, unpack),
            BuildSet { size, unpack } => w!(BuildSet, size, unpack),
            BuildMap {
                size,
                unpack,
                for_call,
            } => w!(BuildMap, size, unpack, for_call),
            BuildSlice { size } => w!(BuildSlice, size),
            ListAppend { i } => w!(ListAppend, i),
            SetAdd { i } => w!(SetAdd, i),
            MapAdd { i } => w!(MapAdd, i),
            PrintExpr => w!(PrintExpr),
            LoadBuildClass => w!(LoadBuildClass),
            UnpackSequence { size } => w!(UnpackSequence, size),
            UnpackEx { before, after } => w!(UnpackEx, before, after),
            FormatValue { spec, .. } => w!(FormatValue, spec), // TODO: write conversion
            PopException => w!(PopException),
            Reverse { amount } => w!(Reverse, amount),
            GetAwaitable => w!(GetAwaitable),
            GetAIter => w!(GetAIter),
            GetANext => w!(GetANext),
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

pub struct FrozenModule {
    pub code: CodeObject,
    pub package: bool,
}

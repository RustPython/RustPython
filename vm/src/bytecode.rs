//! Implement python as a virtual machine with bytecodes. This module
//! implements bytecode structure.

/*
 * Primitive instruction type, which can be encoded and decoded.
 */

use num_bigint::BigInt;
use num_complex::Complex64;
use rustpython_parser::ast;
use std::collections::HashMap;
use std::fmt;

/// Primary container of a single code object. Each python function has
/// a codeobject. Also a module has a codeobject.
#[derive(Clone, PartialEq)]
pub struct CodeObject {
    pub instructions: Vec<Instruction>,
    pub label_map: HashMap<Label, usize>,
    pub locations: Vec<ast::Location>,
    pub arg_names: Vec<String>,          // Names of positional arguments
    pub varargs: Option<Option<String>>, // *args or *
    pub kwonlyarg_names: Vec<String>,
    pub varkeywords: Option<Option<String>>, // **kwargs or **
    pub source_path: String,
    pub first_line_number: usize,
    pub obj_name: String, // Name of the object that created this code object
    pub is_generator: bool,
}

impl CodeObject {
    pub fn new(
        arg_names: Vec<String>,
        varargs: Option<Option<String>>,
        kwonlyarg_names: Vec<String>,
        varkeywords: Option<Option<String>>,
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
}

bitflags! {
    pub struct FunctionOpArg: u8 {
        const HAS_DEFAULTS = 0x01;
    }
}

pub type Label = usize;

/// A Single bytecode instruction.
#[derive(Debug, Clone, PartialEq)]
pub enum Instruction {
    Import {
        name: String,
        symbol: Option<String>,
    },
    ImportStar {
        name: String,
    },
    LoadName {
        name: String,
    },
    StoreName {
        name: String,
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
    StoreLocals,
    UnpackSequence {
        size: usize,
    },
    UnpackEx {
        before: usize,
        after: usize,
    },
    Unpack,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CallType {
    Positional(usize),
    Keyword(usize),
    Ex(bool),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Constant {
    Integer { value: BigInt },
    Float { value: f64 },
    Complex { value: Complex64 },
    Boolean { value: bool },
    String { value: String },
    Bytes { value: Vec<u8> },
    Code { code: CodeObject },
    Tuple { elements: Vec<Constant> },
    None,
}

#[derive(Debug, Clone, PartialEq)]
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

#[derive(Debug, Clone, PartialEq)]
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

#[derive(Debug, Clone, PartialEq)]
pub enum UnaryOperator {
    Not,
    Invert,
    Minus,
    Plus,
}

/*
Maintain a stack of blocks on the VM.
pub enum BlockType {
    Loop,
    Except,
}
*/

impl fmt::Debug for CodeObject {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let inst_str = self
            .instructions
            .iter()
            .zip(self.locations.iter())
            .enumerate()
            .map(|(i, inst)| format!("Inst {}: {:?}", i, inst))
            .collect::<Vec<_>>()
            .join("\n");
        let labelmap_str = format!("label_map: {:?}", self.label_map);
        write!(f, "Code Object {{ \n{}\n{} }}", inst_str, labelmap_str)
    }
}

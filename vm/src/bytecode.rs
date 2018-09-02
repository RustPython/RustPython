/*
 * Implement python as a virtual machine with bytecodes.
 */

/*
let load_const_string = 0x16;
let call_function = 0x64;
*/

/*
 * Primitive instruction type, which can be encoded and decoded.
 */
extern crate rustpython_parser;

use self::rustpython_parser::ast;
use std::collections::HashMap;
use std::fmt;

#[derive(Clone, PartialEq)]
pub struct CodeObject {
    pub instructions: Vec<Instruction>,
    pub label_map: HashMap<Label, usize>,
    pub locations: Vec<ast::Location>,
    pub arg_names: Vec<String>,
    pub source_path: Option<String>,
    pub obj_name: String, // Name of the object that created this code object
}

impl CodeObject {
    pub fn new(
        arg_names: Vec<String>,
        source_path: Option<String>,
        obj_name: String,
    ) -> CodeObject {
        CodeObject {
            instructions: Vec::new(),
            label_map: HashMap::new(),
            locations: Vec::new(),
            arg_names: arg_names,
            source_path: source_path,
            obj_name: obj_name,
        }
    }
}

bitflags! {
    pub struct FunctionOpArg: u8 {
        const HAS_DEFAULTS = 0x01;
    }
}

pub type Label = usize;

#[derive(Debug, Clone, PartialEq)]
pub enum Instruction {
    Import {
        name: String,
        symbol: Option<String>,
    },
    LoadName {
        name: String,
    },
    StoreName {
        name: String,
    },
    StoreSubscript,
    StoreAttr {
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
        count: usize,
    },
    CallFunctionKw {
        count: usize,
    },
    ForIter,
    ReturnValue,
    SetupLoop {
        start: Label,
        end: Label,
    },
    SetupExcept {
        handler: Label,
    },
    PopBlock,
    Raise {
        argc: usize,
    },
    BuildTuple {
        size: usize,
    },
    BuildList {
        size: usize,
    },
    BuildMap {
        size: usize,
    },
    BuildSlice {
        size: usize,
    },
    PrintExpr,
    LoadBuildClass,
    StoreLocals,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Constant {
    Integer { value: i32 }, // TODO: replace by arbitrary big int math.
    Float { value: f64 },
    Boolean { value: bool },
    String { value: String },
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

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
use std::collections::HashMap;
use std::fmt;

#[derive(Clone)]
pub struct CodeObject {
    pub instructions: Vec<Instruction>,
    pub label_map: HashMap<Label, usize>,
    pub arg_names: Vec<String>,
}

impl CodeObject {
    pub fn new(arg_names : Vec<String>) -> CodeObject {
        CodeObject {
            instructions: Vec::new(),
            label_map: HashMap::new(),
            arg_names: arg_names,
        }
    }
}

pub type Label = usize;

#[derive(Debug, Clone)]
pub enum Instruction {
    Import {
        name: String,
        symbol: Option<String>,
    },
    LoadName { name: String },
    StoreName { name: String },
    StoreSubscript,
    StoreAttr { name: String },
    LoadConst { value: Constant },
    UnaryOperation { op: UnaryOperator },
    BinaryOperation { op: BinaryOperator },
    LoadAttr { name: String },
    CompareOperation { op: ComparisonOperator },
    Pop,
    Rotate { amount: usize },
    Duplicate,
    GetIter,
    Pass,
    Continue,
    Break,
    Jump { target: Label },
    JumpIf { target: Label },
    MakeFunction,
    CallFunction { count: usize },
    ForIter,
    ReturnValue,
    SetupLoop { start: Label, end: Label },
    PopBlock,
    Raise { argc: usize },
    BuildTuple { size: usize },
    BuildList { size: usize },
    BuildMap { size: usize },
    BuildSlice { size: usize },
    PrintExpr,
    LoadBuildClass,
    StoreLocals,
}

#[derive(Debug, Clone)]
pub enum Constant {
    Integer { value: i32 }, // TODO: replace by arbitrary big int math.
    // TODO: Float { value: f64 },
    Boolean { value: bool },
    String { value: String },
    Code { code: CodeObject },
    None,
}

#[derive(Debug, Clone)]
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

#[derive(Debug, Clone)]
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

#[derive(Debug, Clone)]
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
        let inst_str = self.instructions
            .iter()
            .enumerate()
            .map(|(i, inst)| format!("Inst {}: {:?}", i, inst))
            .collect::<Vec<_>>()
            .join("\n");
        let labelmap_str = format!("label_map: {:?}", self.label_map);
        write!(f, "Code Object {{ \n{}\n{} }}", inst_str, labelmap_str)
    }
}

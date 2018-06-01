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

#[derive(Debug)]
pub struct CodeObject {
    pub instructions: Vec<Instruction>,
    pub label_map: HashMap<Label, usize>,
}

impl CodeObject {
    pub fn new() -> CodeObject {
        CodeObject {
            instructions: Vec::new(),
            label_map: HashMap::new(),
        }
    }
}

pub type Label = usize;

#[derive(Debug)]
pub enum Instruction {
    LoadName { name: String },
    StoreName { name: String },
    LoadConst { value: Constant },
    UnaryOperation { op: UnaryOperator },
    BinaryOperation { op: BinaryOperator },
    Pop,
    GetIter,
    Pass,
    Continue,
    Break,
    Jump { target: Label },
    JumpIf { target: Label },
    CallFunction { count: usize },
    ForIter,
    ReturnValue,
    PushBlock { start: Label, end: Label },
    PopBlock,
    BuildTuple { size: usize },
    BuildList { size: usize },
    BuildMap { size: usize },
}

#[derive(Debug)]
pub enum Constant {
    Integer { value: i32 },
    // TODO: Float { value: f64 },
    String { value: String },
}

#[derive(Debug)]
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

#[derive(Debug)]
pub enum UnaryOperator {
    Not,
    Minus,
}

/*
Maintain a stack of blocks on the VM.
pub enum BlockType {
    Loop,
    Except,
}
*/

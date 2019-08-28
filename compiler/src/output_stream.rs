use rustpython_bytecode::bytecode::{CodeObject, Instruction, Label, Location};

use crate::stack_effect::stack_effect;

pub trait OutputStream: From<CodeObject> + Into<CodeObject> {
    /// Output an instruction
    fn emit(&mut self, instruction: Instruction, location: Location);
    /// Set a label on an instruction
    fn set_label(&mut self, label: Label);
    /// Mark the inner CodeObject as a generator
    fn mark_generator(&mut self);
}

pub struct CodeObjectStream {
    code: CodeObject,
    max_stack: isize,
    current_stack_size: isize,
}

impl From<CodeObject> for CodeObjectStream {
    fn from(code: CodeObject) -> Self {
        CodeObjectStream {
            code,
            max_stack: 0,
            current_stack_size: 0,
        }
    }
}
impl From<CodeObjectStream> for CodeObject {
    fn from(stream: CodeObjectStream) -> Self {
        stream.code
    }
}

impl OutputStream for CodeObjectStream {
    fn emit(&mut self, instruction: Instruction, location: Location) {
        let effect = stack_effect(&instruction);
        self.current_stack_size += effect;
        if self.current_stack_size > self.max_stack {
            self.max_stack = self.current_stack_size;
        }

        self.code.instructions.push(instruction);
        self.code.locations.push(location);
    }

    fn set_label(&mut self, label: Label) {
        let position = self.code.instructions.len();
        self.code.label_map.insert(label, position);
    }

    fn mark_generator(&mut self) {
        self.code.is_generator = true;
    }
}

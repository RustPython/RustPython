use rustpython_bytecode::bytecode::{CodeFlags, CodeObject, Instruction, Label, Location};

pub trait OutputStream: From<CodeObject> + Into<CodeObject> {
    /// Output an instruction
    fn emit(&mut self, instruction: Instruction, location: Location);
    /// Set a label on an instruction
    fn set_label(&mut self, label: Label);
    /// Mark the inner CodeObject as a generator
    fn mark_generator(&mut self);
    /// Check to see if the inner CodeObject is a generator
    fn is_generator(&self) -> bool;
}

pub struct CodeObjectStream {
    code: CodeObject,
}

impl From<CodeObject> for CodeObjectStream {
    fn from(code: CodeObject) -> Self {
        CodeObjectStream { code }
    }
}
impl From<CodeObjectStream> for CodeObject {
    fn from(stream: CodeObjectStream) -> Self {
        stream.code
    }
}

impl OutputStream for CodeObjectStream {
    fn emit(&mut self, instruction: Instruction, location: Location) {
        self.code.instructions.push(instruction);
        self.code.locations.push(location);
    }
    fn set_label(&mut self, label: Label) {
        let position = self.code.instructions.len();
        self.code.label_map.insert(label, position);
    }
    fn mark_generator(&mut self) {
        self.code.flags |= CodeFlags::IS_GENERATOR;
    }
    fn is_generator(&self) -> bool {
        self.code.flags.contains(CodeFlags::IS_GENERATOR)
    }
}

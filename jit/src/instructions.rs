use cranelift::prelude::*;
use num_traits::cast::ToPrimitive;
use rustpython_bytecode::bytecode::{Constant, Instruction};

use super::JITCompileError;

pub struct FunctionCompiler<'a, 'b> {
    builder: &'a mut FunctionBuilder<'b>,
    stack: Vec<Value>,
}

impl<'a, 'b> FunctionCompiler<'a, 'b> {
    pub fn new(builder: &'a mut FunctionBuilder<'b>) -> FunctionCompiler<'a, 'b> {
        FunctionCompiler {
            builder,
            stack: Vec::new(),
        }
    }

    pub fn add_instruction(&mut self, instruction: &Instruction) -> Result<(), JITCompileError> {
        match instruction {
            Instruction::LoadConst {
                value: Constant::Integer { value },
            } => {
                let val = self.builder.ins().iconst(
                    types::I64,
                    value.to_i64().ok_or(JITCompileError::NotSupported)?,
                );
                self.stack.push(val);
                Ok(())
            }
            Instruction::ReturnValue => {
                self.builder
                    .ins()
                    .return_(&[self.stack.pop().ok_or(JITCompileError::BadBytecode)?]);
                Ok(())
            }
            _ => Err(JITCompileError::NotSupported),
        }
    }
}

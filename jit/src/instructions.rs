use cranelift::prelude::*;
use num_traits::cast::ToPrimitive;
use rustpython_bytecode::bytecode::{BinaryOperator, Constant, Instruction, NameScope};
use std::collections::HashMap;

use super::JITCompileError;

pub struct FunctionCompiler<'a, 'b> {
    builder: &'a mut FunctionBuilder<'b>,
    stack: Vec<Value>,
    variables: HashMap<String, Variable>,
}

impl<'a, 'b> FunctionCompiler<'a, 'b> {
    pub fn new(builder: &'a mut FunctionBuilder<'b>) -> FunctionCompiler<'a, 'b> {
        FunctionCompiler {
            builder,
            stack: Vec::new(),
            variables: HashMap::new(),
        }
    }

    pub fn add_instruction(&mut self, instruction: &Instruction) -> Result<(), JITCompileError> {
        match instruction {
            Instruction::LoadName {
                name,
                scope: NameScope::Local,
            } => {
                let var = self
                    .variables
                    .get(name)
                    .ok_or(JITCompileError::BadBytecode)?;
                self.stack.push(self.builder.use_var(*var));
                Ok(())
            }
            Instruction::StoreName {
                name,
                scope: NameScope::Local,
            } => {
                let var = match self.variables.get(name) {
                    Some(var) => *var,
                    None => {
                        let var = Variable::new(self.variables.len());
                        self.variables.insert(name.clone(), var);
                        self.builder.declare_var(var, types::I64);
                        var
                    }
                };
                self.builder
                    .def_var(var, self.stack.pop().ok_or(JITCompileError::BadBytecode)?);
                Ok(())
            }
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
            Instruction::BinaryOperation { op, .. } => {
                let a = self.stack.pop().ok_or(JITCompileError::BadBytecode)?;
                let b = self.stack.pop().ok_or(JITCompileError::BadBytecode)?;
                match op {
                    BinaryOperator::Add => {
                        let (out, carry) = self.builder.ins().iadd_ifcout(a, b);
                        self.builder.ins().trapif(
                            IntCC::Overflow,
                            carry,
                            TrapCode::IntegerOverflow,
                        );
                        self.stack.push(out);
                        Ok(())
                    }
                    BinaryOperator::Subtract => {
                        let (out, carry) = self.builder.ins().isub_ifbout(a, b);
                        self.builder.ins().trapif(
                            IntCC::Overflow,
                            carry,
                            TrapCode::IntegerOverflow,
                        );
                        self.stack.push(out);
                        Ok(())
                    }
                    _ => Err(JITCompileError::NotSupported),
                }
            }
            _ => Err(JITCompileError::NotSupported),
        }
    }
}

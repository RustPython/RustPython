use cranelift::prelude::*;
use num_traits::cast::ToPrimitive;
use rustpython_bytecode::bytecode::{BinaryOperator, Constant, Instruction, NameScope};
use std::collections::HashMap;

use super::{JitCompileError, JitSig, JitType};

#[derive(Clone)]
struct Local {
    var: Variable,
    ty: JitType,
}

struct JitValue {
    val: Value,
    ty: JitType,
}

impl JitValue {
    fn new(val: Value, ty: JitType) -> JitValue {
        JitValue { val, ty }
    }
}

pub struct FunctionCompiler<'a, 'b> {
    builder: &'a mut FunctionBuilder<'b>,
    stack: Vec<JitValue>,
    variables: HashMap<String, Local>,
    pub(crate) sig: JitSig,
}

impl<'a, 'b> FunctionCompiler<'a, 'b> {
    pub fn new(
        builder: &'a mut FunctionBuilder<'b>,
        arg_names: &[String],
        arg_types: &[JitType],
        entry_block: Block,
    ) -> FunctionCompiler<'a, 'b> {
        let mut compiler = FunctionCompiler {
            builder,
            stack: Vec::new(),
            variables: HashMap::new(),
            sig: JitSig {
                args: arg_types.to_vec(),
                ret: None,
            },
        };
        let params = compiler.builder.func.dfg.block_params(entry_block).to_vec();
        debug_assert_eq!(arg_names.len(), arg_types.len());
        debug_assert_eq!(arg_names.len(), params.len());
        for ((name, ty), val) in arg_names.iter().zip(arg_types).zip(params) {
            compiler
                .store_variable(name.clone(), JitValue::new(val, ty.clone()))
                .unwrap();
        }
        compiler
    }

    fn store_variable(&mut self, name: String, val: JitValue) -> Result<(), JitCompileError> {
        let len = self.variables.len();
        let builder = &mut self.builder;
        let local = self.variables.entry(name).or_insert_with(|| {
            let var = Variable::new(len);
            let local = Local {
                var,
                ty: val.ty.clone(),
            };
            builder.declare_var(var, val.ty.to_cranelift());
            local
        });
        if val.ty != local.ty {
            Err(JitCompileError::NotSupported)
        } else {
            self.builder.def_var(local.var, val.val);
            Ok(())
        }
    }

    pub fn add_instruction(&mut self, instruction: &Instruction) -> Result<(), JitCompileError> {
        match instruction {
            Instruction::LoadName {
                name,
                scope: NameScope::Local,
            } => {
                let local = self
                    .variables
                    .get(name)
                    .ok_or(JitCompileError::BadBytecode)?;
                self.stack.push(JitValue {
                    val: self.builder.use_var(local.var),
                    ty: local.ty.clone(),
                });
                Ok(())
            }
            Instruction::StoreName {
                name,
                scope: NameScope::Local,
            } => {
                let val = self.stack.pop().ok_or(JitCompileError::BadBytecode)?;
                self.store_variable(name.clone(), val)
            }
            Instruction::LoadConst {
                value: Constant::Integer { value },
            } => {
                let val = self.builder.ins().iconst(
                    types::I64,
                    value.to_i64().ok_or(JitCompileError::NotSupported)?,
                );
                self.stack.push(JitValue {
                    val,
                    ty: JitType::Int,
                });
                Ok(())
            }
            Instruction::LoadConst {
                value: Constant::Float { value },
            } => {
                let val = self.builder.ins().f64const(*value);
                self.stack.push(JitValue {
                    val,
                    ty: JitType::Float,
                });
                Ok(())
            }
            Instruction::ReturnValue => {
                let val = self.stack.pop().ok_or(JitCompileError::BadBytecode)?;
                if let Some(ref ty) = self.sig.ret {
                    if val.ty != *ty {
                        return Err(JitCompileError::NotSupported);
                    }
                } else {
                    self.sig.ret = Some(val.ty.clone());
                    self.builder
                        .func
                        .signature
                        .returns
                        .push(AbiParam::new(val.ty.to_cranelift()));
                }
                self.builder.ins().return_(&[val.val]);
                Ok(())
            }
            Instruction::BinaryOperation { op, .. } => {
                // the rhs is popped off first
                let b = self.stack.pop().ok_or(JitCompileError::BadBytecode)?;
                let a = self.stack.pop().ok_or(JitCompileError::BadBytecode)?;
                match (a.ty, b.ty) {
                    (JitType::Int, JitType::Int) => match op {
                        BinaryOperator::Add => {
                            let (out, carry) = self.builder.ins().iadd_ifcout(a.val, b.val);
                            self.builder.ins().trapif(
                                IntCC::Overflow,
                                carry,
                                TrapCode::IntegerOverflow,
                            );
                            self.stack.push(JitValue {
                                val: out,
                                ty: JitType::Int,
                            });
                            Ok(())
                        }
                        BinaryOperator::Subtract => {
                            let (out, carry) = self.builder.ins().isub_ifbout(a.val, b.val);
                            self.builder.ins().trapif(
                                IntCC::Overflow,
                                carry,
                                TrapCode::IntegerOverflow,
                            );
                            self.stack.push(JitValue {
                                val: out,
                                ty: JitType::Int,
                            });
                            Ok(())
                        }
                        _ => Err(JitCompileError::NotSupported),
                    },
                    (JitType::Float, JitType::Float) => match op {
                        BinaryOperator::Add => {
                            self.stack.push(JitValue {
                                val: self.builder.ins().fadd(a.val, b.val),
                                ty: JitType::Float,
                            });
                            Ok(())
                        }
                        BinaryOperator::Subtract => {
                            self.stack.push(JitValue {
                                val: self.builder.ins().fsub(a.val, b.val),
                                ty: JitType::Float,
                            });
                            Ok(())
                        }
                        BinaryOperator::Multiply => {
                            self.stack.push(JitValue {
                                val: self.builder.ins().fmul(a.val, b.val),
                                ty: JitType::Float,
                            });
                            Ok(())
                        }
                        BinaryOperator::Divide => {
                            self.stack.push(JitValue {
                                val: self.builder.ins().fdiv(a.val, b.val),
                                ty: JitType::Float,
                            });
                            Ok(())
                        }
                        _ => Err(JitCompileError::NotSupported),
                    },
                    _ => Err(JitCompileError::NotSupported),
                }
            }
            _ => Err(JitCompileError::NotSupported),
        }
    }
}

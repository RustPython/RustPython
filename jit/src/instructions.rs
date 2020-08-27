use cranelift::prelude::*;
use num_traits::cast::ToPrimitive;
use rustpython_bytecode::bytecode::{BinaryOperator, Constant, Instruction, NameScope};
use std::collections::HashMap;

use super::JitCompileError;

#[derive(Default)]
pub struct JitSig {
    pub ret: Option<JitType>,
}

impl JitSig {
    pub fn to_cif(&self) -> libffi::middle::Cif {
        let ret = match self.ret {
            Some(ref ty) => ty.to_libffi(),
            None => libffi::middle::Type::void(),
        };
        libffi::middle::Cif::new(Vec::new(), ret)
    }
}

#[derive(Clone, PartialEq)]
pub enum JitType {
    Int,
    Float,
}

impl JitType {
    fn to_cranelift(&self) -> types::Type {
        match self {
            Self::Int => types::I64,
            Self::Float => types::F64,
        }
    }

    fn to_libffi(&self) -> libffi::middle::Type {
        match self {
            Self::Int => libffi::middle::Type::i64(),
            Self::Float => libffi::middle::Type::f64(),
        }
    }
}

#[derive(Clone)]
struct Local {
    var: Variable,
    ty: JitType,
}

struct JitValue {
    val: Value,
    ty: JitType,
}

pub struct FunctionCompiler<'a, 'b> {
    builder: &'a mut FunctionBuilder<'b>,
    stack: Vec<JitValue>,
    variables: HashMap<String, Local>,
    pub sig: JitSig,
}

impl<'a, 'b> FunctionCompiler<'a, 'b> {
    pub fn new(builder: &'a mut FunctionBuilder<'b>) -> FunctionCompiler<'a, 'b> {
        FunctionCompiler {
            builder,
            stack: Vec::new(),
            variables: HashMap::new(),
            sig: JitSig::default(),
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
                let len = self.variables.len();
                let builder = &mut self.builder;
                let local = self.variables.entry(name.clone()).or_insert_with(|| {
                    let var = Variable::new(len);
                    let local = Local {
                        var,
                        ty: val.ty.clone(),
                    };
                    builder.declare_var(var, val.ty.to_cranelift());
                    local
                });
                if val.ty != local.ty {
                    return Err(JitCompileError::NotSupported);
                }
                self.builder.def_var(local.var, val.val);
                Ok(())
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

use cranelift::prelude::*;
use num_traits::cast::ToPrimitive;
use rustpython_bytecode::{
    self as bytecode, BinaryOperator, BorrowedConstant, CodeObject, ComparisonOperator,
    Instruction, Label, UnaryOperator,
};
use std::collections::HashMap;

use super::{JitCompileError, JitSig, JitType};

#[derive(Clone)]
struct Local {
    var: Variable,
    ty: JitType,
}

#[derive(Debug)]
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
    variables: Box<[Option<Local>]>,
    label_to_block: HashMap<Label, Block>,
    pub(crate) sig: JitSig,
}

impl<'a, 'b> FunctionCompiler<'a, 'b> {
    pub fn new(
        builder: &'a mut FunctionBuilder<'b>,
        num_variables: usize,
        arg_types: &[JitType],
        entry_block: Block,
    ) -> FunctionCompiler<'a, 'b> {
        let mut compiler = FunctionCompiler {
            builder,
            stack: Vec::new(),
            variables: vec![None; num_variables].into_boxed_slice(),
            label_to_block: HashMap::new(),
            sig: JitSig {
                args: arg_types.to_vec(),
                ret: None,
            },
        };
        let params = compiler.builder.func.dfg.block_params(entry_block).to_vec();
        for (i, (ty, val)) in arg_types.iter().zip(params).enumerate() {
            compiler
                .store_variable(i as u32, JitValue::new(val, ty.clone()))
                .unwrap();
        }
        compiler
    }

    fn store_variable(
        &mut self,
        idx: bytecode::NameIdx,
        val: JitValue,
    ) -> Result<(), JitCompileError> {
        let builder = &mut self.builder;
        let local = self.variables[idx as usize].get_or_insert_with(|| {
            let var = Variable::new(idx as usize);
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

    fn boolean_val(&mut self, val: JitValue) -> Result<Value, JitCompileError> {
        match val.ty {
            JitType::Float => {
                let zero = self.builder.ins().f64const(0);
                let val = self.builder.ins().fcmp(FloatCC::NotEqual, val.val, zero);
                Ok(self.builder.ins().bint(types::I8, val))
            }
            JitType::Int => {
                let zero = self.builder.ins().iconst(types::I64, 0);
                let val = self.builder.ins().icmp(IntCC::NotEqual, val.val, zero);
                Ok(self.builder.ins().bint(types::I8, val))
            }
            JitType::Bool => Ok(val.val),
        }
    }

    fn get_or_create_block(&mut self, label: Label) -> Block {
        let builder = &mut self.builder;
        *self
            .label_to_block
            .entry(label)
            .or_insert_with(|| builder.create_block())
    }

    pub fn compile<C: bytecode::Constant>(
        &mut self,
        bytecode: &CodeObject<C>,
    ) -> Result<(), JitCompileError> {
        // TODO: figure out if this is sufficient -- previously individual labels were associated
        // pretty much per-bytecode that uses them, or at least per "type" of block -- in theory an
        // if block and a with block might jump to the same place. Now it's all "flattened", so
        // there might be less distinction between different types of blocks going off
        // label_targets alone
        let label_targets = bytecode.label_targets();

        for (offset, instruction) in bytecode.instructions.iter().enumerate() {
            let label = Label(offset as u32);
            if label_targets.contains(&label) {
                let block = self.get_or_create_block(label);

                // If the current block is not terminated/filled just jump
                // into the new block.
                if !self.builder.is_filled() {
                    self.builder.ins().jump(block, &[]);
                }

                self.builder.switch_to_block(block);
            }

            // Sometimes the bytecode contains instructions after a return
            // just ignore those until we are at the next label
            if self.builder.is_filled() {
                continue;
            }

            self.add_instruction(&instruction, &bytecode.constants)?;
        }

        Ok(())
    }

    fn load_const<C: bytecode::Constant>(
        &mut self,
        constant: BorrowedConstant<C>,
    ) -> Result<(), JitCompileError> {
        match constant {
            BorrowedConstant::Integer { value } => {
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
            BorrowedConstant::Float { value } => {
                let val = self.builder.ins().f64const(value);
                self.stack.push(JitValue {
                    val,
                    ty: JitType::Float,
                });
                Ok(())
            }
            BorrowedConstant::Boolean { value } => {
                let val = self.builder.ins().iconst(types::I8, value as i64);
                self.stack.push(JitValue {
                    val,
                    ty: JitType::Bool,
                });
                Ok(())
            }
            _ => Err(JitCompileError::NotSupported),
        }
    }

    pub fn add_instruction<C: bytecode::Constant>(
        &mut self,
        instruction: &Instruction,
        constants: &[C],
    ) -> Result<(), JitCompileError> {
        match instruction {
            Instruction::JumpIfFalse { target } => {
                let cond = self.stack.pop().ok_or(JitCompileError::BadBytecode)?;

                let val = self.boolean_val(cond)?;
                let then_block = self.get_or_create_block(*target);
                self.builder.ins().brz(val, then_block, &[]);

                let block = self.builder.create_block();
                self.builder.ins().fallthrough(block, &[]);
                self.builder.switch_to_block(block);

                Ok(())
            }
            Instruction::JumpIfTrue { target } => {
                let cond = self.stack.pop().ok_or(JitCompileError::BadBytecode)?;

                let val = self.boolean_val(cond)?;
                let then_block = self.get_or_create_block(*target);
                self.builder.ins().brnz(val, then_block, &[]);

                let block = self.builder.create_block();
                self.builder.ins().fallthrough(block, &[]);
                self.builder.switch_to_block(block);

                Ok(())
            }
            Instruction::Jump { target } => {
                let target_block = self.get_or_create_block(*target);
                self.builder.ins().jump(target_block, &[]);

                Ok(())
            }
            Instruction::LoadFast(idx) => {
                let local = self.variables[*idx as usize]
                    .as_ref()
                    .ok_or(JitCompileError::BadBytecode)?;
                self.stack.push(JitValue {
                    val: self.builder.use_var(local.var),
                    ty: local.ty.clone(),
                });
                Ok(())
            }
            Instruction::StoreFast(idx) => {
                let val = self.stack.pop().ok_or(JitCompileError::BadBytecode)?;
                self.store_variable(*idx, val)
            }
            Instruction::LoadConst { idx } => {
                self.load_const(constants[*idx as usize].borrow_constant())
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
            Instruction::CompareOperation { op, .. } => {
                // the rhs is popped off first
                let b = self.stack.pop().ok_or(JitCompileError::BadBytecode)?;
                let a = self.stack.pop().ok_or(JitCompileError::BadBytecode)?;

                match (a.ty, b.ty) {
                    (JitType::Int, JitType::Int) => {
                        let cond = match op {
                            ComparisonOperator::Equal => IntCC::Equal,
                            ComparisonOperator::NotEqual => IntCC::NotEqual,
                            ComparisonOperator::Less => IntCC::SignedLessThan,
                            ComparisonOperator::LessOrEqual => IntCC::SignedLessThanOrEqual,
                            ComparisonOperator::Greater => IntCC::SignedGreaterThan,
                            ComparisonOperator::GreaterOrEqual => IntCC::SignedLessThanOrEqual,
                            _ => return Err(JitCompileError::NotSupported),
                        };

                        let val = self.builder.ins().icmp(cond, a.val, b.val);
                        self.stack.push(JitValue {
                            val: self.builder.ins().bint(types::I8, val),
                            ty: JitType::Bool,
                        });

                        Ok(())
                    }
                    _ => Err(JitCompileError::NotSupported),
                }
            }
            Instruction::UnaryOperation { op, .. } => {
                let a = self.stack.pop().ok_or(JitCompileError::BadBytecode)?;

                match a.ty {
                    JitType::Int => match op {
                        UnaryOperator::Minus => {
                            // Compile minus as 0 - a.
                            let zero = self.builder.ins().iconst(types::I64, 0);
                            let (out, carry) = self.builder.ins().isub_ifbout(zero, a.val);
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
                        UnaryOperator::Plus => {
                            // Nothing to do
                            self.stack.push(a);
                            Ok(())
                        }
                        _ => Err(JitCompileError::NotSupported),
                    },
                    JitType::Bool => match op {
                        UnaryOperator::Not => {
                            let val = self.boolean_val(a)?;
                            let not_val = self.builder.ins().bxor_imm(val, 1);
                            self.stack.push(JitValue {
                                val: not_val,
                                ty: JitType::Bool,
                            });
                            Ok(())
                        }
                        _ => Err(JitCompileError::NotSupported),
                    },
                    _ => Err(JitCompileError::NotSupported),
                }
            }
            Instruction::BinaryOperation { op } | Instruction::BinaryOperationInplace { op } => {
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
            Instruction::SetupLoop { .. } | Instruction::PopBlock => {
                // TODO: block support
                Ok(())
            }
            _ => Err(JitCompileError::NotSupported),
        }
    }
}

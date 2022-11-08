use cranelift::prelude::*;
use cranelift_jit::JITModule;
use cranelift_module::Module;
use num_traits::cast::ToPrimitive;
use rustpython_compiler_core::{
    self as bytecode, BinaryOperator, BorrowedConstant, CodeObject, ComparisonOperator,
    Instruction, Label, UnaryOperator,
};
use std::collections::HashMap;

use super::{JitCompileError, JitSig, JitType};

#[repr(u16)]
enum CustomTrapCode {
    /// Raised when shifting by a negative number
    NegativeShiftCount = 0,
}

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

pub struct FunctionCompiler<'a, 'b, 'c> {
    builder: &'a mut FunctionBuilder<'b>,
    module: &'c mut JITModule,
    stack: Vec<JitValue>,
    variables: Box<[Option<Local>]>,
    label_to_block: HashMap<Label, Block>,
    pub(crate) sig: JitSig,
}

impl<'a, 'b, 'c> FunctionCompiler<'a, 'b, 'c> {
    pub fn new(
        builder: &'a mut FunctionBuilder<'b>,
        module: &'c mut JITModule,
        num_variables: usize,
        arg_types: &[JitType],
        entry_block: Block,
    ) -> FunctionCompiler<'a, 'b, 'c> {
        let mut compiler = FunctionCompiler {
            builder,
            module,
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
            JitType::PrintFunction => Err(JitCompileError::NotSupported),
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

            self.add_instruction(instruction, &bytecode.constants, &bytecode.names)?;
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
        names: &[C::Name],
    ) -> Result<(), JitCompileError> {
        match instruction {
            Instruction::JumpIfFalse { target } => {
                let cond = self.stack.pop().ok_or(JitCompileError::BadBytecode)?;

                let val = self.boolean_val(cond)?;
                let then_block = self.get_or_create_block(*target);
                self.builder.ins().brz(val, then_block, &[]);

                let block = self.builder.create_block();
                self.builder.ins().jump(block, &[]);
                self.builder.switch_to_block(block);

                Ok(())
            }
            Instruction::JumpIfTrue { target } => {
                let cond = self.stack.pop().ok_or(JitCompileError::BadBytecode)?;

                let val = self.boolean_val(cond)?;
                let then_block = self.get_or_create_block(*target);
                self.builder.ins().brnz(val, then_block, &[]);

                let block = self.builder.create_block();
                self.builder.ins().jump(block, &[]);
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
                        };

                        let val = self.builder.ins().icmp(cond, a.val, b.val);
                        self.stack.push(JitValue {
                            // TODO: Remove this `bint` in cranelift 0.90 as icmp now returns i8
                            val: self.builder.ins().bint(types::I8, val),
                            ty: JitType::Bool,
                        });

                        Ok(())
                    }
                    (JitType::Float, JitType::Float) => {
                        let cond = match op {
                            ComparisonOperator::Equal => FloatCC::Equal,
                            ComparisonOperator::NotEqual => FloatCC::NotEqual,
                            ComparisonOperator::Less => FloatCC::LessThan,
                            ComparisonOperator::LessOrEqual => FloatCC::LessThanOrEqual,
                            ComparisonOperator::Greater => FloatCC::GreaterThan,
                            ComparisonOperator::GreaterOrEqual => FloatCC::GreaterThanOrEqual,
                        };

                        let val = self.builder.ins().fcmp(cond, a.val, b.val);
                        self.stack.push(JitValue {
                            // TODO: Remove this `bint` in cranelift 0.90 as fcmp now returns i8
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
                            let out = self.compile_sub(zero, a.val);
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
                let (val, ty) = match (op, a.ty, b.ty) {
                    (BinaryOperator::Add, JitType::Int, JitType::Int) => {
                        let (out, carry) = self.builder.ins().iadd_ifcout(a.val, b.val);
                        self.builder.ins().trapif(
                            IntCC::Overflow,
                            carry,
                            TrapCode::IntegerOverflow,
                        );
                        (out, JitType::Int)
                    }
                    (BinaryOperator::Subtract, JitType::Int, JitType::Int) => {
                        (self.compile_sub(a.val, b.val), JitType::Int)
                    }
                    (BinaryOperator::FloorDivide, JitType::Int, JitType::Int) => {
                        (self.builder.ins().sdiv(a.val, b.val), JitType::Int)
                    }
                    (BinaryOperator::Modulo, JitType::Int, JitType::Int) => {
                        (self.builder.ins().srem(a.val, b.val), JitType::Int)
                    }
                    (
                        BinaryOperator::Lshift | BinaryOperator::Rshift,
                        JitType::Int,
                        JitType::Int,
                    ) => {
                        // Shifts throw an exception if we have a negative shift count
                        // Remove all bits except the sign bit, and trap if its 1 (i.e. negative).
                        let sign = self.builder.ins().ushr_imm(b.val, 63);
                        self.builder.ins().trapnz(
                            sign,
                            TrapCode::User(CustomTrapCode::NegativeShiftCount as u16),
                        );

                        let out = if *op == BinaryOperator::Lshift {
                            self.builder.ins().ishl(a.val, b.val)
                        } else {
                            self.builder.ins().sshr(a.val, b.val)
                        };

                        (out, JitType::Int)
                    }
                    (BinaryOperator::And, JitType::Int, JitType::Int) => {
                        (self.builder.ins().band(a.val, b.val), JitType::Int)
                    }
                    (BinaryOperator::Or, JitType::Int, JitType::Int) => {
                        (self.builder.ins().bor(a.val, b.val), JitType::Int)
                    }
                    (BinaryOperator::Xor, JitType::Int, JitType::Int) => {
                        (self.builder.ins().bxor(a.val, b.val), JitType::Int)
                    }

                    // Floats
                    (BinaryOperator::Add, JitType::Float, JitType::Float) => {
                        (self.builder.ins().fadd(a.val, b.val), JitType::Float)
                    }
                    (BinaryOperator::Subtract, JitType::Float, JitType::Float) => {
                        (self.builder.ins().fsub(a.val, b.val), JitType::Float)
                    }
                    (BinaryOperator::Multiply, JitType::Float, JitType::Float) => {
                        (self.builder.ins().fmul(a.val, b.val), JitType::Float)
                    }
                    (BinaryOperator::Divide, JitType::Float, JitType::Float) => {
                        (self.builder.ins().fdiv(a.val, b.val), JitType::Float)
                    }
                    _ => return Err(JitCompileError::NotSupported),
                };

                self.stack.push(JitValue { val, ty });

                Ok(())
            }
            Instruction::SetupLoop { .. } | Instruction::PopBlock => {
                // TODO: block support
                Ok(())
            }
            Instruction::LoadGlobal(idx) => {
                let name = &names[*idx as usize];
                if name.as_ref() == "print" {
                    self.stack.push(JitValue {
                        val: self.builder.ins().iconst(types::I8, 0), // Not used
                        ty: JitType::PrintFunction,
                    });
                    Ok(())
                } else {
                    Err(JitCompileError::NotSupported)
                }
            }
            Instruction::CallFunctionPositional { nargs } => {
                if nargs != &1 {
                    return Err(JitCompileError::NotSupported);
                }

                let arg1 = self.stack.pop().ok_or(JitCompileError::BadBytecode)?;
                let function = self.stack.pop().ok_or(JitCompileError::BadBytecode)?;

                match (arg1.ty, function.ty) {
                    (JitType::Int, JitType::PrintFunction) => {
                        let mut sig = self.module.make_signature();
                        sig.params.push(AbiParam::new(types::I64));
                        sig.returns.push(AbiParam::new(types::I64));

                        let callee = self
                            .module
                            .declare_function("print_fun", cranelift_module::Linkage::Import, &sig)
                            .unwrap();

                        let local_callee =
                            self.module.declare_func_in_func(callee, self.builder.func);

                        let args = vec![arg1.val];
                        let call = self.builder.ins().call(local_callee, &args);
                        let res = self.builder.inst_results(call)[0];

                        self.stack.push(JitValue {
                            val: res,
                            ty: JitType::Int,
                        });
                        Ok(())
                    }
                    _ => Err(JitCompileError::NotSupported),
                }
            }
            Instruction::Pop => {
                self.stack.pop();
                Ok(())
            }
            inst => {
                println!("Unsupported instruction: {:?}", inst);
                Err(JitCompileError::NotSupported)
            }
        }
    }

    fn compile_sub(&mut self, a: Value, b: Value) -> Value {
        // TODO: this should be fine, but cranelift doesn't special-case isub_ifbout
        // let (out, carry) = self.builder.ins().isub_ifbout(a, b);
        // self.builder
        //     .ins()
        //     .trapif(IntCC::Overflow, carry, TrapCode::IntegerOverflow);
        // TODO: this shouldn't wrap
        let neg_b = self.builder.ins().ineg(b);
        let (out, carry) = self.builder.ins().iadd_ifcout(a, neg_b);
        self.builder
            .ins()
            .trapif(IntCC::Overflow, carry, TrapCode::IntegerOverflow);
        out
    }
}

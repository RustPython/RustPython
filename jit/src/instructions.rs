use cranelift::prelude::*;
use num_traits::cast::ToPrimitive;
use rustpython_compiler_core::{
    self as bytecode, BinaryOperator, BorrowedConstant, CodeObject, ComparisonOperator,
    Instruction, Label, OpArg, OpArgState, UnaryOperator,
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
enum JitValue {
    Int(Value),
    Float(Value),
    Bool(Value),
    None,
    Tuple(Vec<JitValue>),
}

impl JitValue {
    fn from_type_and_value(ty: JitType, val: Value) -> JitValue {
        match ty {
            JitType::Int => JitValue::Int(val),
            JitType::Float => JitValue::Float(val),
            JitType::Bool => JitValue::Bool(val),
        }
    }

    fn to_jit_type(&self) -> Option<JitType> {
        match self {
            JitValue::Int(_) => Some(JitType::Int),
            JitValue::Float(_) => Some(JitType::Float),
            JitValue::Bool(_) => Some(JitType::Bool),
            JitValue::None | JitValue::Tuple(_) => None,
        }
    }

    fn into_value(self) -> Option<Value> {
        match self {
            JitValue::Int(val) | JitValue::Float(val) | JitValue::Bool(val) => Some(val),
            JitValue::None | JitValue::Tuple(_) => None,
        }
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
                .store_variable(i as u32, JitValue::from_type_and_value(ty.clone(), val))
                .unwrap();
        }
        compiler
    }

    fn pop_multiple(&mut self, count: usize) -> Vec<JitValue> {
        let stack_len = self.stack.len();
        self.stack.drain(stack_len - count..).collect()
    }

    fn store_variable(
        &mut self,
        idx: bytecode::NameIdx,
        val: JitValue,
    ) -> Result<(), JitCompileError> {
        let builder = &mut self.builder;
        let ty = val.to_jit_type().ok_or(JitCompileError::NotSupported)?;
        let local = self.variables[idx as usize].get_or_insert_with(|| {
            let var = Variable::new(idx as usize);
            let local = Local {
                var,
                ty: ty.clone(),
            };
            builder.declare_var(var, ty.to_cranelift());
            local
        });
        if ty != local.ty {
            Err(JitCompileError::NotSupported)
        } else {
            self.builder.def_var(local.var, val.into_value().unwrap());
            Ok(())
        }
    }

    fn boolean_val(&mut self, val: JitValue) -> Result<Value, JitCompileError> {
        match val {
            JitValue::Float(val) => {
                let zero = self.builder.ins().f64const(0);
                let val = self.builder.ins().fcmp(FloatCC::NotEqual, val, zero);
                Ok(self.builder.ins().bint(types::I8, val))
            }
            JitValue::Int(val) => {
                let zero = self.builder.ins().iconst(types::I64, 0);
                let val = self.builder.ins().icmp(IntCC::NotEqual, val, zero);
                Ok(self.builder.ins().bint(types::I8, val))
            }
            JitValue::Bool(val) => Ok(val),
            JitValue::None => Ok(self.builder.ins().iconst(types::I8, 0)),
            JitValue::Tuple(_) => Err(JitCompileError::NotSupported),
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

        let mut arg_state = OpArgState::default();
        for (offset, instruction) in bytecode.instructions.iter().enumerate() {
            let (instruction, arg) = arg_state.get(*instruction);
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

            self.add_instruction(instruction, arg, &bytecode.constants)?;
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
                self.stack.push(JitValue::Int(val));
                Ok(())
            }
            BorrowedConstant::Float { value } => {
                let val = self.builder.ins().f64const(value);
                self.stack.push(JitValue::Float(val));
                Ok(())
            }
            BorrowedConstant::Boolean { value } => {
                let val = self.builder.ins().iconst(types::I8, value as i64);
                self.stack.push(JitValue::Bool(val));
                Ok(())
            }
            BorrowedConstant::None => {
                self.stack.push(JitValue::None);
                Ok(())
            }
            _ => Err(JitCompileError::NotSupported),
        }
    }

    pub fn add_instruction<C: bytecode::Constant>(
        &mut self,
        instruction: Instruction,
        arg: OpArg,
        constants: &[C],
    ) -> Result<(), JitCompileError> {
        match instruction {
            Instruction::ExtendedArg => Ok(()),
            Instruction::JumpIfFalse { target } => {
                let cond = self.stack.pop().ok_or(JitCompileError::BadBytecode)?;

                let val = self.boolean_val(cond)?;
                let then_block = self.get_or_create_block(target.get(arg));
                self.builder.ins().brz(val, then_block, &[]);

                let block = self.builder.create_block();
                self.builder.ins().jump(block, &[]);
                self.builder.switch_to_block(block);

                Ok(())
            }
            Instruction::JumpIfTrue { target } => {
                let cond = self.stack.pop().ok_or(JitCompileError::BadBytecode)?;

                let val = self.boolean_val(cond)?;
                let then_block = self.get_or_create_block(target.get(arg));
                self.builder.ins().brnz(val, then_block, &[]);

                let block = self.builder.create_block();
                self.builder.ins().jump(block, &[]);
                self.builder.switch_to_block(block);

                Ok(())
            }
            Instruction::Jump { target } => {
                let target_block = self.get_or_create_block(target.get(arg));
                self.builder.ins().jump(target_block, &[]);

                Ok(())
            }
            Instruction::LoadFast(idx) => {
                let local = self.variables[idx.get(arg) as usize]
                    .as_ref()
                    .ok_or(JitCompileError::BadBytecode)?;
                self.stack.push(JitValue::from_type_and_value(
                    local.ty.clone(),
                    self.builder.use_var(local.var),
                ));
                Ok(())
            }
            Instruction::StoreFast(idx) => {
                let val = self.stack.pop().ok_or(JitCompileError::BadBytecode)?;
                self.store_variable(idx.get(arg), val)
            }
            Instruction::LoadConst { idx } => {
                self.load_const(constants[idx.get(arg) as usize].borrow_constant())
            }
            Instruction::BuildTuple { size } => {
                let elements = self.pop_multiple(size.get(arg) as usize);
                self.stack.push(JitValue::Tuple(elements));
                Ok(())
            }
            Instruction::UnpackSequence { size } => {
                let val = self.stack.pop().ok_or(JitCompileError::BadBytecode)?;

                let elements = match val {
                    JitValue::Tuple(elements) => elements,
                    _ => return Err(JitCompileError::NotSupported),
                };

                if elements.len() != size.get(arg) as usize {
                    return Err(JitCompileError::NotSupported);
                }

                self.stack.extend(elements.into_iter().rev());
                Ok(())
            }
            Instruction::ReturnValue => {
                let val = self.stack.pop().ok_or(JitCompileError::BadBytecode)?;
                if let Some(ref ty) = self.sig.ret {
                    if val.to_jit_type().as_ref() != Some(ty) {
                        return Err(JitCompileError::NotSupported);
                    }
                } else {
                    let ty = val.to_jit_type().ok_or(JitCompileError::NotSupported)?;
                    self.sig.ret = Some(ty.clone());
                    self.builder
                        .func
                        .signature
                        .returns
                        .push(AbiParam::new(ty.to_cranelift()));
                }
                self.builder.ins().return_(&[val.into_value().unwrap()]);
                Ok(())
            }
            Instruction::CompareOperation { op, .. } => {
                let op = op.get(arg);
                // the rhs is popped off first
                let b = self.stack.pop().ok_or(JitCompileError::BadBytecode)?;
                let a = self.stack.pop().ok_or(JitCompileError::BadBytecode)?;

                match (a, b) {
                    (JitValue::Int(a), JitValue::Int(b)) => {
                        let cond = match op {
                            ComparisonOperator::Equal => IntCC::Equal,
                            ComparisonOperator::NotEqual => IntCC::NotEqual,
                            ComparisonOperator::Less => IntCC::SignedLessThan,
                            ComparisonOperator::LessOrEqual => IntCC::SignedLessThanOrEqual,
                            ComparisonOperator::Greater => IntCC::SignedGreaterThan,
                            ComparisonOperator::GreaterOrEqual => IntCC::SignedLessThanOrEqual,
                        };

                        let val = self.builder.ins().icmp(cond, a, b);
                        // TODO: Remove this `bint` in cranelift 0.90 as icmp now returns i8
                        self.stack
                            .push(JitValue::Bool(self.builder.ins().bint(types::I8, val)));
                        Ok(())
                    }
                    (JitValue::Float(a), JitValue::Float(b)) => {
                        let cond = match op {
                            ComparisonOperator::Equal => FloatCC::Equal,
                            ComparisonOperator::NotEqual => FloatCC::NotEqual,
                            ComparisonOperator::Less => FloatCC::LessThan,
                            ComparisonOperator::LessOrEqual => FloatCC::LessThanOrEqual,
                            ComparisonOperator::Greater => FloatCC::GreaterThan,
                            ComparisonOperator::GreaterOrEqual => FloatCC::GreaterThanOrEqual,
                        };

                        let val = self.builder.ins().fcmp(cond, a, b);
                        // TODO: Remove this `bint` in cranelift 0.90 as fcmp now returns i8
                        self.stack
                            .push(JitValue::Bool(self.builder.ins().bint(types::I8, val)));
                        Ok(())
                    }
                    _ => Err(JitCompileError::NotSupported),
                }
            }
            Instruction::UnaryOperation { op, .. } => {
                let op = op.get(arg);
                let a = self.stack.pop().ok_or(JitCompileError::BadBytecode)?;
                match (op, a) {
                    (UnaryOperator::Minus, JitValue::Int(val)) => {
                        // Compile minus as 0 - a.
                        let zero = self.builder.ins().iconst(types::I64, 0);
                        let out = self.compile_sub(zero, val);
                        self.stack.push(JitValue::Int(out));
                        Ok(())
                    }
                    (UnaryOperator::Plus, JitValue::Int(val)) => {
                        // Nothing to do
                        self.stack.push(JitValue::Int(val));
                        Ok(())
                    }
                    (UnaryOperator::Not, a) => {
                        let boolean = self.boolean_val(a)?;
                        let not_boolean = self.builder.ins().bxor_imm(boolean, 1);
                        self.stack.push(JitValue::Bool(not_boolean));
                        Ok(())
                    }
                    _ => Err(JitCompileError::NotSupported),
                }
            }
            Instruction::BinaryOperation { op } | Instruction::BinaryOperationInplace { op } => {
                let op = op.get(arg);
                // the rhs is popped off first
                let b = self.stack.pop().ok_or(JitCompileError::BadBytecode)?;
                let a = self.stack.pop().ok_or(JitCompileError::BadBytecode)?;
                let val = match (op, a, b) {
                    (BinaryOperator::Add, JitValue::Int(a), JitValue::Int(b)) => {
                        let (out, carry) = self.builder.ins().iadd_ifcout(a, b);
                        self.builder.ins().trapif(
                            IntCC::Overflow,
                            carry,
                            TrapCode::IntegerOverflow,
                        );
                        JitValue::Int(out)
                    }
                    (BinaryOperator::Subtract, JitValue::Int(a), JitValue::Int(b)) => {
                        JitValue::Int(self.compile_sub(a, b))
                    }
                    (BinaryOperator::FloorDivide, JitValue::Int(a), JitValue::Int(b)) => {
                        JitValue::Int(self.builder.ins().sdiv(a, b))
                    }
                    (BinaryOperator::Modulo, JitValue::Int(a), JitValue::Int(b)) => {
                        JitValue::Int(self.builder.ins().srem(a, b))
                    }
                    (
                        BinaryOperator::Lshift | BinaryOperator::Rshift,
                        JitValue::Int(a),
                        JitValue::Int(b),
                    ) => {
                        // Shifts throw an exception if we have a negative shift count
                        // Remove all bits except the sign bit, and trap if its 1 (i.e. negative).
                        let sign = self.builder.ins().ushr_imm(b, 63);
                        self.builder.ins().trapnz(
                            sign,
                            TrapCode::User(CustomTrapCode::NegativeShiftCount as u16),
                        );

                        let out = if op == BinaryOperator::Lshift {
                            self.builder.ins().ishl(a, b)
                        } else {
                            self.builder.ins().sshr(a, b)
                        };
                        JitValue::Int(out)
                    }
                    (BinaryOperator::And, JitValue::Int(a), JitValue::Int(b)) => {
                        JitValue::Int(self.builder.ins().band(a, b))
                    }
                    (BinaryOperator::Or, JitValue::Int(a), JitValue::Int(b)) => {
                        JitValue::Int(self.builder.ins().bor(a, b))
                    }
                    (BinaryOperator::Xor, JitValue::Int(a), JitValue::Int(b)) => {
                        JitValue::Int(self.builder.ins().bxor(a, b))
                    }

                    // Floats
                    (BinaryOperator::Add, JitValue::Float(a), JitValue::Float(b)) => {
                        JitValue::Float(self.builder.ins().fadd(a, b))
                    }
                    (BinaryOperator::Subtract, JitValue::Float(a), JitValue::Float(b)) => {
                        JitValue::Float(self.builder.ins().fsub(a, b))
                    }
                    (BinaryOperator::Multiply, JitValue::Float(a), JitValue::Float(b)) => {
                        JitValue::Float(self.builder.ins().fmul(a, b))
                    }
                    (BinaryOperator::Divide, JitValue::Float(a), JitValue::Float(b)) => {
                        JitValue::Float(self.builder.ins().fdiv(a, b))
                    }
                    _ => return Err(JitCompileError::NotSupported),
                };
                self.stack.push(val);

                Ok(())
            }
            Instruction::SetupLoop { .. } | Instruction::PopBlock => {
                // TODO: block support
                Ok(())
            }
            _ => Err(JitCompileError::NotSupported),
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

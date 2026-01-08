// spell-checker: disable
use super::{JitCompileError, JitSig, JitType};
use cranelift::codegen::ir::FuncRef;
use cranelift::prelude::*;
use num_traits::cast::ToPrimitive;
use rustpython_compiler_core::bytecode::{
    self, BinaryOperator, BorrowedConstant, CodeObject, ComparisonOperator, Instruction,
    IntrinsicFunction1, Label, OpArg, OpArgState,
};
use std::collections::HashMap;

#[repr(u16)]
enum CustomTrapCode {
    /// Raised when shifting by a negative number
    NegativeShiftCount = 1,
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
    Null,
    Tuple(Vec<JitValue>),
    FuncRef(FuncRef),
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
            JitValue::None | JitValue::Null | JitValue::Tuple(_) | JitValue::FuncRef(_) => None,
        }
    }

    fn into_value(self) -> Option<Value> {
        match self {
            JitValue::Int(val) | JitValue::Float(val) | JitValue::Bool(val) => Some(val),
            JitValue::None | JitValue::Null | JitValue::Tuple(_) | JitValue::FuncRef(_) => None,
        }
    }
}

#[derive(Clone)]
struct DDValue {
    hi: Value,
    lo: Value,
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
        ret_type: Option<JitType>,
        entry_block: Block,
    ) -> FunctionCompiler<'a, 'b> {
        let mut compiler = FunctionCompiler {
            builder,
            stack: Vec::new(),
            variables: vec![None; num_variables].into_boxed_slice(),
            label_to_block: HashMap::new(),
            sig: JitSig {
                args: arg_types.to_vec(),
                ret: ret_type,
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
            let var = builder.declare_var(ty.to_cranelift());
            Local {
                var,
                ty: ty.clone(),
            }
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
                let zero = self.builder.ins().f64const(0.0);
                let val = self.builder.ins().fcmp(FloatCC::NotEqual, val, zero);
                Ok(val)
            }
            JitValue::Int(val) => {
                let zero = self.builder.ins().iconst(types::I64, 0);
                let val = self.builder.ins().icmp(IntCC::NotEqual, val, zero);
                Ok(val)
            }
            JitValue::Bool(val) => Ok(val),
            JitValue::None => Ok(self.builder.ins().iconst(types::I8, 0)),
            JitValue::Null | JitValue::Tuple(_) | JitValue::FuncRef(_) => {
                Err(JitCompileError::NotSupported)
            }
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
        func_ref: FuncRef,
        bytecode: &CodeObject<C>,
    ) -> Result<(), JitCompileError> {
        let label_targets = bytecode.label_targets();
        let mut arg_state = OpArgState::default();

        // Track whether we have "returned" in the current block
        let mut in_unreachable_code = false;

        for (offset, &raw_instr) in bytecode.instructions.iter().enumerate() {
            let label = Label(offset as u32);
            let (instruction, arg) = arg_state.get(raw_instr);

            // If this is a label that some earlier jump can target,
            // treat it as the start of a new reachable block:
            if label_targets.contains(&label) {
                // Create or get the block for this label:
                let target_block = self.get_or_create_block(label);

                // If the current block isn't terminated, add a fallthrough jump
                if let Some(cur) = self.builder.current_block()
                    && cur != target_block
                {
                    // Check if the block needs a terminator by examining the last instruction
                    let needs_terminator = match self.builder.func.layout.last_inst(cur) {
                        None => true, // Empty block needs terminator
                        Some(inst) => {
                            // Check if the last instruction is a terminator
                            !self.builder.func.dfg.insts[inst].opcode().is_terminator()
                        }
                    };
                    if needs_terminator {
                        self.builder.ins().jump(target_block, &[]);
                    }
                }
                // Switch to the target block
                if self.builder.current_block() != Some(target_block) {
                    self.builder.switch_to_block(target_block);
                }

                // We are definitely reachable again at this label
                in_unreachable_code = false;
            }

            // If we're in unreachable code, skip this instruction unless the label re-entered above.
            if in_unreachable_code {
                continue;
            }

            // Actually compile this instruction:
            self.add_instruction(func_ref, bytecode, instruction, arg)?;

            // If that was an unconditional branch or return, mark future instructions unreachable
            match instruction {
                Instruction::ReturnValue
                | Instruction::ReturnConst { .. }
                | Instruction::Jump { .. }
                | Instruction::JumpBackward { .. }
                | Instruction::JumpBackwardNoInterrupt { .. }
                | Instruction::JumpForward { .. } => {
                    in_unreachable_code = true;
                }
                _ => {}
            }
        }

        // After processing, if the current block is unterminated, insert a trap
        if let Some(cur) = self.builder.current_block() {
            let needs_terminator = match self.builder.func.layout.last_inst(cur) {
                None => true,
                Some(inst) => !self.builder.func.dfg.insts[inst].opcode().is_terminator(),
            };
            if needs_terminator {
                self.builder.ins().trap(TrapCode::user(0).unwrap());
            }
        }
        Ok(())
    }

    fn prepare_const<C: bytecode::Constant>(
        &mut self,
        constant: BorrowedConstant<'_, C>,
    ) -> Result<JitValue, JitCompileError> {
        let value = match constant {
            BorrowedConstant::Integer { value } => {
                let val = self.builder.ins().iconst(
                    types::I64,
                    value.to_i64().ok_or(JitCompileError::NotSupported)?,
                );
                JitValue::Int(val)
            }
            BorrowedConstant::Float { value } => {
                let val = self.builder.ins().f64const(value);
                JitValue::Float(val)
            }
            BorrowedConstant::Boolean { value } => {
                let val = self.builder.ins().iconst(types::I8, value as i64);
                JitValue::Bool(val)
            }
            BorrowedConstant::None => JitValue::None,
            _ => return Err(JitCompileError::NotSupported),
        };
        Ok(value)
    }

    fn return_value(&mut self, val: JitValue) -> Result<(), JitCompileError> {
        if let Some(ref ty) = self.sig.ret {
            // If the signature has a return type, enforce it
            if val.to_jit_type().as_ref() != Some(ty) {
                return Err(JitCompileError::NotSupported);
            }
        } else {
            // First time we see a return, define it in the signature
            let ty = val.to_jit_type().ok_or(JitCompileError::NotSupported)?;
            self.sig.ret = Some(ty.clone());
            self.builder
                .func
                .signature
                .returns
                .push(AbiParam::new(ty.to_cranelift()));
        }

        // If this is e.g. an Int, Float, or Bool we have a Cranelift `Value`.
        // If we have JitValue::None or .Tuple(...) but can't handle that, error out (or handle differently).
        let cr_val = val.into_value().ok_or(JitCompileError::NotSupported)?;

        self.builder.ins().return_(&[cr_val]);
        Ok(())
    }

    pub fn add_instruction<C: bytecode::Constant>(
        &mut self,
        func_ref: FuncRef,
        bytecode: &CodeObject<C>,
        instruction: Instruction,
        arg: OpArg,
    ) -> Result<(), JitCompileError> {
        match instruction {
            Instruction::BinaryOp { op } => {
                let op = op.get(arg);
                // the rhs is popped off first
                let b = self.stack.pop().ok_or(JitCompileError::BadBytecode)?;
                let a = self.stack.pop().ok_or(JitCompileError::BadBytecode)?;

                let a_type = a.to_jit_type();
                let b_type = b.to_jit_type();

                let val = match (op, a, b) {
                    (
                        BinaryOperator::Add | BinaryOperator::InplaceAdd,
                        JitValue::Int(a),
                        JitValue::Int(b),
                    ) => {
                        let (out, carry) = self.builder.ins().sadd_overflow(a, b);
                        self.builder.ins().trapnz(carry, TrapCode::INTEGER_OVERFLOW);
                        JitValue::Int(out)
                    }
                    (
                        BinaryOperator::Subtract | BinaryOperator::InplaceSubtract,
                        JitValue::Int(a),
                        JitValue::Int(b),
                    ) => JitValue::Int(self.compile_sub(a, b)),
                    (
                        BinaryOperator::FloorDivide | BinaryOperator::InplaceFloorDivide,
                        JitValue::Int(a),
                        JitValue::Int(b),
                    ) => JitValue::Int(self.builder.ins().sdiv(a, b)),
                    (
                        BinaryOperator::TrueDivide | BinaryOperator::InplaceTrueDivide,
                        JitValue::Int(a),
                        JitValue::Int(b),
                    ) => {
                        // Check if b == 0, If so trap with a division by zero error
                        self.builder
                            .ins()
                            .trapz(b, TrapCode::INTEGER_DIVISION_BY_ZERO);
                        // Else convert to float and divide
                        let a_float = self.builder.ins().fcvt_from_sint(types::F64, a);
                        let b_float = self.builder.ins().fcvt_from_sint(types::F64, b);
                        JitValue::Float(self.builder.ins().fdiv(a_float, b_float))
                    }
                    (
                        BinaryOperator::Multiply | BinaryOperator::InplaceMultiply,
                        JitValue::Int(a),
                        JitValue::Int(b),
                    ) => JitValue::Int(self.builder.ins().imul(a, b)),
                    (
                        BinaryOperator::Remainder | BinaryOperator::InplaceRemainder,
                        JitValue::Int(a),
                        JitValue::Int(b),
                    ) => JitValue::Int(self.builder.ins().srem(a, b)),
                    (
                        BinaryOperator::Power | BinaryOperator::InplacePower,
                        JitValue::Int(a),
                        JitValue::Int(b),
                    ) => JitValue::Int(self.compile_ipow(a, b)),
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
                            TrapCode::user(CustomTrapCode::NegativeShiftCount as u8).unwrap(),
                        );

                        let out =
                            if matches!(op, BinaryOperator::Lshift | BinaryOperator::InplaceLshift)
                            {
                                self.builder.ins().ishl(a, b)
                            } else {
                                self.builder.ins().sshr(a, b)
                            };
                        JitValue::Int(out)
                    }
                    (
                        BinaryOperator::And | BinaryOperator::InplaceAnd,
                        JitValue::Int(a),
                        JitValue::Int(b),
                    ) => JitValue::Int(self.builder.ins().band(a, b)),
                    (
                        BinaryOperator::Or | BinaryOperator::InplaceOr,
                        JitValue::Int(a),
                        JitValue::Int(b),
                    ) => JitValue::Int(self.builder.ins().bor(a, b)),
                    (
                        BinaryOperator::Xor | BinaryOperator::InplaceXor,
                        JitValue::Int(a),
                        JitValue::Int(b),
                    ) => JitValue::Int(self.builder.ins().bxor(a, b)),

                    // Floats
                    (
                        BinaryOperator::Add | BinaryOperator::InplaceAdd,
                        JitValue::Float(a),
                        JitValue::Float(b),
                    ) => JitValue::Float(self.builder.ins().fadd(a, b)),
                    (
                        BinaryOperator::Subtract | BinaryOperator::InplaceSubtract,
                        JitValue::Float(a),
                        JitValue::Float(b),
                    ) => JitValue::Float(self.builder.ins().fsub(a, b)),
                    (
                        BinaryOperator::Multiply | BinaryOperator::InplaceMultiply,
                        JitValue::Float(a),
                        JitValue::Float(b),
                    ) => JitValue::Float(self.builder.ins().fmul(a, b)),
                    (
                        BinaryOperator::TrueDivide | BinaryOperator::InplaceTrueDivide,
                        JitValue::Float(a),
                        JitValue::Float(b),
                    ) => JitValue::Float(self.builder.ins().fdiv(a, b)),
                    (
                        BinaryOperator::Power | BinaryOperator::InplacePower,
                        JitValue::Float(a),
                        JitValue::Float(b),
                    ) => JitValue::Float(self.compile_fpow(a, b)),

                    // Floats and Integers
                    (_, JitValue::Int(a), JitValue::Float(b))
                    | (_, JitValue::Float(a), JitValue::Int(b)) => {
                        let operand_one = match a_type.unwrap() {
                            JitType::Int => self.builder.ins().fcvt_from_sint(types::F64, a),
                            _ => a,
                        };

                        let operand_two = match b_type.unwrap() {
                            JitType::Int => self.builder.ins().fcvt_from_sint(types::F64, b),
                            _ => b,
                        };

                        match op {
                            BinaryOperator::Add | BinaryOperator::InplaceAdd => {
                                JitValue::Float(self.builder.ins().fadd(operand_one, operand_two))
                            }
                            BinaryOperator::Subtract | BinaryOperator::InplaceSubtract => {
                                JitValue::Float(self.builder.ins().fsub(operand_one, operand_two))
                            }
                            BinaryOperator::Multiply | BinaryOperator::InplaceMultiply => {
                                JitValue::Float(self.builder.ins().fmul(operand_one, operand_two))
                            }
                            BinaryOperator::TrueDivide | BinaryOperator::InplaceTrueDivide => {
                                JitValue::Float(self.builder.ins().fdiv(operand_one, operand_two))
                            }
                            BinaryOperator::Power | BinaryOperator::InplacePower => {
                                JitValue::Float(self.compile_fpow(operand_one, operand_two))
                            }
                            _ => return Err(JitCompileError::NotSupported),
                        }
                    }
                    _ => return Err(JitCompileError::NotSupported),
                };
                self.stack.push(val);

                Ok(())
            }
            Instruction::BuildTuple { size } => {
                let elements = self.pop_multiple(size.get(arg) as usize);
                self.stack.push(JitValue::Tuple(elements));
                Ok(())
            }
            Instruction::Call { nargs } => {
                let nargs = nargs.get(arg);

                let mut args = Vec::new();
                for _ in 0..nargs {
                    let arg = self.stack.pop().ok_or(JitCompileError::BadBytecode)?;
                    args.push(arg.into_value().unwrap());
                }

                // Pop self_or_null (should be Null for JIT-compiled recursive calls)
                let self_or_null = self.stack.pop().ok_or(JitCompileError::BadBytecode)?;
                if !matches!(self_or_null, JitValue::Null) {
                    return Err(JitCompileError::NotSupported);
                }

                match self.stack.pop().ok_or(JitCompileError::BadBytecode)? {
                    JitValue::FuncRef(reference) => {
                        let call = self.builder.ins().call(reference, &args);
                        let returns = self.builder.inst_results(call);
                        self.stack.push(JitValue::Int(returns[0]));

                        Ok(())
                    }
                    _ => Err(JitCompileError::BadBytecode),
                }
            }
            Instruction::PushNull => {
                self.stack.push(JitValue::Null);
                Ok(())
            }
            Instruction::CallIntrinsic1 { func } => {
                match func.get(arg) {
                    IntrinsicFunction1::UnaryPositive => {
                        match self.stack.pop().ok_or(JitCompileError::BadBytecode)? {
                            JitValue::Int(val) => {
                                // Nothing to do
                                self.stack.push(JitValue::Int(val));
                                Ok(())
                            }
                            _ => Err(JitCompileError::NotSupported),
                        }
                    }
                    _ => Err(JitCompileError::NotSupported),
                }
            }
            Instruction::CompareOp { op, .. } => {
                let op = op.get(arg);
                // the rhs is popped off first
                let b = self.stack.pop().ok_or(JitCompileError::BadBytecode)?;
                let a = self.stack.pop().ok_or(JitCompileError::BadBytecode)?;

                let a_type: Option<JitType> = a.to_jit_type();
                let b_type: Option<JitType> = b.to_jit_type();

                match (a, b) {
                    (JitValue::Int(a), JitValue::Int(b))
                    | (JitValue::Bool(a), JitValue::Bool(b))
                    | (JitValue::Bool(a), JitValue::Int(b))
                    | (JitValue::Int(a), JitValue::Bool(b)) => {
                        let operand_one = match a_type.unwrap() {
                            JitType::Bool => self.builder.ins().uextend(types::I64, a),
                            _ => a,
                        };

                        let operand_two = match b_type.unwrap() {
                            JitType::Bool => self.builder.ins().uextend(types::I64, b),
                            _ => b,
                        };

                        let cond = match op {
                            ComparisonOperator::Equal => IntCC::Equal,
                            ComparisonOperator::NotEqual => IntCC::NotEqual,
                            ComparisonOperator::Less => IntCC::SignedLessThan,
                            ComparisonOperator::LessOrEqual => IntCC::SignedLessThanOrEqual,
                            ComparisonOperator::Greater => IntCC::SignedGreaterThan,
                            ComparisonOperator::GreaterOrEqual => IntCC::SignedGreaterThanOrEqual,
                        };

                        let val = self.builder.ins().icmp(cond, operand_one, operand_two);
                        self.stack.push(JitValue::Bool(val));
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
                        self.stack.push(JitValue::Bool(val));
                        Ok(())
                    }
                    _ => Err(JitCompileError::NotSupported),
                }
            }
            Instruction::ExtendedArg => Ok(()),

            Instruction::Jump { target }
            | Instruction::JumpBackward { target }
            | Instruction::JumpBackwardNoInterrupt { target }
            | Instruction::JumpForward { target } => {
                let target_block = self.get_or_create_block(target.get(arg));
                self.builder.ins().jump(target_block, &[]);
                Ok(())
            }
            Instruction::LoadConst { idx } => {
                let val = self
                    .prepare_const(bytecode.constants[idx.get(arg) as usize].borrow_constant())?;
                self.stack.push(val);
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
            Instruction::LoadGlobal(idx) => {
                let name = &bytecode.names[idx.get(arg) as usize];

                if name.as_ref() != bytecode.obj_name.as_ref() {
                    Err(JitCompileError::NotSupported)
                } else {
                    self.stack.push(JitValue::FuncRef(func_ref));
                    Ok(())
                }
            }
            Instruction::Nop => Ok(()),
            Instruction::PopBlock => {
                // TODO: block support
                Ok(())
            }
            Instruction::PopJumpIfFalse { target } => {
                let cond = self.stack.pop().ok_or(JitCompileError::BadBytecode)?;
                let val = self.boolean_val(cond)?;
                let then_block = self.get_or_create_block(target.get(arg));
                let else_block = self.builder.create_block();

                self.builder
                    .ins()
                    .brif(val, else_block, &[], then_block, &[]);
                self.builder.switch_to_block(else_block);

                Ok(())
            }
            Instruction::PopJumpIfTrue { target } => {
                let cond = self.stack.pop().ok_or(JitCompileError::BadBytecode)?;
                let val = self.boolean_val(cond)?;
                let then_block = self.get_or_create_block(target.get(arg));
                let else_block = self.builder.create_block();

                self.builder
                    .ins()
                    .brif(val, then_block, &[], else_block, &[]);
                self.builder.switch_to_block(else_block);

                Ok(())
            }
            Instruction::PopTop => {
                self.stack.pop();
                Ok(())
            }
            Instruction::Resume { arg: _resume_arg } => {
                // TODO: Implement the resume instruction
                Ok(())
            }
            Instruction::ReturnConst { idx } => {
                let val = self
                    .prepare_const(bytecode.constants[idx.get(arg) as usize].borrow_constant())?;
                self.return_value(val)
            }
            Instruction::ReturnValue => {
                let val = self.stack.pop().ok_or(JitCompileError::BadBytecode)?;
                self.return_value(val)
            }
            Instruction::StoreFast(idx) => {
                let val = self.stack.pop().ok_or(JitCompileError::BadBytecode)?;
                self.store_variable(idx.get(arg), val)
            }
            Instruction::Swap { index } => {
                let len = self.stack.len();
                let i = len - 1;
                let j = len - 1 - index.get(arg) as usize;
                self.stack.swap(i, j);
                Ok(())
            }
            Instruction::ToBool => {
                let a = self.stack.pop().ok_or(JitCompileError::BadBytecode)?;
                let value = self.boolean_val(a)?;
                self.stack.push(JitValue::Bool(value));
                Ok(())
            }
            Instruction::UnaryNot => {
                let boolean = match self.stack.pop().ok_or(JitCompileError::BadBytecode)? {
                    JitValue::Bool(val) => val,
                    _ => return Err(JitCompileError::BadBytecode),
                };
                let not_boolean = self.builder.ins().bxor_imm(boolean, 1);
                self.stack.push(JitValue::Bool(not_boolean));
                Ok(())
            }
            Instruction::UnaryNegative => {
                match self.stack.pop().ok_or(JitCompileError::BadBytecode)? {
                    JitValue::Int(val) => {
                        // Compile minus as 0 - val.
                        let zero = self.builder.ins().iconst(types::I64, 0);
                        let out = self.compile_sub(zero, val);
                        self.stack.push(JitValue::Int(out));
                        Ok(())
                    }
                    _ => Err(JitCompileError::NotSupported),
                }
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
            _ => Err(JitCompileError::NotSupported),
        }
    }

    fn compile_sub(&mut self, a: Value, b: Value) -> Value {
        let (out, carry) = self.builder.ins().ssub_overflow(a, b);
        self.builder.ins().trapnz(carry, TrapCode::INTEGER_OVERFLOW);
        out
    }

    /// Creates a double–double (DDValue) from a regular f64 constant.
    /// The high part is set to x and the low part is set to 0.0.
    fn dd_from_f64(&mut self, x: f64) -> DDValue {
        DDValue {
            hi: self.builder.ins().f64const(x),
            lo: self.builder.ins().f64const(0.0),
        }
    }

    /// Creates a DDValue from a Value (assumed to represent an f64).
    /// This function initializes the high part with x and the low part to 0.0.
    fn dd_from_value(&mut self, x: Value) -> DDValue {
        DDValue {
            hi: x,
            lo: self.builder.ins().f64const(0.0),
        }
    }

    /// Creates a DDValue from two f64 parts.
    /// The 'hi' parameter sets the high part and 'lo' sets the low part.
    fn dd_from_parts(&mut self, hi: f64, lo: f64) -> DDValue {
        DDValue {
            hi: self.builder.ins().f64const(hi),
            lo: self.builder.ins().f64const(lo),
        }
    }

    /// Converts a DDValue back to a single f64 value by adding the high and low parts.
    fn dd_to_f64(&mut self, dd: DDValue) -> Value {
        self.builder.ins().fadd(dd.hi, dd.lo)
    }

    /// Computes the negation of a DDValue.
    /// It subtracts both the high and low parts from zero.
    fn dd_neg(&mut self, dd: DDValue) -> DDValue {
        let zero = self.builder.ins().f64const(0.0);
        DDValue {
            hi: self.builder.ins().fsub(zero, dd.hi),
            lo: self.builder.ins().fsub(zero, dd.lo),
        }
    }

    /// Adds two DDValue numbers using error-free transformations to maintain extra precision.
    /// It carefully adds the high parts, computes the rounding error, adds the low parts along with the error,
    /// and then normalizes the result.
    fn dd_add(&mut self, a: DDValue, b: DDValue) -> DDValue {
        // Compute the sum of the high parts.
        let s = self.builder.ins().fadd(a.hi, b.hi);
        // Compute t = s - a.hi to capture part of the rounding error.
        let t = self.builder.ins().fsub(s, a.hi);
        // Compute the error e from the high part additions.
        let s_minus_t = self.builder.ins().fsub(s, t);
        let part1 = self.builder.ins().fsub(a.hi, s_minus_t);
        let part2 = self.builder.ins().fsub(b.hi, t);
        let e = self.builder.ins().fadd(part1, part2);
        // Sum the low parts along with the error.
        let lo = self.builder.ins().fadd(a.lo, b.lo);
        let lo_sum = self.builder.ins().fadd(lo, e);
        // Renormalize: add the low sum to s and compute a new low component.
        let hi_new = self.builder.ins().fadd(s, lo_sum);
        let hi_new_minus_s = self.builder.ins().fsub(hi_new, s);
        let lo_new = self.builder.ins().fsub(lo_sum, hi_new_minus_s);
        DDValue {
            hi: hi_new,
            lo: lo_new,
        }
    }

    /// Subtracts DDValue b from DDValue a by negating b and then using the addition function.
    fn dd_sub(&mut self, a: DDValue, b: DDValue) -> DDValue {
        let neg_b = self.dd_neg(b);
        self.dd_add(a, neg_b)
    }

    /// Multiplies two DDValue numbers using double–double arithmetic.
    /// It calculates the high product, uses a fused multiply–add (FMA) to capture rounding error,
    /// computes the cross products, and then normalizes the result.
    fn dd_mul(&mut self, a: DDValue, b: DDValue) -> DDValue {
        // p = a.hi * b.hi (primary product)
        let p = self.builder.ins().fmul(a.hi, b.hi);
        // err = fma(a.hi, b.hi, -p) recovers the rounding error.
        let zero = self.builder.ins().f64const(0.0);
        let neg_p = self.builder.ins().fsub(zero, p);
        let err = self.builder.ins().fma(a.hi, b.hi, neg_p);
        // Compute cross terms: a.hi*b.lo + a.lo*b.hi.
        let a_hi_b_lo = self.builder.ins().fmul(a.hi, b.lo);
        let a_lo_b_hi = self.builder.ins().fmul(a.lo, b.hi);
        let cross = self.builder.ins().fadd(a_hi_b_lo, a_lo_b_hi);
        // Sum p and the cross terms.
        let s = self.builder.ins().fadd(p, cross);
        // Isolate rounding error from the addition.
        let t = self.builder.ins().fsub(s, p);
        let s_minus_t = self.builder.ins().fsub(s, t);
        let part1 = self.builder.ins().fsub(p, s_minus_t);
        let part2 = self.builder.ins().fsub(cross, t);
        let e = self.builder.ins().fadd(part1, part2);
        // Include the error from the low parts multiplication.
        let a_lo_b_lo = self.builder.ins().fmul(a.lo, b.lo);
        let err_plus_e = self.builder.ins().fadd(err, e);
        let lo_sum = self.builder.ins().fadd(err_plus_e, a_lo_b_lo);
        // Renormalize the sum.
        let hi_new = self.builder.ins().fadd(s, lo_sum);
        let hi_new_minus_s = self.builder.ins().fsub(hi_new, s);
        let lo_new = self.builder.ins().fsub(lo_sum, hi_new_minus_s);
        DDValue {
            hi: hi_new,
            lo: lo_new,
        }
    }

    /// Multiplies a DDValue by a regular f64 (Value) using similar techniques as dd_mul.
    /// It multiplies both the high and low parts by b, computes the rounding error,
    /// and then renormalizes the result.
    fn dd_mul_f64(&mut self, a: DDValue, b: Value) -> DDValue {
        // p = a.hi * b (primary product)
        let p = self.builder.ins().fmul(a.hi, b);
        // Compute the rounding error using fma.
        let zero = self.builder.ins().f64const(0.0);
        let neg_p = self.builder.ins().fsub(zero, p);
        let err = self.builder.ins().fma(a.hi, b, neg_p);
        // Multiply the low part.
        let cross = self.builder.ins().fmul(a.lo, b);
        // Sum the primary product and the low multiplication.
        let s = self.builder.ins().fadd(p, cross);
        // Capture rounding error from addition.
        let t = self.builder.ins().fsub(s, p);
        let s_minus_t = self.builder.ins().fsub(s, t);
        let part1 = self.builder.ins().fsub(p, s_minus_t);
        let part2 = self.builder.ins().fsub(cross, t);
        let e = self.builder.ins().fadd(part1, part2);
        // Combine the error components.
        let lo_sum = self.builder.ins().fadd(err, e);
        // Renormalize to form the final double–double number.
        let hi_new = self.builder.ins().fadd(s, lo_sum);
        let hi_new_minus_s = self.builder.ins().fsub(hi_new, s);
        let lo_new = self.builder.ins().fsub(lo_sum, hi_new_minus_s);
        DDValue {
            hi: hi_new,
            lo: lo_new,
        }
    }

    /// Scales a DDValue by multiplying both its high and low parts by the given factor.
    fn dd_scale(&mut self, dd: DDValue, factor: Value) -> DDValue {
        DDValue {
            hi: self.builder.ins().fmul(dd.hi, factor),
            lo: self.builder.ins().fmul(dd.lo, factor),
        }
    }

    /// Approximates ln(1+f) using its Taylor series expansion in double–double arithmetic.
    /// It computes the series ∑ (-1)^(i-1) * f^i / i from i = 1 to 1000 for high precision.
    fn dd_ln_1p_series(&mut self, f: Value) -> DDValue {
        // Convert f to a DDValue and initialize the sum and term.
        let f_dd = self.dd_from_value(f);
        let mut sum = f_dd.clone();
        let mut term = f_dd;
        // Alternating sign starts at -1 for the second term.
        let mut sign = -1.0_f64;
        let range = 1000;

        // Loop over terms from i = 2 to 1000.
        for i in 2..=range {
            // Compute f^i by multiplying the previous term by f.
            term = self.dd_mul_f64(term, f);
            // Divide the term by i.
            let inv_i = 1.0 / (i as f64);
            let c_inv_i = self.builder.ins().f64const(inv_i);
            let term_div = self.dd_mul_f64(term.clone(), c_inv_i);
            // Multiply by the alternating sign.
            let dd_sign = self.dd_from_f64(sign);
            let to_add = self.dd_mul(dd_sign, term_div);
            // Add the term to the cumulative sum.
            sum = self.dd_add(sum, to_add);
            // Flip the sign for the next term.
            sign = -sign;
        }
        sum
    }

    /// Computes the natural logarithm ln(x) in double–double arithmetic.
    /// It first checks for domain errors (x ≤ 0 or NaN), then extracts the exponent
    /// and mantissa from the bit-level representation of x. It computes ln(mantissa) using
    /// the ln(1+f) series and adds k*ln2 to obtain ln(x).
    fn dd_ln(&mut self, x: Value) -> DDValue {
        // (A) Prepare a DDValue representing NaN.
        let dd_nan = self.dd_from_f64(f64::NAN);

        // Build a zero constant for comparisons.
        let zero_f64 = self.builder.ins().f64const(0.0);

        // Check if x is less than or equal to 0 or is NaN.
        let cmp_le = self
            .builder
            .ins()
            .fcmp(FloatCC::LessThanOrEqual, x, zero_f64);
        let cmp_nan = self.builder.ins().fcmp(FloatCC::Unordered, x, x);
        let need_nan = self.builder.ins().bor(cmp_le, cmp_nan);

        // (B) Reinterpret the bits of x as an integer.
        let bits = self.builder.ins().bitcast(types::I64, MemFlags::new(), x);

        // (C) Extract the exponent (top 11 bits) from the bit representation.
        let shift_52 = self.builder.ins().ushr_imm(bits, 52);
        let exponent_mask = self.builder.ins().iconst(types::I64, 0x7FF);
        let exponent = self.builder.ins().band(shift_52, exponent_mask);

        // k = exponent - 1023 (unbias the exponent).
        let bias = self.builder.ins().iconst(types::I64, 1023);
        let k_i64 = self.builder.ins().isub(exponent, bias);

        // (D) Extract the fraction (mantissa) from the lower 52 bits.
        let fraction_mask = self.builder.ins().iconst(types::I64, 0x000F_FFFF_FFFF_FFFF);
        let fraction_part = self.builder.ins().band(bits, fraction_mask);

        // (E) For normal numbers (exponent ≠ 0), add the implicit leading 1.
        let implicit_one = self.builder.ins().iconst(types::I64, 1 << 52);
        let zero_exp = self.builder.ins().icmp_imm(IntCC::Equal, exponent, 0);
        let frac_one_bor = self.builder.ins().bor(fraction_part, implicit_one);
        let fraction_with_leading_one = self.builder.ins().select(
            zero_exp,
            fraction_part, // For subnormals, do not add the implicit 1.
            frac_one_bor,
        );

        // (F) Force the exponent bits to 1023, yielding a mantissa m in [1, 2).
        let new_exp = self.builder.ins().iconst(types::I64, 0x3FF0_0000_0000_0000);
        let fraction_bits = self.builder.ins().bor(fraction_with_leading_one, new_exp);
        let m = self
            .builder
            .ins()
            .bitcast(types::F64, MemFlags::new(), fraction_bits);

        // (G) Compute ln(m) using the series ln(1+f) with f = m - 1.
        let one_f64 = self.builder.ins().f64const(1.0);
        let f_val = self.builder.ins().fsub(m, one_f64);
        let dd_ln_m = self.dd_ln_1p_series(f_val);

        // (H) Compute k*ln2 in double–double arithmetic.
        let ln2_dd = self.dd_from_parts(
            f64::from_bits(0x3fe62e42fefa39ef),
            f64::from_bits(0x3c7abc9e3b39803f),
        );
        let k_f64 = self.builder.ins().fcvt_from_sint(types::F64, k_i64);
        let dd_ln2_k = self.dd_mul_f64(ln2_dd, k_f64);

        // Add ln(m) and k*ln2 to get the final ln(x).
        let normal_result = self.dd_add(dd_ln_m, dd_ln2_k);

        // (I) If x was nonpositive or NaN, return NaN; otherwise, return the computed result.
        let final_hi = self
            .builder
            .ins()
            .select(need_nan, dd_nan.hi, normal_result.hi);
        let final_lo = self
            .builder
            .ins()
            .select(need_nan, dd_nan.lo, normal_result.lo);

        DDValue {
            hi: final_hi,
            lo: final_lo,
        }
    }

    /// Computes the exponential function exp(x) in double–double arithmetic.
    /// It uses range reduction to write x = k*ln2 + r, computes exp(r) via a Taylor series,
    /// scales the result by 2^k, and handles overflow by checking if k exceeds the maximum.
    fn dd_exp(&mut self, dd: DDValue) -> DDValue {
        // (A) Range reduction: Convert dd to a single f64 value.
        let x = self.dd_to_f64(dd.clone());
        let ln2_f64 = self
            .builder
            .ins()
            .f64const(f64::from_bits(0x3fe62e42fefa39ef));
        let div = self.builder.ins().fdiv(x, ln2_f64);
        let half = self.builder.ins().f64const(0.5);
        let div_plus_half = self.builder.ins().fadd(div, half);
        // Rounding: floor(div + 0.5) gives the nearest integer k.
        let k = self.builder.ins().fcvt_to_sint(types::I64, div_plus_half);

        // --- OVERFLOW CHECK ---
        // Check if k is greater than the maximum exponent for finite doubles (1023).
        let max_k = self.builder.ins().iconst(types::I64, 1023);
        let is_overflow = self.builder.ins().icmp(IntCC::SignedGreaterThan, k, max_k);

        // Define infinity and zero for the overflow case.
        let inf = self.builder.ins().f64const(f64::INFINITY);
        let zero = self.builder.ins().f64const(0.0);

        // (B) Compute exp(x) normally when not overflowing.
        // Compute k*ln2 in double–double arithmetic and subtract it from x.
        let ln2_dd = self.dd_from_parts(
            f64::from_bits(0x3fe62e42fefa39ef),
            f64::from_bits(0x3c7abc9e3b39803f),
        );
        let k_f64 = self.builder.ins().fcvt_from_sint(types::F64, k);
        let k_ln2 = self.dd_mul_f64(ln2_dd, k_f64);
        let r = self.dd_sub(dd, k_ln2);

        // Compute exp(r) using a Taylor series.
        let mut sum = self.dd_from_f64(1.0); // Initialize sum to 1.
        let mut term = self.dd_from_f64(1.0); // Initialize the first term to 1.
        let n_terms = 1000;
        for i in 1..=n_terms {
            term = self.dd_mul(term, r.clone());
            let inv = 1.0 / (i as f64);
            let inv_const = self.builder.ins().f64const(inv);
            term = self.dd_mul_f64(term, inv_const);
            sum = self.dd_add(sum, term.clone());
        }

        // Reconstruct the final result by scaling with 2^k.
        let bias = self.builder.ins().iconst(types::I64, 1023);
        let k_plus_bias = self.builder.ins().iadd(k, bias);
        let shift_count = self.builder.ins().iconst(types::I64, 52);
        let shifted = self.builder.ins().ishl(k_plus_bias, shift_count);
        let two_to_k = self
            .builder
            .ins()
            .bitcast(types::F64, MemFlags::new(), shifted);
        let result = self.dd_scale(sum, two_to_k);

        // (C) If overflow was detected, return infinity; otherwise, return the computed value.
        let final_hi = self.builder.ins().select(is_overflow, inf, result.hi);
        let final_lo = self.builder.ins().select(is_overflow, zero, result.lo);
        DDValue {
            hi: final_hi,
            lo: final_lo,
        }
    }

    /// Computes the power function a^b (f_pow) for f64 values using double–double arithmetic for high precision.
    /// It handles different cases for the base 'a':
    /// - For a > 0: Computes exp(b * ln(a)).
    /// - For a == 0: Handles special cases for 0^b, including returning 0, 1, or a domain error.
    /// - For a < 0: Allows only an integer exponent b and adjusts the sign if b is odd.
    fn compile_fpow(&mut self, a: Value, b: Value) -> Value {
        let f64_ty = types::F64;
        let i64_ty = types::I64;
        let zero_f = self.builder.ins().f64const(0.0);
        let one_f = self.builder.ins().f64const(1.0);
        let nan_f = self.builder.ins().f64const(f64::NAN);
        let inf_f = self.builder.ins().f64const(f64::INFINITY);
        let neg_inf_f = self.builder.ins().f64const(f64::NEG_INFINITY);

        // Merge block for final result.
        let merge_block = self.builder.create_block();
        self.builder.append_block_param(merge_block, f64_ty);

        // --- Edge Case 1: b == 0.0 → return 1.0
        let cmp_b_zero = self.builder.ins().fcmp(FloatCC::Equal, b, zero_f);
        let b_zero_block = self.builder.create_block();
        let continue_block = self.builder.create_block();
        self.builder
            .ins()
            .brif(cmp_b_zero, b_zero_block, &[], continue_block, &[]);
        self.builder.switch_to_block(b_zero_block);
        self.builder.ins().jump(merge_block, &[one_f.into()]);
        self.builder.switch_to_block(continue_block);

        // --- Edge Case 2: b is NaN → return NaN
        let cmp_b_nan = self.builder.ins().fcmp(FloatCC::Unordered, b, b);
        let b_nan_block = self.builder.create_block();
        let continue_block2 = self.builder.create_block();
        self.builder
            .ins()
            .brif(cmp_b_nan, b_nan_block, &[], continue_block2, &[]);
        self.builder.switch_to_block(b_nan_block);
        self.builder.ins().jump(merge_block, &[nan_f.into()]);
        self.builder.switch_to_block(continue_block2);

        // --- Edge Case 3: a == 0.0 → return 0.0
        let cmp_a_zero = self.builder.ins().fcmp(FloatCC::Equal, a, zero_f);
        let a_zero_block = self.builder.create_block();
        let continue_block3 = self.builder.create_block();
        self.builder
            .ins()
            .brif(cmp_a_zero, a_zero_block, &[], continue_block3, &[]);
        self.builder.switch_to_block(a_zero_block);
        self.builder.ins().jump(merge_block, &[zero_f.into()]);
        self.builder.switch_to_block(continue_block3);

        // --- Edge Case 4: a is NaN → return NaN
        let cmp_a_nan = self.builder.ins().fcmp(FloatCC::Unordered, a, a);
        let a_nan_block = self.builder.create_block();
        let continue_block4 = self.builder.create_block();
        self.builder
            .ins()
            .brif(cmp_a_nan, a_nan_block, &[], continue_block4, &[]);
        self.builder.switch_to_block(a_nan_block);
        self.builder.ins().jump(merge_block, &[nan_f.into()]);
        self.builder.switch_to_block(continue_block4);

        // --- Edge Case 5: b == +infinity → return +infinity
        let cmp_b_inf = self.builder.ins().fcmp(FloatCC::Equal, b, inf_f);
        let b_inf_block = self.builder.create_block();
        let continue_block5 = self.builder.create_block();
        self.builder
            .ins()
            .brif(cmp_b_inf, b_inf_block, &[], continue_block5, &[]);
        self.builder.switch_to_block(b_inf_block);
        self.builder.ins().jump(merge_block, &[inf_f.into()]);
        self.builder.switch_to_block(continue_block5);

        // --- Edge Case 6: b == -infinity → return 0.0
        let cmp_b_neg_inf = self.builder.ins().fcmp(FloatCC::Equal, b, neg_inf_f);
        let b_neg_inf_block = self.builder.create_block();
        let continue_block6 = self.builder.create_block();
        self.builder
            .ins()
            .brif(cmp_b_neg_inf, b_neg_inf_block, &[], continue_block6, &[]);
        self.builder.switch_to_block(b_neg_inf_block);
        self.builder.ins().jump(merge_block, &[zero_f.into()]);
        self.builder.switch_to_block(continue_block6);

        // --- Edge Case 7: a == +infinity → return +infinity
        let cmp_a_inf = self.builder.ins().fcmp(FloatCC::Equal, a, inf_f);
        let a_inf_block = self.builder.create_block();
        let continue_block7 = self.builder.create_block();
        self.builder
            .ins()
            .brif(cmp_a_inf, a_inf_block, &[], continue_block7, &[]);
        self.builder.switch_to_block(a_inf_block);
        self.builder.ins().jump(merge_block, &[inf_f.into()]);
        self.builder.switch_to_block(continue_block7);

        // --- Edge Case 8: a == -infinity → check exponent parity
        let cmp_a_neg_inf = self.builder.ins().fcmp(FloatCC::Equal, a, neg_inf_f);
        let a_neg_inf_block = self.builder.create_block();
        let continue_block8 = self.builder.create_block();
        self.builder
            .ins()
            .brif(cmp_a_neg_inf, a_neg_inf_block, &[], continue_block8, &[]);

        self.builder.switch_to_block(a_neg_inf_block);
        // a is -infinity here. First, ensure that b is an integer.
        let b_floor = self.builder.ins().floor(b);
        let cmp_int = self.builder.ins().fcmp(FloatCC::Equal, b_floor, b);
        let domain_error_blk = self.builder.create_block();
        let continue_neg_inf = self.builder.create_block();
        self.builder
            .ins()
            .brif(cmp_int, continue_neg_inf, &[], domain_error_blk, &[]);

        self.builder.switch_to_block(domain_error_blk);
        self.builder.ins().jump(merge_block, &[nan_f.into()]);

        self.builder.switch_to_block(continue_neg_inf);
        // b is an integer here; convert b_floor to an i64.
        let b_i64 = self.builder.ins().fcvt_to_sint(i64_ty, b_floor);
        let one_i = self.builder.ins().iconst(i64_ty, 1);
        let remainder = self.builder.ins().band(b_i64, one_i);
        let zero_i = self.builder.ins().iconst(i64_ty, 0);
        let is_odd = self.builder.ins().icmp(IntCC::NotEqual, remainder, zero_i);

        // Create separate blocks for odd and even cases.
        let odd_block = self.builder.create_block();
        let even_block = self.builder.create_block();
        self.builder.append_block_param(odd_block, f64_ty);
        self.builder.append_block_param(even_block, f64_ty);
        self.builder.ins().brif(
            is_odd,
            odd_block,
            &[neg_inf_f.into()],
            even_block,
            &[inf_f.into()],
        );

        self.builder.switch_to_block(odd_block);
        let phi_neg_inf = self.builder.block_params(odd_block)[0];
        self.builder.ins().jump(merge_block, &[phi_neg_inf.into()]);

        self.builder.switch_to_block(even_block);
        let phi_inf = self.builder.block_params(even_block)[0];
        self.builder.ins().jump(merge_block, &[phi_inf.into()]);

        self.builder.switch_to_block(continue_block8);

        // --- Normal branch: neither a nor b hit the special cases.
        // Here we branch based on the sign of a.
        let cmp_lt = self.builder.ins().fcmp(FloatCC::LessThan, a, zero_f);
        let a_neg_block = self.builder.create_block();
        let a_pos_block = self.builder.create_block();
        self.builder
            .ins()
            .brif(cmp_lt, a_neg_block, &[], a_pos_block, &[]);

        // ----- Case: a > 0: Compute a^b = exp(b * ln(a)) using double–double arithmetic.
        self.builder.switch_to_block(a_pos_block);
        let ln_a_dd = self.dd_ln(a);
        let b_dd = self.dd_from_value(b);
        let product_dd = self.dd_mul(ln_a_dd, b_dd);
        let exp_dd = self.dd_exp(product_dd);
        let pos_res = self.dd_to_f64(exp_dd);
        self.builder.ins().jump(merge_block, &[pos_res.into()]);

        // ----- Case: a < 0: Only allow an integral exponent.
        self.builder.switch_to_block(a_neg_block);
        let b_floor = self.builder.ins().floor(b);
        let cmp_int = self.builder.ins().fcmp(FloatCC::Equal, b_floor, b);
        let neg_int_block = self.builder.create_block();
        let domain_error_blk = self.builder.create_block();
        self.builder
            .ins()
            .brif(cmp_int, neg_int_block, &[], domain_error_blk, &[]);

        // Domain error: non-integer exponent for negative base
        self.builder.switch_to_block(domain_error_blk);
        self.builder.ins().jump(merge_block, &[nan_f.into()]);

        // For negative base with an integer exponent:
        self.builder.switch_to_block(neg_int_block);
        let abs_a = self.builder.ins().fabs(a);
        let ln_abs_dd = self.dd_ln(abs_a);
        let b_dd = self.dd_from_value(b);
        let product_dd = self.dd_mul(ln_abs_dd, b_dd);
        let exp_dd = self.dd_exp(product_dd);
        let mag_val = self.dd_to_f64(exp_dd);

        let b_i64 = self.builder.ins().fcvt_to_sint(i64_ty, b_floor);
        let one_i = self.builder.ins().iconst(i64_ty, 1);
        let remainder = self.builder.ins().band(b_i64, one_i);
        let zero_i = self.builder.ins().iconst(i64_ty, 0);
        let is_odd = self.builder.ins().icmp(IntCC::NotEqual, remainder, zero_i);

        let odd_block = self.builder.create_block();
        let even_block = self.builder.create_block();
        // Append block parameters for both branches:
        self.builder.append_block_param(odd_block, f64_ty);
        self.builder.append_block_param(even_block, f64_ty);
        // Pass mag_val to both branches:
        self.builder.ins().brif(
            is_odd,
            odd_block,
            &[mag_val.into()],
            even_block,
            &[mag_val.into()],
        );

        self.builder.switch_to_block(odd_block);
        let phi_mag_val = self.builder.block_params(odd_block)[0];
        let neg_val = self.builder.ins().fneg(phi_mag_val);
        self.builder.ins().jump(merge_block, &[neg_val.into()]);

        self.builder.switch_to_block(even_block);
        let phi_mag_val_even = self.builder.block_params(even_block)[0];
        self.builder
            .ins()
            .jump(merge_block, &[phi_mag_val_even.into()]);

        // ----- Merge: Return the final result.
        self.builder.switch_to_block(merge_block);
        self.builder.block_params(merge_block)[0]
    }

    fn compile_ipow(&mut self, a: Value, b: Value) -> Value {
        let zero = self.builder.ins().iconst(types::I64, 0);
        let one_i64 = self.builder.ins().iconst(types::I64, 1);

        // Create required blocks
        let check_negative = self.builder.create_block();
        let handle_negative = self.builder.create_block();
        let loop_block = self.builder.create_block();
        let continue_block = self.builder.create_block();
        let exit_block = self.builder.create_block();

        // Set up block parameters
        self.builder.append_block_param(check_negative, types::I64); // exponent
        self.builder.append_block_param(check_negative, types::I64); // base

        self.builder.append_block_param(handle_negative, types::I64); // abs(exponent)
        self.builder.append_block_param(handle_negative, types::I64); // base

        self.builder.append_block_param(loop_block, types::I64); // exponent
        self.builder.append_block_param(loop_block, types::I64); // result
        self.builder.append_block_param(loop_block, types::I64); // base

        self.builder.append_block_param(exit_block, types::I64); // final result

        // Set up parameters for continue_block
        self.builder.append_block_param(continue_block, types::I64); // exponent
        self.builder.append_block_param(continue_block, types::I64); // result
        self.builder.append_block_param(continue_block, types::I64); // base

        // Initial jump to check if exponent is negative
        self.builder
            .ins()
            .jump(check_negative, &[b.into(), a.into()]);

        // Check if exponent is negative
        self.builder.switch_to_block(check_negative);
        let params = self.builder.block_params(check_negative);
        let exp_check = params[0];
        let base_check = params[1];

        let is_negative = self
            .builder
            .ins()
            .icmp(IntCC::SignedLessThan, exp_check, zero);
        self.builder.ins().brif(
            is_negative,
            handle_negative,
            &[exp_check.into(), base_check.into()],
            loop_block,
            &[exp_check.into(), one_i64.into(), base_check.into()],
        );

        // Handle negative exponent (return 0 for integer exponentiation)
        self.builder.switch_to_block(handle_negative);
        self.builder.ins().jump(exit_block, &[zero.into()]); // Return 0 for negative exponents

        // Loop block logic (square-and-multiply algorithm)
        self.builder.switch_to_block(loop_block);
        let params = self.builder.block_params(loop_block);
        let exp_phi = params[0];
        let result_phi = params[1];
        let base_phi = params[2];

        // Check if exponent is zero
        let is_zero = self.builder.ins().icmp(IntCC::Equal, exp_phi, zero);
        self.builder.ins().brif(
            is_zero,
            exit_block,
            &[result_phi.into()],
            continue_block,
            &[exp_phi.into(), result_phi.into(), base_phi.into()],
        );

        // Continue block for non-zero case
        self.builder.switch_to_block(continue_block);
        let params = self.builder.block_params(continue_block);
        let exp_phi = params[0];
        let result_phi = params[1];
        let base_phi = params[2];

        // If exponent is odd, multiply result by base
        let is_odd = self.builder.ins().band_imm(exp_phi, 1);
        let is_odd = self.builder.ins().icmp_imm(IntCC::Equal, is_odd, 1);
        let mul_result = self.builder.ins().imul(result_phi, base_phi);
        let new_result = self.builder.ins().select(is_odd, mul_result, result_phi);

        // Square the base and divide exponent by 2
        let squared_base = self.builder.ins().imul(base_phi, base_phi);
        let new_exp = self.builder.ins().sshr_imm(exp_phi, 1);
        self.builder.ins().jump(
            loop_block,
            &[new_exp.into(), new_result.into(), squared_base.into()],
        );

        // Exit block
        self.builder.switch_to_block(exit_block);
        let res = self.builder.block_params(exit_block)[0];

        // Seal all blocks
        self.builder.seal_block(check_negative);
        self.builder.seal_block(handle_negative);
        self.builder.seal_block(loop_block);
        self.builder.seal_block(continue_block);
        self.builder.seal_block(exit_block);

        res
    }
}

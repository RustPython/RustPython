// spell-checker: disable
use super::{
    DEOPT_HEADER_SLOTS, DeoptSite, JitCompileError, JitSig, JitType, MAX_DEOPT_SLOTS, SLOT_SIZE,
    Safety, StackEntry,
};
use alloc::collections::BTreeSet;
use cranelift::codegen::ir::FuncRef;
use cranelift::prelude::*;
use num_traits::cast::ToPrimitive;
use rustpython_compiler_core::bytecode::{
    self, BinaryOperator, BorrowedConstant, CodeObject, ComparisonOperator, Instruction,
    IntrinsicFunction1, Label, OpArg, OpArgState, oparg,
};
use std::collections::HashMap;

#[derive(Clone)]
struct Local {
    var: Variable,
    ty: JitType,
}

#[derive(Debug, Clone)]
enum JitValue {
    Int(Value),
    Float(Value),
    Bool(Value),
    None,
    Null,
    Tuple(Vec<Self>),
    FuncRef(FuncRef),
}

impl JitValue {
    fn from_type_and_value(ty: JitType, val: Value) -> Self {
        match ty {
            JitType::Int => Self::Int(val),
            JitType::Float => Self::Float(val),
            JitType::Bool => Self::Bool(val),
            JitType::None => unreachable!("None cannot be used as an argument type"),
        }
    }

    fn to_jit_type(&self) -> Option<JitType> {
        match self {
            Self::Int(_) => Some(JitType::Int),
            Self::Float(_) => Some(JitType::Float),
            Self::Bool(_) => Some(JitType::Bool),
            Self::None => Some(JitType::None),
            Self::Null | Self::Tuple(_) | Self::FuncRef(_) => None,
        }
    }

    fn into_value(self) -> Option<Value> {
        match self {
            Self::Int(val) | Self::Float(val) | Self::Bool(val) => Some(val),
            Self::None | Self::Null | Self::Tuple(_) | Self::FuncRef(_) => None,
        }
    }

    /// The cranelift value, without consuming the wrapper.
    fn value(&self) -> Option<Value> {
        match *self {
            Self::Int(val) | Self::Float(val) | Self::Bool(val) => Some(val),
            Self::None | Self::Null | Self::Tuple(_) | Self::FuncRef(_) => None,
        }
    }
}

pub(crate) struct FunctionCompiler<'a, 'b> {
    builder: &'a mut FunctionBuilder<'b>,
    /// The buffer a guard spills its record into, parameter 0 of the function.
    deopt_ptr: Value,
    /// The block every guard leaves through, created with the first one.
    deopt_exit: Option<Block>,
    /// `jit_powf`, imported into this function so `compile_fpow` can call it.
    powf_func: FuncRef,
    /// Bytecode offset the instruction being lowered would be re-entered at.
    resume_offset: u32,
    stack: Vec<JitValue>,
    variables: Box<[Option<Local>]>,
    /// One flag per varname slot, 1 once a local has been stored there. A local
    /// declared inside a branch is not assigned on every path that reaches a
    /// guard. They are all declared up front so that they can be zeroed in the
    /// entry block, which is out of reach once compilation has started.
    bound_flags: Box<[Variable]>,
    label_to_block: HashMap<Label, Block>,
    safety: Safety,
    pub(crate) sig: JitSig,
    pub(crate) deopt_sites: Vec<DeoptSite>,
}

/// Whether [`FunctionCompiler::add_instruction`] has a lowering for this opcode.
///
/// This mirrors the match in that method so a caller can rule a code object out
/// before any compilation state is set up. It only has to be right in one
/// direction: claiming support for something the match rejects merely wastes a
/// compile attempt, and denying something it handles only costs an
/// optimization. Neither can produce wrong code.
pub(crate) const fn instruction_is_supported(instruction: Instruction) -> bool {
    matches!(
        instruction,
        Instruction::BinaryOp { .. }
            | Instruction::BuildTuple { .. }
            | Instruction::Cache
            | Instruction::Call { .. }
            | Instruction::CallIntrinsic1 { .. }
            | Instruction::CompareOp { .. }
            | Instruction::CopyFreeVars { .. }
            | Instruction::ExtendedArg
            | Instruction::JumpBackward { .. }
            | Instruction::JumpBackwardNoInterrupt { .. }
            | Instruction::JumpForward { .. }
            | Instruction::LoadConst { .. }
            | Instruction::LoadFast { .. }
            | Instruction::LoadFastBorrow { .. }
            | Instruction::LoadFastBorrowLoadFastBorrow { .. }
            | Instruction::LoadFastLoadFast { .. }
            | Instruction::LoadGlobal { .. }
            | Instruction::LoadSmallInt { .. }
            | Instruction::MakeCell { .. }
            | Instruction::Nop
            | Instruction::NotTaken
            | Instruction::PopJumpIfFalse { .. }
            | Instruction::PopJumpIfTrue { .. }
            | Instruction::PopTop
            | Instruction::PushNull
            | Instruction::Resume { .. }
            | Instruction::ReturnValue
            | Instruction::StoreFast { .. }
            | Instruction::StoreFastLoadFast { .. }
            | Instruction::StoreFastStoreFast { .. }
            | Instruction::Swap { .. }
            | Instruction::ToBool
            | Instruction::UnaryNegative
            | Instruction::UnaryNot
            | Instruction::UnpackSequence { .. }
    )
}

impl<'a, 'b> FunctionCompiler<'a, 'b> {
    pub(crate) fn new(
        builder: &'a mut FunctionBuilder<'b>,
        num_variables: usize,
        arg_types: &[JitType],
        ret_type: Option<JitType>,
        entry_block: Block,
        safety: Safety,
        powf_func: FuncRef,
    ) -> Self {
        let params = builder.func.dfg.block_params(entry_block).to_vec();
        let (deopt_ptr, arg_params) = params.split_first().expect("the deopt buffer parameter");
        // The builder still sits in the entry block, which is the only place a
        // flag can be given the value every path into the function starts from.
        let bound_flags: Box<[Variable]> = (0..num_variables)
            .map(|_| {
                let flag = builder.declare_var(types::I8);
                let unbound = builder.ins().iconst(types::I8, 0);
                builder.def_var(flag, unbound);
                flag
            })
            .collect();
        let mut compiler = Self {
            builder,
            deopt_ptr: *deopt_ptr,
            deopt_exit: None,
            powf_func,
            resume_offset: 0,
            stack: Vec::new(),
            variables: vec![None; num_variables].into_boxed_slice(),
            bound_flags,
            label_to_block: HashMap::new(),
            safety,
            sig: JitSig {
                args: arg_types.to_vec(),
                ret: ret_type,
            },
            deopt_sites: Vec::new(),
        };
        for (i, (ty, val)) in arg_types.iter().zip(arg_params.iter().copied()).enumerate() {
            compiler
                .store_variable(
                    (i as u32).into(),
                    JitValue::from_type_and_value(ty.clone(), val),
                )
                .unwrap();
        }
        compiler
    }

    fn pop_multiple(&mut self, count: usize) -> Vec<JitValue> {
        let stack_len = self.stack.len();
        self.stack.drain(stack_len - count..).collect()
    }

    fn store_variable(&mut self, idx: oparg::VarNum, val: JitValue) -> Result<(), JitCompileError> {
        #[expect(clippy::mut_mut, reason = "This seems like a false positive")]
        let builder = &mut self.builder;
        let ty = val.to_jit_type().ok_or(JitCompileError::NotSupported)?;
        let cranelift_ty = ty.to_cranelift().ok_or(JitCompileError::NotSupported)?;
        let bound = self.bound_flags[idx];
        let local = self.variables[idx].get_or_insert_with(|| {
            let var = builder.declare_var(cranelift_ty);
            Local {
                var,
                ty: ty.clone(),
            }
        });
        if ty != local.ty {
            Err(JitCompileError::NotSupported)
        } else {
            self.builder.def_var(local.var, val.into_value().unwrap());
            let is_bound = self.builder.ins().iconst(types::I8, 1);
            self.builder.def_var(bound, is_bound);
            Ok(())
        }
    }

    /// Emit the two-way branch a guard is: when `cond` is non-zero the compiled
    /// code gives up, spilling everything the interpreter needs to re-execute
    /// this instruction from the start; otherwise it falls through and carries
    /// on.
    ///
    /// `popped` is what this instruction has already taken off the stack, in
    /// the order it must go back on. The interpreter re-executes the whole
    /// instruction, so its operands have to be there.
    fn deopt_branch(&mut self, cond: Value, popped: &[JitValue]) -> Result<(), JitCompileError> {
        let live: Vec<Option<JitType>> = self
            .variables
            .iter()
            .map(|local| local.as_ref().map(|l| l.ty.clone()))
            .collect();
        // The callable and the null a call pushes beside it are the same on
        // every path that reaches the guard, so they are described rather than
        // written. Anything else without a slot encoding - a tuple, a bare
        // `None` - is not statically known either, so a guard cannot be placed
        // above one.
        let mut entries = Vec::with_capacity(self.stack.len() + popped.len());
        let mut spilled = Vec::new();
        for val in self.stack.iter().chain(popped) {
            match val {
                JitValue::FuncRef(_) => entries.push(StackEntry::Callee),
                JitValue::Null => entries.push(StackEntry::Null),
                _ => {
                    let (ty, value) = val
                        .to_jit_type()
                        .zip(val.value())
                        .ok_or(JitCompileError::NotSupported)?;
                    entries.push(StackEntry::Value(ty.clone()));
                    spilled.push((ty, value));
                }
            }
        }

        let slots = DEOPT_HEADER_SLOTS + live.iter().flatten().count() + spilled.len();
        if slots > MAX_DEOPT_SLOTS || live.len() > u64::BITS as usize {
            return Err(JitCompileError::NotSupported);
        }

        let site = self.deopt_sites.len();
        self.deopt_sites.push(DeoptSite {
            offset: self.resume_offset,
            locals: live.clone().into_boxed_slice(),
            stack: entries.into_boxed_slice(),
        });

        self.deopt_if(cond, |this| {
            let mut offset = DEOPT_HEADER_SLOTS;
            let mut mask = this.builder.ins().iconst(types::I64, 0);
            for (i, ty) in live.iter().enumerate() {
                let Some(ty) = ty else { continue };
                let var = this.variables[i].as_ref().expect("live local").var;
                let value = this.builder.use_var(var);
                this.store_slot(ty, value, offset);
                let bound = this.builder.use_var(this.bound_flags[i]);
                let bound = this.builder.ins().uextend(types::I64, bound);
                let bit = this.builder.ins().ishl_imm(bound, i as i64);
                mask = this.builder.ins().bor(mask, bit);
                offset += 1;
            }
            for (ty, value) in spilled {
                this.store_slot(&ty, value, offset);
                offset += 1;
            }
            this.store_raw(mask, 1);
            let status = this.builder.ins().iconst(types::I64, site as i64 + 1);
            this.store_raw(status, 0);
        });
        Ok(())
    }

    /// Emit "when `cond` is non-zero, leave through the shared deopt exit;
    /// otherwise carry on". `spill` fills the block that is taken, which is
    /// where a guard writes its record; a caller whose record is already
    /// written leaves it empty. The builder is left in the fall-through block,
    /// so lowering continues where it left off.
    fn deopt_if(&mut self, cond: Value, spill: impl FnOnce(&mut Self)) {
        let taken = self.builder.create_block();
        let carry_on = self.builder.create_block();
        self.builder.ins().brif(cond, taken, &[], carry_on, &[]);

        self.builder.switch_to_block(taken);
        spill(self);
        let exit = self.deopt_exit();
        self.builder.ins().jump(exit, &[]);

        self.builder.switch_to_block(carry_on);
    }

    /// Write one value into the deopt buffer, in the 64-bit encoding
    /// `AbiValue::from_slot` reads back.
    fn store_slot(&mut self, ty: &JitType, value: Value, slot: usize) {
        let value = match ty {
            JitType::Int | JitType::Float => value,
            // A flag occupies a whole slot, so the bits above it have to be zero.
            JitType::Bool => self.builder.ins().uextend(types::I64, value),
            // Neither a local nor a spilled stack entry can carry one: a local
            // of no cranelift type is rejected when it is stored, and a `None`
            // on the stack has no value to pair with its type.
            JitType::None => return,
        };
        self.store_raw(value, slot);
    }

    fn store_raw(&mut self, value: Value, slot: usize) {
        let offset = i32::try_from(slot * SLOT_SIZE).expect("slot count is capped");
        self.builder
            .ins()
            .store(MemFlags::trusted(), value, self.deopt_ptr, offset);
    }

    /// The one block every guard leaves through. Its return is emitted at the
    /// end of compilation, because until then the function's return type may
    /// still be unknown.
    fn deopt_exit(&mut self) -> Block {
        #[expect(clippy::mut_mut, reason = "This seems like a false positive")]
        let builder = &mut self.builder;
        *self
            .deopt_exit
            .get_or_insert_with(|| builder.create_block())
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
        #[expect(clippy::mut_mut, reason = "This seems like a false positive")]
        let builder = &mut self.builder;
        *self
            .label_to_block
            .entry(label)
            .or_insert_with(|| builder.create_block())
    }

    fn jump_target_forward(offset: u32, caches: u32, arg: OpArg) -> Result<Label, JitCompileError> {
        let after = offset
            .checked_add(1)
            .and_then(|i| i.checked_add(caches))
            .ok_or(JitCompileError::BadBytecode)?;
        let target = after
            .checked_add(u32::from(arg))
            .ok_or(JitCompileError::BadBytecode)?;
        Ok(Label::from_u32(target))
    }

    fn jump_target_backward(
        offset: u32,
        caches: u32,
        arg: OpArg,
    ) -> Result<Label, JitCompileError> {
        let after = offset
            .checked_add(1)
            .and_then(|i| i.checked_add(caches))
            .ok_or(JitCompileError::BadBytecode)?;
        let target = after
            .checked_sub(u32::from(arg))
            .ok_or(JitCompileError::BadBytecode)?;
        Ok(Label::from_u32(target))
    }

    fn instruction_target(
        offset: u32,
        instruction: Instruction,
        arg: OpArg,
    ) -> Result<Option<Label>, JitCompileError> {
        let caches = instruction.cache_entries() as u32;
        let target = match instruction {
            Instruction::JumpForward { .. } => {
                Some(Self::jump_target_forward(offset, caches, arg)?)
            }
            Instruction::JumpBackward { .. } | Instruction::JumpBackwardNoInterrupt { .. } => {
                Some(Self::jump_target_backward(offset, caches, arg)?)
            }
            Instruction::PopJumpIfFalse { .. }
            | Instruction::PopJumpIfTrue { .. }
            | Instruction::PopJumpIfNone { .. }
            | Instruction::PopJumpIfNotNone { .. }
            | Instruction::ForIter { .. }
            | Instruction::Send { .. } => Some(Self::jump_target_forward(offset, caches, arg)?),
            _ => None,
        };
        Ok(target)
    }

    pub(crate) fn compile<C: bytecode::Constant>(
        &mut self,
        func_ref: FuncRef,
        bytecode: &CodeObject<C>,
    ) -> Result<(), JitCompileError> {
        // JIT should consume a stable instruction stream: de-specialized opcodes
        // with zeroed CACHE entries, not runtime-mutated quickened code.
        let clean_instructions: bytecode::CodeUnits = bytecode
            .instructions
            .original_bytes()
            .as_slice()
            .try_into()
            .map_err(|_| JitCompileError::BadBytecode)?;

        let mut label_targets = BTreeSet::new();
        let mut target_arg_state = OpArgState::default();
        for (offset, &raw_instr) in clean_instructions.iter().enumerate() {
            let (instruction, arg) = target_arg_state.get(raw_instr);
            if let Some(target) = Self::instruction_target(offset as u32, instruction, arg)? {
                label_targets.insert(target);
            }
        }
        let mut arg_state = OpArgState::default();

        // Track whether we have "returned" in the current block
        let mut in_unreachable_code = false;
        let mut extended_start: Option<u32> = None;

        for (offset, &raw_instr) in clean_instructions.iter().enumerate() {
            let label = Label::from_u32(offset as u32);
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

            // The oparg of an instruction preceded by EXTENDED_ARG is
            // accumulated across the group, so a resume has to start where the
            // group starts.
            self.resume_offset = extended_start.unwrap_or(offset as u32);

            // Actually compile this instruction:
            self.add_instruction(func_ref, bytecode, offset as u32, instruction, arg)?;

            if matches!(instruction, Instruction::ExtendedArg) {
                extended_start.get_or_insert(offset as u32);
            } else {
                extended_start = None;
            }

            // If that was an unconditional branch or return, mark future instructions unreachable
            match instruction {
                Instruction::ReturnValue
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

        // The deopt exit returns whatever the function's signature ended up
        // saying it returns; the value is never looked at, because the status
        // says the call did not return one.
        if let Some(exit) = self.deopt_exit {
            self.builder.switch_to_block(exit);
            match self.sig.ret.as_ref().and_then(JitType::to_cranelift) {
                Some(ty) => {
                    let filler = match ty {
                        types::F64 => self.builder.ins().f64const(0.0),
                        ty => self.builder.ins().iconst(ty, 0),
                    };
                    self.builder.ins().return_(&[filler]);
                }
                None => {
                    self.builder.ins().return_(&[]);
                }
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
        let val_type = val.to_jit_type().ok_or(JitCompileError::NotSupported)?;
        if let Some(ref ret_type) = self.sig.ret {
            if ret_type != &val_type {
                return Err(JitCompileError::NotSupported);
            }
        } else {
            self.sig.ret = Some(val_type.clone());
            if let Some(val_type) = val_type.to_cranelift() {
                self.builder
                    .func
                    .signature
                    .returns
                    .push(AbiParam::new(val_type));
            }
        }

        if let Some(cr_val) = val.into_value() {
            self.builder.ins().return_(&[cr_val]);
        } else {
            self.builder.ins().return_(&[]);
        }
        Ok(())
    }

    pub(crate) fn add_instruction<C: bytecode::Constant>(
        &mut self,
        func_ref: FuncRef,
        bytecode: &CodeObject<C>,
        offset: u32,
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
                        // Overflow is not an error: it is where the interpreter
                        // stops using a machine word and starts using a bignum,
                        // so the operands go back to it intact.
                        self.deopt_branch(carry, &[JitValue::Int(a), JitValue::Int(b)])?;
                        JitValue::Int(out)
                    }
                    (
                        BinaryOperator::Subtract | BinaryOperator::InplaceSubtract,
                        JitValue::Int(a),
                        JitValue::Int(b),
                    ) => {
                        let out = self.compile_sub(a, b, &[JitValue::Int(a), JitValue::Int(b)])?;
                        JitValue::Int(out)
                    }
                    (
                        BinaryOperator::FloorDivide | BinaryOperator::InplaceFloorDivide,
                        JitValue::Int(a),
                        JitValue::Int(b),
                    ) => JitValue::Int(self.compile_floor_div(a, b)?.0),
                    (
                        BinaryOperator::TrueDivide | BinaryOperator::InplaceTrueDivide,
                        JitValue::Int(a),
                        JitValue::Int(b),
                    ) => {
                        let operands = [JitValue::Int(a), JitValue::Int(b)];
                        let by_zero = self.builder.ins().icmp_imm(IntCC::Equal, b, 0);
                        self.deopt_branch(by_zero, &operands)?;

                        // `int.__truediv__` is correctly rounded. Converting both
                        // operands to double and dividing rounds twice, so it can be
                        // a ulp out as soon as either conversion is inexact - which
                        // is exactly when the operand does not fit in a double's
                        // significand.
                        let too_wide = |compiler: &mut Self, v: Value| {
                            let magnitude = compiler.builder.ins().iabs(v);
                            compiler.builder.ins().icmp_imm(
                                IntCC::UnsignedGreaterThanOrEqual,
                                magnitude,
                                1 << 53,
                            )
                        };
                        let a_wide = too_wide(self, a);
                        let b_wide = too_wide(self, b);
                        let wide = self.builder.ins().bor(a_wide, b_wide);
                        self.deopt_branch(wide, &operands)?;

                        let a_float = self.builder.ins().fcvt_from_sint(types::F64, a);
                        let b_float = self.builder.ins().fcvt_from_sint(types::F64, b);
                        JitValue::Float(self.builder.ins().fdiv(a_float, b_float))
                    }
                    (
                        BinaryOperator::Multiply | BinaryOperator::InplaceMultiply,
                        JitValue::Int(a),
                        JitValue::Int(b),
                    ) => {
                        let (out, carry) = self.builder.ins().smul_overflow(a, b);
                        self.deopt_branch(carry, &[JitValue::Int(a), JitValue::Int(b)])?;
                        JitValue::Int(out)
                    }
                    (
                        BinaryOperator::Remainder | BinaryOperator::InplaceRemainder,
                        JitValue::Int(a),
                        JitValue::Int(b),
                    ) => JitValue::Int(self.compile_floor_div(a, b)?.1),
                    (
                        BinaryOperator::Power | BinaryOperator::InplacePower,
                        JitValue::Int(a),
                        JitValue::Int(b),
                    ) => {
                        let operands = [JitValue::Int(a), JitValue::Int(b)];
                        JitValue::Int(self.compile_ipow(a, b, &operands)?)
                    }
                    (
                        BinaryOperator::Lshift
                        | BinaryOperator::InplaceLshift
                        | BinaryOperator::Rshift
                        | BinaryOperator::InplaceRshift,
                        JitValue::Int(a),
                        JitValue::Int(b),
                    ) => {
                        let operands = [JitValue::Int(a), JitValue::Int(b)];
                        // A count outside `0..64` is not a machine shift at all: negative
                        // raises ValueError, and 64 or more is a well-defined answer the
                        // instruction does not give. An unsigned comparison catches both,
                        // because a negative count reads as huge.
                        let out_of_range =
                            self.builder
                                .ins()
                                .icmp_imm(IntCC::UnsignedGreaterThanOrEqual, b, 64);
                        self.deopt_branch(out_of_range, &operands)?;

                        let left =
                            matches!(op, BinaryOperator::Lshift | BinaryOperator::InplaceLshift);
                        let out = if left {
                            let out = self.builder.ins().ishl(a, b);
                            // Shifting back has to give the operand again, or the bits
                            // that fell off the top are digits the interpreter would
                            // have kept.
                            let back = self.builder.ins().sshr(out, b);
                            let lost = self.builder.ins().icmp(IntCC::NotEqual, back, a);
                            self.deopt_branch(lost, &operands)?;
                            out
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
                    ) => {
                        let operands = [JitValue::Float(a), JitValue::Float(b)];
                        let zero = self.builder.ins().f64const(0.0);
                        let by_zero = self.builder.ins().fcmp(FloatCC::Equal, b, zero);
                        self.deopt_branch(by_zero, &operands)?;
                        JitValue::Float(self.builder.ins().fdiv(a, b))
                    }
                    (
                        BinaryOperator::Power | BinaryOperator::InplacePower,
                        JitValue::Float(a),
                        JitValue::Float(b),
                    ) => {
                        let operands = [JitValue::Float(a), JitValue::Float(b)];
                        JitValue::Float(self.compile_fpow(a, b, &operands)?)
                    }

                    // Floats and Integers
                    (_, JitValue::Int(a), JitValue::Float(b))
                    | (_, JitValue::Float(a), JitValue::Int(b)) => {
                        let a_ty = a_type.unwrap();
                        let b_ty = b_type.unwrap();

                        let operand_one = match &a_ty {
                            JitType::Int => self.builder.ins().fcvt_from_sint(types::F64, a),
                            _ => a,
                        };

                        let operand_two = match &b_ty {
                            JitType::Int => self.builder.ins().fcvt_from_sint(types::F64, b),
                            _ => b,
                        };

                        // The original operands, for a guard to hand back on
                        // deopt - `operand_one`/`operand_two` above are the
                        // converted doubles and do not describe the stack
                        // the interpreter had. Only `TrueDivide` and `Power`
                        // ever guard, so this is built lazily rather than on
                        // every arm.
                        let operands = || {
                            [
                                JitValue::from_type_and_value(a_ty.clone(), a),
                                JitValue::from_type_and_value(b_ty.clone(), b),
                            ]
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
                                let zero = self.builder.ins().f64const(0.0);
                                let by_zero =
                                    self.builder.ins().fcmp(FloatCC::Equal, operand_two, zero);
                                self.deopt_branch(by_zero, &operands())?;
                                JitValue::Float(self.builder.ins().fdiv(operand_one, operand_two))
                            }
                            BinaryOperator::Power | BinaryOperator::InplacePower => {
                                JitValue::Float(self.compile_fpow(
                                    operand_one,
                                    operand_two,
                                    &operands(),
                                )?)
                            }
                            _ => return Err(JitCompileError::NotSupported),
                        }
                    }
                    _ => return Err(JitCompileError::NotSupported),
                };
                self.stack.push(val);

                Ok(())
            }
            Instruction::BuildTuple { count } => {
                let elements = self.pop_multiple(count.get(arg) as usize);
                self.stack.push(JitValue::Tuple(elements));
                Ok(())
            }
            Instruction::Call { argc } => {
                let nargs = argc.get(arg);

                let mut args = Vec::with_capacity(nargs as usize + 1);
                for _ in 0..nargs {
                    let arg = self.stack.pop().ok_or(JitCompileError::BadBytecode)?;
                    args.push(arg.into_value().unwrap());
                }
                // Popping walks the arguments backwards.
                args.reverse();
                args.insert(0, self.deopt_ptr);

                // Pop self_or_null (should be Null for JIT-compiled recursive calls)
                let self_or_null = self.stack.pop().ok_or(JitCompileError::BadBytecode)?;
                if !matches!(self_or_null, JitValue::Null) {
                    return Err(JitCompileError::NotSupported);
                }

                match self.stack.pop().ok_or(JitCompileError::BadBytecode)? {
                    JitValue::FuncRef(reference) => {
                        let call = self.builder.ins().call(reference, &args);
                        // The only callable reachable here is this function itself,
                        // so the result carries the declared return type - it is not
                        // always an Int. A function whose return type is still
                        // unknown has no return slot in the signature it was
                        // declared with, and there is nothing to type the result as.
                        let ret = match *self.builder.inst_results(call) {
                            [] => None,
                            [val] => Some(val),
                            _ => return Err(JitCompileError::NotSupported),
                        };
                        let val = match (self.sig.ret.clone(), ret) {
                            (Some(JitType::None), None) => JitValue::None,
                            (Some(ty), Some(val)) => JitValue::from_type_and_value(ty, val),
                            _ => return Err(JitCompileError::NotSupported),
                        };

                        // A nested frame that gave up has already written its
                        // record, and returned a filler in place of a result.
                        // Anything this frame computes from here on is built on
                        // that filler, so stop and leave its record standing.
                        let status = self.builder.ins().load(
                            types::I64,
                            MemFlags::trusted(),
                            self.deopt_ptr,
                            0,
                        );
                        let nested = self.builder.ins().icmp_imm(IntCC::NotEqual, status, 0);
                        self.deopt_if(nested, |_| {});

                        self.stack.push(val);

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
            Instruction::CompareOp { opname } => {
                let op = opname.get(arg);
                // the rhs is popped off first
                let b = self.stack.pop().ok_or(JitCompileError::BadBytecode)?;
                let a = self.stack.pop().ok_or(JitCompileError::BadBytecode)?;

                let a_type: Option<JitType> = a.to_jit_type();
                let b_type: Option<JitType> = b.to_jit_type();

                match (a, b) {
                    (
                        JitValue::Int(a) | JitValue::Bool(a),
                        JitValue::Int(b) | JitValue::Bool(b),
                    ) => {
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
            Instruction::ExtendedArg
            | Instruction::Cache
            | Instruction::MakeCell { .. }
            | Instruction::CopyFreeVars { .. } => Ok(()),

            Instruction::JumpBackward { .. }
            | Instruction::JumpBackwardNoInterrupt { .. }
            | Instruction::JumpForward { .. } => {
                let target = Self::instruction_target(offset, instruction, arg)?
                    .ok_or(JitCompileError::BadBytecode)?;
                let target_block = self.get_or_create_block(target);
                self.builder.ins().jump(target_block, &[]);
                Ok(())
            }
            Instruction::LoadConst { consti } => {
                let val =
                    self.prepare_const(bytecode.constants[consti.get(arg)].borrow_constant())?;
                self.stack.push(val);
                Ok(())
            }
            Instruction::LoadSmallInt { i } => {
                let small_int = i.get(arg) as i64;
                let val = self.builder.ins().iconst(types::I64, small_int);
                self.stack.push(JitValue::Int(val));
                Ok(())
            }
            Instruction::LoadFast { var_num } | Instruction::LoadFastBorrow { var_num } => {
                let local = self.variables[var_num.get(arg)]
                    .as_ref()
                    .ok_or(JitCompileError::BadBytecode)?;
                self.stack.push(JitValue::from_type_and_value(
                    local.ty.clone(),
                    self.builder.use_var(local.var),
                ));
                Ok(())
            }
            Instruction::LoadFastLoadFast { var_nums }
            | Instruction::LoadFastBorrowLoadFastBorrow { var_nums } => {
                let oparg = var_nums.get(arg);
                let (idx1, idx2) = oparg.indexes();

                #[expect(
                    clippy::tuple_array_conversions,
                    reason = "Seems like a false positive"
                )]
                for idx in [idx1, idx2] {
                    let local = self.variables[idx]
                        .as_ref()
                        .ok_or(JitCompileError::BadBytecode)?;
                    self.stack.push(JitValue::from_type_and_value(
                        local.ty.clone(),
                        self.builder.use_var(local.var),
                    ));
                }
                Ok(())
            }
            Instruction::LoadGlobal { namei } => {
                let oparg = namei.get(arg);
                let name = &bytecode.names[(oparg >> 1) as usize];

                // The only global with a lowering is this function itself,
                // matched by name. Strict turns even that down: the interpreter
                // reads the globals dict on every call, so rebinding the name -
                // a decorator applied later, a test patching the module - makes
                // the two disagree about what gets called.
                let is_self_call = self.safety == Safety::Permissive
                    && name.as_ref() == bytecode.obj_name.as_ref();
                if !is_self_call {
                    return Err(JitCompileError::NotSupported);
                }

                self.stack.push(JitValue::FuncRef(func_ref));
                if (oparg & 1) != 0 {
                    self.stack.push(JitValue::Null);
                }
                Ok(())
            }
            Instruction::Nop | Instruction::NotTaken => Ok(()),
            Instruction::PopJumpIfFalse { .. } => {
                let cond = self.stack.pop().ok_or(JitCompileError::BadBytecode)?;
                let val = self.boolean_val(cond)?;
                let then_label = Self::instruction_target(offset, instruction, arg)?
                    .ok_or(JitCompileError::BadBytecode)?;
                let then_block = self.get_or_create_block(then_label);
                let else_block = self.builder.create_block();

                self.builder
                    .ins()
                    .brif(val, else_block, &[], then_block, &[]);
                self.builder.switch_to_block(else_block);

                Ok(())
            }
            Instruction::PopJumpIfTrue { .. } => {
                let cond = self.stack.pop().ok_or(JitCompileError::BadBytecode)?;
                let val = self.boolean_val(cond)?;
                let then_label = Self::instruction_target(offset, instruction, arg)?
                    .ok_or(JitCompileError::BadBytecode)?;
                let then_block = self.get_or_create_block(then_label);
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
            Instruction::Resume { .. } => {
                // TODO: Implement the resume instruction
                Ok(())
            }
            Instruction::ReturnValue => {
                let val = self.stack.pop().ok_or(JitCompileError::BadBytecode)?;
                self.return_value(val)
            }
            Instruction::StoreFast { var_num } => {
                let val = self.stack.pop().ok_or(JitCompileError::BadBytecode)?;
                self.store_variable(var_num.get(arg), val)
            }
            Instruction::StoreFastLoadFast { var_nums } => {
                let oparg = var_nums.get(arg);
                let (store_idx, load_idx) = oparg.indexes();
                let val = self.stack.pop().ok_or(JitCompileError::BadBytecode)?;
                self.store_variable(store_idx, val)?;
                let local = self.variables[load_idx]
                    .as_ref()
                    .ok_or(JitCompileError::BadBytecode)?;
                self.stack.push(JitValue::from_type_and_value(
                    local.ty.clone(),
                    self.builder.use_var(local.var),
                ));
                Ok(())
            }
            Instruction::StoreFastStoreFast { var_nums } => {
                let oparg = var_nums.get(arg);
                let (idx1, idx2) = oparg.indexes();
                let val1 = self.stack.pop().ok_or(JitCompileError::BadBytecode)?;
                self.store_variable(idx1, val1)?;
                let val2 = self.stack.pop().ok_or(JitCompileError::BadBytecode)?;
                self.store_variable(idx2, val2)
            }
            Instruction::Swap { i: index } => {
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
                        // Compile minus as 0 - val. The zero is not on the
                        // interpreter's stack, so only `val` is recorded.
                        let zero = self.builder.ins().iconst(types::I64, 0);
                        let out = self.compile_sub(zero, val, &[JitValue::Int(val)])?;
                        self.stack.push(JitValue::Int(out));
                        Ok(())
                    }
                    _ => Err(JitCompileError::NotSupported),
                }
            }
            Instruction::UnpackSequence { count } => {
                let val = self.stack.pop().ok_or(JitCompileError::BadBytecode)?;

                let elements = match val {
                    JitValue::Tuple(elements) => elements,
                    _ => return Err(JitCompileError::NotSupported),
                };

                if elements.len() != count.get(arg) as usize {
                    return Err(JitCompileError::NotSupported);
                }

                self.stack.extend(elements.into_iter().rev());
                Ok(())
            }
            _ => Err(JitCompileError::NotSupported),
        }
    }

    fn compile_sub(
        &mut self,
        a: Value,
        b: Value,
        popped: &[JitValue],
    ) -> Result<Value, JitCompileError> {
        let (out, carry) = self.builder.ins().ssub_overflow(a, b);
        self.deopt_branch(carry, popped)?;
        Ok(out)
    }

    /// Floor division and its remainder, which are defined together: the
    /// quotient rounds toward negative infinity and the remainder takes the
    /// divisor's sign. `sdiv` and `srem` round toward zero and take the
    /// dividend's sign, so both need a correction on the same condition.
    ///
    /// Two cases have no 64-bit answer at all and deoptimize: a zero divisor,
    /// which raises, and `i64::MIN // -1`, whose quotient is one past the top
    /// of the range. The guard is shared between the two results, so
    /// `i64::MIN % -1` deopts alongside it even though `0` is a perfectly
    /// good remainder - the pair is computed together.
    fn compile_floor_div(&mut self, a: Value, b: Value) -> Result<(Value, Value), JitCompileError> {
        let operands = [JitValue::Int(a), JitValue::Int(b)];
        let by_zero = self.builder.ins().icmp_imm(IntCC::Equal, b, 0);
        self.deopt_branch(by_zero, &operands)?;

        let min = self.builder.ins().icmp_imm(IntCC::Equal, a, i64::MIN);
        let neg_one = self.builder.ins().icmp_imm(IntCC::Equal, b, -1);
        let overflows = self.builder.ins().band(min, neg_one);
        self.deopt_branch(overflows, &operands)?;

        let quotient = self.builder.ins().sdiv(a, b);
        // The remainder as `a - quotient * b` rather than a second `srem`:
        // cranelift keeps a trapping division live even when nothing reads
        // its result, so asking for both would cost two hardware divisions
        // instead of one. The multiply cannot overflow: `quotient` truncates
        // toward zero, so `|quotient * b| <= |a|` and `quotient * b` shares
        // `a`'s sign, making the subtraction one between same-signed values
        // with the smaller magnitude second.
        let product = self.builder.ins().imul(quotient, b);
        let remainder = self.builder.ins().isub(a, product);

        // The two disagree with Python exactly when the division was
        // inexact and the operands had opposite signs.
        let inexact = self.builder.ins().icmp_imm(IntCC::NotEqual, remainder, 0);
        let mixed = self.builder.ins().bxor(a, b);
        let mixed = self.builder.ins().icmp_imm(IntCC::SignedLessThan, mixed, 0);
        let correct = self.builder.ins().band(inexact, mixed);

        let one = self.builder.ins().iconst(types::I64, 1);
        let zero = self.builder.ins().iconst(types::I64, 0);
        let quotient_adjust = self.builder.ins().select(correct, one, zero);
        // `isub` cannot overflow: it only subtracts when `correct` holds,
        // which requires `remainder != 0`. A quotient of `i64::MIN` is still
        // reachable here - after the guards above, only `b == 1` produces
        // it - but that division is exact, so `remainder` is `0` and
        // `correct` is false there.
        let quotient = self.builder.ins().isub(quotient, quotient_adjust);

        let remainder_adjust = self.builder.ins().select(correct, b, zero);
        let remainder = self.builder.ins().iadd(remainder, remainder_adjust);

        Ok((quotient, remainder))
    }

    /// The sign bit of an f64, matching `f64::is_sign_negative` - true for
    /// `-0.0` and a negative NaN, not just an ordinary negative number.
    /// `fcmp` compares values and cannot see this bit; reinterpreting the
    /// bits as a signed integer and testing that sign is what
    /// `f64::is_sign_negative` itself does.
    fn is_sign_negative(&mut self, v: Value) -> Value {
        let bits = self.builder.ins().bitcast(types::I64, MemFlags::new(), v);
        self.builder.ins().icmp_imm(IntCC::SignedLessThan, bits, 0)
    }

    /// Computes a raised to the power b by calling the same `f64::powf` the
    /// interpreter's `float_pow` calls, once the two guards ahead of it in
    /// `float_pow` rule out its other two outcomes: `ZeroDivisionError` and a
    /// complex result. This is `float_pow` translated guard for guard,
    /// including the parts that look wrong - the first guard below reads the
    /// SIGN BIT of the exponent, not its value, because that is what
    /// `is_sign_negative` does, so `0.0 ** -0.0` deopts exactly as
    /// `0.0 ** -1.0` does.
    fn compile_fpow(
        &mut self,
        a: Value,
        b: Value,
        operands: &[JitValue],
    ) -> Result<Value, JitCompileError> {
        let zero_f = self.builder.ins().f64const(0.0);

        // v1.is_zero() && v2.is_sign_negative() -> ZeroDivisionError.
        let base_zero = self.builder.ins().fcmp(FloatCC::Equal, a, zero_f);
        let exp_sign_negative = self.is_sign_negative(b);
        let divides_by_zero = self.builder.ins().band(base_zero, exp_sign_negative);
        self.deopt_branch(divides_by_zero, operands)?;

        // v1.is_sign_negative() && (v2.floor() - v2).abs() > f64::EPSILON ->
        // complex result - a `-0.0` base is caught by its sign bit here too,
        // the same as the exponent above.
        let base_sign_negative = self.is_sign_negative(a);
        let b_floor = self.builder.ins().floor(b);
        let diff = self.builder.ins().fsub(b_floor, b);
        let abs_diff = self.builder.ins().fabs(diff);
        let epsilon = self.builder.ins().f64const(f64::EPSILON);
        let fractional = self
            .builder
            .ins()
            .fcmp(FloatCC::GreaterThan, abs_diff, epsilon);
        let complex = self.builder.ins().band(base_sign_negative, fractional);
        self.deopt_branch(complex, operands)?;

        // ans = v1.powf(v2), through the exact function the interpreter
        // calls - see `jit_powf`'s doc comment for why it is looked up by an
        // explicit symbol rather than left for the JIT to resolve.
        let call = self.builder.ins().call(self.powf_func, &[a, b]);
        let ans = match *self.builder.inst_results(call) {
            [ans] => ans,
            _ => return Err(JitCompileError::NotSupported),
        };

        // ans.is_infinite() && !(v1.is_infinite() || v2.is_infinite()) ->
        // OverflowError. An already-infinite operand (`inf ** 2.0`,
        // `2.0 ** inf`) must keep answering with its infinity, hence the
        // exemption.
        let inf_f = self.builder.ins().f64const(f64::INFINITY);
        let abs_ans = self.builder.ins().fabs(ans);
        let ans_infinite = self.builder.ins().fcmp(FloatCC::Equal, abs_ans, inf_f);
        let abs_a = self.builder.ins().fabs(a);
        let a_infinite = self.builder.ins().fcmp(FloatCC::Equal, abs_a, inf_f);
        let abs_b = self.builder.ins().fabs(b);
        let b_infinite = self.builder.ins().fcmp(FloatCC::Equal, abs_b, inf_f);
        let either_infinite = self.builder.ins().bor(a_infinite, b_infinite);
        let overflowed = self.builder.ins().band_not(ans_infinite, either_infinite);
        self.deopt_branch(overflowed, operands)?;

        Ok(ans)
    }

    fn compile_ipow(
        &mut self,
        a: Value,
        b: Value,
        operands: &[JitValue],
    ) -> Result<Value, JitCompileError> {
        // A negative exponent makes this a float; the loop below only
        // computes non-negative integer powers.
        let negative = self.builder.ins().icmp_imm(IntCC::SignedLessThan, b, 0);
        self.deopt_branch(negative, operands)?;

        let zero = self.builder.ins().iconst(types::I64, 0);
        let one_i64 = self.builder.ins().iconst(types::I64, 1);

        // Create required blocks
        let loop_block = self.builder.create_block();
        let continue_block = self.builder.create_block();
        let exit_block = self.builder.create_block();

        // Set up block parameters
        self.builder.append_block_param(loop_block, types::I64); // exponent
        self.builder.append_block_param(loop_block, types::I64); // result
        self.builder.append_block_param(loop_block, types::I64); // base

        self.builder.append_block_param(exit_block, types::I64); // final result

        // Set up parameters for continue_block
        self.builder.append_block_param(continue_block, types::I64); // exponent
        self.builder.append_block_param(continue_block, types::I64); // result
        self.builder.append_block_param(continue_block, types::I64); // base

        // The exponent is known non-negative, so jump straight into the loop.
        self.builder
            .ins()
            .jump(loop_block, &[b.into(), one_i64.into(), a.into()]);

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

        // Unlike the squaring below, this carry does not need masking to
        // stay exact: `continue_block` only runs with `exp != 0`, so a clear
        // low bit still forces `exp >= 2`, and once `|base| >= 2` an
        // overflowing `result * base` means `result * base^exp` overflows
        // too - a later guard always catches it - while with `|base| <= 1`
        // the product cannot overflow at all.
        let (mul_result, mul_carry) = self.builder.ins().smul_overflow(result_phi, base_phi);
        self.deopt_branch(mul_carry, operands)?;
        let new_result = self.builder.ins().select(is_odd, mul_result, result_phi);

        // The squared base is read only if there is another iteration to
        // read it, and this mask is the one that is measurably load-bearing:
        // dropping it deopts `2 ** 33` and `2 ** 62` on a squaring whose
        // overflowed result the exit branch above never reads.
        let (squared_base, square_carry) = self.builder.ins().smul_overflow(base_phi, base_phi);
        let new_exp = self.builder.ins().sshr_imm(exp_phi, 1);
        let more = self.builder.ins().icmp_imm(IntCC::NotEqual, new_exp, 0);
        let square_overflows = self.builder.ins().band(more, square_carry);
        self.deopt_branch(square_overflows, operands)?;

        self.builder.ins().jump(
            loop_block,
            &[new_exp.into(), new_result.into(), squared_base.into()],
        );

        // Exit block
        self.builder.switch_to_block(exit_block);
        let res = self.builder.block_params(exit_block)[0];

        // Seal all blocks
        self.builder.seal_block(loop_block);
        self.builder.seal_block(continue_block);
        self.builder.seal_block(exit_block);

        Ok(res)
    }
}

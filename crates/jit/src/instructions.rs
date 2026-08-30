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

#[derive(Clone)]
struct DDValue {
    hi: Value,
    lo: Value,
}

pub(crate) struct FunctionCompiler<'a, 'b> {
    builder: &'a mut FunctionBuilder<'b>,
    /// The buffer a guard spills its record into, parameter 0 of the function.
    deopt_ptr: Value,
    /// The block every guard leaves through, created with the first one.
    deopt_exit: Option<Block>,
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

/// Whether the machine code emitted for `op` answers the way the interpreter
/// does for every pair of values of these types.
///
/// Integer arithmetic either traps on overflow and division by zero - and a
/// trap has no handler, so it takes the process down instead of raising - or
/// wraps where Python would widen to an arbitrary-precision integer.
/// Float `/` is a bare `fdiv` with none of the checks that raise
/// ZeroDivisionError, and float `**` neither raises for `0.0 ** -1.0` nor
/// produces the complex result Python gives a negative base.
fn binary_op_is_faithful(op: BinaryOperator, a: Option<&JitType>, b: Option<&JitType>) -> bool {
    let traps_or_wraps_on_ints = matches!(
        op,
        BinaryOperator::Add
            | BinaryOperator::InplaceAdd
            | BinaryOperator::Subtract
            | BinaryOperator::InplaceSubtract
            | BinaryOperator::Multiply
            | BinaryOperator::InplaceMultiply
            | BinaryOperator::TrueDivide
            | BinaryOperator::InplaceTrueDivide
            | BinaryOperator::FloorDivide
            | BinaryOperator::InplaceFloorDivide
            | BinaryOperator::Remainder
            | BinaryOperator::InplaceRemainder
            | BinaryOperator::Power
            | BinaryOperator::InplacePower
            | BinaryOperator::Lshift
            | BinaryOperator::InplaceLshift
            | BinaryOperator::Rshift
            | BinaryOperator::InplaceRshift
    );
    let diverges_on_floats = matches!(
        op,
        BinaryOperator::TrueDivide
            | BinaryOperator::InplaceTrueDivide
            | BinaryOperator::Power
            | BinaryOperator::InplacePower
    );

    match (a, b) {
        (Some(JitType::Int), Some(JitType::Int)) => !traps_or_wraps_on_ints,
        // `(Int, Int)` is taken by the arm above, so it cannot land here.
        (Some(JitType::Float | JitType::Int), Some(JitType::Float))
        | (Some(JitType::Float), Some(JitType::Int)) => !diverges_on_floats,
        // Any other combination has no lowering at all and is rejected anyway.
        _ => true,
    }
}

impl<'a, 'b> FunctionCompiler<'a, 'b> {
    pub(crate) fn new(
        builder: &'a mut FunctionBuilder<'b>,
        num_variables: usize,
        arg_types: &[JitType],
        ret_type: Option<JitType>,
        entry_block: Block,
        safety: Safety,
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

                if self.safety == Safety::Strict
                    && !binary_op_is_faithful(op, a_type.as_ref(), b_type.as_ref())
                {
                    return Err(JitCompileError::NotSupported);
                }

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
                        let inexact = |compiler: &mut Self, v: Value| {
                            let magnitude = compiler.builder.ins().iabs(v);
                            compiler.builder.ins().icmp_imm(
                                IntCC::UnsignedGreaterThanOrEqual,
                                magnitude,
                                1 << 53,
                            )
                        };
                        let a_wide = inexact(self, a);
                        let b_wide = inexact(self, b);
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
                    ) => JitValue::Int(self.compile_ipow(a, b)),
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
    /// of the range.
    fn compile_floor_div(&mut self, a: Value, b: Value) -> Result<(Value, Value), JitCompileError> {
        let operands = [JitValue::Int(a), JitValue::Int(b)];
        let by_zero = self.builder.ins().icmp_imm(IntCC::Equal, b, 0);
        self.deopt_branch(by_zero, &operands)?;

        let min = self.builder.ins().icmp_imm(IntCC::Equal, a, i64::MIN);
        let neg_one = self.builder.ins().icmp_imm(IntCC::Equal, b, -1);
        let overflows = self.builder.ins().band(min, neg_one);
        self.deopt_branch(overflows, &operands)?;

        let quotient = self.builder.ins().sdiv(a, b);
        let remainder = self.builder.ins().srem(a, b);

        // The two disagree with Python exactly when the division was
        // inexact and the operands had opposite signs.
        let inexact = self.builder.ins().icmp_imm(IntCC::NotEqual, remainder, 0);
        let mixed = self.builder.ins().bxor(a, b);
        let mixed = self.builder.ins().icmp_imm(IntCC::SignedLessThan, mixed, 0);
        let correct = self.builder.ins().band(inexact, mixed);

        let one = self.builder.ins().iconst(types::I64, 1);
        let zero = self.builder.ins().iconst(types::I64, 0);
        let quotient_adjust = self.builder.ins().select(correct, one, zero);
        let quotient = self.builder.ins().isub(quotient, quotient_adjust);

        let remainder_adjust = self.builder.ins().select(correct, b, zero);
        let remainder = self.builder.ins().iadd(remainder, remainder_adjust);

        Ok((quotient, remainder))
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

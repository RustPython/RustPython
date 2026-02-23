/*! The python `frame` type.

*/

use super::{PyCode, PyDictRef, PyIntRef, PyStrRef};
use crate::{
    Context, Py, PyObjectRef, PyRef, PyResult, VirtualMachine,
    class::PyClassImpl,
    frame::{Frame, FrameOwner, FrameRef},
    function::PySetterValue,
    types::Representable,
};
use num_traits::Zero;
use rustpython_compiler_core::bytecode::{
    self, Constant, Instruction, InstructionMetadata, StackEffect,
};
use stack_analysis::*;

/// Stack state analysis for safe line-number jumps.
///
/// Models the evaluation stack as a 64-bit integer, encoding the kind of each
/// stack entry in 3-bit blocks. Used by `set_f_lineno` to verify that a jump
/// is safe and to determine how many values need to be popped.
pub(crate) mod stack_analysis {
    use super::*;

    const BITS_PER_BLOCK: u32 = 3;
    const MASK: i64 = (1 << BITS_PER_BLOCK) - 1; // 0b111
    const MAX_STACK_ENTRIES: u32 = 63 / BITS_PER_BLOCK; // 21
    const WILL_OVERFLOW: u64 = 1u64 << ((MAX_STACK_ENTRIES - 1) * BITS_PER_BLOCK);

    pub const EMPTY_STACK: i64 = 0;
    pub const UNINITIALIZED: i64 = -2;
    pub const OVERFLOWED: i64 = -1;

    /// Kind of a stack entry.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    #[repr(i64)]
    pub enum Kind {
        Iterator = 1,
        Except = 2,
        Object = 3,
        Null = 4,
        Lasti = 5,
    }

    impl Kind {
        fn from_i64(v: i64) -> Option<Self> {
            match v {
                1 => Some(Kind::Iterator),
                2 => Some(Kind::Except),
                3 => Some(Kind::Object),
                4 => Some(Kind::Null),
                5 => Some(Kind::Lasti),
                _ => None,
            }
        }
    }

    pub fn push_value(stack: i64, kind: i64) -> i64 {
        if (stack as u64) >= WILL_OVERFLOW {
            OVERFLOWED
        } else {
            (stack << BITS_PER_BLOCK) | kind
        }
    }

    pub fn pop_value(stack: i64) -> i64 {
        stack >> BITS_PER_BLOCK
    }

    pub fn top_of_stack(stack: i64) -> i64 {
        stack & MASK
    }

    fn peek(stack: i64, n: u32) -> i64 {
        debug_assert!(n >= 1);
        (stack >> (BITS_PER_BLOCK * (n - 1))) & MASK
    }

    fn stack_swap(stack: i64, n: u32) -> i64 {
        debug_assert!(n >= 1);
        let to_swap = peek(stack, n);
        let top = top_of_stack(stack);
        let shift = BITS_PER_BLOCK * (n - 1);
        let replaced_low = (stack & !(MASK << shift)) | (top << shift);
        (replaced_low & !MASK) | to_swap
    }

    fn pop_to_level(mut stack: i64, level: u32) -> i64 {
        if level == 0 {
            return EMPTY_STACK;
        }
        let max_item: i64 = (1 << BITS_PER_BLOCK) - 1;
        let level_max_stack = max_item << ((level - 1) * BITS_PER_BLOCK);
        while stack > level_max_stack {
            stack = pop_value(stack);
        }
        stack
    }

    fn compatible_kind(from: i64, to: i64) -> bool {
        if to == 0 {
            return false;
        }
        if to == Kind::Object as i64 {
            return from != Kind::Null as i64;
        }
        if to == Kind::Null as i64 {
            return true;
        }
        from == to
    }

    pub fn compatible_stack(from_stack: i64, to_stack: i64) -> bool {
        if from_stack < 0 || to_stack < 0 {
            return false;
        }
        let mut from = from_stack;
        let mut to = to_stack;
        while from > to {
            from = pop_value(from);
        }
        while from != 0 {
            let from_top = top_of_stack(from);
            let to_top = top_of_stack(to);
            if !compatible_kind(from_top, to_top) {
                return false;
            }
            from = pop_value(from);
            to = pop_value(to);
        }
        to == 0
    }

    pub fn explain_incompatible_stack(to_stack: i64) -> &'static str {
        debug_assert!(to_stack != 0);
        if to_stack == OVERFLOWED {
            return "stack is too deep to analyze";
        }
        if to_stack == UNINITIALIZED {
            return "can't jump into an exception handler, or code may be unreachable";
        }
        match Kind::from_i64(top_of_stack(to_stack)) {
            Some(Kind::Except) => "can't jump into an 'except' block as there's no exception",
            Some(Kind::Lasti) => "can't jump into a re-raising block as there's no location",
            Some(Kind::Iterator) => "can't jump into the body of a for loop",
            _ => "incompatible stacks",
        }
    }

    /// Analyze bytecode and compute the stack state at each instruction index.
    pub fn mark_stacks<C: Constant>(code: &bytecode::CodeObject<C>) -> Vec<i64> {
        let instructions = &*code.instructions;
        let len = instructions.len();

        let mut stacks = vec![UNINITIALIZED; len + 1];
        stacks[0] = EMPTY_STACK;

        let mut todo = true;
        while todo {
            todo = false;

            let mut i = 0;
            while i < len {
                let mut next_stack = stacks[i];
                let mut opcode = instructions[i].op;
                let mut oparg: u32 = 0;

                // Accumulate EXTENDED_ARG prefixes
                while matches!(opcode, Instruction::ExtendedArg) {
                    oparg = (oparg << 8) | u32::from(u8::from(instructions[i].arg));
                    i += 1;
                    if i >= len {
                        break;
                    }
                    stacks[i] = next_stack;
                    opcode = instructions[i].op;
                }
                if i >= len {
                    break;
                }
                oparg = (oparg << 8) | u32::from(u8::from(instructions[i].arg));

                // De-instrument: get the underlying real instruction
                let opcode = opcode.to_base().unwrap_or(opcode);

                let next_i = i + 1; // No inline caches in RustPython

                if next_stack == UNINITIALIZED {
                    i = next_i;
                    continue;
                }

                match opcode {
                    Instruction::PopJumpIfFalse { .. }
                    | Instruction::PopJumpIfTrue { .. }
                    | Instruction::PopJumpIfNone { .. }
                    | Instruction::PopJumpIfNotNone { .. } => {
                        // Jump target is absolute instruction index
                        let j = oparg as usize;
                        next_stack = pop_value(next_stack);
                        let target_stack = next_stack;
                        if j < stacks.len() && stacks[j] == UNINITIALIZED {
                            stacks[j] = target_stack;
                        }
                        if next_i < stacks.len() {
                            stacks[next_i] = next_stack;
                        }
                    }
                    Instruction::Send { .. } => {
                        // target is absolute
                        let j = oparg as usize;
                        if j < stacks.len() && stacks[j] == UNINITIALIZED {
                            stacks[j] = next_stack;
                        }
                        if next_i < stacks.len() {
                            stacks[next_i] = next_stack;
                        }
                    }
                    Instruction::JumpForward { .. } => {
                        // target is absolute in RustPython
                        let j = oparg as usize;
                        if j < stacks.len() && stacks[j] == UNINITIALIZED {
                            stacks[j] = next_stack;
                        }
                    }
                    Instruction::JumpBackward { .. }
                    | Instruction::JumpBackwardNoInterrupt { .. } => {
                        // target is absolute in RustPython
                        let j = oparg as usize;
                        if j < stacks.len() && stacks[j] == UNINITIALIZED {
                            stacks[j] = next_stack;
                            if j < i {
                                todo = true;
                            }
                        }
                    }
                    Instruction::GetIter | Instruction::GetAIter => {
                        next_stack = push_value(pop_value(next_stack), Kind::Iterator as i64);
                        if next_i < stacks.len() {
                            stacks[next_i] = next_stack;
                        }
                    }
                    Instruction::ForIter { .. } => {
                        // Fall-through (iteration continues): pushes the next value
                        let body_stack = push_value(next_stack, Kind::Object as i64);
                        if next_i < stacks.len() {
                            stacks[next_i] = body_stack;
                        }
                        // Exhaustion path: execute_for_iter skips END_FOR and
                        // jumps directly to POP_ITER. The iterator stays on
                        // the stack and POP_ITER removes it.
                        let mut j = oparg as usize;
                        if j < instructions.len() {
                            let target_op =
                                instructions[j].op.to_base().unwrap_or(instructions[j].op);
                            if matches!(target_op, Instruction::EndFor) {
                                j += 1;
                            }
                        }
                        if j < stacks.len() && stacks[j] == UNINITIALIZED {
                            stacks[j] = next_stack;
                        }
                    }
                    Instruction::EndAsyncFor => {
                        next_stack = pop_value(pop_value(next_stack));
                        if next_i < stacks.len() {
                            stacks[next_i] = next_stack;
                        }
                    }
                    Instruction::PushExcInfo => {
                        next_stack = push_value(next_stack, Kind::Except as i64);
                        if next_i < stacks.len() {
                            stacks[next_i] = next_stack;
                        }
                    }
                    Instruction::PopExcept => {
                        next_stack = pop_value(next_stack);
                        if next_i < stacks.len() {
                            stacks[next_i] = next_stack;
                        }
                    }
                    Instruction::ReturnValue => {
                        // End of block, no fall-through
                    }
                    Instruction::RaiseVarargs { .. } => {
                        // End of block, no fall-through
                    }
                    Instruction::Reraise { .. } => {
                        // End of block, no fall-through
                    }
                    Instruction::PushNull => {
                        next_stack = push_value(next_stack, Kind::Null as i64);
                        if next_i < stacks.len() {
                            stacks[next_i] = next_stack;
                        }
                    }
                    Instruction::LoadGlobal(_) => {
                        // RustPython's LOAD_GLOBAL doesn't encode push_null in oparg
                        // (separate PUSH_NULL instructions are used instead)
                        next_stack = push_value(next_stack, Kind::Object as i64);
                        if next_i < stacks.len() {
                            stacks[next_i] = next_stack;
                        }
                    }
                    Instruction::LoadAttr { .. } => {
                        // LoadAttr: pops object, pushes result
                        // If oparg & 1, it also pushes Null (method load)
                        let attr_oparg = oparg;
                        if attr_oparg & 1 != 0 {
                            next_stack = pop_value(next_stack);
                            next_stack = push_value(next_stack, Kind::Object as i64);
                            next_stack = push_value(next_stack, Kind::Null as i64);
                        }
                        // else: default stack_effect handles it
                        else {
                            let effect: StackEffect = opcode.stack_effect_info(oparg);
                            let popped = effect.popped() as i64;
                            let pushed = effect.pushed() as i64;
                            for _ in 0..popped {
                                next_stack = pop_value(next_stack);
                            }
                            for _ in 0..pushed {
                                next_stack = push_value(next_stack, Kind::Object as i64);
                            }
                        }
                        if next_i < stacks.len() {
                            stacks[next_i] = next_stack;
                        }
                    }
                    Instruction::Swap { .. } => {
                        let n = oparg;
                        next_stack = stack_swap(next_stack, n);
                        if next_i < stacks.len() {
                            stacks[next_i] = next_stack;
                        }
                    }
                    Instruction::Copy { .. } => {
                        let n = oparg;
                        next_stack = push_value(next_stack, peek(next_stack, n));
                        if next_i < stacks.len() {
                            stacks[next_i] = next_stack;
                        }
                    }
                    _ => {
                        // Default: use stack_effect
                        let effect: StackEffect = opcode.stack_effect_info(oparg);
                        let popped = effect.popped() as i64;
                        let pushed = effect.pushed() as i64;
                        let mut ns = next_stack;
                        for _ in 0..popped {
                            ns = pop_value(ns);
                        }
                        for _ in 0..pushed {
                            ns = push_value(ns, Kind::Object as i64);
                        }
                        next_stack = ns;
                        if next_i < stacks.len() {
                            stacks[next_i] = next_stack;
                        }
                    }
                }
                i = next_i;
            }

            // Scan exception table
            let exception_table = bytecode::decode_exception_table(&code.exceptiontable);
            for entry in &exception_table {
                let start_offset = entry.start as usize;
                let handler = entry.target as usize;
                let level = entry.depth as u32;
                let has_lasti = entry.push_lasti;

                if start_offset < stacks.len()
                    && stacks[start_offset] != UNINITIALIZED
                    && handler < stacks.len()
                    && stacks[handler] == UNINITIALIZED
                {
                    todo = true;
                    let mut target_stack = pop_to_level(stacks[start_offset], level);
                    if has_lasti {
                        target_stack = push_value(target_stack, Kind::Lasti as i64);
                    }
                    target_stack = push_value(target_stack, Kind::Except as i64);
                    stacks[handler] = target_stack;
                }
            }
        }

        stacks
    }

    /// Build a mapping from instruction index to line number.
    /// Returns -1 for indices with no line start.
    pub fn mark_lines<C: Constant>(code: &bytecode::CodeObject<C>) -> Vec<i32> {
        let len = code.instructions.len();
        let mut line_starts = vec![-1i32; len];
        let mut last_line: i32 = -1;

        for (i, (loc, _)) in code.locations.iter().enumerate() {
            if i >= len {
                break;
            }
            let line = loc.line.get() as i32;
            if line != last_line && line > 0 {
                line_starts[i] = line;
                last_line = line;
            }
        }
        line_starts
    }

    /// Find the first line number >= `line` that has code.
    pub fn first_line_not_before(lines: &[i32], line: i32) -> i32 {
        let mut result = i32::MAX;
        for &l in lines {
            if l >= line && l < result {
                result = l;
            }
        }
        if result == i32::MAX { -1 } else { result }
    }
}

pub fn init(context: &Context) {
    Frame::extend_class(context, context.types.frame_type);
}

impl Representable for Frame {
    #[inline]
    fn repr(_zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        const REPR: &str = "<frame object at .. >";
        Ok(vm.ctx.intern_str(REPR).to_owned())
    }

    #[cold]
    fn repr_str(_zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
        unreachable!("use repr instead")
    }
}

#[pyclass(flags(DISALLOW_INSTANTIATION), with(Py))]
impl Frame {
    #[pygetset]
    fn f_globals(&self) -> PyDictRef {
        self.globals.clone()
    }

    #[pygetset]
    fn f_builtins(&self) -> PyObjectRef {
        self.builtins.clone()
    }

    #[pygetset]
    fn f_locals(&self, vm: &VirtualMachine) -> PyResult {
        let result = self.locals(vm).map(Into::into);
        self.locals_dirty
            .store(true, core::sync::atomic::Ordering::Release);
        result
    }

    #[pygetset]
    pub fn f_code(&self) -> PyRef<PyCode> {
        self.code.clone()
    }

    #[pygetset]
    fn f_lasti(&self) -> u32 {
        // Return byte offset (each instruction is 2 bytes) for compatibility
        self.lasti() * 2
    }

    #[pygetset]
    pub fn f_lineno(&self) -> usize {
        // If lasti is 0, execution hasn't started yet - use first line number
        // Similar to PyCode_Addr2Line which returns co_firstlineno for addr_q < 0
        if self.lasti() == 0 {
            self.code.first_line_number.map(|n| n.get()).unwrap_or(1)
        } else {
            self.current_location().line.get()
        }
    }

    #[pygetset(setter)]
    fn set_f_lineno(&self, value: PySetterValue, vm: &VirtualMachine) -> PyResult<()> {
        let l_new_lineno = match value {
            PySetterValue::Assign(val) => {
                let line_ref: PyIntRef = val
                    .downcast()
                    .map_err(|_| vm.new_value_error("lineno must be an integer".to_owned()))?;
                line_ref
                    .try_to_primitive::<i32>(vm)
                    .map_err(|_| vm.new_value_error("lineno must be an integer".to_owned()))?
            }
            PySetterValue::Delete => {
                return Err(vm.new_type_error("can't delete f_lineno attribute".to_owned()));
            }
        };

        let first_line = self
            .code
            .first_line_number
            .map(|n| n.get() as i32)
            .unwrap_or(1);

        if l_new_lineno < first_line {
            return Err(vm.new_value_error(format!(
                "line {l_new_lineno} comes before the current code block"
            )));
        }

        let py_code: &PyCode = &self.code;
        let code = &py_code.code;
        let lines = mark_lines(code);

        // Find the first line >= target that has actual code
        let new_lineno = first_line_not_before(&lines, l_new_lineno);
        if new_lineno < 0 {
            return Err(vm.new_value_error(format!(
                "line {l_new_lineno} comes after the current code block"
            )));
        }

        let stacks = mark_stacks(code);
        let len = self.code.instructions.len();

        // lasti points past the current instruction (already incremented).
        // stacks[lasti - 1] gives the stack state before executing the
        // instruction that triggered this trace event, which is the current
        // evaluation stack.
        let current_lasti = self.lasti() as usize;
        let start_idx = current_lasti.saturating_sub(1);
        let start_stack = if start_idx < stacks.len() {
            stacks[start_idx]
        } else {
            OVERFLOWED
        };
        let mut best_stack = OVERFLOWED;
        let mut best_addr: i32 = -1;
        let mut err: i32 = -1;
        let mut msg = "cannot find bytecode for specified line";

        for i in 0..len {
            if lines[i] == new_lineno {
                let target_stack = stacks[i];
                if compatible_stack(start_stack, target_stack) {
                    err = 0;
                    if target_stack > best_stack {
                        best_stack = target_stack;
                        best_addr = i as i32;
                    }
                } else if err < 0 {
                    if start_stack == OVERFLOWED {
                        msg = "stack to deep to analyze";
                    } else if start_stack == UNINITIALIZED {
                        msg = "can't jump from unreachable code";
                    } else {
                        msg = explain_incompatible_stack(target_stack);
                        err = 1;
                    }
                }
            }
        }

        if err != 0 {
            return Err(vm.new_value_error(msg.to_owned()));
        }

        // Count how many entries to pop
        let mut pop_count = 0usize;
        {
            let mut s = start_stack;
            while s > best_stack {
                pop_count += 1;
                s = pop_value(s);
            }
        }

        // Store the pending unwind for the execution loop to perform.
        // We cannot pop stack entries here because the execution loop
        // holds the state mutex, and trying to lock it again would deadlock.
        self.set_pending_stack_pops(pop_count as u32);
        self.set_pending_unwind_from_stack(start_stack);

        // Set lasti to best_addr. The executor will read lasti and execute
        // the instruction at that index next.
        self.set_lasti(best_addr as u32);
        Ok(())
    }

    #[pygetset]
    fn f_trace(&self) -> PyObjectRef {
        let boxed = self.trace.lock();
        boxed.clone()
    }

    #[pygetset(setter)]
    fn set_f_trace(&self, value: PySetterValue, vm: &VirtualMachine) {
        let mut storage = self.trace.lock();
        *storage = value.unwrap_or_none(vm);
    }

    #[pymember(type = "bool")]
    fn f_trace_lines(vm: &VirtualMachine, zelf: PyObjectRef) -> PyResult {
        let zelf: FrameRef = zelf.downcast().unwrap_or_else(|_| unreachable!());

        let boxed = zelf.trace_lines.lock();
        Ok(vm.ctx.new_bool(*boxed).into())
    }

    #[pymember(type = "bool", setter)]
    fn set_f_trace_lines(
        vm: &VirtualMachine,
        zelf: PyObjectRef,
        value: PySetterValue,
    ) -> PyResult<()> {
        match value {
            PySetterValue::Assign(value) => {
                let zelf: FrameRef = zelf.downcast().unwrap_or_else(|_| unreachable!());

                let value: PyIntRef = value
                    .downcast()
                    .map_err(|_| vm.new_type_error("attribute value type must be bool"))?;

                let mut trace_lines = zelf.trace_lines.lock();
                *trace_lines = !value.as_bigint().is_zero();

                Ok(())
            }
            PySetterValue::Delete => Err(vm.new_type_error("can't delete numeric/char attribute")),
        }
    }

    #[pymember(type = "bool")]
    fn f_trace_opcodes(vm: &VirtualMachine, zelf: PyObjectRef) -> PyResult {
        let zelf: FrameRef = zelf.downcast().unwrap_or_else(|_| unreachable!());
        let trace_opcodes = zelf.trace_opcodes.lock();
        Ok(vm.ctx.new_bool(*trace_opcodes).into())
    }

    #[pymember(type = "bool", setter)]
    fn set_f_trace_opcodes(
        vm: &VirtualMachine,
        zelf: PyObjectRef,
        value: PySetterValue,
    ) -> PyResult<()> {
        match value {
            PySetterValue::Assign(value) => {
                let zelf: FrameRef = zelf.downcast().unwrap_or_else(|_| unreachable!());

                let value: PyIntRef = value
                    .downcast()
                    .map_err(|_| vm.new_type_error("attribute value type must be bool"))?;

                let mut trace_opcodes = zelf.trace_opcodes.lock();
                *trace_opcodes = !value.as_bigint().is_zero();

                // TODO: Implement the equivalent of _PyEval_SetOpcodeTrace()

                Ok(())
            }
            PySetterValue::Delete => Err(vm.new_type_error("can't delete numeric/char attribute")),
        }
    }
}

#[pyclass]
impl Py<Frame> {
    #[pymethod]
    // = frame_clear_impl
    fn clear(&self, vm: &VirtualMachine) -> PyResult<()> {
        let owner = FrameOwner::from_i8(self.owner.load(core::sync::atomic::Ordering::Acquire));
        match owner {
            FrameOwner::Generator => {
                // Generator frame: check if suspended (lasti > 0 means
                // FRAME_SUSPENDED). lasti == 0 means FRAME_CREATED and
                // can be cleared.
                if self.lasti() != 0 {
                    return Err(vm.new_runtime_error("cannot clear a suspended frame".to_owned()));
                }
            }
            FrameOwner::Thread => {
                // Thread-owned frame: always executing, cannot clear.
                return Err(vm.new_runtime_error("cannot clear an executing frame".to_owned()));
            }
            FrameOwner::FrameObject => {
                // Detached frame: safe to clear.
            }
        }

        // Clear fastlocals
        {
            let mut fastlocals = self.fastlocals.lock();
            for slot in fastlocals.iter_mut() {
                *slot = None;
            }
        }

        // Clear the evaluation stack and cell references
        self.clear_stack_and_cells();

        // Clear temporary refs
        self.temporary_refs.lock().clear();

        Ok(())
    }

    #[pygetset]
    fn f_generator(&self) -> Option<PyObjectRef> {
        self.generator.to_owned()
    }

    #[pygetset]
    pub fn f_back(&self, vm: &VirtualMachine) -> Option<PyRef<Frame>> {
        let previous = self.previous_frame();
        if previous.is_null() {
            return None;
        }

        if let Some(frame) = vm
            .frames
            .borrow()
            .iter()
            .find(|fp| {
                // SAFETY: the caller keeps the FrameRef alive while it's in the Vec
                let py: &crate::Py<Frame> = unsafe { fp.as_ref() };
                let ptr: *const Frame = &**py;
                core::ptr::eq(ptr, previous)
            })
            .map(|fp| unsafe { fp.as_ref() }.to_owned())
        {
            return Some(frame);
        }

        #[cfg(feature = "threading")]
        {
            let registry = vm.state.thread_frames.lock();
            for slot in registry.values() {
                let frames = slot.frames.lock();
                // SAFETY: the owning thread can't pop while we hold the Mutex,
                // so FramePtr is valid for the duration of the lock.
                if let Some(frame) = frames.iter().find_map(|fp| {
                    let f = unsafe { fp.as_ref() };
                    let ptr: *const Frame = &**f;
                    core::ptr::eq(ptr, previous).then(|| f.to_owned())
                }) {
                    return Some(frame);
                }
            }
        }

        None
    }
}

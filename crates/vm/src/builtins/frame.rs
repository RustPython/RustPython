/*! The python `frame` type.

*/

use super::{PyCode, PyDictRef, PyIntRef, PyStrRef};
use crate::{
    Context, Py, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    class::PyClassImpl,
    frame::{FrameObject, FrameObjectRef, FrameOwner},
    function::PySetterValue,
    types::Representable,
};
use core::sync::atomic::Ordering::Relaxed;
use num_traits::Zero;
#[allow(unused_imports)]
use rustpython_common::atomic::Radium;
use rustpython_compiler_core::bytecode::{self, Constant, Instruction, StackEffect};
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

    pub(crate) const EMPTY_STACK: i64 = 0;
    pub(crate) const UNINITIALIZED: i64 = -2;
    pub(crate) const OVERFLOWED: i64 = -1;

    /// Kind of a stack entry.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    #[repr(i64)]
    pub(crate) enum Kind {
        Iterator = 1,
        Except = 2,
        Object = 3,
        Null = 4,
        Lasti = 5,
    }

    impl Kind {
        const fn from_i64(v: i64) -> Option<Self> {
            Some(match v {
                1 => Self::Iterator,
                2 => Self::Except,
                3 => Self::Object,
                4 => Self::Null,
                5 => Self::Lasti,
                _ => return None,
            })
        }
    }

    pub(crate) const fn push_value(stack: i64, kind: i64) -> i64 {
        if (stack as u64) >= WILL_OVERFLOW {
            OVERFLOWED
        } else {
            (stack << BITS_PER_BLOCK) | kind
        }
    }

    pub(crate) const fn pop_value(stack: i64) -> i64 {
        stack >> BITS_PER_BLOCK
    }

    pub(crate) const fn top_of_stack(stack: i64) -> i64 {
        stack & MASK
    }

    const fn peek(stack: i64, n: u32) -> i64 {
        debug_assert!(n >= 1);
        (stack >> (BITS_PER_BLOCK * (n - 1))) & MASK
    }

    const fn stack_swap(stack: i64, n: u32) -> i64 {
        debug_assert!(n >= 1);
        let to_swap = peek(stack, n);
        let top = top_of_stack(stack);
        let shift = BITS_PER_BLOCK * (n - 1);
        let replaced_low = (stack & !(MASK << shift)) | (top << shift);
        (replaced_low & !MASK) | to_swap
    }

    const fn pop_to_level(mut stack: i64, level: u32) -> i64 {
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

    #[must_use]
    const fn compatible_kind(from: i64, to: i64) -> bool {
        if to == 0 {
            false
        } else if to == Kind::Object as i64 {
            from != Kind::Null as i64
        } else if to == Kind::Null as i64 {
            true
        } else {
            from == to
        }
    }

    #[must_use]
    pub(crate) const fn compatible_stack(from_stack: i64, to_stack: i64) -> bool {
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

    pub(crate) const fn explain_incompatible_stack(to_stack: i64) -> &'static str {
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
    pub(crate) fn mark_stacks<C: Constant>(code: &bytecode::CodeObject<C>) -> Vec<i64> {
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

                // De-instrument and de-specialize: get the underlying base instruction
                let opcode = opcode.to_base().unwrap_or(opcode).deoptimize();

                let caches = opcode.cache_entries();
                let next_i = i + 1 + caches;

                if next_stack == UNINITIALIZED {
                    i = next_i;
                    continue;
                }

                match opcode {
                    Instruction::PopJumpIfFalse { .. }
                    | Instruction::PopJumpIfTrue { .. }
                    | Instruction::PopJumpIfNone { .. }
                    | Instruction::PopJumpIfNotNone { .. } => {
                        // Relative forward: target = after_caches + delta
                        let j = next_i + oparg as usize;
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
                        // Relative forward: target = after_caches + delta
                        let j = next_i + oparg as usize;
                        if j < stacks.len() && stacks[j] == UNINITIALIZED {
                            stacks[j] = next_stack;
                        }
                        if next_i < stacks.len() {
                            stacks[next_i] = next_stack;
                        }
                    }
                    Instruction::JumpForward { .. } => {
                        // Relative forward: target = after_caches + delta
                        let j = next_i + oparg as usize;
                        if j < stacks.len() && stacks[j] == UNINITIALIZED {
                            stacks[j] = next_stack;
                        }
                    }
                    Instruction::JumpBackward { .. }
                    | Instruction::JumpBackwardNoInterrupt { .. } => {
                        // Relative backward: target = after_caches - delta
                        let j = next_i - oparg as usize;
                        if j < stacks.len() && stacks[j] == UNINITIALIZED {
                            stacks[j] = next_stack;
                            if j < i {
                                todo = true;
                            }
                        }
                    }
                    Instruction::GetIter | Instruction::GetAiter => {
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
                        // Exhaustion path: relative forward from after_caches
                        let mut j = next_i + oparg as usize;
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
                    Instruction::LoadGlobal { .. } => {
                        next_stack = push_value(next_stack, Kind::Object as i64);
                        if oparg & 1 != 0 {
                            next_stack = push_value(next_stack, Kind::Null as i64);
                        }
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
    pub(crate) fn mark_lines<C: Constant>(code: &bytecode::CodeObject<C>) -> Vec<i32> {
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
    pub(crate) fn first_line_not_before(lines: &[i32], line: i32) -> i32 {
        let mut result = i32::MAX;
        for &l in lines {
            if l >= line && l < result {
                result = l;
            }
        }
        if result == i32::MAX { -1 } else { result }
    }
}

pub(crate) fn init(context: &'static Context) {
    FrameObject::extend_class(context, context.types.frame_type);
}

impl Representable for FrameObject {
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

impl FrameObject {
    /// Find the live source InterpreterFrame on the TLS chain for a
    /// materialized FrameObject. Returns the raw pointer if found, or null
    /// if this FrameObject has no live source (already returned or not
    /// currently executing on this thread).
    pub(crate) fn find_live_source_iframe(&self) -> *const crate::frame::InterpreterFrame {
        let self_py_ptr = unsafe { Py::<Self>::from_payload_ptr(self) } as usize;
        let mut cur = crate::vm::thread::get_current_frame();
        while !cur.is_null() {
            let materialized = unsafe { (*cur).materialized.load(Relaxed) };
            if materialized == self_py_ptr {
                return cur;
            }
            cur = unsafe { &*cur }.previous();
        }
        core::ptr::null()
    }

    /// Find the live source InterpreterFrame for a materialized FrameObject
    /// on the chain of thread `tid`, which must not be this one — a thread
    /// publishes its top frame for other threads, but keeps its own in TLS.
    /// Returns null if that thread is not running the source frame.
    ///
    /// # Safety
    /// Caller must hold the world stopped, so the owning thread is parked and
    /// its chain is not being popped while it is walked.
    #[cfg(feature = "threading")]
    unsafe fn find_live_source_iframe_on(
        &self,
        tid: u64,
        vm: &VirtualMachine,
    ) -> *const crate::frame::InterpreterFrame {
        let self_py_ptr = unsafe { Py::<Self>::from_payload_ptr(self) } as usize;
        let registry = vm.state.thread_frames.lock();
        let Some(slot) = registry.get(&tid) else {
            return core::ptr::null();
        };
        let mut cur = slot.top_iframe.load(Relaxed) as *const crate::frame::InterpreterFrame;
        while !cur.is_null() {
            if unsafe { (*cur).materialized.load(Relaxed) } == self_py_ptr {
                return cur;
            }
            cur = unsafe { &*cur }.previous();
        }
        core::ptr::null()
    }

    /// The other thread that is still running the frame this object was
    /// materialized from, or `None`. A source frame on this thread does not
    /// count: `find_live_source_iframe` already covers this thread in full,
    /// so anything it misses is running elsewhere or has returned.
    #[cfg(feature = "threading")]
    fn source_thread(&self) -> Option<u64> {
        let tid = self.iframe().attached_tid();
        (tid != 0 && tid != crate::stdlib::_thread::get_ident()).then_some(tid)
    }
}

#[pyclass(flags(DISALLOW_INSTANTIATION), with(Py))]
impl FrameObject {
    #[pygetset]
    fn f_globals(&self) -> PyDictRef {
        self.iframe().globals().to_owned()
    }

    #[pygetset]
    fn f_builtins(&self) -> PyObjectRef {
        self.iframe().builtins().to_owned()
    }

    #[pygetset]
    pub fn f_code(&self) -> PyRef<PyCode> {
        self.iframe().code().to_owned()
    }

    #[pygetset]
    fn f_lasti(&self) -> u32 {
        // Return byte offset (each instruction is 2 bytes) for compatibility.
        // For materialized frames, read live lasti from the source iframe on
        // the TLS chain so f_lasti reflects the current execution position.
        let live = self.find_live_source_iframe();
        let val = if !live.is_null() {
            unsafe { (*live).lasti.load(Relaxed) }
        } else {
            self.lasti()
        };
        val * 2
    }

    #[pygetset]
    pub fn f_lineno(&self) -> usize {
        let code = self.iframe().code();
        let first_line = || code.first_line_number.map_or(1, |n| n.get());
        // A running frame advances lasti past the instruction it is about to
        // execute, so the line being executed is the one before it. The live
        // position has to be read rather than this object's copy: a frame
        // observed from inside a call it made still reports that call's line.
        let live = self.find_live_source_iframe();
        let lasti = if live.is_null() {
            self.lasti()
        } else {
            unsafe { (*live).lasti.load(Relaxed) }
        };
        let Some(idx) = lasti.checked_sub(1) else {
            // Execution has not started.
            return first_line();
        };
        // A live lasti can move between the read and the lookup.
        code.locations
            .get(idx as usize)
            .map_or_else(first_line, |(loc, _)| loc.line.get())
    }

    #[pygetset(setter)]
    fn set_f_lineno(&self, value: PySetterValue, vm: &VirtualMachine) -> PyResult<()> {
        let l_new_lineno = match value {
            PySetterValue::Assign(val) => {
                let line_ref: PyIntRef = val
                    .downcast()
                    .map_err(|_| vm.new_value_error("lineno must be an integer"))?;
                line_ref
                    .try_to_primitive::<i32>(vm)
                    .map_err(|_| vm.new_value_error("lineno must be an integer"))?
            }
            PySetterValue::Delete => {
                return Err(vm.new_type_error("can't delete f_lineno attribute"));
            }
        };

        let first_line = self
            .iframe()
            .code()
            .first_line_number
            .map_or(1, |n| n.get() as i32);

        if l_new_lineno < first_line {
            return Err(vm.new_value_error(format!(
                "line {l_new_lineno} comes before the current code block"
            )));
        }

        let py_code: &PyCode = self.iframe().code();
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
        let len = self.iframe().code().instructions.len();

        // lasti points past the current instruction (already incremented).
        // stacks[lasti - 1] gives the stack state before executing the
        // instruction that triggered this trace event, which is the current
        // evaluation stack.  Read from the live iframe when available so the
        // value reflects the actual execution position.
        let live = self.find_live_source_iframe();
        let current_lasti = if !live.is_null() {
            (unsafe { (*live).lasti.load(Relaxed) }) as usize
        } else {
            self.lasti() as usize
        };
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

        // Store the pending unwind and new lasti. When this frame is backed
        // by a live stack-allocated iframe, write to the live iframe so the
        // execution loop picks up the jump target.  Reuse `live` from above.
        let target = if !live.is_null() {
            unsafe { &*live }
        } else {
            self.iframe()
        };
        target
            .cold()
            .pending_stack_pops
            .store(pop_count as u32, Relaxed);
        target
            .cold()
            .pending_unwind_from_stack
            .store(start_stack, Relaxed);
        target.lasti.store(best_addr as u32, Relaxed);
        Ok(())
    }

    #[pygetset]
    fn f_trace(&self, vm: &VirtualMachine) -> PyObjectRef {
        // Read from live source iframe if available.
        let live = self.find_live_source_iframe();
        let trace = if !live.is_null() {
            unsafe { &*live }.cold().trace.lock().clone()
        } else {
            self.iframe().cold().trace.lock().clone()
        };
        trace.unwrap_or_else(|| vm.ctx.none())
    }

    #[pygetset(setter)]
    fn set_f_trace(&self, value: PySetterValue, vm: &VirtualMachine) {
        let trace = match value {
            PySetterValue::Assign(v) => {
                if vm.is_none(&v) {
                    None
                } else {
                    Some(v)
                }
            }
            PySetterValue::Delete => None,
        };
        // Set on the materialized FrameObject.
        (*self.iframe().cold().trace.lock()).clone_from(&trace);
        // Also propagate to the live source iframe if this is a
        // materialized copy of a stack-allocated frame, so pdb's
        // f_trace assignment takes effect on the executing frame.
        let live = self.find_live_source_iframe();
        if !live.is_null() {
            *unsafe { &*live }.cold().trace.lock() = trace;
        }
    }

    #[expect(clippy::unnecessary_wraps, reason = "Needs to comply with a signature")]
    #[pymember(type = "bool")]
    fn f_trace_lines(vm: &VirtualMachine, zelf: PyObjectRef) -> PyResult {
        let zelf: FrameObjectRef = zelf.downcast().unwrap_or_else(|_| unreachable!());

        let boxed = zelf.iframe().cold().trace_lines.lock();
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
                let zelf: FrameObjectRef = zelf.downcast().unwrap_or_else(|_| unreachable!());

                let value: PyIntRef = value
                    .downcast()
                    .map_err(|_| vm.new_type_error("attribute value type must be bool"))?;

                let val = !value.as_bigint().is_zero();
                *zelf.iframe().cold().trace_lines.lock() = val;
                // Propagate to live source iframe.
                let live = zelf.find_live_source_iframe();
                if !live.is_null() {
                    *unsafe { &*live }.cold().trace_lines.lock() = val;
                }

                Ok(())
            }
            PySetterValue::Delete => Err(vm.new_type_error("can't delete numeric/char attribute")),
        }
    }

    #[expect(clippy::unnecessary_wraps, reason = "Needs to comply with a signature")]
    #[pymember(type = "bool")]
    fn f_trace_opcodes(vm: &VirtualMachine, zelf: PyObjectRef) -> PyResult {
        let zelf: FrameObjectRef = zelf.downcast().unwrap_or_else(|_| unreachable!());
        let trace_opcodes = zelf.iframe().cold().trace_opcodes.lock();
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
                let zelf: FrameObjectRef = zelf.downcast().unwrap_or_else(|_| unreachable!());

                let value: PyIntRef = value
                    .downcast()
                    .map_err(|_| vm.new_type_error("attribute value type must be bool"))?;

                let val = !value.as_bigint().is_zero();
                *zelf.iframe().cold().trace_opcodes.lock() = val;
                // Propagate to live source iframe.
                let live = zelf.find_live_source_iframe();
                if !live.is_null() {
                    *unsafe { &*live }.cold().trace_opcodes.lock() = val;
                }

                // TODO: Implement the equivalent of _PyEval_SetOpcodeTrace()

                Ok(())
            }
            PySetterValue::Delete => Err(vm.new_type_error("can't delete numeric/char attribute")),
        }
    }
}

#[cfg(feature = "threading")]
impl Py<FrameObject> {
    /// `f_back` for a frame materialized from one that is still running on
    /// thread `tid`. Such a copy carries no `previous` of its own, and the
    /// chain it was taken from lives on that thread's stack.
    #[cold]
    fn back_from_thread(&self, tid: u64, vm: &VirtualMachine) -> Option<PyRef<FrameObject>> {
        vm.state.stop_the_world.stop_the_world(&vm.state);
        scopeguard::defer! { vm.state.stop_the_world.start_the_world(&vm.state); }
        // SAFETY: the world is stopped, so the owning thread is parked.
        let live = unsafe { self.find_live_source_iframe_on(tid, vm) };
        if live.is_null() {
            // The source frame returned between the read of `attached_tid`
            // and the stop, so its caller is recorded by now.
            let retained = self.iframe().cold().retained_back.lock().clone();
            if let Some(frame) = &retained {
                frame.mark_escaped();
            }
            return retained;
        }
        let prev = unsafe { (*live).previous() };
        if prev.is_null() {
            return None;
        }
        let prev_ref = unsafe { &*prev };
        if let Some(fo) = prev_ref.frame_obj() {
            fo.mark_escaped();
            return Some(fo.to_owned());
        }
        // SAFETY: the world is stopped, so the owning thread is parked.
        let fo = unsafe { prev_ref.materialize_detached_chain(vm) };
        fo.mark_escaped();
        Some(fo)
    }
}

#[pyclass]
impl Py<FrameObject> {
    #[pymethod]
    // = frame_clear_impl
    fn clear(&self, vm: &VirtualMachine) -> PyResult<()> {
        // Materialized stack frames are FrameObject-owned even while their
        // source iframe is still executing. TLS lookup below only sees the
        // calling thread, so attached_tid is the cross-thread-safe execution
        // state check.
        if self.iframe().attached_tid() != 0 {
            return Err(vm.new_runtime_error("cannot clear an executing frame"));
        }
        let owner = FrameOwner::from_i8(
            self.iframe()
                .owner
                .load(core::sync::atomic::Ordering::Acquire),
        );
        match owner {
            FrameOwner::Generator => {
                // Generator frame: check if suspended (lasti > 0 means
                // FRAME_SUSPENDED). lasti == 0 means FRAME_CREATED and
                // can be cleared.
                if self.lasti() != 0 {
                    return Err(vm.new_runtime_error("cannot clear a suspended frame"));
                }
            }
            FrameOwner::Thread => {
                // Thread-owned frame: always executing, cannot clear.
                return Err(vm.new_runtime_error("cannot clear an executing frame"));
            }
            FrameOwner::FrameObject => {
                // Check if this materialized frame is backed by a live
                // stack-allocated iframe — if so, the frame is executing.
                if !self.find_live_source_iframe().is_null() {
                    return Err(vm.new_runtime_error("cannot clear an executing frame"));
                }
            }
        }

        // Move references out before dropping them. Their finalizers may
        // re-enter this frame, so no locals borrow or cold-data lock may be
        // held while they run.
        let fastlocals = {
            // SAFETY: FrameObject is not executing (detached or stopped).
            let slots = unsafe { self.fastlocals_mut() };
            slots
                .iter_mut()
                .filter_map(Option::take)
                .collect::<Vec<_>>()
        };

        // Clear the evaluation stack and cell references
        self.clear_stack_and_cells();

        let cold = self.iframe().cold();
        let temporary_refs = {
            let mut guard = cold.temporary_refs.lock();
            core::mem::take(&mut *guard)
        };
        let extra_locals = {
            let mut guard = cold.f_extra_locals.lock();
            guard.take()
        };
        let locals_cache = {
            let mut guard = cold.f_locals_cache.lock();
            guard.take()
        };
        let overwritten = {
            let mut guard = cold.f_overwritten_fast_locals.lock();
            core::mem::take(&mut *guard)
        };
        let retained_back = {
            let mut guard = cold.retained_back.lock();
            guard.take()
        };
        drop((
            fastlocals,
            temporary_refs,
            extra_locals,
            locals_cache,
            overwritten,
            retained_back,
        ));

        Ok(())
    }

    #[pygetset]
    fn f_locals(&self, vm: &VirtualMachine) -> PyResult {
        if self.uses_locals_proxy(vm)? {
            self.mark_escaped();
            let proxy = crate::builtins::FrameLocalsProxy::new(self.to_owned());
            Ok(proxy.into_ref(&vm.ctx).into())
        } else {
            Ok(self.iframe().locals.clone_mapping(vm).into())
        }
    }

    #[pygetset]
    fn f_generator(&self) -> Option<PyObjectRef> {
        self.iframe().generator.to_owned()
    }

    #[pygetset]
    pub fn f_back(&self, #[allow(unused)] vm: &VirtualMachine) -> Option<PyRef<FrameObject>> {
        let mut prev = self.previous_iframe();

        // For materialized frames (previous == 0), find the source iframe on
        // the TLS chain and use its `previous` instead.
        if prev.is_null() {
            // materialized stores `*const Py<FrameObject>` as usize.
            // `self` is `&Py<FrameObject>` — compare addresses directly.
            let self_py_ptr = self as *const Self as usize;
            let mut cur = crate::vm::thread::get_current_frame();
            while !cur.is_null() {
                let materialized = unsafe { (*cur).materialized.load(Relaxed) };
                if materialized == self_py_ptr {
                    // Found the source iframe — use its previous
                    prev = unsafe { (*cur).previous() };
                    break;
                }
                cur = unsafe { (*cur).previous() };
            }
            if prev.is_null() {
                // Check retained_back for frames whose callers have returned
                let retained = self.iframe().cold().retained_back.lock().clone();
                if let Some(frame) = retained {
                    frame.mark_escaped();
                    return Some(frame);
                }
                #[cfg(feature = "threading")]
                if let Some(tid) = self.source_thread() {
                    return self.back_from_thread(tid, vm);
                }
                return None;
            }
        }

        // Walk the TLS chain to find the prev iframe and materialize it.
        // This handles both heap-allocated FrameObjects and stack-allocated
        // iframes that haven't been observed yet.
        {
            let mut cur = crate::vm::thread::get_current_frame();
            while !cur.is_null() {
                if core::ptr::eq(cur, prev) {
                    let iframe_ref = unsafe { &*cur };
                    let fo = iframe_ref.materialize(vm);
                    fo.mark_escaped();
                    return Some(fo.to_owned());
                }
                cur = unsafe { (*cur).previous() };
            }
        }

        // The caller already returned — check retained_back
        let retained = self.iframe().cold().retained_back.lock().clone();
        if let Some(frame) = retained {
            frame.mark_escaped();
            return Some(frame);
        }

        // The caller lives on another thread. Use stop-the-world to
        // safely materialize the cross-thread frame chain.
        #[cfg(feature = "threading")]
        {
            // Enter STW before dereferencing `prev` — the owning thread may
            // return and free the stack-allocated iframe at any time.
            vm.state.stop_the_world.stop_the_world(&vm.state);
            scopeguard::defer! { vm.state.stop_the_world.start_the_world(&vm.state); }
            let prev_ref = unsafe { &*prev };
            // Fast path: already materialized.
            if let Some(fo) = prev_ref.frame_obj() {
                fo.mark_escaped();
                return Some(fo.to_owned());
            }
            // Slow path: copy the whole chain, linked through retained_back.
            // SAFETY: the world is stopped, so the owning thread is parked.
            let fo = unsafe { prev_ref.materialize_detached_chain(vm) };
            fo.mark_escaped();
            return Some(fo);
        }

        #[allow(unreachable_code)]
        None
    }
}

use crate::{
    AsObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    builtins::{PyCode, PyDictRef, PyNamespace, PyUtf8StrRef, code::CoMonitoringData},
    function::FuncArgs,
};
use core::sync::atomic::Ordering;
use crossbeam_utils::atomic::AtomicCell;
use std::collections::{HashMap, HashSet};

pub const TOOL_LIMIT: usize = 6;
const EVENTS_COUNT: usize = 19;
const LOCAL_EVENTS_COUNT: usize = 11;
const UNGROUPED_EVENTS_COUNT: usize = 18;

// Event bit positions
bitflags::bitflags! {
    #[derive(Copy, Clone, Debug, PartialEq, Eq)]
    pub struct MonitoringEvents: u32 {
        const PY_START           = 1 << 0;
        const PY_RESUME          = 1 << 1;
        const PY_RETURN          = 1 << 2;
        const PY_YIELD           = 1 << 3;
        const CALL               = 1 << 4;
        const LINE               = 1 << 5;
        const INSTRUCTION        = 1 << 6;
        const JUMP               = 1 << 7;
        const BRANCH_LEFT        = 1 << 8;
        const BRANCH_RIGHT       = 1 << 9;
        const STOP_ITERATION     = 1 << 10;
        const RAISE              = 1 << 11;
        const EXCEPTION_HANDLED  = 1 << 12;
        const PY_UNWIND          = 1 << 13;
        const PY_THROW           = 1 << 14;
        const RERAISE            = 1 << 15;
        const C_RETURN           = 1 << 16;
        const C_RAISE            = 1 << 17;
        const BRANCH             = 1 << 18;
    }
}

// Re-export as plain u32 constants for use in frame.rs hot-path checks
pub const EVENT_PY_START: u32 = MonitoringEvents::PY_START.bits();
pub const EVENT_PY_RESUME: u32 = MonitoringEvents::PY_RESUME.bits();
pub const EVENT_PY_RETURN: u32 = MonitoringEvents::PY_RETURN.bits();
pub const EVENT_PY_YIELD: u32 = MonitoringEvents::PY_YIELD.bits();
pub const EVENT_CALL: u32 = MonitoringEvents::CALL.bits();
pub const EVENT_LINE: u32 = MonitoringEvents::LINE.bits();
pub const EVENT_INSTRUCTION: u32 = MonitoringEvents::INSTRUCTION.bits();
pub const EVENT_JUMP: u32 = MonitoringEvents::JUMP.bits();
pub const EVENT_BRANCH_LEFT: u32 = MonitoringEvents::BRANCH_LEFT.bits();
pub const EVENT_BRANCH_RIGHT: u32 = MonitoringEvents::BRANCH_RIGHT.bits();
pub const EVENT_RAISE: u32 = MonitoringEvents::RAISE.bits();
pub const EVENT_EXCEPTION_HANDLED: u32 = MonitoringEvents::EXCEPTION_HANDLED.bits();
pub const EVENT_PY_UNWIND: u32 = MonitoringEvents::PY_UNWIND.bits();
pub const EVENT_C_RETURN: u32 = MonitoringEvents::C_RETURN.bits();
const EVENT_C_RAISE: u32 = MonitoringEvents::C_RAISE.bits();
pub const EVENT_STOP_ITERATION: u32 = MonitoringEvents::STOP_ITERATION.bits();
pub const EVENT_PY_THROW: u32 = MonitoringEvents::PY_THROW.bits();
const EVENT_BRANCH: u32 = MonitoringEvents::BRANCH.bits();
pub const EVENT_RERAISE: u32 = MonitoringEvents::RERAISE.bits();
const EVENT_C_RETURN_MASK: u32 = EVENT_C_RETURN | EVENT_C_RAISE;

const EVENT_NAMES: [&str; EVENTS_COUNT] = [
    "PY_START",
    "PY_RESUME",
    "PY_RETURN",
    "PY_YIELD",
    "CALL",
    "LINE",
    "INSTRUCTION",
    "JUMP",
    "BRANCH_LEFT",
    "BRANCH_RIGHT",
    "STOP_ITERATION",
    "RAISE",
    "EXCEPTION_HANDLED",
    "PY_UNWIND",
    "PY_THROW",
    "RERAISE",
    "C_RETURN",
    "C_RAISE",
    "BRANCH",
];

/// Interpreter-level monitoring state, shared by all threads.
pub struct MonitoringState {
    pub tool_names: [Option<String>; TOOL_LIMIT],
    pub global_events: [u32; TOOL_LIMIT],
    pub local_events: HashMap<(usize, usize), u32>,
    pub callbacks: HashMap<(usize, usize), PyObjectRef>,
    /// Per-instruction disabled tools: (code_id, offset, tool)
    pub disabled: HashSet<(usize, usize, usize)>,
    /// Cached MISSING sentinel singleton
    pub missing: Option<PyObjectRef>,
    /// Cached DISABLE sentinel singleton
    pub disable: Option<PyObjectRef>,
}

impl Default for MonitoringState {
    fn default() -> Self {
        Self {
            tool_names: Default::default(),
            global_events: [0; TOOL_LIMIT],
            local_events: HashMap::new(),
            callbacks: HashMap::new(),
            disabled: HashSet::new(),
            missing: None,
            disable: None,
        }
    }
}

impl MonitoringState {
    /// Compute the OR of all tools' global_events + local_events.
    /// This is used for the fast-path atomic mask to skip monitoring
    /// when no events are registered at all.
    pub fn combined_events(&self) -> u32 {
        let global = self.global_events.iter().fold(0, |acc, &e| acc | e);
        let local = self.local_events.values().fold(0, |acc, &e| acc | e);
        global | local
    }
}

/// Global atomic mask: OR of all tools' events. Checked in the hot path
/// to skip monitoring overhead when no events are registered.
/// Lives in PyGlobalState alongside the PyMutex<MonitoringState>.
pub type MonitoringEventsMask = AtomicCell<u32>;

/// Get the MISSING sentinel, creating it if necessary.
pub fn get_missing(vm: &VirtualMachine) -> PyObjectRef {
    let mut state = vm.state.monitoring.lock();
    if let Some(ref m) = state.missing {
        m.clone()
    } else {
        let m: PyObjectRef = sys_monitoring::MonitoringSentinel.into_ref(&vm.ctx).into();
        state.missing = Some(m.clone());
        m
    }
}

/// Get the DISABLE sentinel, creating it if necessary.
pub fn get_disable(vm: &VirtualMachine) -> PyObjectRef {
    let mut state = vm.state.monitoring.lock();
    if let Some(ref d) = state.disable {
        d.clone()
    } else {
        let d: PyObjectRef = sys_monitoring::MonitoringSentinel.into_ref(&vm.ctx).into();
        state.disable = Some(d.clone());
        d
    }
}

fn check_valid_tool(tool_id: i32, vm: &VirtualMachine) -> PyResult<usize> {
    if !(0..TOOL_LIMIT as i32).contains(&tool_id) {
        return Err(vm.new_value_error(format!("invalid tool {tool_id} (must be between 0 and 5)")));
    }
    Ok(tool_id as usize)
}

fn check_tool_in_use(tool: usize, vm: &VirtualMachine) -> PyResult<()> {
    let state = vm.state.monitoring.lock();
    if state.tool_names[tool].is_some() {
        Ok(())
    } else {
        Err(vm.new_value_error(format!("tool {tool} is not in use")))
    }
}

fn parse_single_event(event: i32, vm: &VirtualMachine) -> PyResult<usize> {
    let event = u32::try_from(event)
        .map_err(|_| vm.new_value_error("The callback can only be set for one event at a time"))?;
    if event.count_ones() != 1 {
        return Err(vm.new_value_error("The callback can only be set for one event at a time"));
    }
    let event_id = event.trailing_zeros() as usize;
    if event_id >= EVENTS_COUNT {
        return Err(vm.new_value_error(format!("invalid event {event}")));
    }
    Ok(event_id)
}

fn normalize_event_set(event_set: i32, local: bool, vm: &VirtualMachine) -> PyResult<u32> {
    let kind = if local {
        "local event set"
    } else {
        "event set"
    };
    if event_set < 0 {
        return Err(vm.new_value_error(format!("invalid {kind} 0x{event_set:x}")));
    }

    let mut event_set = event_set as u32;
    if event_set >= (1 << EVENTS_COUNT) {
        return Err(vm.new_value_error(format!("invalid {kind} 0x{event_set:x}")));
    }

    if (event_set & EVENT_C_RETURN_MASK) != 0 && (event_set & EVENT_CALL) != EVENT_CALL {
        return Err(vm.new_value_error("cannot set C_RETURN or C_RAISE events independently"));
    }

    event_set &= !EVENT_C_RETURN_MASK;

    if (event_set & EVENT_BRANCH) != 0 {
        event_set &= !EVENT_BRANCH;
        event_set |= EVENT_BRANCH_LEFT | EVENT_BRANCH_RIGHT;
    }

    if local && event_set >= (1 << LOCAL_EVENTS_COUNT) {
        return Err(vm.new_value_error(format!("invalid local event set 0x{event_set:x}")));
    }

    Ok(event_set)
}

/// Rewrite a code object's bytecode in-place with layered instrumentation.
///
/// Three layers (outermost first):
/// 1. INSTRUMENTED_LINE — wraps line-start instructions (stores original in side-table)
/// 2. INSTRUMENTED_INSTRUCTION — wraps all traceable instructions (stores original in side-table)
/// 3. Regular INSTRUMENTED_* — direct 1:1 opcode swap (no side-table needed)
///
/// De-instrumentation peels layers in reverse order.
pub fn instrument_code(code: &PyCode, events: u32) {
    use rustpython_compiler_core::bytecode::{self, Instruction};

    let len = code.code.instructions.len();
    let mut monitoring_data = code.monitoring_data.lock();

    // === Phase 1-3: De-instrument all layers (outermost first) ===

    // Phase 1: Remove INSTRUMENTED_LINE → restore from side-table
    if let Some(data) = monitoring_data.as_mut() {
        for i in 0..len {
            if data.line_opcodes[i] != 0 {
                let original = Instruction::try_from(data.line_opcodes[i])
                    .expect("invalid opcode in line side-table");
                unsafe {
                    code.code.instructions.replace_op(i, original);
                }
                data.line_opcodes[i] = 0;
            }
        }
    }

    // Phase 2: Remove INSTRUMENTED_INSTRUCTION → restore from side-table
    if let Some(data) = monitoring_data.as_mut() {
        for i in 0..len {
            if data.per_instruction_opcodes[i] != 0 {
                let original = Instruction::try_from(data.per_instruction_opcodes[i])
                    .expect("invalid opcode in instruction side-table");
                unsafe {
                    code.code.instructions.replace_op(i, original);
                }
                data.per_instruction_opcodes[i] = 0;
            }
        }
    }

    // Phase 3: Remove regular INSTRUMENTED_* → restore base opcodes
    for i in 0..len {
        let op = code.code.instructions[i].op;
        if let Some(base) = op.to_base() {
            unsafe {
                code.code.instructions.replace_op(i, base);
            }
        }
    }

    // All opcodes are now base opcodes.

    if events == 0 {
        *monitoring_data = None;
        return;
    }

    // === Phase 4-6: Re-instrument (innermost first) ===

    // Ensure monitoring data exists
    if monitoring_data.is_none() {
        *monitoring_data = Some(CoMonitoringData {
            line_opcodes: vec![0u8; len],
            per_instruction_opcodes: vec![0u8; len],
        });
    }
    let data = monitoring_data.as_mut().unwrap();
    // Resize if code length changed (shouldn't happen, but be safe)
    data.line_opcodes.resize(len, 0);
    data.per_instruction_opcodes.resize(len, 0);

    // Find _co_firsttraceable: index of first RESUME instruction
    let first_traceable = code
        .code
        .instructions
        .iter()
        .position(|u| matches!(u.op, Instruction::Resume { .. }))
        .unwrap_or(0);

    // Phase 4: Place regular INSTRUMENTED_* opcodes
    for i in 0..len {
        let op = code.code.instructions[i].op;
        if let Some(instrumented) = op.to_instrumented() {
            unsafe {
                code.code.instructions.replace_op(i, instrumented);
            }
        }
    }

    // Phase 5: Place INSTRUMENTED_INSTRUCTION (if EVENT_INSTRUCTION is active)
    if events & EVENT_INSTRUCTION != 0 {
        for i in first_traceable..len {
            let op = code.code.instructions[i].op;
            // Skip ExtendedArg
            if matches!(op, Instruction::ExtendedArg) {
                continue;
            }
            // Excluded: RESUME and END_FOR (and their instrumented variants)
            let base = op.to_base().map_or(op, |b| b);
            if matches!(base, Instruction::Resume { .. } | Instruction::EndFor) {
                continue;
            }
            // Store current opcode (may already be INSTRUMENTED_*) and replace
            data.per_instruction_opcodes[i] = u8::from(op);
            unsafe {
                code.code
                    .instructions
                    .replace_op(i, Instruction::InstrumentedInstruction);
            }
        }
    }

    // Phase 6: Place INSTRUMENTED_LINE (if EVENT_LINE is active)
    // Mirrors CPython's initialize_lines: first determine which positions
    // are line starts, then mark branch/jump targets, then place opcodes.
    if events & EVENT_LINE != 0 {
        // is_line_start[i] = true if position i should have INSTRUMENTED_LINE
        let mut is_line_start = vec![false; len];

        // First pass: mark positions where the source line changes
        let mut prev_line: Option<u32> = None;
        for (i, unit) in code
            .code
            .instructions
            .iter()
            .enumerate()
            .take(len)
            .skip(first_traceable)
        {
            let op = unit.op;
            let base = op.to_base().map_or(op, |b| b);
            if matches!(base, Instruction::ExtendedArg) {
                continue;
            }
            // Excluded opcodes
            if matches!(
                base,
                Instruction::Resume { .. }
                    | Instruction::EndFor
                    | Instruction::EndSend
                    | Instruction::PopIter
                    | Instruction::EndAsyncFor
            ) {
                continue;
            }
            if let Some((loc, _)) = code.code.locations.get(i) {
                let line = loc.line.get() as u32;
                let is_new = prev_line != Some(line);
                prev_line = Some(line);
                if is_new && line > 0 {
                    is_line_start[i] = true;
                }
            }
        }

        // Second pass: mark branch/jump targets as line starts.
        // Every jump/branch target must be a line start, even if on the
        // same source line as the preceding instruction. Critical for loops
        // (JUMP_BACKWARD → FOR_ITER).
        let mut arg_state = bytecode::OpArgState::default();
        for unit in code.code.instructions[first_traceable..len].iter().copied() {
            let (op, arg) = arg_state.get(unit);
            let base = op.to_base().map_or(op, |b| b);

            if matches!(base, Instruction::ExtendedArg) {
                continue;
            }

            let target: Option<usize> = match base {
                Instruction::PopJumpIfFalse { .. }
                | Instruction::PopJumpIfTrue { .. }
                | Instruction::PopJumpIfNone { .. }
                | Instruction::PopJumpIfNotNone { .. }
                | Instruction::JumpForward { .. }
                | Instruction::JumpBackward { .. }
                | Instruction::JumpBackwardNoInterrupt { .. } => Some(u32::from(arg) as usize),
                Instruction::ForIter { .. } | Instruction::Send { .. } => {
                    // Skip over END_FOR/END_SEND
                    Some(u32::from(arg) as usize + 1)
                }
                _ => None,
            };

            if let Some(target_idx) = target
                && target_idx < len
                && !is_line_start[target_idx]
            {
                let target_op = code.code.instructions[target_idx].op;
                let target_base = target_op.to_base().map_or(target_op, |b| b);
                // Skip POP_ITER targets
                if matches!(target_base, Instruction::PopIter) {
                    continue;
                }
                if let Some((loc, _)) = code.code.locations.get(target_idx)
                    && loc.line.get() > 0
                {
                    is_line_start[target_idx] = true;
                }
            }
        }

        // Third pass: mark exception handler targets as line starts.
        for entry in bytecode::decode_exception_table(&code.code.exceptiontable) {
            let target_idx = entry.target as usize;
            if target_idx < len && !is_line_start[target_idx] {
                let target_op = code.code.instructions[target_idx].op;
                let target_base = target_op.to_base().map_or(target_op, |b| b);
                if !matches!(target_base, Instruction::PopIter)
                    && let Some((loc, _)) = code.code.locations.get(target_idx)
                    && loc.line.get() > 0
                {
                    is_line_start[target_idx] = true;
                }
            }
        }

        // Fourth pass: actually place INSTRUMENTED_LINE at all marked positions
        for (i, marked) in is_line_start
            .iter()
            .copied()
            .enumerate()
            .take(len)
            .skip(first_traceable)
        {
            if marked {
                let op = code.code.instructions[i].op;
                data.line_opcodes[i] = u8::from(op);
                unsafe {
                    code.code
                        .instructions
                        .replace_op(i, Instruction::InstrumentedLine);
                }
            }
        }
    }
}

/// Update the global monitoring_events atomic mask from current state.
fn update_events_mask(vm: &VirtualMachine, state: &MonitoringState) {
    let events = state.combined_events();
    vm.state.monitoring_events.store(events);
    let new_ver = vm
        .state
        .instrumentation_version
        .fetch_add(1, Ordering::Release)
        + 1;
    // Eagerly re-instrument all frames on the current thread's stack so that
    // code objects already past their RESUME pick up the new event set.
    for fp in vm.frames.borrow().iter() {
        // SAFETY: frames in the Vec are alive while their FrameRef is on the call stack.
        let frame = unsafe { fp.as_ref() };
        let code = &frame.code;
        let code_ver = code.instrumentation_version.load(Ordering::Acquire);
        if code_ver != new_ver {
            instrument_code(code, events);
            code.instrumentation_version
                .store(new_ver, Ordering::Release);
        }
    }
}

fn use_tool_id(tool_id: i32, name: &str, vm: &VirtualMachine) -> PyResult<()> {
    let tool = check_valid_tool(tool_id, vm)?;
    let mut state = vm.state.monitoring.lock();
    if state.tool_names[tool].is_some() {
        return Err(vm.new_value_error(format!("tool {tool_id} is already in use")));
    }
    state.tool_names[tool] = Some(name.to_owned());
    Ok(())
}

fn clear_tool_id(tool_id: i32, vm: &VirtualMachine) -> PyResult<()> {
    let tool = check_valid_tool(tool_id, vm)?;
    let mut state = vm.state.monitoring.lock();
    if state.tool_names[tool].is_some() {
        state.global_events[tool] = 0;
        state
            .local_events
            .retain(|(local_tool, _), _| *local_tool != tool);
        state.callbacks.retain(|(cb_tool, _), _| *cb_tool != tool);
        state.disabled.retain(|&(_, _, t)| t != tool);
    }
    update_events_mask(vm, &state);
    Ok(())
}

fn free_tool_id(tool_id: i32, vm: &VirtualMachine) -> PyResult<()> {
    let tool = check_valid_tool(tool_id, vm)?;
    let mut state = vm.state.monitoring.lock();
    if state.tool_names[tool].is_some() {
        state.global_events[tool] = 0;
        state
            .local_events
            .retain(|(local_tool, _), _| *local_tool != tool);
        state.callbacks.retain(|(cb_tool, _), _| *cb_tool != tool);
        state.disabled.retain(|&(_, _, t)| t != tool);
        state.tool_names[tool] = None;
    }
    update_events_mask(vm, &state);
    Ok(())
}

fn get_tool(tool_id: i32, vm: &VirtualMachine) -> PyResult<Option<String>> {
    let tool = check_valid_tool(tool_id, vm)?;
    let state = vm.state.monitoring.lock();
    Ok(state.tool_names[tool].clone())
}

fn register_callback(
    tool_id: i32,
    event: i32,
    func: PyObjectRef,
    vm: &VirtualMachine,
) -> PyResult<PyObjectRef> {
    let tool = check_valid_tool(tool_id, vm)?;
    let event_id = parse_single_event(event, vm)?;

    let mut state = vm.state.monitoring.lock();
    let prev = state
        .callbacks
        .remove(&(tool, event_id))
        .unwrap_or_else(|| vm.ctx.none());
    let branch_id = EVENT_BRANCH.trailing_zeros() as usize;
    let branch_left_id = EVENT_BRANCH_LEFT.trailing_zeros() as usize;
    let branch_right_id = EVENT_BRANCH_RIGHT.trailing_zeros() as usize;
    if !vm.is_none(&func) {
        state.callbacks.insert((tool, event_id), func.clone());
        // BRANCH is a composite event: also register for BRANCH_LEFT/RIGHT
        if event_id == branch_id {
            state.callbacks.insert((tool, branch_left_id), func.clone());
            state.callbacks.insert((tool, branch_right_id), func);
        }
    } else {
        // Also clear BRANCH_LEFT/RIGHT when clearing BRANCH
        if event_id == branch_id {
            state.callbacks.remove(&(tool, branch_left_id));
            state.callbacks.remove(&(tool, branch_right_id));
        }
    }
    Ok(prev)
}

fn get_events(tool_id: i32, vm: &VirtualMachine) -> PyResult<u32> {
    let tool = check_valid_tool(tool_id, vm)?;
    let state = vm.state.monitoring.lock();
    Ok(state.global_events[tool])
}

fn set_events(tool_id: i32, event_set: i32, vm: &VirtualMachine) -> PyResult<()> {
    let tool = check_valid_tool(tool_id, vm)?;
    check_tool_in_use(tool, vm)?;
    let normalized = normalize_event_set(event_set, false, vm)?;
    let mut state = vm.state.monitoring.lock();
    state.global_events[tool] = normalized;
    update_events_mask(vm, &state);
    Ok(())
}

fn get_local_events(tool_id: i32, code: PyObjectRef, vm: &VirtualMachine) -> PyResult<u32> {
    if code.downcast_ref::<PyCode>().is_none() {
        return Err(vm.new_type_error("code must be a code object"));
    }
    let tool = check_valid_tool(tool_id, vm)?;
    let code_id = code.get_id();
    let state = vm.state.monitoring.lock();
    Ok(state
        .local_events
        .get(&(tool, code_id))
        .copied()
        .unwrap_or(0))
}

fn set_local_events(
    tool_id: i32,
    code: PyObjectRef,
    event_set: i32,
    vm: &VirtualMachine,
) -> PyResult<()> {
    if code.downcast_ref::<PyCode>().is_none() {
        return Err(vm.new_type_error("code must be a code object"));
    }
    let tool = check_valid_tool(tool_id, vm)?;
    check_tool_in_use(tool, vm)?;
    let normalized = normalize_event_set(event_set, true, vm)?;
    let code_id = code.get_id();
    let mut state = vm.state.monitoring.lock();
    if normalized == 0 {
        state.local_events.remove(&(tool, code_id));
    } else {
        state.local_events.insert((tool, code_id), normalized);
    }
    update_events_mask(vm, &state);
    Ok(())
}

fn restart_events(vm: &VirtualMachine) {
    let mut state = vm.state.monitoring.lock();
    state.disabled.clear();
}

fn all_events(vm: &VirtualMachine) -> PyResult<PyDictRef> {
    // Collect data under the lock, then release before calling into Python VM.
    let masks: Vec<(&str, u8)> = {
        let state = vm.state.monitoring.lock();
        EVENT_NAMES
            .iter()
            .take(UNGROUPED_EVENTS_COUNT)
            .enumerate()
            .filter_map(|(event_id, event_name)| {
                let event_bit = 1u32 << event_id;
                let mut tools_mask = 0u8;
                for tool in 0..TOOL_LIMIT {
                    if (state.global_events[tool] & event_bit) != 0 {
                        tools_mask |= 1 << tool;
                    }
                }
                if tools_mask != 0 {
                    Some((*event_name, tools_mask))
                } else {
                    None
                }
            })
            .collect()
    };
    let all_events = vm.ctx.new_dict();
    for (name, mask) in masks {
        all_events.set_item(name, vm.ctx.new_int(mask).into(), vm)?;
    }
    Ok(all_events)
}

// Event dispatch

use core::cell::Cell;

thread_local! {
    /// Re-entrancy guard: prevents monitoring callbacks from triggering
    /// additional monitoring events (which would cause infinite recursion).
    static FIRING: Cell<bool> = const { Cell::new(false) };

    /// Tracks whether a RERAISE event has been fired since the last
    /// EXCEPTION_HANDLED. Used to suppress duplicate RERAISE from
    /// cleanup handlers that chain through multiple exception table entries.
    static RERAISE_PENDING: Cell<bool> = const { Cell::new(false) };
}

/// Fire an event for all tools that have the event bit set.
/// `cb_extra` contains the callback arguments after the code object.
fn fire(
    vm: &VirtualMachine,
    event: u32,
    code: &PyRef<PyCode>,
    offset: u32,
    cb_extra: &[PyObjectRef],
) -> PyResult<()> {
    // Prevent recursive event firing
    if FIRING.with(|f| f.get()) {
        return Ok(());
    }

    let event_id = event.trailing_zeros() as usize;
    let code_id = code.get_id();

    // C_RETURN and C_RAISE are implicitly enabled when CALL is set.
    let check_bit = if event & EVENT_C_RETURN_MASK != 0 {
        event | EVENT_CALL
    } else {
        event
    };

    // Collect callbacks and snapshot the DISABLE sentinel under a single lock.
    let (callbacks, disable_sentinel): (Vec<(usize, PyObjectRef)>, Option<PyObjectRef>) = {
        let state = vm.state.monitoring.lock();
        let mut cbs = Vec::new();
        for tool in 0..TOOL_LIMIT {
            let global = state.global_events[tool];
            let local = state
                .local_events
                .get(&(tool, code_id))
                .copied()
                .unwrap_or(0);
            if ((global | local) & check_bit) == 0 {
                continue;
            }
            if state.disabled.contains(&(code_id, offset as usize, tool)) {
                continue;
            }
            if let Some(cb) = state.callbacks.get(&(tool, event_id)) {
                cbs.push((tool, cb.clone()));
            }
        }
        (cbs, state.disable.clone())
    };

    if callbacks.is_empty() {
        return Ok(());
    }

    let mut args_vec = Vec::with_capacity(1 + cb_extra.len());
    args_vec.push(code.clone().into());
    args_vec.extend_from_slice(cb_extra);
    let args = FuncArgs::from(args_vec);

    FIRING.with(|f| f.set(true));
    let result = (|| {
        for (tool, cb) in callbacks {
            let result = cb.call(args.clone(), vm)?;
            if disable_sentinel.as_ref().is_some_and(|d| result.is(d)) {
                // Only local events (event_id < LOCAL_EVENTS_COUNT) can be disabled.
                // Non-local events (RAISE, EXCEPTION_HANDLED, PY_UNWIND, etc.)
                // cannot be disabled per code object.
                if event_id >= LOCAL_EVENTS_COUNT {
                    // Remove the callback, matching CPython behavior.
                    let mut state = vm.state.monitoring.lock();
                    state.callbacks.remove(&(tool, event_id));
                    return Err(vm.new_value_error(format!(
                        "Cannot disable {} events. Callback removed.",
                        EVENT_NAMES[event_id]
                    )));
                }
                let mut state = vm.state.monitoring.lock();
                state.disabled.insert((code_id, offset as usize, tool));
            }
        }
        Ok(())
    })();
    FIRING.with(|f| f.set(false));
    result
}

// Public dispatch functions (called from frame.rs)

pub fn fire_py_start(vm: &VirtualMachine, code: &PyRef<PyCode>, offset: u32) -> PyResult<()> {
    fire(
        vm,
        EVENT_PY_START,
        code,
        offset,
        &[vm.ctx.new_int(offset).into()],
    )
}

pub fn fire_py_resume(vm: &VirtualMachine, code: &PyRef<PyCode>, offset: u32) -> PyResult<()> {
    fire(
        vm,
        EVENT_PY_RESUME,
        code,
        offset,
        &[vm.ctx.new_int(offset).into()],
    )
}

pub fn fire_py_return(
    vm: &VirtualMachine,
    code: &PyRef<PyCode>,
    offset: u32,
    retval: &PyObjectRef,
) -> PyResult<()> {
    fire(
        vm,
        EVENT_PY_RETURN,
        code,
        offset,
        &[vm.ctx.new_int(offset).into(), retval.clone()],
    )
}

pub fn fire_py_yield(
    vm: &VirtualMachine,
    code: &PyRef<PyCode>,
    offset: u32,
    retval: &PyObjectRef,
) -> PyResult<()> {
    fire(
        vm,
        EVENT_PY_YIELD,
        code,
        offset,
        &[vm.ctx.new_int(offset).into(), retval.clone()],
    )
}

pub fn fire_call(
    vm: &VirtualMachine,
    code: &PyRef<PyCode>,
    offset: u32,
    callable: &PyObjectRef,
    arg0: PyObjectRef,
) -> PyResult<()> {
    fire(
        vm,
        EVENT_CALL,
        code,
        offset,
        &[vm.ctx.new_int(offset).into(), callable.clone(), arg0],
    )
}

pub fn fire_c_return(
    vm: &VirtualMachine,
    code: &PyRef<PyCode>,
    offset: u32,
    callable: &PyObjectRef,
    arg0: PyObjectRef,
) -> PyResult<()> {
    fire(
        vm,
        EVENT_C_RETURN,
        code,
        offset,
        &[vm.ctx.new_int(offset).into(), callable.clone(), arg0],
    )
}

pub fn fire_c_raise(
    vm: &VirtualMachine,
    code: &PyRef<PyCode>,
    offset: u32,
    callable: &PyObjectRef,
    arg0: PyObjectRef,
) -> PyResult<()> {
    fire(
        vm,
        EVENT_C_RAISE,
        code,
        offset,
        &[vm.ctx.new_int(offset).into(), callable.clone(), arg0],
    )
}

pub fn fire_line(
    vm: &VirtualMachine,
    code: &PyRef<PyCode>,
    offset: u32,
    line: u32,
) -> PyResult<()> {
    fire(vm, EVENT_LINE, code, offset, &[vm.ctx.new_int(line).into()])
}

pub fn fire_instruction(vm: &VirtualMachine, code: &PyRef<PyCode>, offset: u32) -> PyResult<()> {
    fire(
        vm,
        EVENT_INSTRUCTION,
        code,
        offset,
        &[vm.ctx.new_int(offset).into()],
    )
}

pub fn fire_raise(
    vm: &VirtualMachine,
    code: &PyRef<PyCode>,
    offset: u32,
    exception: &PyObjectRef,
) -> PyResult<()> {
    fire(
        vm,
        EVENT_RAISE,
        code,
        offset,
        &[vm.ctx.new_int(offset).into(), exception.clone()],
    )
}

/// Only fires if no RERAISE has been fired since the last EXCEPTION_HANDLED,
/// preventing duplicate events from chained cleanup handlers.
pub fn fire_reraise(
    vm: &VirtualMachine,
    code: &PyRef<PyCode>,
    offset: u32,
    exception: &PyObjectRef,
) -> PyResult<()> {
    if RERAISE_PENDING.with(|f| f.get()) {
        return Ok(());
    }
    RERAISE_PENDING.with(|f| f.set(true));
    let result = fire(
        vm,
        EVENT_RERAISE,
        code,
        offset,
        &[vm.ctx.new_int(offset).into(), exception.clone()],
    );
    if result.is_err() {
        RERAISE_PENDING.with(|f| f.set(false));
    }
    result
}

pub fn fire_exception_handled(
    vm: &VirtualMachine,
    code: &PyRef<PyCode>,
    offset: u32,
    exception: &PyObjectRef,
) -> PyResult<()> {
    RERAISE_PENDING.with(|f| f.set(false));
    fire(
        vm,
        EVENT_EXCEPTION_HANDLED,
        code,
        offset,
        &[vm.ctx.new_int(offset).into(), exception.clone()],
    )
}

pub fn fire_py_unwind(
    vm: &VirtualMachine,
    code: &PyRef<PyCode>,
    offset: u32,
    exception: &PyObjectRef,
) -> PyResult<()> {
    RERAISE_PENDING.with(|f| f.set(false));
    fire(
        vm,
        EVENT_PY_UNWIND,
        code,
        offset,
        &[vm.ctx.new_int(offset).into(), exception.clone()],
    )
}

pub fn fire_py_throw(
    vm: &VirtualMachine,
    code: &PyRef<PyCode>,
    offset: u32,
    exception: &PyObjectRef,
) -> PyResult<()> {
    fire(
        vm,
        EVENT_PY_THROW,
        code,
        offset,
        &[vm.ctx.new_int(offset).into(), exception.clone()],
    )
}

pub fn fire_stop_iteration(
    vm: &VirtualMachine,
    code: &PyRef<PyCode>,
    offset: u32,
    exception: &PyObjectRef,
) -> PyResult<()> {
    fire(
        vm,
        EVENT_STOP_ITERATION,
        code,
        offset,
        &[vm.ctx.new_int(offset).into(), exception.clone()],
    )
}

pub fn fire_jump(
    vm: &VirtualMachine,
    code: &PyRef<PyCode>,
    offset: u32,
    destination: u32,
) -> PyResult<()> {
    fire(
        vm,
        EVENT_JUMP,
        code,
        offset,
        &[
            vm.ctx.new_int(offset).into(),
            vm.ctx.new_int(destination).into(),
        ],
    )
}

pub fn fire_branch_left(
    vm: &VirtualMachine,
    code: &PyRef<PyCode>,
    offset: u32,
    destination: u32,
) -> PyResult<()> {
    fire(
        vm,
        EVENT_BRANCH_LEFT,
        code,
        offset,
        &[
            vm.ctx.new_int(offset).into(),
            vm.ctx.new_int(destination).into(),
        ],
    )
}

pub fn fire_branch_right(
    vm: &VirtualMachine,
    code: &PyRef<PyCode>,
    offset: u32,
    destination: u32,
) -> PyResult<()> {
    fire(
        vm,
        EVENT_BRANCH_RIGHT,
        code,
        offset,
        &[
            vm.ctx.new_int(offset).into(),
            vm.ctx.new_int(destination).into(),
        ],
    )
}

#[pymodule(sub)]
pub(super) mod sys_monitoring {
    use super::*;

    #[pyclass(no_attr, module = "sys.monitoring", name = "_Sentinel")]
    #[derive(Debug, PyPayload)]
    pub(super) struct MonitoringSentinel;

    #[pyclass]
    impl MonitoringSentinel {}

    #[pyattr(name = "DEBUGGER_ID")]
    const DEBUGGER_ID: u8 = 0;
    #[pyattr(name = "COVERAGE_ID")]
    const COVERAGE_ID: u8 = 1;
    #[pyattr(name = "PROFILER_ID")]
    const PROFILER_ID: u8 = 2;
    #[pyattr(name = "OPTIMIZER_ID")]
    const OPTIMIZER_ID: u8 = 5;

    #[pyattr(once, name = "DISABLE")]
    fn disable(vm: &VirtualMachine) -> PyObjectRef {
        super::get_disable(vm)
    }

    #[pyattr(once, name = "MISSING")]
    fn missing(vm: &VirtualMachine) -> PyObjectRef {
        super::get_missing(vm)
    }

    #[pyattr(once)]
    fn events(vm: &VirtualMachine) -> PyRef<PyNamespace> {
        let events = PyNamespace::default().into_ref(&vm.ctx);
        for (event_id, event_name) in EVENT_NAMES.iter().enumerate() {
            events
                .as_object()
                .set_attr(*event_name, vm.ctx.new_int(1u32 << event_id), vm)
                .expect("setting sys.monitoring.events attribute should not fail");
        }
        events
            .as_object()
            .set_attr("NO_EVENTS", vm.ctx.new_int(0), vm)
            .expect("setting sys.monitoring.events.NO_EVENTS should not fail");
        events
    }

    #[pyfunction]
    fn use_tool_id(tool_id: i32, name: PyUtf8StrRef, vm: &VirtualMachine) -> PyResult<()> {
        super::use_tool_id(tool_id, name.as_str(), vm)
    }

    #[pyfunction]
    fn clear_tool_id(tool_id: i32, vm: &VirtualMachine) -> PyResult<()> {
        super::clear_tool_id(tool_id, vm)
    }

    #[pyfunction]
    fn free_tool_id(tool_id: i32, vm: &VirtualMachine) -> PyResult<()> {
        super::free_tool_id(tool_id, vm)
    }

    #[pyfunction]
    fn get_tool(tool_id: i32, vm: &VirtualMachine) -> PyResult<Option<String>> {
        super::get_tool(tool_id, vm)
    }

    #[pyfunction]
    fn register_callback(
        tool_id: i32,
        event: i32,
        func: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        super::register_callback(tool_id, event, func, vm)
    }

    #[pyfunction]
    fn get_events(tool_id: i32, vm: &VirtualMachine) -> PyResult<u32> {
        super::get_events(tool_id, vm)
    }

    #[pyfunction]
    fn set_events(tool_id: i32, event_set: i32, vm: &VirtualMachine) -> PyResult<()> {
        super::set_events(tool_id, event_set, vm)
    }

    #[pyfunction]
    fn get_local_events(tool_id: i32, code: PyObjectRef, vm: &VirtualMachine) -> PyResult<u32> {
        super::get_local_events(tool_id, code, vm)
    }

    #[pyfunction]
    fn set_local_events(
        tool_id: i32,
        code: PyObjectRef,
        event_set: i32,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        super::set_local_events(tool_id, code, event_set, vm)
    }

    #[pyfunction]
    fn restart_events(vm: &VirtualMachine) {
        super::restart_events(vm)
    }

    #[pyfunction]
    fn _all_events(vm: &VirtualMachine) -> PyResult<PyDictRef> {
        super::all_events(vm)
    }
}

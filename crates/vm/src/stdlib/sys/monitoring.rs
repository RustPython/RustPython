use crate::{
    AsObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    builtins::{PyCode, PyDictRef, PyNamespace, PyStrRef},
    function::FuncArgs,
};
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
const EVENT_STOP_ITERATION: u32 = MonitoringEvents::STOP_ITERATION.bits();
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
    if event_set < 0 {
        let kind = if local {
            "local event set"
        } else {
            "event set"
        };
        return Err(vm.new_value_error(format!("invalid {kind} 0x{event_set:x}")));
    }

    let mut event_set = event_set as u32;
    if event_set >= (1 << EVENTS_COUNT) {
        let kind = if local {
            "local event set"
        } else {
            "event set"
        };
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

/// Update the global monitoring_events atomic mask from current state.
fn update_events_mask(vm: &VirtualMachine, state: &MonitoringState) {
    vm.state.monitoring_events.store(state.combined_events());
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
    clear_tool_id(tool_id, vm)?;
    let mut state = vm.state.monitoring.lock();
    state.tool_names[tool] = None;
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
    if !vm.is_none(&func) {
        state.callbacks.insert((tool, event_id), func.clone());
        // BRANCH is a composite event: also register for BRANCH_LEFT/RIGHT
        if event_id == 18 {
            // BRANCH → BRANCH_LEFT (8) + BRANCH_RIGHT (9)
            state.callbacks.insert((tool, 8), func.clone());
            state.callbacks.insert((tool, 9), func);
        }
    } else {
        // Also clear BRANCH_LEFT/RIGHT when clearing BRANCH
        if event_id == 18 {
            state.callbacks.remove(&(tool, 8));
            state.callbacks.remove(&(tool, 9));
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
    let all_events = vm.ctx.new_dict();
    let state = vm.state.monitoring.lock();
    for (event_id, event_name) in EVENT_NAMES.iter().take(UNGROUPED_EVENTS_COUNT).enumerate() {
        let event_bit = 1u32 << event_id;
        let mut tools_mask = 0u8;
        for tool in 0..TOOL_LIMIT {
            if (state.global_events[tool] & event_bit) != 0 {
                tools_mask |= 1 << tool;
            }
        }
        if tools_mask != 0 {
            all_events.set_item(*event_name, vm.ctx.new_int(tools_mask).into(), vm)?;
        }
    }
    Ok(all_events)
}

// ── Event dispatch ──────────────────────────────────────────────────────

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

/// Check if DISABLE sentinel was returned by a callback.
fn is_disable(obj: &PyObjectRef, vm: &VirtualMachine) -> bool {
    // DISABLE is the _Sentinel singleton stored on the monitoring module.
    // We check its type name to avoid needing a reference to the exact object.
    let name = obj.class().name();
    let name_str: &str = &name;
    name_str == "_Sentinel" && !vm.is_none(obj)
}

/// Fire an event for all tools that have the event bit set.
fn fire_event_inner(
    vm: &VirtualMachine,
    event_id: usize,
    event_bit: u32,
    code_id: usize,
    offset: u32,
    args: &FuncArgs,
) -> PyResult<()> {
    // Prevent recursive event firing
    if FIRING.with(|f| f.get()) {
        return Ok(());
    }

    // C_RETURN and C_RAISE are implicitly enabled when CALL is set.
    // Expand the check bit to include CALL for these events.
    let check_bit = if event_bit & EVENT_C_RETURN_MASK != 0 {
        event_bit | EVENT_CALL
    } else {
        event_bit
    };

    // Collect callbacks first, then release the lock before calling them.
    let callbacks: Vec<(usize, PyObjectRef)> = {
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
        cbs
    };

    if callbacks.is_empty() {
        return Ok(());
    }

    FIRING.with(|f| f.set(true));
    let result = (|| {
        for (tool, cb) in callbacks {
            let result = cb.call(args.clone(), vm)?;
            if is_disable(&result, vm) {
                // Only local events (event_id < LOCAL_EVENTS_COUNT) can be disabled.
                // Non-local events (RAISE, EXCEPTION_HANDLED, PY_UNWIND, etc.)
                // cannot be disabled per code object.
                if event_id >= LOCAL_EVENTS_COUNT {
                    return Err(vm.new_value_error(format!(
                        "cannot disable {} events",
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

// Public dispatch functions called from frame.rs

/// PY_START: fired at function entry (Resume with arg=0)
pub fn fire_py_start(vm: &VirtualMachine, code: &PyRef<PyCode>, offset: u32) -> PyResult<()> {
    let code_id = code.get_id();
    let args = FuncArgs::from(vec![code.clone().into(), vm.ctx.new_int(offset).into()]);
    fire_event_inner(vm, 0, EVENT_PY_START, code_id, offset, &args)
}

/// PY_RESUME: fired when generator/coroutine resumes (Resume with arg>0)
pub fn fire_py_resume(vm: &VirtualMachine, code: &PyRef<PyCode>, offset: u32) -> PyResult<()> {
    let code_id = code.get_id();
    let args = FuncArgs::from(vec![code.clone().into(), vm.ctx.new_int(offset).into()]);
    fire_event_inner(vm, 1, EVENT_PY_RESUME, code_id, offset, &args)
}

/// PY_RETURN: fired when a function returns
pub fn fire_py_return(
    vm: &VirtualMachine,
    code: &PyRef<PyCode>,
    offset: u32,
    retval: &PyObjectRef,
) -> PyResult<()> {
    let code_id = code.get_id();
    let args = FuncArgs::from(vec![
        code.clone().into(),
        vm.ctx.new_int(offset).into(),
        retval.clone(),
    ]);
    fire_event_inner(vm, 2, EVENT_PY_RETURN, code_id, offset, &args)
}

/// PY_YIELD: fired when a generator yields
pub fn fire_py_yield(
    vm: &VirtualMachine,
    code: &PyRef<PyCode>,
    offset: u32,
    retval: &PyObjectRef,
) -> PyResult<()> {
    let code_id = code.get_id();
    let args = FuncArgs::from(vec![
        code.clone().into(),
        vm.ctx.new_int(offset).into(),
        retval.clone(),
    ]);
    fire_event_inner(vm, 3, EVENT_PY_YIELD, code_id, offset, &args)
}

/// CALL: fired when a function/method is called
pub fn fire_call(
    vm: &VirtualMachine,
    code: &PyRef<PyCode>,
    offset: u32,
    callable: &PyObjectRef,
    arg0: PyObjectRef,
) -> PyResult<()> {
    let code_id = code.get_id();
    let args = FuncArgs::from(vec![
        code.clone().into(),
        vm.ctx.new_int(offset).into(),
        callable.clone(),
        arg0,
    ]);
    fire_event_inner(vm, 4, EVENT_CALL, code_id, offset, &args)
}

/// C_RETURN: fired when a C function returns
pub fn fire_c_return(
    vm: &VirtualMachine,
    code: &PyRef<PyCode>,
    offset: u32,
    callable: &PyObjectRef,
    arg0: PyObjectRef,
) -> PyResult<()> {
    let code_id = code.get_id();
    let args = FuncArgs::from(vec![
        code.clone().into(),
        vm.ctx.new_int(offset).into(),
        callable.clone(),
        arg0,
    ]);
    fire_event_inner(vm, 16, EVENT_C_RETURN, code_id, offset, &args)
}

/// C_RAISE: fired when a C function raises
pub fn fire_c_raise(
    vm: &VirtualMachine,
    code: &PyRef<PyCode>,
    offset: u32,
    callable: &PyObjectRef,
    arg0: PyObjectRef,
) -> PyResult<()> {
    let code_id = code.get_id();
    let args = FuncArgs::from(vec![
        code.clone().into(),
        vm.ctx.new_int(offset).into(),
        callable.clone(),
        arg0,
    ]);
    fire_event_inner(vm, 17, EVENT_C_RAISE, code_id, offset, &args)
}

/// LINE: fired when execution reaches a new line
pub fn fire_line(
    vm: &VirtualMachine,
    code: &PyRef<PyCode>,
    offset: u32,
    line: u32,
) -> PyResult<()> {
    let code_id = code.get_id();
    let args = FuncArgs::from(vec![code.clone().into(), vm.ctx.new_int(line).into()]);
    fire_event_inner(vm, 5, EVENT_LINE, code_id, offset, &args)
}

/// INSTRUCTION: fired before each instruction
pub fn fire_instruction(vm: &VirtualMachine, code: &PyRef<PyCode>, offset: u32) -> PyResult<()> {
    let code_id = code.get_id();
    let args = FuncArgs::from(vec![code.clone().into(), vm.ctx.new_int(offset).into()]);
    fire_event_inner(vm, 6, EVENT_INSTRUCTION, code_id, offset, &args)
}

/// RAISE: fired when an exception is raised
pub fn fire_raise(
    vm: &VirtualMachine,
    code: &PyRef<PyCode>,
    offset: u32,
    exception: &PyObjectRef,
) -> PyResult<()> {
    let code_id = code.get_id();
    let args = FuncArgs::from(vec![
        code.clone().into(),
        vm.ctx.new_int(offset).into(),
        exception.clone(),
    ]);
    fire_event_inner(vm, 11, EVENT_RAISE, code_id, offset, &args)
}

/// RERAISE: fired when an exception is re-raised.
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
    let code_id = code.get_id();
    let args = FuncArgs::from(vec![
        code.clone().into(),
        vm.ctx.new_int(offset).into(),
        exception.clone(),
    ]);
    fire_event_inner(vm, 15, EVENT_RERAISE, code_id, offset, &args)
}

/// EXCEPTION_HANDLED: fired when entering an exception handler
pub fn fire_exception_handled(
    vm: &VirtualMachine,
    code: &PyRef<PyCode>,
    offset: u32,
    exception: &PyObjectRef,
) -> PyResult<()> {
    RERAISE_PENDING.with(|f| f.set(false));
    let code_id = code.get_id();
    let args = FuncArgs::from(vec![
        code.clone().into(),
        vm.ctx.new_int(offset).into(),
        exception.clone(),
    ]);
    fire_event_inner(vm, 12, EVENT_EXCEPTION_HANDLED, code_id, offset, &args)
}

/// PY_UNWIND: fired when exception propagates out of a frame
pub fn fire_py_unwind(
    vm: &VirtualMachine,
    code: &PyRef<PyCode>,
    offset: u32,
    exception: &PyObjectRef,
) -> PyResult<()> {
    RERAISE_PENDING.with(|f| f.set(false));
    let code_id = code.get_id();
    let args = FuncArgs::from(vec![
        code.clone().into(),
        vm.ctx.new_int(offset).into(),
        exception.clone(),
    ]);
    fire_event_inner(vm, 13, EVENT_PY_UNWIND, code_id, offset, &args)
}

/// PY_THROW: fired when throw() is called on a generator/coroutine
pub fn fire_py_throw(
    vm: &VirtualMachine,
    code: &PyRef<PyCode>,
    offset: u32,
    exception: &PyObjectRef,
) -> PyResult<()> {
    let code_id = code.get_id();
    let args = FuncArgs::from(vec![
        code.clone().into(),
        vm.ctx.new_int(offset).into(),
        exception.clone(),
    ]);
    fire_event_inner(vm, 14, EVENT_PY_THROW, code_id, offset, &args)
}

/// STOP_ITERATION: fired when StopIteration is raised implicitly
#[allow(dead_code)]
pub fn fire_stop_iteration(
    vm: &VirtualMachine,
    code: &PyRef<PyCode>,
    offset: u32,
    exception: &PyObjectRef,
) -> PyResult<()> {
    let code_id = code.get_id();
    let args = FuncArgs::from(vec![
        code.clone().into(),
        vm.ctx.new_int(offset).into(),
        exception.clone(),
    ]);
    fire_event_inner(vm, 10, EVENT_STOP_ITERATION, code_id, offset, &args)
}

/// JUMP: fired when a jump instruction executes
pub fn fire_jump(
    vm: &VirtualMachine,
    code: &PyRef<PyCode>,
    offset: u32,
    destination: u32,
) -> PyResult<()> {
    let code_id = code.get_id();
    let args = FuncArgs::from(vec![
        code.clone().into(),
        vm.ctx.new_int(offset).into(),
        vm.ctx.new_int(destination).into(),
    ]);
    fire_event_inner(vm, 7, EVENT_JUMP, code_id, offset, &args)
}

/// BRANCH_LEFT: fired when a branch goes left (condition true)
pub fn fire_branch_left(
    vm: &VirtualMachine,
    code: &PyRef<PyCode>,
    offset: u32,
    destination: u32,
) -> PyResult<()> {
    let code_id = code.get_id();
    let args = FuncArgs::from(vec![
        code.clone().into(),
        vm.ctx.new_int(offset).into(),
        vm.ctx.new_int(destination).into(),
    ]);
    fire_event_inner(vm, 8, EVENT_BRANCH_LEFT, code_id, offset, &args)
}

/// BRANCH_RIGHT: fired when a branch goes right (condition false, falls through)
pub fn fire_branch_right(
    vm: &VirtualMachine,
    code: &PyRef<PyCode>,
    offset: u32,
    destination: u32,
) -> PyResult<()> {
    let code_id = code.get_id();
    let args = FuncArgs::from(vec![
        code.clone().into(),
        vm.ctx.new_int(offset).into(),
        vm.ctx.new_int(destination).into(),
    ]);
    fire_event_inner(vm, 9, EVENT_BRANCH_RIGHT, code_id, offset, &args)
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
        MonitoringSentinel.into_ref(&vm.ctx).into()
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
    fn use_tool_id(tool_id: i32, name: PyStrRef, vm: &VirtualMachine) -> PyResult<()> {
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

mod metadata;
#[cfg(test)]
mod tests;
use metadata::{MonitoringInstruction, MonitoringMetadata};

use crate::common::rc::PyRc;
use crate::{
    AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    builtins::{PyCode, PyDictRef, PyNamespace, PyUtf8StrRef, code::MonitoringCodeIdentity},
    function::FuncArgs,
};

#[cfg(not(feature = "threading"))]
use alloc::rc::Weak;
#[cfg(feature = "threading")]
use alloc::sync::Weak;
use crossbeam_utils::atomic::AtomicCell;
use std::collections::{HashMap, HashSet};

pub(crate) const TOOL_LIMIT: usize = 6;
const EVENTS_COUNT: usize = 19;
const UNGROUPED_EVENTS_COUNT: usize = 16;

/// Event identifier (`PY_MONITORING_EVENT_*`), stored in `tstate->what_event`.
/// `None` on the VM field is the `< 0` sentinel (not in a monitoring callback).
#[repr(i32)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum MonitoringEvent {
    PyStart = 0,
    PyResume = 1,
    PyReturn = 2,
    PyYield = 3,
    Call = 4,
    Line = 5,
    Instruction = 6,
    Jump = 7,
    BranchLeft = 8,
    BranchRight = 9,
    StopIteration = 10,
    Raise = 11,
    ExceptionHandled = 12,
    PyUnwind = 13,
    PyThrow = 14,
    Reraise = 15,
    CReturn = 16,
    CRaise = 17,
    Branch = 18,
}

impl MonitoringEvent {
    pub(crate) const fn mask(self) -> u32 {
        1 << (self as u32)
    }

    pub(crate) const fn from_id(id: usize) -> Option<Self> {
        match id {
            0 => Some(Self::PyStart),
            1 => Some(Self::PyResume),
            2 => Some(Self::PyReturn),
            3 => Some(Self::PyYield),
            4 => Some(Self::Call),
            5 => Some(Self::Line),
            6 => Some(Self::Instruction),
            7 => Some(Self::Jump),
            8 => Some(Self::BranchLeft),
            9 => Some(Self::BranchRight),
            10 => Some(Self::StopIteration),
            11 => Some(Self::Raise),
            12 => Some(Self::ExceptionHandled),
            13 => Some(Self::PyUnwind),
            14 => Some(Self::PyThrow),
            15 => Some(Self::Reraise),
            16 => Some(Self::CReturn),
            17 => Some(Self::CRaise),
            18 => Some(Self::Branch),
            _ => None,
        }
    }
}

const C_RETURN_MASK: u32 = MonitoringEvent::CReturn.mask() | MonitoringEvent::CRaise.mask();

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

// Scalar counterparts of the canonical direct/LINE/INSTRUCTION operations.
// No executable code or cache storage is modified by these transitions.
#[derive(Clone, Copy)]
pub(crate) struct MonitoringLayerSite {
    base: rustpython_compiler_core::bytecode::Instruction,
    direct: Option<rustpython_compiler_core::bytecode::Instruction>,
    direct_event: Option<MonitoringEvent>,
    line_eligible: bool,
    instruction_eligible: bool,
    pub(crate) top: rustpython_compiler_core::bytecode::Instruction,
    pub(crate) line_next: rustpython_compiler_core::bytecode::Instruction,
    pub(crate) instruction_next: rustpython_compiler_core::bytecode::Instruction,
    pub(crate) direct_tools: u8,
    pub(crate) line_tools: u8,
    pub(crate) instruction_tools: u8,
}

impl MonitoringLayerSite {
    // The metadata builder must supply a proven logical instruction start and
    // line eligibility. This does not validate branch targets or execution.
    pub(crate) fn new(
        base: rustpython_compiler_core::bytecode::Instruction,
        arg: u8,
        line_eligible: bool,
        instruction_eligible: bool,
    ) -> Self {
        Self {
            base,
            direct: base.to_instrumented(),
            direct_event: event_for_opcode(base, arg),
            line_eligible,
            instruction_eligible,
            top: base,
            line_next: base,
            instruction_next: base,
            direct_tools: 0,
            line_tools: 0,
            instruction_tools: 0,
        }
    }

    fn direct_slot(&mut self) -> &mut rustpython_compiler_core::bytecode::Instruction {
        use rustpython_compiler_core::bytecode::Instruction;
        let outer = if matches!(self.top, Instruction::InstrumentedLine) {
            &mut self.line_next
        } else {
            &mut self.top
        };
        if matches!(*outer, Instruction::InstrumentedInstruction) {
            &mut self.instruction_next
        } else {
            outer
        }
    }

    fn remove_direct(&mut self, tools: u8) {
        self.direct_tools &= !tools;
        if self.direct_tools == 0 {
            let base = self.base;
            *self.direct_slot() = base;
        }
    }

    fn add_direct(&mut self, tools: u8) {
        if let Some(direct) = self.direct {
            self.direct_tools |= tools;
            *self.direct_slot() = direct;
        }
    }

    fn remove_line(&mut self, tools: u8) {
        use rustpython_compiler_core::bytecode::Instruction;
        self.line_tools &= !tools;
        if self.line_tools == 0 && matches!(self.top, Instruction::InstrumentedLine) {
            let original = self.line_next;
            if matches!(original, Instruction::InstrumentedInstruction) {
                self.line_next = self.instruction_next;
            }
            // The old original becomes top, while an in-flight LINE reads the
            // updated saved continuation after callbacks.
            self.top = original;
        }
    }

    fn add_line(&mut self, tools: u8) {
        use rustpython_compiler_core::bytecode::Instruction;
        if self.line_eligible {
            self.line_tools |= tools;
            if !matches!(self.top, Instruction::InstrumentedLine) {
                self.line_next = self.top;
                self.top = Instruction::InstrumentedLine;
            }
        }
    }

    fn remove_instruction(&mut self, tools: u8) {
        use rustpython_compiler_core::bytecode::Instruction;
        self.instruction_tools &= !tools;
        if self.instruction_tools == 0 {
            let outer = if matches!(self.top, Instruction::InstrumentedLine) {
                &mut self.line_next
            } else {
                &mut self.top
            };
            if matches!(*outer, Instruction::InstrumentedInstruction) {
                *outer = self.instruction_next;
            }
        }
    }

    fn add_instruction(&mut self, tools: u8) {
        use rustpython_compiler_core::bytecode::Instruction;
        if self.instruction_eligible {
            self.instruction_tools |= tools;
            let outer = if matches!(self.top, Instruction::InstrumentedLine) {
                &mut self.line_next
            } else {
                &mut self.top
            };
            if !matches!(*outer, Instruction::InstrumentedInstruction) {
                self.instruction_next = *outer;
                *outer = Instruction::InstrumentedInstruction;
            }
        }
    }

    fn transition(
        &mut self,
        removed: &[u8; UNGROUPED_EVENTS_COUNT],
        added: &[u8; UNGROUPED_EVENTS_COUNT],
    ) {
        if let Some(event) = self.direct_event {
            if removed[event as usize] != 0 {
                self.remove_direct(removed[event as usize]);
            }
            if added[event as usize] != 0 {
                self.add_direct(added[event as usize]);
            }
        }
        let line = MonitoringEvent::Line as usize;
        let instruction = MonitoringEvent::Instruction as usize;
        if removed[line] != 0 {
            self.remove_line(removed[line]);
        }
        if removed[instruction] != 0 {
            self.remove_instruction(removed[instruction]);
        }
        if added[line] != 0 {
            self.add_line(added[line]);
        }
        if added[instruction] != 0 {
            self.add_instruction(added[instruction]);
        }
    }

    fn disable(&mut self, event: usize, tool: usize) {
        let tools = 1 << tool;
        if event == MonitoringEvent::Line as usize {
            self.remove_line(tools);
        } else if event == MonitoringEvent::Instruction as usize {
            self.remove_instruction(tools);
        } else if self
            .direct_event
            .is_some_and(|direct| direct as usize == event)
        {
            self.remove_direct(tools);
        }
    }
}

fn event_tools(events: &[u32; TOOL_LIMIT]) -> [u8; UNGROUPED_EVENTS_COUNT] {
    core::array::from_fn(|event| {
        events.iter().enumerate().fold(0, |tools, (tool, mask)| {
            tools
                | if mask & (1 << event) != 0 {
                    1 << tool
                } else {
                    0
                }
        })
    })
}

// All per-code state shares this native lifetime witness. It owns no Python
// references and does not keep the code object alive.
struct CodeMonitoringEntry {
    identity: Weak<MonitoringCodeIdentity>,
    local_events: [u32; TOOL_LIMIT],
    disabled: HashSet<(usize, usize, usize)>,
    active_events: [u32; TOOL_LIMIT],
    layers: Option<HashMap<usize, MonitoringLayerSite>>,
    metadata: Option<MonitoringMetadata>,
}

impl CodeMonitoringEntry {
    fn new(identity: &PyRc<MonitoringCodeIdentity>) -> Self {
        Self {
            identity: PyRc::downgrade(identity),
            local_events: [0; TOOL_LIMIT],
            disabled: HashSet::new(),
            active_events: [0; TOOL_LIMIT],
            layers: None,
            metadata: None,
        }
    }

    fn disabled_tools(&self, offset: usize) -> [u8; UNGROUPED_EVENTS_COUNT] {
        core::array::from_fn(|event| {
            (0..TOOL_LIMIT).fold(0, |tools, tool| {
                tools
                    | if self.disabled.contains(&(offset, event, tool)) {
                        1 << tool
                    } else {
                        0
                    }
            })
        })
    }

    fn synchronize(&mut self, global: &[u32; TOOL_LIMIT], restart: bool) {
        let next = core::array::from_fn(|tool| global[tool] | self.local_events[tool]);
        let old_tools = event_tools(&self.active_events);
        let new_tools = event_tools(&next);
        let removed = core::array::from_fn(|event| {
            if restart {
                old_tools[event]
            } else {
                old_tools[event] & !new_tools[event]
            }
        });
        let added = core::array::from_fn(|event| {
            if restart {
                new_tools[event]
            } else {
                new_tools[event] & !old_tools[event]
            }
        });
        self.active_events = next;
        if restart {
            self.disabled.clear();
        } else {
            self.disabled.retain(|&(_, event, tool)| {
                event >= UNGROUPED_EVENTS_COUNT || added[event] & (1 << tool) == 0
            });
        }
        if let Some(layers) = &mut self.layers {
            // Each site updates only its own scalar layers; no callbacks or owners are dropped.
            #[allow(clippy::iter_over_hash_type)]
            for site in layers.values_mut() {
                site.transition(&removed, &added);
            }
        }
    }

    fn disable_site(&mut self, offset: usize, event: usize, tool: usize) {
        self.disabled.insert((offset, event, tool));
        if let Some(layers) = &mut self.layers
            && let Some(site) = layers.get_mut(&(offset / 2))
        {
            site.disable(event, tool);
        }
    }
}

fn identity_key(identity: &PyRc<MonitoringCodeIdentity>) -> usize {
    PyRc::as_ptr(identity) as usize
}

/// Interpreter-level monitoring state, shared by all threads.
pub struct MonitoringState {
    pub tool_names: [Option<String>; TOOL_LIMIT],
    pub global_events: [u32; TOOL_LIMIT],
    codes: HashMap<usize, CodeMonitoringEntry>,
    pub callbacks: HashMap<(usize, usize), PyObjectRef>,
    // Legacy BRANCH callbacks also check whether the source instruction was
    // disabled. Direct BRANCH_LEFT/RIGHT registrations bypass that check.
    legacy_branch_callbacks: HashSet<(usize, usize)>,
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
            codes: HashMap::new(),
            callbacks: HashMap::new(),
            legacy_branch_callbacks: HashSet::new(),
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
        let local = self.codes.values().fold(0, |acc, entry| {
            acc | entry.local_events.iter().fold(0, |mask, &e| mask | e)
        });
        global | local
    }

    /// Reading event masks must not lazily allocate an identity under the lock.
    pub fn events_for_code(&self, code: &PyCode) -> u32 {
        let global = self.global_events.iter().fold(0, |acc, &e| acc | e);
        let local = code
            .existing_monitoring_identity()
            .and_then(|identity| self.codes.get(&identity_key(identity)))
            .map_or(0, |entry| {
                entry.local_events.iter().fold(0, |acc, &e| acc | e)
            });
        global | local
    }

    fn local_events(&self, key: usize, tool: usize) -> u32 {
        self.codes
            .get(&key)
            .map_or(0, |entry| entry.local_events[tool])
    }

    fn disabled(&self, key: usize, offset: usize, event: usize, tool: usize) -> bool {
        self.codes
            .get(&key)
            .is_some_and(|entry| entry.disabled.contains(&(offset, event, tool)))
    }

    // The caller retains the code while using its key. A stored Weak keeps its
    // allocation from being reused even after code destruction. Live entries
    // survive clear/re-enable, including during callbacks.
    fn ensure_code(&mut self, identity: &PyRc<MonitoringCodeIdentity>) {
        self.codes.entry(identity_key(identity)).or_insert_with(|| {
            let mut entry = CodeMonitoringEntry::new(identity);
            entry.synchronize(&self.global_events, false);
            entry
        });
    }

    // Installation is staged for the immutable metadata builder. False means
    // unavailable metadata, not a new public constructor or monitoring error.
    pub(crate) fn install_layers(
        &mut self,
        code: &PyCode,
        mut layers: HashMap<usize, MonitoringLayerSite>,
    ) -> bool {
        // Only bounded base-instruction starts may own a virtual layer.
        // An opaque suffix remains outside this map and uses no invented base.
        use rustpython_compiler_core::bytecode::{Instruction, oparg};
        let source = code.instructions.base_instruction_prefix();
        let mut index = 0;
        let mut checked = 0;
        let mut first_traceable = None;
        while index < source.len() {
            let base = source[index].op.deoptimize();
            let Some(end) = index.checked_add(1 + base.cache_entries()) else {
                return false;
            };
            if end > source.len() {
                return false;
            }
            let arg = u8::from(source[index].arg);
            if first_traceable.is_none()
                && matches!(base, Instruction::Resume { .. })
                && u32::from(arg) & oparg::ResumeContext::LOCATION_MASK
                    != oparg::ResumeLocation::AtGenExprStart.as_u32()
            {
                first_traceable = Some(index);
            }
            if let Some(site) = layers.get(&index) {
                if first_traceable.is_none()
                    || u8::from(site.base) != u8::from(base)
                    || site.direct_event.map(|event| event as usize)
                        != event_for_opcode(base, arg).map(|event| event as usize)
                {
                    return false;
                }
                checked += 1;
            }
            index = end;
        }
        if checked != layers.len() {
            return false;
        }
        let Some(identity) = code.existing_monitoring_identity() else {
            return false;
        };
        let Some(entry) = self.codes.get_mut(&identity_key(identity)) else {
            return false;
        };
        if entry.layers.is_some() {
            return true;
        }
        let added = event_tools(&entry.active_events);
        // Initial installation only: different sites are independent, while
        // each site's disabled events use the fixed direct/LINE/INSTRUCTION order.
        #[allow(clippy::iter_over_hash_type)]
        for (&index, site) in &mut layers {
            site.transition(&[0; UNGROUPED_EVENTS_COUNT], &added);
            site.transition(
                &entry.disabled_tools(index * 2),
                &[0; UNGROUPED_EVENTS_COUNT],
            );
        }
        entry.layers = Some(layers);
        true
    }

    pub(crate) fn layer_site(&self, code: &PyCode, index: usize) -> Option<MonitoringLayerSite> {
        let identity = code.existing_monitoring_identity()?;
        self.codes
            .get(&identity_key(identity))?
            .layers
            .as_ref()?
            .get(&index)
            .copied()
    }

    fn synchronize_layers(&mut self, restart: bool) {
        // Entries contain independent native state; iteration invokes no Python callbacks.
        #[allow(clippy::iter_over_hash_type)]
        for entry in self.codes.values_mut() {
            entry.synchronize(&self.global_events, restart);
        }
    }

    // Return retired allocations so callers destroy them after unlocking.
    fn sweep_dead_codes(&mut self) -> Vec<CodeMonitoringEntry> {
        self.codes
            .extract_if(|_, entry| entry.identity.strong_count() == 0)
            .map(|(_, entry)| entry)
            .collect()
    }
}

/// Global atomic mask: OR of all tools' events. Checked in the hot path
/// to skip monitoring overhead when no events are registered.
/// Lives in PyGlobalState alongside the PyMutex<MonitoringState>.
pub(crate) type MonitoringEventsMask = AtomicCell<u32>;

/// Get the MISSING sentinel, creating it if necessary.
pub(crate) fn get_missing(vm: &VirtualMachine) -> PyObjectRef {
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
pub(crate) fn get_disable(vm: &VirtualMachine) -> PyObjectRef {
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
    if MonitoringEvent::from_id(event_id).is_none() {
        return Err(vm.new_value_error(format!("invalid event {event}")));
    }
    Ok(event_id)
}

/// `PY_MONITORING_IS_INSTRUMENTED_EVENT`
const fn is_instrumented_event(event_id: usize) -> bool {
    event_id <= MonitoringEvent::StopIteration as usize
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

    if (event_set & C_RETURN_MASK) != 0
        && (event_set & MonitoringEvent::Call.mask()) != MonitoringEvent::Call.mask()
    {
        return Err(vm.new_value_error("cannot set C_RETURN or C_RAISE events independently"));
    }

    event_set &= !C_RETURN_MASK;

    if (event_set & MonitoringEvent::Branch.mask()) != 0 {
        event_set &= !MonitoringEvent::Branch.mask();
        event_set |= MonitoringEvent::BranchLeft.mask() | MonitoringEvent::BranchRight.mask();
    }

    if local && event_set >= (1 << UNGROUPED_EVENTS_COUNT) {
        return Err(vm.new_value_error(format!("invalid local event set 0x{event_set:x}")));
    }

    Ok(event_set)
}

/// Event that causes this opcode to be rewritten to INSTRUMENTED_*.
/// RESUME uses `oparg`: 0 → PY_START, nonzero → PY_RESUME.
fn event_for_opcode(
    op: rustpython_compiler_core::bytecode::Instruction,
    oparg: u8,
) -> Option<MonitoringEvent> {
    use rustpython_compiler_core::bytecode::Opcode;
    match op.deoptimize().as_opcode() {
        Opcode::ReturnValue => Some(MonitoringEvent::PyReturn),
        Opcode::Call | Opcode::CallKw | Opcode::CallFunctionEx | Opcode::LoadSuperAttr => {
            Some(MonitoringEvent::Call)
        }
        Opcode::YieldValue => Some(MonitoringEvent::PyYield),
        Opcode::JumpForward | Opcode::JumpBackward => Some(MonitoringEvent::Jump),
        Opcode::PopJumpIfFalse
        | Opcode::PopJumpIfTrue
        | Opcode::PopJumpIfNone
        | Opcode::PopJumpIfNotNone => Some(MonitoringEvent::BranchRight),
        Opcode::ForIter => Some(MonitoringEvent::BranchLeft),
        Opcode::PopIter => Some(MonitoringEvent::BranchRight),
        Opcode::EndFor | Opcode::EndSend => Some(MonitoringEvent::StopIteration),
        Opcode::NotTaken => Some(MonitoringEvent::BranchLeft),
        Opcode::EndAsyncFor => Some(MonitoringEvent::BranchRight),
        Opcode::Resume => {
            if oparg != 0 {
                Some(MonitoringEvent::PyResume)
            } else {
                Some(MonitoringEvent::PyStart)
            }
        }
        _ => None,
    }
}

/// Update only this interpreter's scalar subscriptions. Executable bytecode
/// and adaptive caches are never rewritten by monitoring state changes.
fn update_events_mask(vm: &VirtualMachine, state: &mut MonitoringState) {
    state.synchronize_layers(false);
    vm.state.monitoring_events.store(state.combined_events());
}

pub(crate) fn prepare_code(vm: &VirtualMachine, code: &PyCode) {
    let identity = code.monitoring_identity();
    let key = identity_key(identity);
    let (needed, retired) = {
        let mut state = vm.state.monitoring.lock();
        let retired = if state.codes.contains_key(&key) {
            Vec::new()
        } else {
            state.sweep_dead_codes()
        };
        state.ensure_code(identity);
        if !retired.is_empty() {
            vm.state.monitoring_events.store(state.combined_events());
        }
        (state.codes[&key].metadata.is_none(), retired)
    };
    drop(retired);
    if !needed {
        return;
    }
    // Native metadata construction and losing candidate destruction occur
    // outside the interpreter state guard.
    let metadata = MonitoringMetadata::build(code);
    let layers = metadata.layer_sites();
    let mut candidate = Some(metadata);
    {
        let mut state = vm.state.monitoring.lock();
        if state.codes[&key].metadata.is_none() && state.install_layers(code, layers) {
            state.codes.get_mut(&key).unwrap().metadata = candidate.take();
        }
    }
    drop(candidate);
}

pub(crate) fn dispatch_operation(
    vm: &VirtualMachine,
    code: &PyCode,
    index: usize,
) -> Option<(
    MonitoringInstruction,
    rustpython_compiler_core::bytecode::Instruction,
)> {
    if vm.state.monitoring_events.load() == 0 {
        return None;
    }
    prepare_code(vm, code);
    let state = vm.state.monitoring.lock();
    if state.events_for_code(code) == 0 {
        return None;
    }
    let key = identity_key(code.existing_monitoring_identity()?);
    let entry = state.codes.get(&key)?;
    let metadata = entry.metadata.as_ref()?;
    if index >= metadata.prefix_len
        || (state.events_for_code(code) & MonitoringEvent::Line.mask() != 0
            && !metadata.line_info_valid)
    {
        return None;
    }
    let operation = *metadata.get(index)?;
    let top = entry
        .layers
        .as_ref()?
        .get(&index)
        .map_or(operation.base, |site| site.top);
    Some((operation, top))
}

pub(crate) fn layer_site(
    vm: &VirtualMachine,
    code: &PyCode,
    index: usize,
) -> Option<MonitoringLayerSite> {
    vm.state.monitoring.lock().layer_site(code, index)
}

// This arbitrary-offset lookup scans the bounded prefix in O(index) time.
// Dispatch-proven instruction starts can instead read and deoptimize directly.
pub(crate) fn source_operation(
    code: &PyCode,
    index: usize,
) -> Option<rustpython_compiler_core::bytecode::CodeUnit> {
    code.instructions
        .base_instructions()
        .take_while(|(at, _)| *at <= index)
        .find_map(|(at, unit)| (at == index).then_some(unit))
}

pub(crate) fn known_operation(
    vm: &VirtualMachine,
    code: &PyCode,
    index: usize,
) -> Option<MonitoringInstruction> {
    let identity = code.existing_monitoring_identity()?;
    let state = vm.state.monitoring.lock();
    state
        .codes
        .get(&identity_key(identity))?
        .metadata
        .as_ref()?
        .get(index)
        .copied()
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
    release_tool_callbacks(tool_id, false, vm)
}

fn free_tool_id(tool_id: i32, vm: &VirtualMachine) -> PyResult<()> {
    release_tool_callbacks(tool_id, true, vm)
}

fn release_tool_callbacks(tool_id: i32, free: bool, vm: &VirtualMachine) -> PyResult<()> {
    let tool = check_valid_tool(tool_id, vm)?;
    let (callbacks, retired) = {
        let mut state = vm.state.monitoring.lock();
        let retired = state.sweep_dead_codes();
        let callbacks = if state.tool_names[tool].is_some() {
            state.global_events[tool] = 0;
            // Only independent scalar masks and scalar disabled keys are cleared here.
            #[allow(clippy::iter_over_hash_type)]
            for entry in state.codes.values_mut() {
                entry.local_events[tool] = 0;
                entry.disabled.retain(|&(_, _, t)| t != tool);
            }
            let callbacks: Vec<_> = state
                .callbacks
                .extract_if(|(cb_tool, _), _| *cb_tool == tool)
                .map(|(_, callback)| callback)
                .collect();
            state
                .legacy_branch_callbacks
                .retain(|(cb_tool, _)| *cb_tool != tool);
            callbacks
        } else {
            Vec::new()
        };
        update_events_mask(vm, &mut state);
        (callbacks, retired)
    };
    // Callback finalizers may reenter monitoring; release owners after unlocking.
    drop(callbacks);
    drop(retired);
    // Keep the name visible during callback finalization, as CPython does.
    if free {
        vm.state.monitoring.lock().tool_names[tool] = None;
    }
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

    vm.audit("sys.monitoring.register_callback", || (func.clone(),))?;

    let (prev, displaced) = {
        let mut state = vm.state.monitoring.lock();
        let prev = state
            .callbacks
            .remove(&(tool, event_id))
            .unwrap_or_else(|| vm.ctx.none());
        let mut displaced = Vec::new();
        state.legacy_branch_callbacks.remove(&(tool, event_id));
        if !vm.is_none(&func) {
            state.callbacks.insert((tool, event_id), func.clone());
        }
        // BRANCH replaces both edge registrations, including independently
        // registered callbacks. Keep their owners alive until after unlocking.
        if event_id == MonitoringEvent::Branch as usize {
            for edge in [MonitoringEvent::BranchLeft, MonitoringEvent::BranchRight] {
                let key = (tool, edge as usize);
                let old = if vm.is_none(&func) {
                    state.legacy_branch_callbacks.remove(&key);
                    state.callbacks.remove(&key)
                } else {
                    state.legacy_branch_callbacks.insert(key);
                    state.callbacks.insert(key, func.clone())
                };
                displaced.extend(old);
            }
        }
        (prev, displaced)
    };
    drop(displaced);
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
    let retired = {
        let mut state = vm.state.monitoring.lock();
        let retired = state.sweep_dead_codes();
        state.global_events[tool] = normalized;
        update_events_mask(vm, &mut state);
        retired
    };
    drop(retired);
    Ok(())
}

fn get_local_events(tool_id: i32, code: &PyObject, vm: &VirtualMachine) -> PyResult<u32> {
    let code = code
        .downcast_ref::<PyCode>()
        .ok_or_else(|| vm.new_type_error("code must be a code object"))?;
    let tool = check_valid_tool(tool_id, vm)?;
    let Some(identity) = code.existing_monitoring_identity() else {
        return Ok(0);
    };
    Ok(vm
        .state
        .monitoring
        .lock()
        .local_events(identity_key(identity), tool))
}

fn set_local_events(
    tool_id: i32,
    code: &PyObject,
    event_set: i32,
    vm: &VirtualMachine,
) -> PyResult<()> {
    let code = code
        .downcast_ref::<PyCode>()
        .ok_or_else(|| vm.new_type_error("code must be a code object"))?;
    let tool = check_valid_tool(tool_id, vm)?;
    check_tool_in_use(tool, vm)?;
    let normalized = normalize_event_set(event_set, true, vm)?;
    let identity = code.monitoring_identity();
    let retired = {
        let mut state = vm.state.monitoring.lock();
        let retired = state.sweep_dead_codes();
        state.ensure_code(identity);
        state
            .codes
            .get_mut(&identity_key(identity))
            .unwrap()
            .local_events[tool] = normalized;
        update_events_mask(vm, &mut state);
        retired
    };
    drop(retired);
    Ok(())
}

fn restart_events(vm: &VirtualMachine) {
    let retired = {
        let mut state = vm.state.monitoring.lock();
        let retired = state.sweep_dead_codes();
        state.synchronize_layers(true);
        vm.state.monitoring_events.store(state.combined_events());
        retired
    };
    drop(retired);
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
                for (i, tool) in state.global_events.iter().enumerate().take(TOOL_LIMIT) {
                    if (tool & event_bit) != 0 {
                        tools_mask |= 1 << i;
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
    event: MonitoringEvent,
    code: &Py<PyCode>,
    offset: u32,
    cb_extra: &[PyObjectRef],
) -> PyResult<()> {
    fire_at(vm, event, code, offset, offset, cb_extra)
}

// The physical instruction owns DISABLE state; branch callback source offsets
// can differ from that instruction when NOT_TAKEN follows a branch or wrapper.
fn fire_at(
    vm: &VirtualMachine,
    event: MonitoringEvent,
    code: &Py<PyCode>,
    offset: u32,
    source_offset: u32,
    cb_extra: &[PyObjectRef],
) -> PyResult<()> {
    let tools = snapshot_tools(vm, event, code, offset);
    fire_snapshot(vm, event, code, offset, source_offset, cb_extra, tools)
}

// Snapshot eligibility independently of callback presence. An earlier selected
// tool may replace/install a later selected tool's callback before its turn.
pub(crate) fn snapshot_tools(
    vm: &VirtualMachine,
    event: MonitoringEvent,
    code: &Py<PyCode>,
    offset: u32,
) -> u8 {
    if vm.tracing_is_suppressed() || FIRING.with(|f| f.get()) {
        return 0;
    }
    prepare_code(vm, code);
    let key = identity_key(code.monitoring_identity());
    let event_id = if matches!(event, MonitoringEvent::CReturn | MonitoringEvent::CRaise) {
        MonitoringEvent::Call as usize
    } else {
        event as usize
    };
    let check_bit = if matches!(event, MonitoringEvent::CReturn | MonitoringEvent::CRaise) {
        event.mask() | MonitoringEvent::Call.mask()
    } else {
        event.mask()
    };
    let state = vm.state.monitoring.lock();
    (0..TOOL_LIMIT).fold(0, |tools, tool| {
        let active = state.global_events[tool] | state.local_events(key, tool);
        tools
            | if active & check_bit != 0 && !state.disabled(key, offset as usize, event_id, tool) {
                1 << tool
            } else {
                0
            }
    })
}

pub(crate) fn fire_snapshot(
    vm: &VirtualMachine,
    event: MonitoringEvent,
    code: &Py<PyCode>,
    offset: u32,
    source_offset: u32,
    cb_extra: &[PyObjectRef],
    tools: u8,
) -> PyResult<()> {
    if tools == 0 {
        return Ok(());
    }
    let code_id = identity_key(code.monitoring_identity());
    let event_id = event as usize;
    let event_mask = event.mask();
    let mut args_vec = Vec::with_capacity(1 + cb_extra.len());
    args_vec.push(code.to_owned().into());
    args_vec.extend_from_slice(cb_extra);
    let args = FuncArgs::from(args_vec);
    for tool in (0..TOOL_LIMIT)
        .rev()
        .filter(|tool| tools & (1 << tool) != 0)
    {
        let callback = {
            let mut state = vm.state.monitoring.lock();
            let legacy = state.legacy_branch_callbacks.contains(&(tool, event_id));
            let source_instrumented = state
                .layer_site(code, source_offset as usize / 2)
                .is_some_and(|site| site.top.is_instrumented());
            if legacy && !source_instrumented {
                state.codes.get_mut(&code_id).unwrap().disable_site(
                    offset as usize,
                    event_id,
                    tool,
                );
                None
            } else {
                state.callbacks.get(&(tool, event_id)).cloned()
            }
        };
        let Some(callback) = callback else {
            continue;
        };
        if call_instrument(vm, event, &callback, args.clone())? {
            if is_instrumented_event(event_id) {
                vm.state
                    .monitoring
                    .lock()
                    .codes
                    .get_mut(&code_id)
                    .unwrap()
                    .disable_site(offset as usize, event_id, tool);
            } else if event_id < UNGROUPED_EVENTS_COUNT {
                let mut state = vm.state.monitoring.lock();
                let local = &mut state.codes.get_mut(&code_id).unwrap().local_events[tool];
                if *local != 0 {
                    *local &= !event_mask;
                    update_events_mask(vm, &mut state);
                }
            } else {
                return Err(cannot_disable(vm, event, tool));
            }
        }
    }
    Ok(())
}

fn call_instrument(
    vm: &VirtualMachine,
    event: MonitoringEvent,
    callback: &PyObject,
    args: FuncArgs,
) -> PyResult<bool> {
    if vm.tracing_is_suppressed() || FIRING.with(|f| f.get()) {
        return Ok(false);
    }
    let disable_sentinel = vm.state.monitoring.lock().disable.clone();
    FIRING.with(|f| f.set(true));
    vm.enter_tracing();
    let old_what = vm.what_event.replace(Some(event));
    let result = callback.call(args, vm);
    vm.what_event.set(old_what);
    vm.leave_tracing();
    FIRING.with(|f| f.set(false));
    let result = result?;
    let disable = disable_sentinel.as_ref().is_some_and(|d| result.is(d));
    Ok(disable)
}

fn cannot_disable(
    vm: &VirtualMachine,
    event: MonitoringEvent,
    tool: usize,
) -> crate::builtins::PyBaseExceptionRef {
    let callback = {
        let mut state = vm.state.monitoring.lock();
        state.callbacks.remove(&(tool, event as usize))
    };
    drop(callback);
    vm.new_value_error(format!(
        "Cannot disable {} events. Callback removed.",
        EVENT_NAMES[event as usize]
    ))
}

// Public dispatch functions (called from frame.rs)

pub(crate) fn fire_py_yield(
    vm: &VirtualMachine,
    code: &Py<PyCode>,
    offset: u32,
    retval: &PyObject,
) -> PyResult<()> {
    fire(
        vm,
        MonitoringEvent::PyYield,
        code,
        offset,
        &[vm.ctx.new_int(offset).into(), retval.to_owned()],
    )
}

pub(crate) fn fire_call(
    vm: &VirtualMachine,
    code: &Py<PyCode>,
    offset: u32,
    callable: &PyObject,
    arg0: PyObjectRef,
) -> PyResult<()> {
    fire(
        vm,
        MonitoringEvent::Call,
        code,
        offset,
        &[vm.ctx.new_int(offset).into(), callable.to_owned(), arg0],
    )
}

pub(crate) fn fire_c_return(
    vm: &VirtualMachine,
    code: &Py<PyCode>,
    offset: u32,
    callable: &PyObject,
    arg0: PyObjectRef,
) -> PyResult<()> {
    fire(
        vm,
        MonitoringEvent::CReturn,
        code,
        offset,
        &[vm.ctx.new_int(offset).into(), callable.to_owned(), arg0],
    )
}

pub(crate) fn fire_c_raise(
    vm: &VirtualMachine,
    code: &Py<PyCode>,
    offset: u32,
    callable: &PyObject,
    arg0: PyObjectRef,
) -> PyResult<()> {
    fire(
        vm,
        MonitoringEvent::CRaise,
        code,
        offset,
        &[vm.ctx.new_int(offset).into(), callable.to_owned(), arg0],
    )
}

pub(crate) fn fire_raise(
    vm: &VirtualMachine,
    code: &Py<PyCode>,
    offset: u32,
    exception: &PyObject,
) -> PyResult<()> {
    fire(
        vm,
        MonitoringEvent::Raise,
        code,
        offset,
        &[vm.ctx.new_int(offset).into(), exception.to_owned()],
    )
}

/// End suppression when the exception reaches a handler or escapes a frame,
/// independently of whether either lifecycle event has a registered callback.
pub(crate) fn reset_reraise_pending() {
    RERAISE_PENDING.with(|f| f.set(false));
}

/// Only fires if no RERAISE has been fired since the last EXCEPTION_HANDLED,
/// preventing duplicate events from chained cleanup handlers.
pub(crate) fn fire_reraise(
    vm: &VirtualMachine,
    code: &Py<PyCode>,
    offset: u32,
    exception: &PyObject,
) -> PyResult<()> {
    if RERAISE_PENDING.with(|f| f.get()) {
        return Ok(());
    }
    RERAISE_PENDING.with(|f| f.set(true));
    let result = fire(
        vm,
        MonitoringEvent::Reraise,
        code,
        offset,
        &[vm.ctx.new_int(offset).into(), exception.to_owned()],
    );
    if result.is_err() {
        RERAISE_PENDING.with(|f| f.set(false));
    }
    result
}

pub(crate) fn fire_exception_handled(
    vm: &VirtualMachine,
    code: &Py<PyCode>,
    offset: u32,
    exception: &PyObject,
) -> PyResult<()> {
    reset_reraise_pending();
    fire(
        vm,
        MonitoringEvent::ExceptionHandled,
        code,
        offset,
        &[vm.ctx.new_int(offset).into(), exception.to_owned()],
    )
}

pub(crate) fn fire_py_unwind(
    vm: &VirtualMachine,
    code: &Py<PyCode>,
    offset: u32,
    exception: &PyObject,
) -> PyResult<()> {
    reset_reraise_pending();
    fire(
        vm,
        MonitoringEvent::PyUnwind,
        code,
        offset,
        &[vm.ctx.new_int(offset).into(), exception.to_owned()],
    )
}

pub(crate) fn fire_py_throw(
    vm: &VirtualMachine,
    code: &Py<PyCode>,
    offset: u32,
    exception: &PyObject,
) -> PyResult<()> {
    fire(
        vm,
        MonitoringEvent::PyThrow,
        code,
        offset,
        &[vm.ctx.new_int(offset).into(), exception.to_owned()],
    )
}

/// If `value` is already a `StopIteration`, pass it directly; otherwise wrap
/// it in a new `StopIteration(value)` — matching `PyMonitoring_FireStopIterationEvent`.
pub(crate) fn fire_stop_iteration(
    vm: &VirtualMachine,
    code: &Py<PyCode>,
    offset: u32,
    value: &PyObject,
) -> PyResult<()> {
    let exc: PyObjectRef = if value.fast_isinstance(vm.ctx.exceptions.stop_iteration) {
        value.to_owned()
    } else {
        vm.new_stop_iteration(Some(value.to_owned())).into()
    };
    fire(
        vm,
        MonitoringEvent::StopIteration,
        code,
        offset,
        &[vm.ctx.new_int(offset).into(), exc],
    )
}

pub(crate) fn fire_branch_left(
    vm: &VirtualMachine,
    code: &Py<PyCode>,
    site: u32,
    offset: u32,
    destination: u32,
) -> PyResult<()> {
    fire_at(
        vm,
        MonitoringEvent::BranchLeft,
        code,
        site,
        offset,
        &[
            vm.ctx.new_int(offset).into(),
            vm.ctx.new_int(destination).into(),
        ],
    )
}

pub(crate) fn fire_branch_right(
    vm: &VirtualMachine,
    code: &Py<PyCode>,
    offset: u32,
    destination: u32,
) -> PyResult<()> {
    fire(
        vm,
        MonitoringEvent::BranchRight,
        code,
        offset,
        &[
            vm.ctx.new_int(offset).into(),
            vm.ctx.new_int(destination).into(),
        ],
    )
}

#[pymodule(sub, name = "sys.monitoring")]
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
        super::get_local_events(tool_id, &code, vm)
    }

    #[pyfunction]
    fn set_local_events(
        tool_id: i32,
        code: PyObjectRef,
        event_set: i32,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        super::set_local_events(tool_id, &code, event_set, vm)
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

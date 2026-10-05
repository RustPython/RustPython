use super::{
    INSTRUMENTED_EVENTS_COUNT, MonitoringEvent, TOOL_LIMIT, call_instrument, cannot_disable,
};
use crate::{
    AsObject, PyObject, PyObjectRef, PyResult, VirtualMachine, builtins::PyTuple,
    function::FuncArgs,
};

/// Native event state with the layout of CPython's PyMonitoringState.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct EventState {
    pub active: u8,
    pub opaque: u8,
}

/// Refresh native event states only when the global event configuration changes.
pub fn enter_scope(
    vm: &VirtualMachine,
    states: &mut [EventState],
    version: &mut u64,
    event_types: &[u8],
) -> PyResult<()> {
    if states.len() != event_types.len()
        || event_types.iter().any(|&event| {
            MonitoringEvent::from_id(event as usize)
                .is_none_or(|event| event == MonitoringEvent::Branch)
        })
    {
        return Err(vm.new_value_error("invalid monitoring scope events"));
    }
    let monitoring = vm.state.monitoring.lock();
    if *version == monitoring.native_version {
        return Ok(());
    }
    for (state, &event) in states.iter_mut().zip(event_types) {
        let check_event = match event {
            16 | 17 => MonitoringEvent::Call as u8,
            event => event,
        };
        state.active = 0;
        for tool in 0..TOOL_LIMIT {
            if monitoring.global_events[tool] & (1 << check_event) != 0 {
                state.active |= 1 << tool;
            }
        }
    }
    *version = monitoring.native_version;
    Ok(())
}

/// The current API does not keep a stack of scopes.
pub fn exit_scope() {}

/// Fire a native event using the interpreter's monitoring callbacks.
/// Exception events consume the raised exception during callbacks and restore it
/// on success. A callback error replaces it, as for the C exception indicator.
///
/// # Safety
/// `state` must remain valid throughout the call. It is accessed through a raw
/// pointer so a Python callback may refresh the same state with EnterScope.
pub unsafe fn fire_event(
    vm: &VirtualMachine,
    state: *mut EventState,
    codelike: &PyObject,
    offset: i32,
    event: u8,
    extra: &[PyObjectRef],
) -> PyResult<()> {
    struct NativeState(*mut EventState);
    impl EventStateAccess for NativeState {
        fn active(&self) -> u8 {
            unsafe { (*self.0).active }
        }

        fn disable(&self, tool: usize) {
            unsafe { (*self.0).active &= !(1 << tool) };
        }
    }
    fire_event_with_state(vm, &NativeState(state), codelike, offset, event, extra)
}

pub(crate) trait EventStateAccess {
    fn active(&self) -> u8;
    fn disable(&self, tool: usize);
}

// The native ABI and thread-safe test fixtures update their actual state after
// each callback, so subsequent callbacks can observe an earlier DISABLE.
pub(crate) fn fire_event_with_state(
    vm: &VirtualMachine,
    state: &impl EventStateAccess,
    codelike: &PyObject,
    offset: i32,
    event: u8,
    extra: &[PyObjectRef],
) -> PyResult<()> {
    let tools = state.active();
    if tools == 0 {
        return Ok(());
    }
    let event = MonitoringEvent::from_id(event as usize)
        .ok_or_else(|| vm.new_value_error("invalid monitoring event"))?;
    let exception = if matches!(
        event,
        MonitoringEvent::PyThrow
            | MonitoringEvent::Raise
            | MonitoringEvent::Reraise
            | MonitoringEvent::ExceptionHandled
            | MonitoringEvent::PyUnwind
            | MonitoringEvent::CRaise
    ) {
        Some(vm.take_raised_exception().ok_or_else(|| {
            vm.new_value_error(format!(
                "Firing event {} with no exception set",
                event as usize
            ))
        })?)
    } else {
        None
    };
    if offset < 0 {
        return Err(vm.new_value_error("offset must be non-negative"));
    }
    let mut args = vec![codelike.to_owned()];
    if event != MonitoringEvent::Line {
        args.push(vm.ctx.new_int(offset).into());
    }
    if let Some(exception) = &exception {
        args.push(exception.clone().into());
    } else if event == MonitoringEvent::StopIteration {
        let exception = match extra.first() {
            Some(value) if value.fast_isinstance(vm.ctx.exceptions.stop_iteration) => value.clone(),
            value => {
                // PyErr_SetObject expands tuple values into constructor args;
                // other values (including unrelated exceptions) stay one arg.
                let args = match value {
                    Some(value) if !vm.is_none(value) => value
                        .downcast_ref::<PyTuple>()
                        .map_or_else(|| vec![value.clone()], |tuple| tuple.as_slice().to_vec()),
                    _ => vec![],
                };
                vm.invoke_exception(vm.ctx.exceptions.stop_iteration, args)?
                    .into()
            }
        };
        args.push(exception);
    } else {
        args.extend_from_slice(extra);
    }
    let args = FuncArgs::from(args);
    // Match CPython's highest-tool-first dispatch. Fetch each callback as it is
    // reached, so an earlier callback may replace a later tool's callback.
    for tool in (0..TOOL_LIMIT).rev() {
        if tools & (1 << tool) == 0 {
            continue;
        }
        let callback = vm
            .state
            .monitoring
            .lock()
            .callbacks
            .get(&(tool, event as usize))
            .cloned();
        if let Some(callback) = callback
            && call_instrument(vm, event, &callback, args.clone())?
        {
            if event as usize >= INSTRUMENTED_EVENTS_COUNT {
                return Err(cannot_disable(vm, event, tool));
            }
            state.disable(tool);
        }
    }
    if let Some(exception) = exception {
        vm.set_raised_exception(Some(exception));
    }
    Ok(())
}

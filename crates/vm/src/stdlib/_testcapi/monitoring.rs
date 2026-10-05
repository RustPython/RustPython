use crate::{
    AsObject, Py, PyObject, PyPayload, PyRef, PyResult, TryFromObject, VirtualMachine,
    builtins::{PyBaseException, PyModule, PyStrRef, PyType},
    class::PyClassImpl,
    common::lock::PyMutex,
    function::{Callee, FuncArgs, ItemDoc, PyMethodDef, PyMethodFlags},
    stdlib::sys::monitoring_capi::{self, EventState, EventStateAccess},
    types::Constructor,
};

#[derive(Debug)]
struct CodeLikeState {
    events: Vec<EventState>,
    version: u64,
}

/// CodeLike objects
#[pyclass(module = "monitoring", name = "CodeLike")]
#[derive(Debug, PyPayload)]
struct CodeLike {
    state: PyMutex<CodeLikeState>,
}

#[pyclass(with(Constructor), flags(IMMUTABLETYPE))]
impl CodeLike {
    #[pyslot]
    fn slot_str(zelf: &PyObject, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        let zelf = zelf.try_downcast_ref::<Self>(vm)?;
        let state = zelf.state.lock();
        let mut parts = vec!["PyCodeLikeObject".to_owned()];
        parts.extend(
            state
                .events
                .iter()
                .map(|event| format!(" {}", event.active)),
        );
        Ok(vm.ctx.new_str(parts.join(": ")))
    }
}

impl Constructor for CodeLike {
    type Args = (i32,);

    fn py_new(_cls: &Py<PyType>, (length,): Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
        let length = usize::try_from(length)
            .map_err(|_| vm.new_value_error("event count must be non-negative"))?;
        let mut events = Vec::new();
        events
            .try_reserve_exact(length)
            .map_err(|_| vm.new_memory_error("cannot allocate monitoring states"))?;
        events.resize(length, EventState::default());
        Ok(Self {
            state: PyMutex::new(CodeLikeState { events, version: 0 }),
        })
    }
}

struct CodeLikeEvent<'a> {
    codelike: &'a CodeLike,
    index: usize,
}

impl EventStateAccess for CodeLikeEvent<'_> {
    fn active(&self) -> u8 {
        self.codelike.state.lock().events[self.index].active
    }

    fn disable(&self, tool: usize) {
        self.codelike.state.lock().events[self.index].active &= !(1 << tool);
    }
}

fn fire<const EVENT: u8>(vm: &VirtualMachine, mut args: FuncArgs, _callee: Callee) -> PyResult {
    // Like the CPython fixtures, offset indexes the codelike's state array as
    // well as supplying the instruction offset delivered to the callback.
    let extra_count = match EVENT {
        0 | 1 => 0,
        4 => 2,
        _ => 1,
    };
    if !args.kwargs.is_empty() || args.args.len() != 3 + extra_count {
        return Err(vm.new_type_error(format!("expected {} positional arguments", 2 + extra_count)));
    }
    args.args.remove(0);
    let codelike = PyRef::<CodeLike>::try_from_object(vm, args.args.remove(0))?;
    let offset = i32::try_from_object(vm, args.args.remove(0))?;
    let index =
        usize::try_from(offset).map_err(|_| vm.new_value_error("offset must be non-negative"))?;
    if index >= codelike.state.lock().events.len() {
        return Err(vm.new_index_error("monitoring state index out of range"));
    }
    let event_state = CodeLikeEvent {
        codelike: &codelike,
        index,
    };
    if matches!(EVENT, 11..=15 | 17) {
        let exception = args.args.remove(0);
        let exception = if vm.is_none(&exception) {
            None
        } else {
            Some(PyRef::<PyBaseException>::try_from_object(vm, exception)?)
        };
        vm.set_raised_exception(exception);
    } else if EVENT == 5 {
        let line = i32::try_from_object(vm, args.args.remove(0))?;
        args.args.push(vm.ctx.new_int(line).into());
    } else if EVENT == 10 && vm.is_none(&args.args[0]) {
        args.args.clear();
    }
    let result = monitoring_capi::fire_event_with_state(
        vm,
        &event_state,
        codelike.as_object(),
        offset,
        EVENT,
        &args.args,
    );
    let active = event_state.active();
    drop(vm.take_raised_exception());
    result?;
    Ok(vm.ctx.new_int(active).into())
}

fn enter_scope(vm: &VirtualMachine, mut args: FuncArgs, _callee: Callee) -> PyResult {
    if !args.kwargs.is_empty() || !(3..=4).contains(&args.args.len()) {
        return Err(vm.new_type_error("expected a code-like and one or two event types"));
    }
    args.args.remove(0);
    let codelike = PyRef::<CodeLike>::try_from_object(vm, args.args.remove(0))?;
    let events = args
        .args
        .into_iter()
        .map(|arg| u8::try_from_object(vm, arg))
        .collect::<PyResult<Vec<_>>>()?;
    let mut state = codelike.state.lock();
    let CodeLikeState {
        events: states,
        version,
    } = &mut *state;
    let states = states
        .get_mut(..events.len())
        .ok_or_else(|| vm.new_value_error("code-like has too few monitoring states"))?;
    monitoring_capi::enter_scope(vm, states, version, &events)?;
    Ok(vm.ctx.none())
}

fn exit_scope(vm: &VirtualMachine, args: FuncArgs, _callee: Callee) -> PyResult {
    if !args.kwargs.is_empty() || args.args.len() != 1 {
        return Err(vm.new_type_error("expected no arguments"));
    }
    monitoring_capi::exit_scope();
    Ok(vm.ctx.none())
}

pub(super) fn extend_module(vm: &VirtualMachine, module: &Py<PyModule>) -> PyResult<()> {
    module.set_attr("CodeLike", CodeLike::make_static_type(), vm)?;
    macro_rules! method {
        ($name:literal, $function:expr) => {
            PyMethodDef::new_raw_const($name, $function, PyMethodFlags::VARARGS, ItemDoc::NONE)
        };
    }
    const METHODS: &[PyMethodDef] = &[
        method!("fire_event_py_start", fire::<0>),
        method!("fire_event_py_resume", fire::<1>),
        method!("fire_event_py_return", fire::<2>),
        method!("fire_event_py_yield", fire::<3>),
        method!("fire_event_call", fire::<4>),
        method!("fire_event_line", fire::<5>),
        method!("fire_event_jump", fire::<7>),
        method!("fire_event_branch_left", fire::<8>),
        method!("fire_event_branch_right", fire::<9>),
        method!("fire_event_stop_iteration", fire::<10>),
        method!("fire_event_raise", fire::<11>),
        method!("fire_event_exception_handled", fire::<12>),
        method!("fire_event_py_unwind", fire::<13>),
        method!("fire_event_py_throw", fire::<14>),
        method!("fire_event_reraise", fire::<15>),
        method!("fire_event_c_return", fire::<16>),
        method!("fire_event_c_raise", fire::<17>),
        method!("monitoring_enter_scope", enter_scope),
        method!("monitoring_exit_scope", exit_scope),
    ];
    for method in METHODS {
        let function = method.build_bound_function(&vm.ctx, module.to_owned().into());
        drop(
            function
                .module
                .store(Some(vm.ctx.new_str("_testcapi").into())),
        );
        module.set_attr(method.name, function, vm)?;
    }
    Ok(())
}

//! Deterministic profiling driven by the VM's execution monitoring events.
// cspell:ignore pystart pythrow creturn

pub(crate) use _lsprof::module_def;

#[pymodule]
mod _lsprof {
    use crate::{
        AsObject, Py, PyObject, PyObjectRef, PyPayload, PyResult, VirtualMachine,
        builtins::{
            PyFloat, PyStr,
            builtin_func::{PyNativeFunction, PyNativeMethod},
            descriptor::PyMethodDescriptor,
        },
        common::lock::PyMutex,
        convert::ToPyObject,
        function::{OptionalArg, PyMethodDef},
        object::{Traverse, TraverseFn},
        stdlib::sys::monitoring::{self, MonitoringEvent},
        types::{Constructor, DefaultConstructor, Initializer, PyStructSequence},
    };
    use std::collections::HashMap;

    const TOOL_ID: i32 = 2;
    const CALLBACKS: &[(MonitoringEvent, &str)] = &[
        (MonitoringEvent::PyStart, "_pystart_callback"),
        (MonitoringEvent::PyResume, "_pystart_callback"),
        (MonitoringEvent::PyThrow, "_pythrow_callback"),
        (MonitoringEvent::PyReturn, "_pyreturn_callback"),
        (MonitoringEvent::PyYield, "_pyreturn_callback"),
        (MonitoringEvent::PyUnwind, "_pyreturn_callback"),
        (MonitoringEvent::Call, "_ccall_callback"),
        (MonitoringEvent::CReturn, "_creturn_callback"),
        (MonitoringEvent::CRaise, "_creturn_callback"),
    ];

    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    enum Key {
        Python(usize),
        Native(usize),
    }

    #[derive(Clone, Debug, Default)]
    struct Counts {
        total: i128,
        inline: i128,
        calls: usize,
        recursive: usize,
        depth: isize,
    }

    impl Counts {
        fn stop(&mut self, total: i128, inline: i128) {
            self.depth -= 1;
            if self.depth == 0 {
                self.total += total;
            } else {
                self.recursive += 1;
            }
            self.inline += inline;
            self.calls += 1;
        }
    }

    #[derive(Clone, Debug)]
    struct Entry {
        code: PyObjectRef,
        // Dynamically allocated native method definitions must outlive their keys.
        owner: Option<PyObjectRef>,
        counts: Counts,
        callees: HashMap<Key, Counts>,
    }

    #[derive(Debug)]
    struct Context {
        key: Key,
        start: i128,
        subtime: i128,
    }

    #[derive(Debug, Default)]
    struct State {
        entries: HashMap<Key, Entry>,
        stack: Vec<Context>,
        timer: Option<PyObjectRef>,
        unit: f64,
        enabled: bool,
        subcalls: bool,
        builtins: bool,
        in_timer: bool,
    }

    #[pyattr]
    #[pyclass(name = "Profiler", module = "_lsprof", traverse = "manual")]
    #[derive(Debug, Default, PyPayload)]
    struct Profiler {
        state: PyMutex<State>,
    }

    // SAFETY: Each owned Python reference is visited exactly once. Contexts
    // contain keys only; they do not own additional references to the entries.
    unsafe impl Traverse for Profiler {
        fn traverse(&self, tracer: &mut TraverseFn<'_>) {
            if let Some(state) = self.state.try_lock() {
                state.timer.traverse(tracer);
                #[expect(
                    clippy::iter_over_hash_type,
                    reason = "GC tracing visits every owned reference regardless of entry order"
                )]
                for entry in state.entries.values() {
                    entry.code.traverse(tracer);
                    entry.owner.traverse(tracer);
                }
            }
        }

        fn clear(&mut self, out: &mut Vec<PyObjectRef>) {
            let state = self.state.get_mut();
            out.extend(state.timer.take());
            #[expect(
                clippy::iter_over_hash_type,
                reason = "GC clearing transfers every owned reference; entry order is irrelevant"
            )]
            for (_, entry) in state.entries.drain() {
                out.push(entry.code);
                out.extend(entry.owner);
            }
            state.stack.clear();
        }
    }

    #[derive(FromArgs)]
    struct InitArgs {
        #[pyarg(any, optional)]
        timer: OptionalArg<PyObjectRef>,
        #[pyarg(any, default = 0.0)]
        timeunit: f64,
        #[pyarg(any, default = true)]
        subcalls: bool,
        #[pyarg(any, default = true)]
        builtins: bool,
    }

    #[derive(FromArgs)]
    struct EnableArgs {
        #[pyarg(any, default = true)]
        subcalls: bool,
        #[pyarg(any, default = true)]
        builtins: bool,
    }

    impl DefaultConstructor for Profiler {}

    impl Initializer for Profiler {
        type Args = InitArgs;

        fn init(zelf: &Py<Self>, args: InitArgs, _vm: &VirtualMachine) -> PyResult<()> {
            let old_timer = {
                let mut state = zelf.state.lock();
                state.unit = args.timeunit;
                state.subcalls = args.subcalls;
                state.builtins = args.builtins;
                core::mem::replace(&mut state.timer, args.timer.into_option())
            };
            drop(old_timer);
            Ok(())
        }
    }

    impl Profiler {
        fn timer(&self, vm: &VirtualMachine) -> i128 {
            let (timer, unit) = {
                let mut state = self.state.lock();
                state.in_timer = true;
                (state.timer.clone(), state.unit)
            };
            // No profiler lock can be held across a custom timer: it may call
            // getstats(), enable(), disable(), clear(), or a Python finalizer.
            let result = if let Some(timer) = &timer {
                timer.call((), vm).and_then(|value| {
                    if unit > 0.0 {
                        value.try_into_value::<i64>(vm).map(i128::from)
                    } else if let Some(value) = value.downcast_ref::<PyFloat>() {
                        let nanos = (value.to_f64() * 1_000_000_000.0).floor();
                        if nanos.is_nan() {
                            Err(vm.new_value_error("Invalid value NaN (not a number)"))
                        } else if !(i64::MIN as f64..-(i64::MIN as f64)).contains(&nanos) {
                            Err(vm
                                .new_overflow_error("timestamp too large to convert to C PyTime_t"))
                        } else {
                            Ok(nanos as i64 as i128)
                        }
                    } else {
                        let seconds = value.try_into_value::<i64>(vm)?;
                        seconds
                            .checked_mul(1_000_000_000)
                            .map(i128::from)
                            .ok_or_else(|| {
                                vm.new_overflow_error(
                                    "timestamp too large to convert to C PyTime_t",
                                )
                            })
                    }
                })
            } else {
                super::super::time::profiler_time(vm).map(|nanos| nanos as i128)
            };
            self.state.lock().in_timer = false;
            match result {
                Ok(time) => time,
                Err(error) => {
                    let message =
                        timer
                            .as_ref()
                            .and_then(|timer| timer.repr(vm).ok())
                            .map(|timer| {
                                format!("Exception ignored while calling _lsprof timer {timer}")
                            });
                    vm.run_unraisable(error, message, vm.ctx.none());
                    0
                }
            }
        }

        fn enter(
            &self,
            key: Key,
            code: PyObjectRef,
            owner: Option<PyObjectRef>,
            vm: &VirtualMachine,
        ) {
            {
                let mut state = self.state.lock();
                if state.in_timer {
                    return;
                }
                state
                    .entries
                    .entry(key)
                    .or_insert_with(|| Entry {
                        code,
                        owner,
                        counts: Counts::default(),
                        callees: HashMap::new(),
                    })
                    .counts
                    .depth += 1;
                let caller = state
                    .subcalls
                    .then(|| state.stack.last().map(|ctx| ctx.key))
                    .flatten();
                if let Some(caller) = caller {
                    state
                        .entries
                        .get_mut(&caller)
                        .unwrap()
                        .callees
                        .entry(key)
                        .or_default()
                        .depth += 1;
                }
                state.stack.push(Context {
                    key,
                    start: 0,
                    subtime: 0,
                });
            }
            let start = self.timer(vm);
            // clear()/disable() are rejected inside the timer, so the context
            // remains valid even when the timer executes arbitrary Python.
            if let Some(context) = self.state.lock().stack.last_mut() {
                context.start = start;
            }
        }

        fn leave(&self, key: Key, vm: &VirtualMachine) {
            {
                let state = self.state.lock();
                if state.in_timer || state.stack.last().is_none_or(|ctx| ctx.key != key) {
                    return;
                }
            }
            let now = self.timer(vm);
            let mut state = self.state.lock();
            let Some(context) = state.stack.pop() else {
                return;
            };
            let total = now - context.start;
            let inline = total - context.subtime;
            if let Some(parent) = state.stack.last_mut() {
                parent.subtime += total;
            }
            if let Some(entry) = state.entries.get_mut(&context.key) {
                entry.counts.stop(total, inline);
            }
            let caller = state
                .subcalls
                .then(|| state.stack.last().map(|ctx| ctx.key))
                .flatten();
            if let Some(caller) = caller
                && let Some(counts) = state
                    .entries
                    .get_mut(&caller)
                    .and_then(|entry| entry.callees.get_mut(&context.key))
            {
                counts.stop(total, inline);
            }
        }
    }

    // Builtin keys use the method definition, never the transient bound method
    // object or its receiver. This both aggregates calls and avoids retaining
    // every receiver that the application has used while profiling.
    enum NativeCall<'a> {
        Function(&'a PyNativeFunction),
        Descriptor(&'a Py<PyMethodDescriptor>, &'a PyObject),
    }

    impl NativeCall<'_> {
        fn key(&self) -> Key {
            let method = match self {
                Self::Function(function) => function.value,
                Self::Descriptor(descriptor, _) => descriptor.method,
            };
            Key::Native(method as *const PyMethodDef as usize)
        }

        fn owner(&self) -> Option<PyObjectRef> {
            match self {
                Self::Function(function) => function._method_def_owner.clone(),
                Self::Descriptor(descriptor, _) => descriptor._method_def_owner.clone(),
            }
        }

        fn label(&self, vm: &VirtualMachine) -> PyObjectRef {
            let function = match self {
                Self::Descriptor(descriptor, receiver) => {
                    let name = descriptor.method.name;
                    if let Some(attribute) = receiver.class().get_attr(vm.ctx.intern_str(name))
                        && let Ok(label) = attribute.repr(vm)
                    {
                        return label.into();
                    }
                    return vm.ctx.new_str(format!("<built-in method {name}>")).into();
                }
                Self::Function(function) => function,
            };
            let name = function.value.name;
            let module = function
                .module
                .deref()
                .and_then(|module| module.downcast_ref::<PyStr>())
                .map(|module| module.to_string_lossy().into_owned());
            let label = if let Some(receiver) = function.get_self() {
                if let Some(descriptor) = receiver.class().get_attr(vm.ctx.intern_str(name))
                    && let Ok(label) = descriptor.repr(vm)
                {
                    return label.into();
                }
                if let Some(module) = module {
                    format!("<built-in method {module}.{name}>")
                } else {
                    format!("<built-in method {name}>")
                }
            } else {
                match module {
                    Some(module) if module != "builtins" => format!("<{module}.{name}>"),
                    _ => format!("<{name}>"),
                }
            };
            vm.ctx.new_str(label).into()
        }
    }

    fn native_call<'a>(
        callable: &'a PyObject,
        arg: &'a PyObject,
        vm: &VirtualMachine,
    ) -> Option<NativeCall<'a>> {
        if let Some(descriptor) = callable.downcast_ref::<PyMethodDescriptor>() {
            if arg.is(&monitoring::get_missing(vm)) || !arg.fast_isinstance(descriptor.common.typ) {
                return None;
            }
            return Some(NativeCall::Descriptor(descriptor, arg));
        }
        let function = if let Some(function) = callable.downcast_ref::<PyNativeFunction>() {
            &**function
        } else {
            let method = callable.downcast_ref::<PyNativeMethod>()?;
            &method.func
        };
        Some(NativeCall::Function(function))
    }

    #[pyclass(with(Constructor, Initializer), flags(BASETYPE))]
    impl Profiler {
        #[pymethod]
        fn enable(zelf: &Py<Self>, args: EnableArgs, vm: &VirtualMachine) -> PyResult<()> {
            {
                let mut state = zelf.state.lock();
                state.subcalls = args.subcalls;
                state.builtins = args.builtins;
            }
            let monitoring = vm.sys_module.get_attr("monitoring", vm)?;
            vm.call_method(&monitoring, "use_tool_id", (TOOL_ID, "cProfile"))?;
            let mut events = 0;
            for &(event, method) in CALLBACKS {
                let callback = zelf.as_object().get_attr(method, vm)?;
                vm.call_method(
                    &monitoring,
                    "register_callback",
                    (TOOL_ID, event.mask(), callback),
                )?;
                events |= event.mask();
            }
            vm.call_method(&monitoring, "set_events", (TOOL_ID, events))?;
            zelf.state.lock().enabled = true;
            Ok(())
        }

        #[pymethod]
        fn disable(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<()> {
            {
                let state = zelf.state.lock();
                if state.in_timer {
                    return Err(vm.new_runtime_error("cannot disable profiler in external timer"));
                }
                if !state.enabled {
                    return Ok(());
                }
            }
            let monitoring = vm.sys_module.get_attr("monitoring", vm)?;
            for &(event, _) in CALLBACKS {
                vm.call_method(
                    &monitoring,
                    "register_callback",
                    (TOOL_ID, event.mask(), vm.ctx.none()),
                )?;
            }
            vm.call_method(&monitoring, "set_events", (TOOL_ID, 0))?;
            vm.call_method(&monitoring, "free_tool_id", (TOOL_ID,))?;
            zelf.state.lock().enabled = false;
            loop {
                let key = zelf.state.lock().stack.last().map(|ctx| ctx.key);
                let Some(key) = key else {
                    break;
                };
                zelf.leave(key, vm);
            }
            Ok(())
        }

        #[pymethod]
        fn clear(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<()> {
            let entries = {
                let mut state = zelf.state.lock();
                if state.in_timer {
                    return Err(vm.new_runtime_error("cannot clear profiler in external timer"));
                }
                state.stack.clear();
                core::mem::take(&mut state.entries)
            };
            drop(entries);
            Ok(())
        }

        #[pymethod]
        fn getstats(zelf: &Py<Self>, vm: &VirtualMachine) -> Vec<PyObjectRef> {
            let (entries, factor) = {
                let state = zelf.state.lock();
                let factor = if state.timer.is_some() && state.unit != 0.0 {
                    state.unit
                } else {
                    1e-9
                };
                (state.entries.clone(), factor)
            };
            entries
                .values()
                .filter(|entry| entry.counts.calls != 0)
                .map(|entry| {
                    let calls = if entry.callees.is_empty() {
                        vm.ctx.none()
                    } else {
                        vm.ctx
                            .new_list(
                                entry
                                    .callees
                                    .iter()
                                    .map(|(key, counts)| {
                                        SubentryData {
                                            code: entries[key].code.clone(),
                                            callcount: counts.calls,
                                            reccallcount: counts.recursive,
                                            totaltime: counts.total as f64 * factor,
                                            inlinetime: counts.inline as f64 * factor,
                                        }
                                        .to_pyobject(vm)
                                    })
                                    .collect(),
                            )
                            .into()
                    };
                    EntryData {
                        code: entry.code.clone(),
                        callcount: entry.counts.calls,
                        reccallcount: entry.counts.recursive,
                        totaltime: entry.counts.total as f64 * factor,
                        inlinetime: entry.counts.inline as f64 * factor,
                        calls,
                    }
                    .to_pyobject(vm)
                })
                .collect()
        }

        #[pymethod]
        fn _pystart_callback(
            zelf: &Py<Self>,
            code: PyObjectRef,
            _offset: PyObjectRef,
            vm: &VirtualMachine,
        ) {
            zelf.enter(Key::Python(code.get_id()), code, None, vm);
        }

        #[pymethod]
        fn _pythrow_callback(
            zelf: &Py<Self>,
            code: PyObjectRef,
            _offset: PyObjectRef,
            _exception: PyObjectRef,
            vm: &VirtualMachine,
        ) {
            zelf.enter(Key::Python(code.get_id()), code, None, vm);
        }

        #[pymethod]
        fn _pyreturn_callback(
            zelf: &Py<Self>,
            code: PyObjectRef,
            _offset: PyObjectRef,
            _retval: PyObjectRef,
            vm: &VirtualMachine,
        ) {
            zelf.leave(Key::Python(code.get_id()), vm);
        }

        #[pymethod]
        fn _ccall_callback(
            zelf: &Py<Self>,
            _code: PyObjectRef,
            _offset: PyObjectRef,
            callable: PyObjectRef,
            arg: PyObjectRef,
            vm: &VirtualMachine,
        ) {
            let builtins = zelf.state.lock().builtins;
            if builtins && let Some(call) = native_call(&callable, &arg, vm) {
                let key = call.key();
                let existing = zelf
                    .state
                    .lock()
                    .entries
                    .get(&key)
                    .map(|entry| entry.code.clone());
                let code = existing.unwrap_or_else(|| call.label(vm));
                zelf.enter(key, code, call.owner(), vm);
            }
        }

        #[pymethod]
        fn _creturn_callback(
            zelf: &Py<Self>,
            _code: PyObjectRef,
            _offset: PyObjectRef,
            callable: PyObjectRef,
            arg: PyObjectRef,
            vm: &VirtualMachine,
        ) {
            let builtins = zelf.state.lock().builtins;
            if builtins && let Some(call) = native_call(&callable, &arg, vm) {
                zelf.leave(call.key(), vm);
            }
        }
    }

    #[pystruct_sequence_data]
    struct EntryData {
        code: PyObjectRef,
        callcount: usize,
        reccallcount: usize,
        totaltime: f64,
        inlinetime: f64,
        calls: PyObjectRef,
    }

    #[pyattr]
    #[pystruct_sequence(name = "profiler_entry", module = "_lsprof", data = "EntryData")]
    struct ProfilerEntry;

    #[pyclass(with(PyStructSequence))]
    impl ProfilerEntry {}

    #[pystruct_sequence_data]
    struct SubentryData {
        code: PyObjectRef,
        callcount: usize,
        reccallcount: usize,
        totaltime: f64,
        inlinetime: f64,
    }

    #[pyattr]
    #[pystruct_sequence(name = "profiler_subentry", module = "_lsprof", data = "SubentryData")]
    struct ProfilerSubentry;

    #[pyclass(with(PyStructSequence))]
    impl ProfilerSubentry {}
}

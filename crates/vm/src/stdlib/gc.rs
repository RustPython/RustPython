pub(crate) use gc::module_def;

#[pymodule]
mod gc {
    use crate::{
        AsObject, PyObject, PyObjectRef, PyResult, TryFromBorrowedObject, VirtualMachine,
        builtins::{PyListRef, PyTupleRef},
        function::{ArgCInt, FuncArgs, NameObjs, OptionalArg, PosArgs},
        gc_state,
    };

    // Debug flag constants
    #[pyattr]
    const DEBUG_STATS: u32 = gc_state::GcDebugFlags::STATS.bits();
    #[pyattr]
    const DEBUG_COLLECTABLE: u32 = gc_state::GcDebugFlags::COLLECTABLE.bits();
    #[pyattr]
    const DEBUG_UNCOLLECTABLE: u32 = gc_state::GcDebugFlags::UNCOLLECTABLE.bits();
    #[pyattr]
    const DEBUG_SAVEALL: u32 = gc_state::GcDebugFlags::SAVEALL.bits();
    #[pyattr]
    const DEBUG_LEAK: u32 = gc_state::GcDebugFlags::LEAK.bits();

    #[pyfunction]
    fn enable(vm: &VirtualMachine) {
        vm.state.gc.enable();
    }

    #[pyfunction]
    fn disable(vm: &VirtualMachine) {
        vm.state.gc.disable();
    }

    #[pyfunction]
    fn isenabled(vm: &VirtualMachine) -> bool {
        vm.state.gc.is_enabled()
    }

    /// Run a garbage collection. Returns the number of unreachable objects found.
    #[derive(FromArgs)]
    struct CollectArgs {
        #[pyarg(any, default = 2)]
        generation: ArgCInt,
    }

    #[pyfunction]
    fn collect(args: CollectArgs, vm: &VirtualMachine) -> PyResult<i32> {
        let generation_num = args.generation.into_primitive();
        if !(0..=2).contains(&generation_num) {
            return Err(vm.new_value_error("invalid generation"));
        }

        let gc = &vm.state.gc;
        let result = gc.collect_force_with_callbacks(
            generation_num as usize,
            || invoke_callbacks(vm, "start", generation_num as usize, &Default::default()),
            |result| {
                // Publish DEBUG_SAVEALL objects before the stop callbacks.
                {
                    let mut state_garbage = gc.garbage.lock();
                    if !state_garbage.is_empty() {
                        let mut garbage_vec = gc.py_garbage.borrow_vec_mut();
                        for obj in state_garbage.drain(..) {
                            garbage_vec.push(obj);
                        }
                    }
                }
                invoke_callbacks(vm, "stop", generation_num as usize, result);
            },
        );

        Ok((result.collected + result.uncollectable) as i32)
    }

    #[pyfunction]
    fn get_threshold(vm: &VirtualMachine) -> PyObjectRef {
        let (t0, t1, t2) = vm.state.gc.get_threshold();
        vm.ctx
            .new_tuple(vec![
                vm.ctx.new_int(t0).into(),
                vm.ctx.new_int(t1).into(),
                vm.ctx.new_int(t2).into(),
            ])
            .into()
    }

    // set_threshold uses the legacy "i" converter, unlike Clinic's
    // PyLong_AsInt converters for collect and set_debug.
    struct Threshold(i32);

    impl<'a> TryFromBorrowedObject<'a> for Threshold {
        fn try_from_borrowed_object(vm: &VirtualMachine, obj: &'a PyObject) -> PyResult<Self> {
            let index = obj.try_index(vm)?;
            i32::try_from(index.as_bigint()).map(Self).map_err(|_| {
                let message = if core::ffi::c_long::try_from(index.as_bigint()).is_err() {
                    "Python int too large to convert to C long"
                } else if index.as_bigint().sign() == malachite_bigint::Sign::Minus {
                    "signed integer is less than minimum"
                } else {
                    "signed integer is greater than maximum"
                };
                vm.new_overflow_error(message)
            })
        }
    }

    #[pyfunction]
    fn set_threshold(args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
        // The generated optional-group wrapper checks the argument count
        // before running any of its sequential integer conversions.
        if !args.kwargs.is_empty() {
            return Err(vm.new_type_error("set_threshold() takes no keyword arguments"));
        }
        if !(1..=3).contains(&args.args.len()) {
            return Err(vm.new_type_error("gc.set_threshold requires 1 to 3 arguments"));
        }
        let (threshold0, threshold1, threshold2) =
            args.bind::<(Threshold, OptionalArg<Threshold>, OptionalArg<Threshold>)>(vm)?;
        vm.state.gc.set_threshold(
            threshold0.0,
            threshold1.into_option().map(|value| value.0),
            threshold2.into_option().map(|value| value.0),
        );
        Ok(())
    }

    #[pyfunction]
    fn get_count(vm: &VirtualMachine) -> PyObjectRef {
        let (c0, c1, c2) = gc_state::gc_state().get_count();
        vm.ctx
            .new_tuple(vec![
                vm.ctx.new_int(c0).into(),
                vm.ctx.new_int(c1).into(),
                vm.ctx.new_int(c2).into(),
            ])
            .into()
    }

    #[pyfunction]
    fn get_debug(vm: &VirtualMachine) -> i32 {
        vm.state.gc.get_debug().bits() as i32
    }

    #[pyfunction]
    fn set_debug(flags: ArgCInt, vm: &VirtualMachine) {
        vm.state
            .gc
            .set_debug(gc_state::GcDebugFlags::from_bits_retain(
                flags.into_primitive() as u32,
            ));
    }

    #[pyfunction]
    fn get_stats(vm: &VirtualMachine) -> PyResult<PyListRef> {
        let stats = vm.state.gc.get_stats();
        let mut result = Vec::with_capacity(3);

        for stat in &stats {
            let dict = vm.ctx.new_dict();
            dict.set_item("collections", vm.ctx.new_int(stat.collections).into(), vm)?;
            dict.set_item("collected", vm.ctx.new_int(stat.collected).into(), vm)?;
            dict.set_item(
                "uncollectable",
                vm.ctx.new_int(stat.uncollectable).into(),
                vm,
            )?;
            dict.set_item("candidates", vm.ctx.new_int(stat.candidates).into(), vm)?;
            dict.set_item("duration", vm.ctx.new_float(stat.duration).into(), vm)?;
            result.push(dict.into());
        }

        Ok(vm.ctx.new_list(result))
    }

    // Clinic's optional Py_ssize_t conversion accepts __index__, but reports
    // missing-index and overflow errors using the original argument's type.
    struct Generation(isize);

    impl<'a> TryFromBorrowedObject<'a> for Generation {
        fn try_from_borrowed_object(vm: &VirtualMachine, obj: &'a PyObject) -> PyResult<Self> {
            let index = obj.try_index_opt(vm).ok_or_else(|| {
                let name = obj.class().slot_name();
                let name = &name[..name.floor_char_boundary(name.len().min(200))];
                vm.new_type_error(format!("argument should be integer or None, not '{name}'"))
            })??;
            isize::try_from(index.as_bigint()).map(Self).map_err(|_| {
                let name = obj.class().slot_name();
                let name = &name[..name.floor_char_boundary(name.len().min(200))];
                vm.new_overflow_error(format!("cannot fit '{name}' into an index-sized integer"))
            })
        }
    }

    /// Return the list of objects tracked by the collector.
    #[derive(FromArgs)]
    struct GetObjectsArgs {
        #[pyarg(any, optional)]
        generation: Option<Generation>,
    }

    #[pyfunction]
    fn get_objects(args: GetObjectsArgs, vm: &VirtualMachine) -> PyResult<PyListRef> {
        let generation = args.generation.map_or(-1, |generation| generation.0);
        vm.audit("gc.get_objects", || (generation,))?;
        if generation >= 3 {
            return Err(vm.new_value_error(
                "generation parameter must be less than the number of available generations (3)",
            ));
        }
        if generation < -1 {
            return Err(vm.new_value_error("generation parameter cannot be negative"));
        }
        let generation = (generation != -1).then_some(generation as i32);
        let objects = vm.state.gc.get_objects_for_introspection(generation);
        Ok(vm.ctx.new_list(objects))
    }

    #[pyfunction]
    fn get_referents(
        objs: PosArgs<PyObjectRef, NameObjs>,
        vm: &VirtualMachine,
    ) -> PyResult<PyListRef> {
        vm.audit("gc.get_referents", || {
            (vm.ctx.new_tuple(objs.as_ref().to_vec()),)
        })?;
        let mut result = Vec::new();

        for obj in objs.iter() {
            // Use the gc_get_referents method to get references
            result.extend(obj.gc_get_referents());
        }

        Ok(vm.ctx.new_list(result))
    }

    #[pyfunction]
    fn get_referrers(
        objs: PosArgs<PyObjectRef, NameObjs>,
        vm: &VirtualMachine,
    ) -> PyResult<PyListRef> {
        use std::collections::HashSet;

        // CPython excludes its varargs tuple from the referrer scan. Build and
        // retain that tuple only when an audit hook actually needs it.
        let mut audit_objects: Option<PyTupleRef> = None;
        vm.audit("gc.get_referrers", || {
            let tuple = vm.ctx.new_tuple(objs.as_ref().to_vec());
            audit_objects = Some(tuple.clone());
            (tuple,)
        })?;
        let audit_objects_id = audit_objects.as_ref().map(|tuple| tuple.get_id());

        // Build a set of target object pointers for fast lookup
        let targets: HashSet<usize> = objs
            .iter()
            .map(|obj| obj.as_ref() as *const crate::PyObject as usize)
            .collect();

        // Collect pointers of frames currently on the execution stack.
        // In CPython, executing frames (_PyInterpreterFrame) are not GC-tracked
        // PyObjects, so they never appear in get_referrers results. Since
        // RustPython materializes every frame as a PyObject, we must exclude
        // them manually to match the expected behavior.
        let mut stack_frames: HashSet<usize> = HashSet::new();
        crate::frame::for_each_current_frame(|frame| {
            let obj: &crate::PyObject = frame.as_ref();
            stack_frames.insert(obj as *const crate::PyObject as usize);
        });

        let refers_to_target = |obj: &crate::PyObject| -> bool {
            let referent_ptrs = unsafe { obj.gc_get_referent_ptrs() };
            referent_ptrs
                .iter()
                .any(|child_ptr| targets.contains(&(child_ptr.as_ptr() as usize)))
        };

        let mut result = Vec::new();

        // Scan all tracked objects across all generations
        let all_objects = vm.state.gc.get_objects_for_introspection(None);
        for obj in all_objects {
            let obj_ptr = obj.as_ref() as *const crate::PyObject as usize;
            // Generator/coroutine frames are embedded in their owner, so
            // locals show up as referents of the owner rather than the frame.
            if let Some(frame) = obj.downcast_ref::<crate::frame::FrameObject>()
                && let Some(owner) = frame.iframe().generator.to_owned()
            {
                if refers_to_target(obj.as_ref()) {
                    result.push(owner);
                }
                continue;
            }
            if Some(obj_ptr) == audit_objects_id || stack_frames.contains(&obj_ptr) {
                continue;
            }
            if refers_to_target(obj.as_ref()) {
                result.push(obj);
            }
        }

        Ok(vm.ctx.new_list(result))
    }

    #[pyfunction]
    fn is_tracked(obj: PyObjectRef) -> bool {
        // An object is tracked if it has IS_TRACE = true (has a trace function)
        obj.is_gc_tracked()
    }

    #[pyfunction]
    fn is_finalized(obj: PyObjectRef) -> bool {
        obj.gc_finalized()
    }

    #[pyfunction]
    fn freeze(vm: &VirtualMachine) {
        vm.state.gc.freeze();
    }

    #[pyfunction]
    fn unfreeze(vm: &VirtualMachine) {
        vm.state.gc.unfreeze();
    }

    #[pyfunction]
    fn get_freeze_count() -> usize {
        gc_state::gc_state().get_freeze_count()
    }

    // gc.garbage - list of uncollectable objects
    #[pyattr]
    fn garbage(vm: &VirtualMachine) -> PyListRef {
        vm.state.gc.py_garbage.clone()
    }

    // gc.callbacks - list of callbacks to be invoked
    #[pyattr]
    fn callbacks(vm: &VirtualMachine) -> PyListRef {
        vm.state.gc.py_callbacks.clone()
    }

    /// Helper function to invoke GC callbacks
    fn invoke_callbacks(
        vm: &VirtualMachine,
        phase: &str,
        generation: usize,
        result: &gc_state::CollectResult,
    ) {
        let callbacks_list = &vm.state.gc.py_callbacks;
        if callbacks_list.borrow_vec().is_empty() {
            return;
        }

        let phase_str: PyObjectRef = vm.ctx.new_str(phase).into();
        let info = vm.ctx.new_dict();
        let _ = info.set_item("generation", vm.ctx.new_int(generation).into(), vm);
        let _ = info.set_item("collected", vm.ctx.new_int(result.collected).into(), vm);
        let _ = info.set_item(
            "uncollectable",
            vm.ctx.new_int(result.uncollectable).into(),
            vm,
        );
        let _ = info.set_item("candidates", vm.ctx.new_int(result.candidates).into(), vm);
        let _ = info.set_item("duration", vm.ctx.new_float(result.duration).into(), vm);

        let mut index = 0;
        loop {
            let Some(callback) = callbacks_list.borrow_vec().get(index).cloned() else {
                break;
            };
            index += 1;
            if let Err(error) = callback.call((phase_str.clone(), info.clone()), vm) {
                let message = callback
                    .repr(vm)
                    .ok()
                    .map(|repr| format!("Exception ignored while calling GC callback {repr}"));
                vm.run_unraisable(error, message, vm.ctx.none());
            }
        }
    }
}

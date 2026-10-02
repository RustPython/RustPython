//! Snapshot, analysis, finalization and commit of a collection.

use super::*;

type WeakCallbacks = Vec<(crate::PyRef<crate::object::PyWeak>, PyObjectRef)>;

fn promote(objects: &[PyObjectRef], generation: usize) {
    // Snapshot order groups objects by heap. Partitioning preserves that order,
    // so promotion needs neither a heap lookup table nor one lock per object.
    let mut start = 0;
    while start < objects.len() {
        let heap = objects[start].gc_handle().heap().unwrap();
        let mut end = start + 1;
        while end < objects.len()
            && objects[end]
                .gc_handle()
                .heap()
                .is_some_and(|h| core::ptr::eq(h, heap))
        {
            end += 1;
        }
        heap.promote(&objects[start..end], generation);
        start = end;
    }
}

fn clear_weakrefs(objects: &[PyObjectRef], callbacks: &mut WeakCallbacks) {
    // Weakrefs within the same unreachable graph must not run callbacks that
    // could resurrect that graph. A reachable weakref still gets its callback.
    for obj in objects {
        obj.start_gc_index(0);
    }
    scopeguard::defer! {
        for obj in objects {
            obj.end_gc_refs();
        }
    }
    for obj in objects {
        callbacks.extend(obj.gc_clear_weakrefs_collect_callbacks());
    }
}

fn run_weak_callbacks(callbacks: WeakCallbacks) {
    for (wr, callback) in callbacks {
        if let Some(Err(error)) =
            crate::vm::thread::with_vm(&callback, |vm| callback.call((wr.clone(),), vm))
        {
            crate::vm::thread::with_vm(&callback, |vm| {
                vm.run_unraisable(
                    error.clone(),
                    Some("weakref callback".to_owned()),
                    callback.clone(),
                );
            });
        }
    }
}

fn callbacks(gc: &GcInterpreterState, phase: &str, generation: usize, result: &CollectResult) {
    crate::vm::thread::try_with_current_vm(|vm| {
        if core::ptr::eq(gc, &vm.state.gc) {
            crate::stdlib::gc::invoke_callbacks(vm, phase, generation, result);
        }
    });
}

/// The caller holds stop-the-world. Only references in `objects` count as
/// collector pins; references from outside this candidate set are live roots.
pub(super) unsafe fn revalidate(objects: Vec<PyObjectRef>) -> (Vec<PyObjectRef>, Vec<PyObjectRef>) {
    // Another thread may freeze the heap while snapshot analysis runs. Those
    // objects are roots, and their edges must not be subtracted as candidates.
    let (mut retained, objects): (Vec<_>, Vec<_>) = objects
        .into_iter()
        .partition(|obj| obj.gc_generation() >= GC_PERMANENT);
    let mut graph = unsafe { Graph::capture(objects) };
    graph.analyze();
    let (live, dead) = graph.partition();
    retained.extend(live);
    (retained, dead)
}

impl GcState {
    pub(super) fn collect_inner(
        &self,
        gc: &GcInterpreterState,
        generation: Option<usize>,
        force: bool,
    ) -> CollectResult {
        if gc
            .heap
            .owner
            .is_some_and(crate::vm::native_types::initializing)
        {
            return CollectResult::default();
        }
        if !force && !gc.is_enabled() {
            #[cfg(feature = "threading")]
            gc.scheduled.store(false, Ordering::Relaxed);
            return CollectResult::default();
        }
        let Some(_collecting) = gc.collecting.try_lock() else {
            return CollectResult::default();
        };
        #[cfg(all(unix, feature = "threading"))]
        let _collecting_thread = {
            gc.collecting_thread
                .store(crate::stdlib::_thread::get_ident(), Ordering::Relaxed);
            scopeguard::guard(&gc.collecting_thread, |thread| {
                thread.store(0, Ordering::Relaxed)
            })
        };
        #[cfg(feature = "threading")]
        gc.scheduled.store(false, Ordering::Relaxed);
        let generation = {
            let mut schedule = gc.schedule.lock();
            let generation = match generation {
                Some(g) => g.min(2),
                None => {
                    let thresholds = gc.get_threshold();
                    if thresholds.0 == 0 {
                        return CollectResult::default();
                    }
                    schedule.generation(thresholds)
                }
            };
            schedule.start_collection(generation);
            generation
        };
        let start_time = cfg_select! {
            target_arch = "wasm32" => (),
            _ => std::time::Instant::now(),
        };
        // Reset only this interpreter, before callbacks can allocate. Never
        // erase another VM's pressure or requests made during this collection.
        gc.heap.allocations.store(0, Ordering::Relaxed);
        callbacks(gc, "start", generation, &CollectResult::default());
        if let Some(state) = gc
            .heap
            .owner
            .and_then(crate::vm::runtime::lookup_interpreter)
        {
            state.type_cache.clear();
        }
        #[cfg(feature = "threading")]
        {
            gc.qsbr().process();
            crate::object::qsbr::shared().process();
        }

        let (mut graph, candidates) = {
            let _world = CollectStopTheWorld::new(gc);
            let objects = gc.heap.snapshot(Some(generation));
            let candidates = objects.len();
            (unsafe { Graph::capture(objects) }, candidates)
        };
        // No Python object reads: the expensive graph walk does not stop peers.
        graph.analyze();
        let (live, mut suspects) = graph.partition();
        gc.schedule.lock().record_survivors(generation, live.len());
        promote(&live, generation);
        drop(live);

        if !suspects.is_empty() {
            let mut world = CollectStopTheWorld::new(gc);
            let (live, dead) = unsafe { revalidate(suspects) };
            let mut weak_callbacks = Vec::new();
            clear_weakrefs(&dead, &mut weak_callbacks);
            world.restart();
            drop(world);
            promote(&live, generation);
            drop(live);
            run_weak_callbacks(weak_callbacks);
            for obj in &dead {
                obj.try_call_finalizer();
            }
            suspects = dead;
        }

        let debug = gc.get_debug();
        let mut collected = 0;
        if !suspects.is_empty() {
            let mut world = CollectStopTheWorld::new(gc);
            let (mut resurrected, mut dead) = unsafe { revalidate(suspects) };
            let mut weak_callbacks = Vec::new();
            // Finalizers may have published fresh weakrefs. Invalidate those
            // before exposing untracked objects to concurrent weakref upgrades.
            clear_weakrefs(&dead, &mut weak_callbacks);
            if !weak_callbacks.is_empty() {
                // Callback references themselves are now held externally by
                // this collector. Account for those pins before committing.
                let (live, remaining) = unsafe { revalidate(dead) };
                resurrected.extend(live);
                dead = remaining;
            }

            // Match Python's object count: the separately allocated instance
            // dictionary is storage of its instance, not another Python object.
            for obj in &dead {
                obj.start_gc_index(0);
            }
            let instance_dicts = dead
                .iter()
                .filter(|obj| {
                    obj.dict()
                        .is_some_and(|dict| dict.as_object().is_gc_collecting())
                })
                .count();
            collected = dead.len().saturating_sub(instance_dicts);
            for obj in &dead {
                obj.end_gc_refs();
            }

            if !debug.contains(GcDebugFlags::SAVEALL) {
                // No external strong roots remain. Once weakrefs and heap
                // membership are removed, no new reference can be acquired via
                // Python introspection after the world resumes.
                for obj in &dead {
                    // A dead weakref can still be found through its live
                    // referent's list. Remove that ingress before restarting.
                    obj.detach_weakref();
                    if let Some(heap) = obj.gc_handle().heap() {
                        heap.untrack(obj);
                    }
                }
            }
            world.restart();
            drop(world);
            promote(&resurrected, generation);
            drop(resurrected);
            run_weak_callbacks(weak_callbacks);
            if debug.contains(GcDebugFlags::SAVEALL) {
                promote(&dead, generation);
                gc.garbage.lock().extend(dead);
            } else {
                // tp_clear may release children and execute finalizers; it must
                // run after restarting other interpreters and releasing locks.
                rustpython_common::refcount::with_deferred_drops(|| {
                    for obj in &dead {
                        if debug.contains(GcDebugFlags::COLLECTABLE) {
                            eprintln!(
                                "gc: collectable <{} {:p}>",
                                obj.class().name(),
                                obj.as_ref()
                            );
                        }
                        if obj.gc_has_clear() {
                            drop(unsafe { obj.gc_clear() });
                        }
                    }
                    drop(dead);
                });
            }
        }
        if generation == 2 {
            gc.heap.trim();
        }
        let result = CollectResult {
            collected,
            uncollectable: 0,
            candidates,
            duration: elapsed_secs(start_time),
        };
        gc.generations[generation].update_stats(collected, 0, candidates, result.duration);
        if debug.contains(GcDebugFlags::STATS) {
            eprintln!(
                "gc: generation {generation}, {candidates} candidates, {collected} collected"
            );
        }
        let saved = core::mem::take(&mut *gc.garbage.lock());
        if !saved.is_empty()
            && let Some(garbage) = gc.garbage_list()
        {
            garbage.borrow_vec_mut().extend(saved);
        }
        callbacks(gc, "stop", generation, &result);
        // Retired heaps need a full graph scan. Keep that work out of young
        // collections; teardown also sweeps them when an interpreter retires.
        if generation == 2 {
            self.collect_retired();
        }
        result
    }
}

impl GcState {
    /// No interpreter can execute in these heaps. Mask callbacks for the whole
    /// sweep, including snapshot pins and unwinding: releasing a pin can itself
    /// become the last reference and invoke native destruction.
    pub(super) fn collect_retired(&self) {
        crate::vm::thread::native_sweep(|| {
            #[cfg(all(unix, feature = "threading"))]
            let _retirement = crate::vm::fork::native_phase();
            let Some(_collecting) = self.collecting.try_lock() else {
                return;
            };
            for heap in self.retired_heaps() {
                let _owner = AllocationScope::new(heap.clone());
                heap.unfreeze();
                let mut graph = unsafe { Graph::capture(heap.snapshot(None)) };
                graph.analyze();
                let (live, dead) = graph.partition();
                drop(live);
                let mut callbacks = Vec::new();
                clear_weakrefs(&dead, &mut callbacks);
                for obj in &dead {
                    obj.detach_weakref();
                    heap.untrack(obj);
                }
                drop(callbacks);
                rustpython_common::refcount::with_deferred_drops(|| {
                    for obj in &dead {
                        if obj.gc_has_clear() {
                            drop(unsafe { obj.gc_clear() });
                        }
                    }
                    drop(dead);
                });
                heap.trim();
                #[cfg(feature = "threading")]
                heap.qsbr.process();
            }
        });
    }
}

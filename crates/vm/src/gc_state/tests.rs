use super::*;

#[cfg(feature = "threading")]
#[test]
fn collection_does_not_wait_for_an_attached_foreign_interpreter() {
    use core::time::Duration;
    use std::sync::mpsc;
    let first = crate::Interpreter::without_stdlib(Default::default());
    let second = first.create_subinterpreter();
    let state = first.enter_raw(|vm| vm.state.clone());
    let before = state.stop_the_world.stats_snapshot().stop_calls;
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let first_worker = first.new_thread();
    let waiting = std::thread::spawn(move || {
        first_worker.run_raw(|_| {
            entered_tx.send(()).unwrap();
            // Deliberately remain attached. Rescue the test if a regression
            // incorrectly waits for this unrelated interpreter's safepoint.
            let _ = release_rx.recv_timeout(Duration::from_secs(10));
        });
    });
    entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let (done_tx, done_rx) = mpsc::channel();
    let second_worker = second.new_thread();
    let collecting = std::thread::spawn(move || {
        second_worker.run_raw(|vm| {
            let node = vm.ctx.new_list(Vec::new());
            node.borrow_vec_mut().push(node.clone().into());
            drop(node);
            let collected = vm.state.gc.collect_force(2).collected;
            done_tx.send(collected).unwrap();
        });
    });
    let result = done_rx.recv_timeout(Duration::from_secs(5));
    let _ = release_tx.send(());
    waiting.join().unwrap();
    collecting.join().unwrap();
    assert!(result.expect("foreign VM delayed collection") >= 1);
    assert_eq!(state.stop_the_world.stats_snapshot().stop_calls, before);
}

#[test]
fn retired_cycles_are_reclaimed_without_a_live_owner() {
    let ctx = crate::Context::genesis();
    let state = GcInterpreterState::new(ctx);
    let heap = PyRc::downgrade(&state.heap);
    {
        let _owner = state.allocation_scope();
        let node = ctx.new_list(Vec::new());
        node.borrow_vec_mut().push(node.clone().into());
    }
    // No VM ever entered this heap, so no Python execution or callbacks are
    // available to collect its cycle after the native state goes away.
    drop(state);
    let deadline = std::time::Instant::now() + core::time::Duration::from_secs(5);
    while heap.strong_count() != 0 {
        gc_state().collect_retired();
        assert!(
            std::time::Instant::now() < deadline,
            "retired heap was retained"
        );
        std::thread::yield_now();
    }
    assert!(heap.upgrade().is_none());
}

#[test]
fn retired_cycles_wait_for_full_collection_and_recheck_external_roots() {
    let ctx = crate::Context::genesis();
    let collector = GcState::new();
    let heap = collector.new_heap(crate::vm::runtime::alloc_interpreter_id());
    let state = GcInterpreterState::new(ctx);
    let node = {
        let _owner = AllocationScope::new(heap.clone());
        let node = ctx.new_list(Vec::new());
        node.borrow_vec_mut().push(node.clone().into());
        node
    };
    heap.retired.store(true, Ordering::Release);
    collector.collect_retired();
    assert_eq!(heap.snapshot(None).len(), 1);
    // Dropping an external root leaves both membership and the cycle intact.
    // A membership-only dirty flag would miss the next chance to collect it.
    drop(node);
    collector.collect_inner(&state, Some(0), true);
    collector.collect_inner(&state, Some(1), true);
    let retained_after_young_collections = heap.snapshot(None).len();
    collector.collect_inner(&state, Some(2), true);
    assert!(heap.snapshot(None).is_empty());
    assert_eq!(retained_after_young_collections, 1);
}

#[test]
fn heap_introspection_and_freezing_are_owner_local() {
    let first = crate::Interpreter::without_stdlib(Default::default());
    let second = first.create_subinterpreter();
    first.enter_raw(|a| {
        a.state.gc.freeze();
        let frozen = a.state.gc.get_freeze_count();
        assert!(frozen > 0);
        second.enter_raw(|b| {
            assert_eq!(b.state.gc.get_freeze_count(), 0);
            b.state.gc.unfreeze();
            let objects = b.state.gc.get_objects(None);
            assert!(!objects.is_empty());
            assert!(objects.iter().all(|obj| {
                obj.gc_handle().heap().unwrap().owner == Some(b.state.interpreter_id)
            }));
        });
        assert_eq!(a.state.gc.get_freeze_count(), frozen);
        a.state.gc.unfreeze();
    });
}

#[test]
fn untracked_allocations_route_callbacks_to_the_owner() {
    use crate::AsObject;
    let first = crate::Interpreter::without_stdlib(Default::default());
    let second = first.create_subinterpreter();
    first.enter_raw(|vm| {
        let object = vm.ctx.new_bytearray(Vec::new());
        assert!(!object.as_object().is_gc_tracked());
        second.enter_raw(|other| {
            let owner =
                crate::vm::thread::with_vm(object.as_object(), |owner| owner.state.interpreter_id);
            assert_eq!(owner, Some(vm.state.interpreter_id));
            crate::vm::thread::with_current_vm(|current| {
                assert_eq!(current.state.interpreter_id, other.state.interpreter_id);
            });
        });
    });
}

#[test]
fn collection_schedule_and_growth_guard() {
    let mut schedule = GcSchedule::default();
    for expected in [0, 0, 1, 0, 0, 1, 2, 0] {
        let generation = schedule.generation((100, 1, 1));
        assert_eq!(generation, expected);
        schedule.start_collection(generation);
        schedule.record_survivors(generation, 100);
    }
    schedule.start_collection(2);
    schedule.record_survivors(2, 1000);
    schedule.start_collection(1);
    schedule.record_survivors(1, 249);
    assert_eq!(schedule.generation((100, 0, 0)), 0);
    schedule.record_survivors(1, 1);
    assert_eq!(schedule.generation((100, 0, 0)), 2);
}

#[test]
fn interpreter_policy_and_pressure_are_independent() {
    let ctx = crate::vm::Context::genesis();
    let first = GcInterpreterState::new(ctx);
    let second = GcInterpreterState::new(ctx);
    assert_eq!(first.get_threshold(), (2000, 10, 10));
    assert!(first.is_enabled());
    first.disable();
    assert!(!first.is_enabled());
    assert!(second.is_enabled());
    first.enable();
    first.set_threshold(123, Some(4), Some(5));
    first.set_debug(GcDebugFlags::SAVEALL);
    assert_eq!(first.get_threshold(), (123, 4, 5));
    assert_eq!(second.get_threshold(), (2000, 10, 10));
    assert_eq!(second.get_debug(), GcDebugFlags::empty());
    first.heap.allocations.store(42, Ordering::Relaxed);
    first.schedule.lock().start_collection(1);
    assert_eq!(first.get_count(), (42, 0, 1));
    assert_eq!(second.get_count(), (0, 0, 0));
}

#[test]
fn dense_graph_counts_duplicate_edges_and_external_roots() {
    let _collector = gc_state().collecting.lock();
    let ctx = crate::vm::Context::genesis();
    for rooted in [false, true] {
        let a = ctx.new_list(Vec::new());
        let b = ctx.new_list(vec![a.clone().into()]);
        a.borrow_vec_mut()
            .extend([b.clone().into(), b.clone().into()]);
        let root = rooted.then(|| a.clone());
        let mut graph = unsafe { Graph::capture(vec![a.into(), b.into()]) };
        graph.analyze();
        let (live, dead) = graph.partition();
        assert_eq!(live.len(), if rooted { 2 } else { 0 });
        assert_eq!(dead.len(), if rooted { 0 } else { 2 });
        for obj in live.iter().chain(&dead) {
            assert!(!obj.is_gc_collecting());
            obj.downcast_ref::<crate::builtins::PyList>()
                .unwrap()
                .borrow_vec_mut()
                .clear();
        }
        drop(root);
    }
}

#[test]
fn revalidation_preserves_newly_frozen_objects_and_their_referents() {
    let _collector = gc_state().collecting.lock();
    let first = crate::Interpreter::without_stdlib(Default::default());
    let a = first.enter_raw(|vm| {
        vm.state.gc.disable();
        vm.ctx.new_list(Vec::new())
    });
    let b = first.enter_raw(|vm| vm.ctx.new_list(vec![a.clone().into()]));
    a.borrow_vec_mut().push(b.clone().into());
    let mut graph = unsafe { Graph::capture(vec![a.into(), b.into()]) };
    graph.analyze();
    let (live, dead) = graph.partition();
    assert!(live.is_empty());
    assert_eq!(dead.len(), 2);
    first.enter_raw(|vm| vm.state.gc.heap.freeze());
    let (live, dead) = unsafe { collector::revalidate(dead) };
    assert_eq!(live.len(), 2);
    assert!(dead.is_empty());
    for obj in live {
        obj.downcast_ref::<crate::builtins::PyList>()
            .unwrap()
            .borrow_vec_mut()
            .clear();
    }
}

#[test]
fn immutable_tuple_tracking_distinguishes_mutable_children_and_placeholders() {
    let ctx = crate::vm::Context::genesis();
    let tuple = ctx.new_tuple(vec![ctx.new_int(1).into()]);
    assert!(!tuple.as_object().is_gc_tracked());
    assert!(tuple.as_object().gc_is_acyclic());
    let nested = ctx.new_tuple(vec![tuple.into()]);
    assert!(!nested.as_object().is_gc_tracked());
    let mutable = ctx.new_tuple(vec![ctx.new_list(Vec::new()).into()]);
    assert!(mutable.as_object().is_gc_tracked());
    let placeholder = crate::PyRef::new_ref_tracked(
        crate::builtins::PyTuple::new_unchecked(vec![ctx.none()].into_boxed_slice()),
        ctx.types.tuple_type.to_owned(),
    );
    assert!(placeholder.as_object().is_gc_tracked());
    assert!(!placeholder.as_object().gc_is_acyclic());
}

#[test]
fn heap_membership_survives_swap_promotion_freeze_and_retirement() {
    let _collector = gc_state().collecting.lock();
    let ctx = crate::vm::Context::genesis();
    let state = GcInterpreterState::new(ctx);
    let weak = PyRc::downgrade(&state.heap);
    // Populate only this heap's objects, without interpreter bootstrap roots.
    let objects: Vec<PyObjectRef> = (0..600)
        .map(|_| {
            let obj: PyObjectRef = ctx.new_list(Vec::new()).into();
            // This fresh, unpublished fixture has no other references. Remove
            // its bootstrap membership before assigning the isolated heap.
            assert_eq!(obj.strong_count(), 1);
            obj.gc_handle().heap().unwrap().untrack(&obj);
            obj.gc_handle().release();
            unsafe {
                GcHeap::track(&state.heap, NonNull::from(obj.as_ref()), true);
            }
            obj
        })
        .collect();
    let heap = weak.upgrade().unwrap();
    heap.promote(&objects[..200], 0);
    heap.promote(&objects[..100], 1);
    heap.untrack(&objects[250]);
    heap.promote(&objects, 2);
    // gc.garbage and gc.callbacks are private bootstrap roots in generation 0.
    assert_eq!(heap.get_objects(Some(0)).len(), 2);
    assert_eq!(heap.get_objects(Some(1)).len(), 0);
    assert_eq!(heap.get_objects(Some(2)).len(), 599);
    heap.freeze();
    assert_eq!(heap.freeze_count(), 601);
    assert!(heap.get_objects(None).is_empty());
    heap.unfreeze();
    assert_eq!(heap.freeze_count(), 0);
    assert_eq!(heap.get_objects(Some(2)).len(), 601);
    drop(state);
    assert!(heap.retired.load(Ordering::Relaxed));
    drop(heap);
    assert!(weak.upgrade().is_some());
    drop(objects);
    assert!(weak.upgrade().is_none());
}

#[cfg(feature = "threading")]
#[test]
fn pending_requests_survive_busy_collectors_and_foreign_resets() {
    let state = GcInterpreterState::new(crate::vm::Context::genesis());
    let other = GcInterpreterState::new(crate::vm::Context::genesis());
    let collector = GcState::new();
    state.set_threshold(1, Some(0), Some(0));
    state.heap.allocations.store(1, Ordering::Relaxed);
    collector.maybe_collect(&state);
    other.heap.allocations.store(0, Ordering::Relaxed);
    {
        let _guard = state.collecting.lock();
        collector.collect_inner(&state, None, false);
        assert!(state.scheduled.load(Ordering::Relaxed));
        assert_eq!(state.get_count(), (1, 0, 0));
    }
    state.set_threshold(0, None, None);
    collector.collect_inner(&state, None, false);
    assert!(!state.scheduled.load(Ordering::Relaxed));
    assert_eq!(state.get_count(), (1, 0, 0));
    state.scheduled.store(true, Ordering::Relaxed);
    state.disable();
    collector.collect_inner(&state, None, false);
    assert!(!state.scheduled.load(Ordering::Relaxed));
}

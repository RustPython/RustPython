/// RAII barrier that parks every other thread for the pointer-reading phases
/// of a collection and lets them run again before finalizers execute.
///
/// Reference subtraction, the reachability walk and the strong-reference
/// snapshot dereference the interpreter state of every tracked object,
/// including the `localsplus` of frames that other threads are actively
/// executing. Those writes carry no synchronization, so the reads are only
/// well-defined while all other threads are parked at a safepoint. Restarting
/// happens explicitly once the snapshot has pinned every object; `Drop` is a
/// backstop that also restarts on the early-return paths.
///
/// A collection acts on one interpreter's objects, but its candidates include
/// the ones no interpreter owns, which every interpreter can reference and so
/// incref. Reading a refcount that another interpreter is changing is what
/// makes an object look unreachable when it is not, so every live interpreter
/// is stopped, not just the collecting one. Stopping in `runtime` id order
/// is deterministic. One process-wide STW transaction covers the entire group,
/// preventing a single-interpreter requester from stopping a collector while
/// that collector waits for a different interpreter.
#[cfg(feature = "threading")]
pub(crate) struct StopTheWorldGroup {
    /// Entire interpreter snapshot, retained through restart even on unwind.
    /// Dropping the last state can release Python objects and run finalizers,
    /// so both stopped and not-yet-stopped states must outlive the transaction
    /// and the collection's generation locks.
    states: Vec<crate::common::rc::PyRc<crate::vm::PyGlobalState>>,
    stopped_count: usize,
    /// Keeps interpreters from registering between the snapshot below and the
    /// restart. One registered in that window would be missing from `states`,
    /// so its bootstrap would keep running — and mutating the shared generation
    /// lists — while this collection reads them.
    admission: Option<parking_lot::MutexGuard<'static, ()>>,
    /// Acquired before admission and retained until every state is restarted.
    transaction: Option<crate::vm::StopTheWorldTransaction>,
    restarted: bool,
}

#[cfg(feature = "threading")]
impl StopTheWorldGroup {
    /// Request stop-the-world on every live interpreter when the current thread
    /// has an attached VM. Falls back to no barrier when no VM is attached (the
    /// tracked-object reads then run without other threads only if the caller
    /// guarantees it).
    pub(crate) fn new() -> Self {
        // No attached VM means no interpreter is running Python on this thread;
        // keep the historical no-barrier fallback.
        if !crate::vm::thread::current_vm_is_set() {
            return Self {
                states: Vec::new(),
                stopped_count: 0,
                admission: None,
                transaction: None,
                restarted: true,
            };
        }

        // Accumulate into a live `Self` rather than a bare Vec: if a later
        // `stop_the_world` unwinds, dropping this guard restarts the
        // interpreters already stopped, instead of leaving their threads parked
        // and their exclusion held forever.
        let transaction = crate::vm::StopTheWorldTransaction::acquire();
        let mut guard = Self {
            states: Vec::new(),
            stopped_count: 0,
            admission: Some(crate::vm::runtime::lock_admission_for_stop()),
            transaction: Some(transaction),
            restarted: false,
        };
        guard.states = crate::vm::runtime::live_interpreter_states();
        for state in &guard.states {
            state
                .stop_the_world
                .stop_with_transaction(state, guard.transaction.as_ref().unwrap());
            guard.stopped_count += 1;
        }
        guard
    }

    /// Restart the world. Idempotent.
    pub(crate) fn restart(&mut self) {
        if self.restarted {
            return;
        }
        self.restarted = true;
        // Reverse of the stop order. The references stay until this guard is
        // dropped; see the field comment.
        for state in self.states[..self.stopped_count].iter().rev() {
            state.stop_the_world.start_the_world(state);
        }
        // Nothing is parked any more, so registration may resume.
        self.admission = None;
        self.transaction = None;
    }

    /// Whether this collection actually stopped the world.
    #[cfg(all(unix, debug_assertions))]
    pub(crate) fn is_stopped(&self) -> bool {
        self.stopped_count != 0
    }
}

#[cfg(feature = "threading")]
impl Drop for StopTheWorldGroup {
    fn drop(&mut self) {
        self.restart();
    }
}

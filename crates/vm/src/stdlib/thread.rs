//! Implementation of the _thread module
#[cfg(unix)]
pub(crate) use _thread::after_fork_child;
#[cfg_attr(target_arch = "wasm32", allow(unused_imports))]
pub(crate) use _thread::{
    CurrentFrameSlot, HandleEntry, RawRMutex, ShutdownEntry, get_all_current_frames, get_ident,
    init_main_thread_ident, make_module,
};

#[pymodule]
pub(crate) mod _thread {
    use crate::{
        AsObject, Py, PyPayload, PyRef, PyResult, VirtualMachine,
        builtins::{PyDictRef, PyStr, PyStrRef, PyTupleRef, PyType, PyTypeRef},
        frame::FrameRef,
        function::{ArgCallable, Either, FuncArgs, KwArgs, OptionalArg, PySetterValue},
        types::{Constructor, GetAttr, Representable, SetAttr},
    };
    use alloc::fmt;
    use core::{cell::RefCell, time::Duration};
    use crossbeam_utils::atomic::AtomicCell;
    use parking_lot::{
        RawMutex, RawThreadId,
        lock_api::{RawMutex as RawMutexT, RawMutexTimed, RawReentrantMutex},
    };
    use std::thread;

    // PYTHREAD_NAME: show current thread name
    pub const PYTHREAD_NAME: Option<&str> = {
        cfg_if::cfg_if! {
            if #[cfg(windows)] {
                Some("nt")
            } else if #[cfg(unix)] {
                Some("pthread")
            } else if #[cfg(any(target_os = "solaris", target_os = "illumos"))] {
                Some("solaris")
            } else {
                None
            }
        }
    };

    // TIMEOUT_MAX_IN_MICROSECONDS is a value in microseconds
    #[cfg(not(target_os = "windows"))]
    const TIMEOUT_MAX_IN_MICROSECONDS: i64 = i64::MAX / 1_000;

    #[cfg(target_os = "windows")]
    const TIMEOUT_MAX_IN_MICROSECONDS: i64 = 0xffffffff * 1_000;

    // this is a value in seconds
    #[pyattr]
    const TIMEOUT_MAX: f64 = (TIMEOUT_MAX_IN_MICROSECONDS / 1_000_000) as f64;

    #[pyattr]
    fn error(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.exceptions.runtime_error.to_owned()
    }

    #[derive(FromArgs)]
    struct AcquireArgs {
        #[pyarg(any, default = true)]
        blocking: bool,
        #[pyarg(any, default = Either::A(-1.0))]
        timeout: Either<f64, i64>,
    }

    macro_rules! acquire_lock_impl {
        ($mu:expr, $args:expr, $vm:expr) => {{
            let (mu, args, vm) = ($mu, $args, $vm);
            let timeout = match args.timeout {
                Either::A(f) => f,
                Either::B(i) => i as f64,
            };
            match args.blocking {
                true if timeout == -1.0 => {
                    mu.lock();
                    Ok(true)
                }
                true if timeout < 0.0 => {
                    Err(vm.new_value_error("timeout value must be positive".to_owned()))
                }
                true => {
                    // modified from std::time::Duration::from_secs_f64 to avoid a panic.
                    // TODO: put this in the Duration::try_from_object impl, maybe?
                    let nanos = timeout * 1_000_000_000.0;
                    if timeout > TIMEOUT_MAX as f64 || nanos < 0.0 || !nanos.is_finite() {
                        return Err(vm.new_overflow_error(
                            "timestamp too large to convert to Rust Duration".to_owned(),
                        ));
                    }

                    Ok(mu.try_lock_for(Duration::from_secs_f64(timeout)))
                }
                false if timeout != -1.0 => Err(vm
                    .new_value_error("can't specify a timeout for a non-blocking call".to_owned())),
                false => Ok(mu.try_lock()),
            }
        }};
    }
    macro_rules! repr_lock_impl {
        ($zelf:expr) => {{
            let status = if $zelf.mu.is_locked() {
                "locked"
            } else {
                "unlocked"
            };
            Ok(format!(
                "<{} {} object at {:#x}>",
                status,
                $zelf.class().name(),
                $zelf.get_id()
            ))
        }};
    }

    #[pyattr(name = "LockType")]
    #[pyattr(name = "lock")]
    #[pyclass(module = "_thread", name = "lock")]
    #[derive(PyPayload)]
    struct Lock {
        mu: RawMutex,
    }

    impl fmt::Debug for Lock {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.pad("Lock")
        }
    }

    #[pyclass(with(Constructor, Representable))]
    impl Lock {
        #[pymethod]
        #[pymethod(name = "acquire_lock")]
        #[pymethod(name = "__enter__")]
        fn acquire(&self, args: AcquireArgs, vm: &VirtualMachine) -> PyResult<bool> {
            acquire_lock_impl!(&self.mu, args, vm)
        }
        #[pymethod]
        #[pymethod(name = "release_lock")]
        fn release(&self, vm: &VirtualMachine) -> PyResult<()> {
            if !self.mu.is_locked() {
                return Err(vm.new_runtime_error("release unlocked lock"));
            }
            unsafe { self.mu.unlock() };
            Ok(())
        }

        #[pymethod]
        fn _at_fork_reinit(&self, _vm: &VirtualMachine) -> PyResult<()> {
            if self.mu.is_locked() {
                unsafe {
                    self.mu.unlock();
                };
            }
            // Casting to AtomicCell is as unsafe as CPython code.
            // Using AtomicCell will prevent compiler optimizer move it to somewhere later unsafe place.
            // It will be not under the cell anymore after init call.

            let new_mut = RawMutex::INIT;
            unsafe {
                let old_mutex: &AtomicCell<RawMutex> = core::mem::transmute(&self.mu);
                old_mutex.swap(new_mut);
            }

            Ok(())
        }

        #[pymethod]
        fn __exit__(&self, _args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
            self.release(vm)
        }

        #[pymethod]
        fn locked(&self) -> bool {
            self.mu.is_locked()
        }
    }

    impl Constructor for Lock {
        type Args = ();

        fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
            Ok(Self { mu: RawMutex::INIT })
        }
    }

    impl Representable for Lock {
        #[inline]
        fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            repr_lock_impl!(zelf)
        }
    }

    pub type RawRMutex = RawReentrantMutex<RawMutex, RawThreadId>;
    #[pyattr]
    #[pyclass(module = "_thread", name = "RLock")]
    #[derive(PyPayload)]
    struct RLock {
        mu: RawRMutex,
        count: core::sync::atomic::AtomicUsize,
    }

    impl fmt::Debug for RLock {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.pad("RLock")
        }
    }

    #[pyclass(with(Representable), flags(BASETYPE))]
    impl RLock {
        #[pyslot]
        fn slot_new(cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            Self {
                mu: RawRMutex::INIT,
                count: core::sync::atomic::AtomicUsize::new(0),
            }
            .into_ref_with_type(vm, cls)
            .map(Into::into)
        }

        #[pymethod]
        #[pymethod(name = "acquire_lock")]
        #[pymethod(name = "__enter__")]
        fn acquire(&self, args: AcquireArgs, vm: &VirtualMachine) -> PyResult<bool> {
            let result = acquire_lock_impl!(&self.mu, args, vm)?;
            if result {
                self.count
                    .fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            }
            Ok(result)
        }
        #[pymethod]
        #[pymethod(name = "release_lock")]
        fn release(&self, vm: &VirtualMachine) -> PyResult<()> {
            if !self.mu.is_locked() {
                return Err(vm.new_runtime_error("release unlocked lock"));
            }
            debug_assert!(
                self.count.load(core::sync::atomic::Ordering::Relaxed) > 0,
                "RLock count underflow"
            );
            self.count
                .fetch_sub(1, core::sync::atomic::Ordering::Relaxed);
            unsafe { self.mu.unlock() };
            Ok(())
        }

        #[pymethod]
        fn _at_fork_reinit(&self, _vm: &VirtualMachine) -> PyResult<()> {
            if self.mu.is_locked() {
                unsafe {
                    self.mu.unlock();
                };
            }
            self.count.store(0, core::sync::atomic::Ordering::Relaxed);
            let new_mut = RawRMutex::INIT;
            unsafe {
                let old_mutex: &AtomicCell<RawRMutex> = core::mem::transmute(&self.mu);
                old_mutex.swap(new_mut);
            }

            Ok(())
        }

        #[pymethod]
        fn locked(&self) -> bool {
            self.mu.is_locked()
        }

        #[pymethod]
        fn _is_owned(&self) -> bool {
            self.mu.is_owned_by_current_thread()
        }

        #[pymethod]
        fn _recursion_count(&self) -> usize {
            if self.mu.is_owned_by_current_thread() {
                self.count.load(core::sync::atomic::Ordering::Relaxed)
            } else {
                0
            }
        }

        #[pymethod]
        fn __exit__(&self, _args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
            self.release(vm)
        }
    }

    impl Representable for RLock {
        #[inline]
        fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            repr_lock_impl!(zelf)
        }
    }

    /// Get thread identity - uses pthread_self() on Unix for fork compatibility
    #[pyfunction]
    pub fn get_ident() -> u64 {
        current_thread_id()
    }

    /// Set the name of the current thread
    #[pyfunction]
    fn set_name(name: PyStrRef) {
        #[cfg(target_os = "linux")]
        {
            use std::ffi::CString;
            if let Ok(c_name) = CString::new(name.as_str()) {
                // pthread_setname_np on Linux has a 16-byte limit including null terminator
                // TODO: Potential UTF-8 boundary issue when truncating thread name on Linux.
                // https://github.com/RustPython/RustPython/pull/6726/changes#r2689379171
                let truncated = if c_name.as_bytes().len() > 15 {
                    CString::new(&c_name.as_bytes()[..15]).unwrap_or(c_name)
                } else {
                    c_name
                };
                unsafe {
                    libc::pthread_setname_np(libc::pthread_self(), truncated.as_ptr());
                }
            }
        }
        #[cfg(target_os = "macos")]
        {
            use std::ffi::CString;
            if let Ok(c_name) = CString::new(name.as_str()) {
                unsafe {
                    libc::pthread_setname_np(c_name.as_ptr());
                }
            }
        }
        #[cfg(windows)]
        {
            // Windows doesn't have a simple pthread_setname_np equivalent
            // SetThreadDescription requires Windows 10+
            let _ = name;
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
        {
            let _ = name;
        }
    }

    /// Get OS-level thread ID (pthread_self on Unix)
    /// This is important for fork compatibility - the ID must remain stable after fork
    #[cfg(unix)]
    fn current_thread_id() -> u64 {
        // pthread_self() like CPython for fork compatibility
        unsafe { libc::pthread_self() as u64 }
    }

    #[cfg(not(unix))]
    fn current_thread_id() -> u64 {
        thread_to_rust_id(&thread::current())
    }

    /// Convert Rust thread to ID (used for non-unix platforms)
    #[cfg(not(unix))]
    fn thread_to_rust_id(t: &thread::Thread) -> u64 {
        use core::hash::{Hash, Hasher};
        struct U64Hash {
            v: Option<u64>,
        }
        impl Hasher for U64Hash {
            fn write(&mut self, _: &[u8]) {
                unreachable!()
            }
            fn write_u64(&mut self, i: u64) {
                self.v = Some(i);
            }
            fn finish(&self) -> u64 {
                self.v.expect("should have written a u64")
            }
        }
        let mut h = U64Hash { v: None };
        t.id().hash(&mut h);
        h.finish()
    }

    /// Get thread ID for a given thread handle (used by start_new_thread)
    fn thread_to_id(handle: &thread::JoinHandle<()>) -> u64 {
        #[cfg(unix)]
        {
            // On Unix, use pthread ID from the handle
            use std::os::unix::thread::JoinHandleExt;
            handle.as_pthread_t() as u64
        }
        #[cfg(not(unix))]
        {
            thread_to_rust_id(handle.thread())
        }
    }

    #[pyfunction]
    const fn allocate_lock() -> Lock {
        Lock { mu: RawMutex::INIT }
    }

    #[pyfunction]
    fn start_new_thread(
        func: ArgCallable,
        args: PyTupleRef,
        kwargs: OptionalArg<PyDictRef>,
        vm: &VirtualMachine,
    ) -> PyResult<u64> {
        let args = FuncArgs::new(
            args.to_vec(),
            kwargs
                .map_or_else(Default::default, |k| k.to_attributes(vm))
                .into_iter()
                .map(|(k, v)| (k.as_str().to_owned(), v))
                .collect::<KwArgs>(),
        );
        let mut thread_builder = thread::Builder::new();
        let stacksize = vm.state.stacksize.load();
        if stacksize != 0 {
            thread_builder = thread_builder.stack_size(stacksize);
        }
        thread_builder
            .spawn(
                vm.new_thread()
                    .make_spawn_func(move |vm| run_thread(func, args, vm)),
            )
            .map(|handle| {
                vm.state.thread_count.fetch_add(1);
                thread_to_id(&handle)
            })
            .map_err(|err| vm.new_runtime_error(format!("can't start new thread: {err}")))
    }

    fn run_thread(func: ArgCallable, args: FuncArgs, vm: &VirtualMachine) {
        match func.invoke(args, vm) {
            Ok(_obj) => {}
            Err(e) if e.fast_isinstance(vm.ctx.exceptions.system_exit) => {}
            Err(exc) => {
                vm.run_unraisable(
                    exc,
                    Some("Exception ignored in thread started by".to_owned()),
                    func.into(),
                );
            }
        }
        for lock in SENTINELS.take() {
            if lock.mu.is_locked() {
                unsafe { lock.mu.unlock() };
            }
        }
        // Clean up thread-local storage while VM context is still active
        // This ensures __del__ methods are called properly
        cleanup_thread_local_data();
        // Clean up frame tracking
        crate::vm::thread::cleanup_current_thread_frames(vm);
        vm.state.thread_count.fetch_sub(1);
    }

    /// Clean up thread-local data for the current thread.
    /// This triggers __del__ on objects stored in thread-local variables.
    fn cleanup_thread_local_data() {
        // Take all guards - this will trigger LocalGuard::drop for each,
        // which removes the thread's dict from each Local instance
        LOCAL_GUARDS.with(|guards| {
            guards.borrow_mut().clear();
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[pyfunction]
    fn interrupt_main(signum: OptionalArg<i32>, vm: &VirtualMachine) -> PyResult<()> {
        crate::signal::set_interrupt_ex(signum.unwrap_or(libc::SIGINT), vm)
    }

    #[pyfunction]
    fn exit(vm: &VirtualMachine) -> PyResult {
        Err(vm.new_exception_empty(vm.ctx.exceptions.system_exit.to_owned()))
    }

    thread_local!(static SENTINELS: RefCell<Vec<PyRef<Lock>>> = const { RefCell::new(Vec::new()) });

    #[pyfunction]
    fn _set_sentinel(vm: &VirtualMachine) -> PyRef<Lock> {
        let lock = Lock { mu: RawMutex::INIT }.into_ref(&vm.ctx);
        SENTINELS.with_borrow_mut(|sentinels| sentinels.push(lock.clone()));
        lock
    }

    #[pyfunction]
    fn stack_size(size: OptionalArg<usize>, vm: &VirtualMachine) -> usize {
        let size = size.unwrap_or(0);
        // TODO: do validation on this to make sure it's not too small
        vm.state.stacksize.swap(size)
    }

    #[pyfunction]
    fn _count(vm: &VirtualMachine) -> usize {
        vm.state.thread_count.load()
    }

    #[pyfunction]
    fn daemon_threads_allowed() -> bool {
        // RustPython always allows daemon threads
        true
    }

    // Registry for non-daemon threads that need to be joined at shutdown
    pub type ShutdownEntry = (
        std::sync::Weak<parking_lot::Mutex<ThreadHandleInner>>,
        std::sync::Weak<(parking_lot::Mutex<bool>, parking_lot::Condvar)>,
    );

    #[pyfunction]
    fn _shutdown(vm: &VirtualMachine) {
        // Wait for all non-daemon threads to finish
        let current_ident = get_ident();

        loop {
            // Find a thread that's not finished and not the current thread
            let handle_to_join = {
                let mut handles = vm.state.shutdown_handles.lock();
                // Clean up finished entries
                handles.retain(|(inner_weak, _): &ShutdownEntry| {
                    inner_weak.upgrade().is_some_and(|inner| {
                        let guard = inner.lock();
                        guard.state != ThreadHandleState::Done && guard.ident != current_ident
                    })
                });

                // Find first unfinished handle
                handles
                    .iter()
                    .find_map(|(inner_weak, done_event_weak): &ShutdownEntry| {
                        let inner = inner_weak.upgrade()?;
                        let done_event = done_event_weak.upgrade()?;
                        let guard = inner.lock();
                        if guard.state != ThreadHandleState::Done && guard.ident != current_ident {
                            Some((inner.clone(), done_event.clone()))
                        } else {
                            None
                        }
                    })
            };

            match handle_to_join {
                Some((_, done_event)) => {
                    // Wait for this thread to finish (infinite timeout)
                    // Only check done flag to avoid lock ordering issues
                    // (done_event lock vs inner lock)
                    let (lock, cvar) = &*done_event;
                    let mut done = lock.lock();
                    while !*done {
                        cvar.wait(&mut done);
                    }
                }
                None => break, // No more threads to wait on
            }
        }
    }

    /// Add a non-daemon thread handle to the shutdown registry
    fn add_to_shutdown_handles(
        vm: &VirtualMachine,
        inner: &std::sync::Arc<parking_lot::Mutex<ThreadHandleInner>>,
        done_event: &std::sync::Arc<(parking_lot::Mutex<bool>, parking_lot::Condvar)>,
    ) {
        let mut handles = vm.state.shutdown_handles.lock();
        handles.push((
            std::sync::Arc::downgrade(inner),
            std::sync::Arc::downgrade(done_event),
        ));
    }

    #[pyfunction]
    fn _make_thread_handle(ident: u64, vm: &VirtualMachine) -> PyRef<ThreadHandle> {
        let handle = ThreadHandle::new(vm);
        {
            let mut inner = handle.inner.lock();
            inner.ident = ident;
            inner.state = ThreadHandleState::Running;
        }
        handle.into_ref(&vm.ctx)
    }

    #[pyfunction]
    fn _get_main_thread_ident(vm: &VirtualMachine) -> u64 {
        vm.state.main_thread_ident.load()
    }

    #[pyfunction]
    fn _is_main_interpreter() -> bool {
        // RustPython only has one interpreter
        true
    }

    /// Initialize the main thread ident. Should be called once at interpreter startup.
    pub fn init_main_thread_ident(vm: &VirtualMachine) {
        let ident = get_ident();
        vm.state.main_thread_ident.store(ident);
    }

    /// ExceptHookArgs - simple class to hold exception hook arguments
    /// This allows threading.py to import _excepthook and _ExceptHookArgs from _thread
    #[pyattr]
    #[pyclass(module = "_thread", name = "_ExceptHookArgs")]
    #[derive(Debug, PyPayload)]
    struct ExceptHookArgs {
        exc_type: crate::PyObjectRef,
        exc_value: crate::PyObjectRef,
        exc_traceback: crate::PyObjectRef,
        thread: crate::PyObjectRef,
    }

    #[pyclass(with(Constructor))]
    impl ExceptHookArgs {
        #[pygetset]
        fn exc_type(&self) -> crate::PyObjectRef {
            self.exc_type.clone()
        }

        #[pygetset]
        fn exc_value(&self) -> crate::PyObjectRef {
            self.exc_value.clone()
        }

        #[pygetset]
        fn exc_traceback(&self) -> crate::PyObjectRef {
            self.exc_traceback.clone()
        }

        #[pygetset]
        fn thread(&self) -> crate::PyObjectRef {
            self.thread.clone()
        }
    }

    impl Constructor for ExceptHookArgs {
        // Takes a single iterable argument like namedtuple
        type Args = (crate::PyObjectRef,);

        fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            // Convert the argument to a list/tuple and extract elements
            let seq: Vec<crate::PyObjectRef> = args.0.try_to_value(vm)?;
            if seq.len() != 4 {
                return Err(vm.new_type_error(format!(
                    "_ExceptHookArgs expected 4 arguments, got {}",
                    seq.len()
                )));
            }
            Ok(Self {
                exc_type: seq[0].clone(),
                exc_value: seq[1].clone(),
                exc_traceback: seq[2].clone(),
                thread: seq[3].clone(),
            })
        }
    }

    /// Handle uncaught exception in Thread.run()
    #[pyfunction]
    fn _excepthook(args: crate::PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        // Type check: args must be _ExceptHookArgs
        let args = args.downcast::<ExceptHookArgs>().map_err(|_| {
            vm.new_type_error(
                "_thread._excepthook argument type must be _ExceptHookArgs".to_owned(),
            )
        })?;

        let exc_type = args.exc_type.clone();
        let exc_value = args.exc_value.clone();
        let exc_traceback = args.exc_traceback.clone();
        let thread = args.thread.clone();

        // Silently ignore SystemExit (identity check)
        if exc_type.is(vm.ctx.exceptions.system_exit.as_ref()) {
            return Ok(());
        }

        // Get stderr - fall back to thread._stderr if sys.stderr is None
        let file = match vm.sys_module.get_attr("stderr", vm) {
            Ok(stderr) if !vm.is_none(&stderr) => stderr,
            _ => {
                if vm.is_none(&thread) {
                    // do nothing if sys.stderr is None and thread is None
                    return Ok(());
                }
                let thread_stderr = thread.get_attr("_stderr", vm)?;
                if vm.is_none(&thread_stderr) {
                    // do nothing if sys.stderr is None and sys.stderr was None
                    // when the thread was created
                    return Ok(());
                }
                thread_stderr
            }
        };

        // Print "Exception in thread {thread.name}:"
        let thread_name = if !vm.is_none(&thread) {
            thread
                .get_attr("name", vm)
                .ok()
                .and_then(|n| n.str(vm).ok())
                .map(|s| s.as_str().to_owned())
        } else {
            None
        };
        let name = thread_name.unwrap_or_else(|| format!("{}", get_ident()));

        let _ = vm.call_method(
            &file,
            "write",
            (format!("Exception in thread {}:\n", name),),
        );

        // Display the traceback
        if let Ok(traceback_mod) = vm.import("traceback", 0)
            && let Ok(print_exc) = traceback_mod.get_attr("print_exception", vm)
        {
            use crate::function::KwArgs;
            let kwargs: KwArgs = vec![("file".to_owned(), file.clone())]
                .into_iter()
                .collect();
            let _ = print_exc.call_with_args(
                crate::function::FuncArgs::new(vec![exc_type, exc_value, exc_traceback], kwargs),
                vm,
            );
        }

        // Flush file
        let _ = vm.call_method(&file, "flush", ());
        Ok(())
    }

    // Thread-local storage for cleanup guards
    // When a thread terminates, the guard is dropped, which triggers cleanup
    thread_local! {
        static LOCAL_GUARDS: std::cell::RefCell<Vec<LocalGuard>> = const { std::cell::RefCell::new(Vec::new()) };
    }

    // Guard that removes thread-local data when dropped
    struct LocalGuard {
        local: std::sync::Weak<LocalData>,
        thread_id: std::thread::ThreadId,
    }

    impl Drop for LocalGuard {
        fn drop(&mut self) {
            if let Some(local_data) = self.local.upgrade() {
                // Remove from map while holding the lock, but drop the value
                // outside the lock to prevent deadlock if __del__ accesses _local
                let removed = local_data.data.lock().remove(&self.thread_id);
                drop(removed);
            }
        }
    }

    // Shared data structure for Local
    struct LocalData {
        data: parking_lot::Mutex<std::collections::HashMap<std::thread::ThreadId, PyDictRef>>,
    }

    impl std::fmt::Debug for LocalData {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("LocalData").finish_non_exhaustive()
        }
    }

    #[pyattr]
    #[pyclass(module = "_thread", name = "_local")]
    #[derive(Debug, PyPayload)]
    struct Local {
        inner: std::sync::Arc<LocalData>,
    }

    #[pyclass(with(GetAttr, SetAttr), flags(BASETYPE))]
    impl Local {
        fn l_dict(&self, vm: &VirtualMachine) -> PyDictRef {
            let thread_id = std::thread::current().id();

            // Fast path: check if dict exists under lock
            if let Some(dict) = self.inner.data.lock().get(&thread_id).cloned() {
                return dict;
            }

            // Slow path: allocate dict outside lock to reduce lock hold time
            let new_dict = vm.ctx.new_dict();

            // Insert with double-check to handle races
            let mut data = self.inner.data.lock();
            use std::collections::hash_map::Entry;
            let (dict, need_guard) = match data.entry(thread_id) {
                Entry::Occupied(e) => (e.get().clone(), false),
                Entry::Vacant(e) => {
                    e.insert(new_dict.clone());
                    (new_dict, true)
                }
            };
            drop(data); // Release lock before TLS access

            // Register cleanup guard only if we inserted a new entry
            if need_guard {
                let guard = LocalGuard {
                    local: std::sync::Arc::downgrade(&self.inner),
                    thread_id,
                };
                LOCAL_GUARDS.with(|guards| {
                    guards.borrow_mut().push(guard);
                });
            }

            dict
        }

        #[pyslot]
        fn slot_new(cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            Self {
                inner: std::sync::Arc::new(LocalData {
                    data: parking_lot::Mutex::new(std::collections::HashMap::new()),
                }),
            }
            .into_ref_with_type(vm, cls)
            .map(Into::into)
        }
    }

    impl GetAttr for Local {
        fn getattro(zelf: &Py<Self>, attr: &Py<PyStr>, vm: &VirtualMachine) -> PyResult {
            let l_dict = zelf.l_dict(vm);
            if attr.as_str() == "__dict__" {
                Ok(l_dict.into())
            } else {
                zelf.as_object()
                    .generic_getattr_opt(attr, Some(l_dict), vm)?
                    .ok_or_else(|| {
                        vm.new_attribute_error(format!(
                            "{} has no attribute '{}'",
                            zelf.class().name(),
                            attr
                        ))
                    })
            }
        }
    }

    impl SetAttr for Local {
        fn setattro(
            zelf: &Py<Self>,
            attr: &Py<PyStr>,
            value: PySetterValue,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            if attr.as_str() == "__dict__" {
                Err(vm.new_attribute_error(format!(
                    "{} attribute '__dict__' is read-only",
                    zelf.class().name()
                )))
            } else {
                let dict = zelf.l_dict(vm);
                if let PySetterValue::Assign(value) = value {
                    dict.set_item(attr, value, vm)?;
                } else {
                    dict.del_item(attr, vm)?;
                }
                Ok(())
            }
        }
    }

    // Registry of all ThreadHandles for fork cleanup
    // Stores weak references so handles can be garbage collected normally
    pub type HandleEntry = (
        std::sync::Weak<parking_lot::Mutex<ThreadHandleInner>>,
        std::sync::Weak<(parking_lot::Mutex<bool>, parking_lot::Condvar)>,
    );

    // Re-export type from vm::thread for PyGlobalState
    pub use crate::vm::thread::CurrentFrameSlot;

    /// Get all threads' current frames. Used by sys._current_frames().
    pub fn get_all_current_frames(vm: &VirtualMachine) -> Vec<(u64, FrameRef)> {
        let registry = vm.state.thread_frames.lock();
        registry
            .iter()
            .filter_map(|(id, slot)| slot.lock().clone().map(|f| (*id, f)))
            .collect()
    }

    /// Called after fork() in child process to mark all other threads as done.
    /// This prevents join() from hanging on threads that don't exist in the child.
    #[cfg(unix)]
    pub fn after_fork_child(vm: &VirtualMachine) {
        let current_ident = get_ident();

        // Update main thread ident - after fork, the current thread becomes the main thread
        vm.state.main_thread_ident.store(current_ident);

        // Reinitialize frame slot for current thread
        crate::vm::thread::reinit_frame_slot_after_fork(vm);

        // Clean up thread handles if we can acquire the lock.
        // Use try_lock because the mutex might have been held during fork.
        // If we can't acquire it, just skip - the child process will work
        // correctly with new handles it creates.
        if let Some(mut handles) = vm.state.thread_handles.try_lock() {
            // Clean up dead weak refs and mark non-current threads as done
            handles.retain(|(inner_weak, done_event_weak): &HandleEntry| {
                let Some(inner) = inner_weak.upgrade() else {
                    return false; // Remove dead entries
                };
                let Some(done_event) = done_event_weak.upgrade() else {
                    return false;
                };

                // Try to lock the inner state - skip if we can't
                let Some(mut inner_guard) = inner.try_lock() else {
                    return false;
                };

                // Skip current thread and not-started threads
                if inner_guard.ident == current_ident {
                    return true;
                }
                if inner_guard.state == ThreadHandleState::NotStarted {
                    return true;
                }

                // Mark as done and notify waiters
                inner_guard.state = ThreadHandleState::Done;
                inner_guard.join_handle = None; // Can't join OS thread from child
                drop(inner_guard);

                // Try to notify waiters - skip if we can't acquire the lock
                let (lock, cvar) = &*done_event;
                if let Some(mut done) = lock.try_lock() {
                    *done = true;
                    cvar.notify_all();
                }

                true
            });
        }
    }

    // Thread handle state enum
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum ThreadHandleState {
        NotStarted,
        Starting,
        Running,
        Done,
    }

    // Internal shared state for thread handle
    pub struct ThreadHandleInner {
        pub state: ThreadHandleState,
        pub ident: u64,
        pub join_handle: Option<thread::JoinHandle<()>>,
        pub joining: bool, // True if a thread is currently joining
        pub joined: bool,  // Track if join has completed
    }

    impl fmt::Debug for ThreadHandleInner {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("ThreadHandleInner")
                .field("state", &self.state)
                .field("ident", &self.ident)
                .field("join_handle", &self.join_handle.is_some())
                .field("joining", &self.joining)
                .field("joined", &self.joined)
                .finish()
        }
    }

    /// _ThreadHandle - handle for joinable threads
    #[pyattr]
    #[pyclass(module = "_thread", name = "_ThreadHandle")]
    #[derive(Debug, PyPayload)]
    struct ThreadHandle {
        inner: std::sync::Arc<parking_lot::Mutex<ThreadHandleInner>>,
        // Event to signal thread completion (for timed join support)
        done_event: std::sync::Arc<(parking_lot::Mutex<bool>, parking_lot::Condvar)>,
    }

    #[pyclass]
    impl ThreadHandle {
        fn new(vm: &VirtualMachine) -> Self {
            let inner = std::sync::Arc::new(parking_lot::Mutex::new(ThreadHandleInner {
                state: ThreadHandleState::NotStarted,
                ident: 0,
                join_handle: None,
                joining: false,
                joined: false,
            }));
            let done_event =
                std::sync::Arc::new((parking_lot::Mutex::new(false), parking_lot::Condvar::new()));

            // Register in global registry for fork cleanup
            vm.state.thread_handles.lock().push((
                std::sync::Arc::downgrade(&inner),
                std::sync::Arc::downgrade(&done_event),
            ));

            Self { inner, done_event }
        }

        #[pygetset]
        fn ident(&self) -> u64 {
            self.inner.lock().ident
        }

        #[pymethod]
        fn is_done(&self) -> bool {
            self.inner.lock().state == ThreadHandleState::Done
        }

        #[pymethod]
        fn _set_done(&self) {
            self.inner.lock().state = ThreadHandleState::Done;
            // Signal waiting threads that this thread is done
            let (lock, cvar) = &*self.done_event;
            *lock.lock() = true;
            cvar.notify_all();
        }

        #[pymethod]
        fn join(
            &self,
            timeout: OptionalArg<Option<Either<f64, i64>>>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            // Convert timeout to Duration (None or negative = infinite wait)
            let timeout_duration = match timeout.flatten() {
                Some(Either::A(t)) if t >= 0.0 => Some(Duration::from_secs_f64(t)),
                Some(Either::B(t)) if t >= 0 => Some(Duration::from_secs(t as u64)),
                _ => None,
            };

            // Check for self-join first
            {
                let inner = self.inner.lock();
                let current_ident = get_ident();
                if inner.ident == current_ident && inner.state == ThreadHandleState::Running {
                    return Err(vm.new_runtime_error("cannot join current thread".to_owned()));
                }
            }

            // Wait for thread completion using Condvar (supports timeout)
            // Loop to handle spurious wakeups
            let (lock, cvar) = &*self.done_event;
            let mut done = lock.lock();

            while !*done {
                if let Some(timeout) = timeout_duration {
                    let result = cvar.wait_for(&mut done, timeout);
                    if result.timed_out() && !*done {
                        // Timeout occurred and done is still false
                        return Ok(());
                    }
                } else {
                    // Infinite wait
                    cvar.wait(&mut done);
                }
            }
            drop(done);

            // Thread is done, now perform cleanup
            let join_handle = {
                let mut inner = self.inner.lock();

                // If already joined, return immediately (idempotent)
                if inner.joined {
                    return Ok(());
                }

                // If another thread is already joining, wait for them to finish
                if inner.joining {
                    drop(inner);
                    // Wait on done_event
                    let (lock, cvar) = &*self.done_event;
                    let mut done = lock.lock();
                    while !*done {
                        cvar.wait(&mut done);
                    }
                    return Ok(());
                }

                // Mark that we're joining
                inner.joining = true;

                // Take the join handle if available
                inner.join_handle.take()
            };

            // Perform the actual join outside the lock
            if let Some(handle) = join_handle {
                // Ignore the result - panics in spawned threads are already handled
                let _ = handle.join();
            }

            // Mark as joined and clear joining flag
            {
                let mut inner = self.inner.lock();
                inner.joined = true;
                inner.joining = false;
            }

            Ok(())
        }

        #[pyslot]
        fn slot_new(cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            ThreadHandle::new(vm)
                .into_ref_with_type(vm, cls)
                .map(Into::into)
        }
    }

    #[derive(FromArgs)]
    struct StartJoinableThreadArgs {
        #[pyarg(positional)]
        function: ArgCallable,
        #[pyarg(any, optional)]
        handle: OptionalArg<PyRef<ThreadHandle>>,
        #[pyarg(any, default = true)]
        daemon: bool,
    }

    #[pyfunction]
    fn start_joinable_thread(
        args: StartJoinableThreadArgs,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<ThreadHandle>> {
        let handle = match args.handle {
            OptionalArg::Present(h) => h,
            OptionalArg::Missing => ThreadHandle::new(vm).into_ref(&vm.ctx),
        };

        // Mark as starting
        handle.inner.lock().state = ThreadHandleState::Starting;

        // Add non-daemon threads to shutdown registry so _shutdown() will wait for them
        if !args.daemon {
            add_to_shutdown_handles(vm, &handle.inner, &handle.done_event);
        }

        let func = args.function;
        let handle_clone = handle.clone();
        let inner_clone = handle.inner.clone();
        let done_event_clone = handle.done_event.clone();

        let mut thread_builder = thread::Builder::new();
        let stacksize = vm.state.stacksize.load();
        if stacksize != 0 {
            thread_builder = thread_builder.stack_size(stacksize);
        }

        let join_handle = thread_builder
            .spawn(vm.new_thread().make_spawn_func(move |vm| {
                // Set ident and mark as running
                {
                    let mut inner = inner_clone.lock();
                    inner.ident = get_ident();
                    inner.state = ThreadHandleState::Running;
                }

                // Ensure cleanup happens even if the function panics
                let inner_for_cleanup = inner_clone.clone();
                let done_event_for_cleanup = done_event_clone.clone();
                let vm_state = vm.state.clone();
                scopeguard::defer! {
                    // Mark as done
                    inner_for_cleanup.lock().state = ThreadHandleState::Done;

                    // Signal waiting threads that this thread is done
                    {
                        let (lock, cvar) = &*done_event_for_cleanup;
                        *lock.lock() = true;
                        cvar.notify_all();
                    }

                    // Handle sentinels
                    for lock in SENTINELS.take() {
                        if lock.mu.is_locked() {
                            unsafe { lock.mu.unlock() };
                        }
                    }

                    // Clean up thread-local data while VM context is still active
                    cleanup_thread_local_data();

                    // Clean up frame tracking
                    crate::vm::thread::cleanup_current_thread_frames(vm);

                    vm_state.thread_count.fetch_sub(1);
                }

                // Run the function
                match func.invoke((), vm) {
                    Ok(_) => {}
                    Err(e) if e.fast_isinstance(vm.ctx.exceptions.system_exit) => {}
                    Err(exc) => {
                        vm.run_unraisable(
                            exc,
                            Some("Exception ignored in thread started by".to_owned()),
                            func.into(),
                        );
                    }
                }
            }))
            .map_err(|err| vm.new_runtime_error(format!("can't start new thread: {err}")))?;

        vm.state.thread_count.fetch_add(1);

        // Store the join handle
        handle.inner.lock().join_handle = Some(join_handle);

        Ok(handle_clone)
    }
}

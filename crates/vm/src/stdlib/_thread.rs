//! Implementation of the _thread module

#[cfg(all(unix, feature = "threading", feature = "host_env"))]
pub(crate) use _thread::after_fork_child;

pub use _thread::get_ident;

#[cfg_attr(target_arch = "wasm32", allow(unused_imports))]
pub(crate) use _thread::{
    CurrentFrameSlot, HandleEntry, RawRMutex, ShutdownEntry, get_all_current_frames,
    init_main_thread_ident, module_def,
};

#[pymodule]
pub(crate) mod _thread {
    use parking_lot::{
        RawMutex, RawThreadId,
        lock_api::{RawMutex as RawMutexT, RawMutexTimed, RawReentrantMutex},
    };

    use crate::{
        AsObject, Py, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
        builtins::{
            PyBaseExceptionRef, PyDictRef, PyIntRef, PyStr, PyStrRef, PyTupleRef, PyType, PyTypeRef,
        },
        common::{lock::PyMutex, rc::PyRc, wtf8::Wtf8Buf},
        convert::ToPyException,
        frame::FrameObjectRef,
        function::{
            ArgCallable, ArgIndex, ArgumentError, DefaultRepr, FromArgs, FuncArgs, KwArgs,
            OptionalArg, Param, ParamKind, PySetterValue,
        },
        object::{Traverse, TraverseFn},
        stdlib::time::pytime,
        types::{Constructor, GetAttr, PyStructSequence, Representable, SetAttr},
    };

    use alloc::sync::{Arc, Weak};
    use core::{cell::RefCell, fmt, time::Duration};
    use std::thread;

    use rustpython_common::str::levenshtein::{MOVE_COST, levenshtein_distance};
    #[cfg(any(unix, windows))]
    use rustpython_host_env::thread as host_thread;

    #[cfg(all(not(target_arch = "wasm32"), feature = "host_env"))]
    use crate::signal::SignalNum;

    // PYTHREAD_NAME: show current thread name
    pub(crate) const PYTHREAD_NAME: Option<&str> = cfg_select! {
        windows => Some("nt"),
        unix => Some("pthread"),
        any(target_os = "solaris", target_os = "illumos") => Some("solaris"),
        _ => None,
    };

    /// [CPython `SYSTEM_PAGE_SIZE`](https://github.com/python/cpython/blob/v3.14.5/Include/internal/pycore_obmalloc.h#L170)
    const SYSTEM_PAGE_SIZE: usize = 4 * 1024;

    // TODO: Check for sanitize once https://github.com/rust-lang/rust/issues/39699 is closed
    /// [CPython `_PyOS_LOG2_STACK_MARGIN`](https://github.com/python/cpython/blob/v3.14.5/Include/internal/pycore_pythonrun.h#L41-L47)
    const PY_OS_LOG2_STACK_MARGIN: u32 = cfg_select! {
        debug_assertions => 12,
        _ => 11,
    };

    /// [CPython `_PyOS_STACK_MARGIN`](https://github.com/python/cpython/blob/v3.14.5/Include/internal/pycore_pythonrun.h#L48)
    const PY_OS_STACK_MARGIN: usize = 1 << PY_OS_LOG2_STACK_MARGIN;

    /// [CPython `_PyOS_STACK_MARGIN_BYTES`](https://github.com/python/cpython/blob/v3.14.5/Include/internal/pycore_pythonrun.h#L49)
    const PY_OS_STACK_MARGIN_BYTES: usize = PY_OS_STACK_MARGIN * size_of::<*const ()>();

    // TODO: Check for sanitize once https://github.com/rust-lang/rust/issues/39699 is closed
    /// [CPython `_PyOS_MIN_STACK_SIZE`](https://github.com/python/cpython/blob/v3.14.5/Include/internal/pycore_pythonrun.h#L57-L61)
    const PY_OS_MIN_STACK_SIZE: usize = PY_OS_STACK_MARGIN_BYTES * 3;

    // this is a value in seconds
    #[pyattr]
    const TIMEOUT_MAX: f64 = pytime::THREAD_TIMEOUT_MAX_SECONDS as f64;

    #[pyattr]
    fn error(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.exceptions.runtime_error.to_owned()
    }

    #[derive(Debug, FromArgs)]
    struct AcquireArgs {
        // Finish binding before truthiness or timeout conversion can run callbacks.
        #[pyarg(any, optional, py_default = "True")]
        blocking: OptionalArg<PyObjectRef>,
        // No timeout; blocks until the lock is acquired. Explicit None is invalid.
        #[pyarg(any, optional, py_default = "-1")]
        timeout: OptionalArg<PyObjectRef>,
    }

    fn default_acquire_args() -> AcquireArgs {
        AcquireArgs {
            blocking: OptionalArg::Missing,
            timeout: OptionalArg::Missing,
        }
    }

    fn validate_acquire_args(
        args: &AcquireArgs,
        vm: &VirtualMachine,
    ) -> PyResult<(bool, Option<Duration>)> {
        let blocking = match &args.blocking {
            OptionalArg::Present(value) => value.try_to_bool(vm)?,
            OptionalArg::Missing => true,
        };
        let unset_timeout = pytime::from_seconds(-1);
        let timeout = match &args.timeout {
            OptionalArg::Present(value) => {
                pytime::from_seconds_object(value, pytime::Round::Up, vm)?
            }
            OptionalArg::Missing => unset_timeout,
        };
        // CPython converts and rounds before checking the indefinite-wait sentinel.
        if !blocking && timeout != unset_timeout {
            return Err(vm.new_value_error("can't specify a timeout for a non-blocking call"));
        }
        if timeout < 0 && timeout != unset_timeout {
            return Err(vm.new_value_error("timeout value must be a non-negative number"));
        }
        let duration = if timeout == unset_timeout {
            None
        } else {
            pytime::check_thread_timeout(timeout, vm)?;
            Some(Duration::from_nanos(timeout as u64))
        };
        Ok((blocking, duration))
    }

    macro_rules! acquire_lock_impl {
        ($mu:expr, $blocking:expr, $timeout:expr, $vm:expr) => {{ acquire_lock_impl!($mu, $blocking, $timeout, $vm, false) }};
        ($mu:expr, $blocking:expr, $timeout:expr, $vm:expr, $owned:expr) => {{
            let (mu, blocking, timeout, vm) = ($mu, $blocking, $timeout, $vm);
            let started = std::time::Instant::now();
            let mut remaining = timeout;
            loop {
                // Ownership gained in a signal handler does not make an
                // already pending acquisition recursive.
                let owned = $owned;
                if !owned && mu.try_lock() {
                    break Ok(true);
                }
                if !blocking || remaining.is_some_and(|remaining| remaining.is_zero()) {
                    break Ok(false);
                }
                if vm
                    .state
                    .finalizing
                    .load(core::sync::atomic::Ordering::Acquire)
                {
                    // Other Python threads cannot release the lock once
                    // finalization begins. Uncontested calls still work.
                    break Err(vm.new_python_finalization_error(
                        "cannot acquire lock at interpreter finalization",
                    ));
                }
                let interval = Duration::from_millis(50);
                let wait = match timeout {
                    Some(timeout) => match timeout.checked_sub(started.elapsed()) {
                        Some(remaining) if !remaining.is_zero() => remaining.min(interval),
                        _ => break Ok(false),
                    },
                    None => interval,
                };
                // parking_lot waits are not interrupted by OS signals. Poll
                // without restarting the deadline or acquiring a lock before
                // running callbacks. A handler may itself retain an RLock.
                let acquired = vm.allow_threads(|| {
                    if owned {
                        thread::sleep(wait);
                        false
                    } else {
                        mu.try_lock_for(wait)
                    }
                });
                if acquired {
                    break Ok(true);
                }
                if timeout.is_some_and(|timeout| started.elapsed() >= timeout) {
                    break Ok(false);
                }
                vm.check_signals()?;
                remaining = timeout.map(|timeout| timeout.saturating_sub(started.elapsed()));
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

    #[pyclass(with(Constructor, Representable), flags(HAS_WEAKREF))]
    impl Py<Lock> {
        #[pymethod]
        #[pymethod(name = "acquire_lock")]
        fn acquire(&self, args: AcquireArgs, vm: &VirtualMachine) -> PyResult<bool> {
            let (blocking, timeout) = validate_acquire_args(&args, vm)?;
            acquire_lock_impl!(&self.mu, blocking, timeout, vm)
        }

        #[pymethod]
        fn __enter__(&self, vm: &VirtualMachine) -> PyResult<bool> {
            self.acquire(default_acquire_args(), vm)
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

        #[cfg(unix)]
        #[pymethod]
        fn _at_fork_reinit(&self, _vm: &VirtualMachine) {
            // Overwrite lock state to unlocked. Do NOT call unlock() here —
            // after fork(), unlock_slow() would try to unpark stale waiters.
            unsafe { rustpython_common::lock::zero_reinit_after_fork(&self.mu) };
        }

        #[pymethod]
        fn __exit__(
            &self,
            _exc_type: crate::PyObjectRef,
            _exc_value: crate::PyObjectRef,
            _exc_tb: crate::PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
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

    pub(crate) type RawRMutex = RawReentrantMutex<RawMutex, RawThreadId>;
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

    #[pyclass(with(Representable), flags(BASETYPE, HAS_WEAKREF))]
    impl Py<RLock> {
        #[pyslot]
        fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            let rlock_type = RLock::class(&vm.ctx);
            let cls_init = cls.slots.init.load().map(crate::types::fn_addr);
            let rlock_init = rlock_type.slots.init.load().map(crate::types::fn_addr);
            if cls.is(rlock_type) || cls_init == rlock_init {
                if !args.args.is_empty() {
                    return Err(vm.new_type_error("RLock() takes no positional arguments"));
                }
                if !args.kwargs.is_empty() {
                    return Err(vm.new_type_error("RLock() takes no keyword arguments"));
                }
            }
            RLock {
                mu: RawRMutex::INIT,
                count: core::sync::atomic::AtomicUsize::new(0),
            }
            .into_ref_with_type(vm, cls)
            .map(Into::into)
        }

        #[pymethod]
        #[pymethod(name = "acquire_lock")]
        fn acquire(&self, args: AcquireArgs, vm: &VirtualMachine) -> PyResult<bool> {
            let (blocking, timeout) = validate_acquire_args(&args, vm)?;
            if self.mu.is_owned_by_current_thread() {
                // Re-entrant acquisition: just increment our count.
                // parking_lot stays at 1 level; we track recursion ourselves.
                self.count
                    .fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                return Ok(true);
            }
            let result = acquire_lock_impl!(
                &self.mu,
                blocking,
                timeout,
                vm,
                self.mu.is_owned_by_current_thread()
            )?;
            if result {
                self.count.store(1, core::sync::atomic::Ordering::Relaxed);
            }
            Ok(result)
        }

        #[pymethod]
        fn __enter__(&self, vm: &VirtualMachine) -> PyResult<bool> {
            self.acquire(default_acquire_args(), vm)
        }

        #[pymethod]
        #[pymethod(name = "release_lock")]
        fn release(&self, vm: &VirtualMachine) -> PyResult<()> {
            if !self.mu.is_owned_by_current_thread() {
                return Err(vm.new_runtime_error("cannot release un-acquired lock"));
            }
            let prev = self
                .count
                .fetch_sub(1, core::sync::atomic::Ordering::Relaxed);
            debug_assert!(prev > 0, "RLock count underflow");
            if prev == 1 {
                unsafe { self.mu.unlock() };
            }
            Ok(())
        }

        #[cfg(unix)]
        #[pymethod]
        fn _at_fork_reinit(&self, _vm: &VirtualMachine) {
            // Overwrite lock state to unlocked. Do NOT call unlock() here —
            // after fork(), unlock_slow() would try to unpark stale waiters.
            self.count.store(0, core::sync::atomic::Ordering::Relaxed);
            unsafe { rustpython_common::lock::zero_reinit_after_fork(&self.mu) };
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
        fn _release_save(&self, vm: &VirtualMachine) -> PyResult<(usize, u64)> {
            if !self.mu.is_owned_by_current_thread() {
                return Err(vm.new_runtime_error("cannot release un-acquired lock"));
            }
            let count = self.count.swap(0, core::sync::atomic::Ordering::Relaxed);
            debug_assert!(count > 0, "RLock count underflow");
            unsafe { self.mu.unlock() };
            Ok((count, current_thread_id()))
        }

        #[pymethod]
        fn _acquire_restore(&self, state: PyTupleRef, vm: &VirtualMachine) -> PyResult<()> {
            let [count_obj, owner_obj] = state.as_slice() else {
                return Err(
                    vm.new_type_error("_acquire_restore() argument 1 must be a 2-item tuple")
                );
            };
            let count: usize = count_obj.clone().try_into_value(vm)?;
            let _owner: u64 = owner_obj.clone().try_into_value(vm)?;
            if count == 0 {
                return Ok(());
            }
            vm.allow_threads(|| self.mu.lock());
            self.count
                .store(count, core::sync::atomic::Ordering::Relaxed);
            Ok(())
        }

        #[pymethod]
        fn __exit__(
            &self,
            _exc_type: crate::PyObjectRef,
            _exc_value: crate::PyObjectRef,
            _exc_tb: crate::PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            self.release(vm)
        }
    }

    impl Representable for RLock {
        #[inline]
        fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            let count = zelf.count.load(core::sync::atomic::Ordering::Relaxed);
            let status = if zelf.mu.is_locked() {
                "locked"
            } else {
                "unlocked"
            };
            Ok(format!(
                "<{} {} object count={} at {:#x}>",
                status,
                zelf.class().name(),
                count,
                zelf.get_id()
            ))
        }
    }

    // uses pthread_self() on Unix for fork compatibility
    #[pyfunction]
    #[must_use]
    pub fn get_ident() -> u64 {
        current_thread_id()
    }

    #[cfg(all(
        feature = "host_env",
        any(
            windows,
            target_os = "linux",
            target_os = "android",
            target_os = "macos",
            target_os = "ios",
            target_os = "freebsd",
            target_os = "openbsd",
            target_os = "netbsd",
        )
    ))]
    #[pyfunction]
    fn get_native_id() -> u64 {
        host_thread::native_id()
    }

    #[cfg(all(unix, feature = "threading"))]
    #[pyfunction]
    fn _stop_the_world_stats(vm: &VirtualMachine) -> PyResult<PyDictRef> {
        let stats = vm.state.stop_the_world.stats_snapshot();
        let d = vm.ctx.new_dict();
        d.set_item("stop_calls", vm.ctx.new_int(stats.stop_calls).into(), vm)?;
        d.set_item(
            "last_wait_ns",
            vm.ctx.new_int(stats.last_wait_ns).into(),
            vm,
        )?;
        d.set_item(
            "total_wait_ns",
            vm.ctx.new_int(stats.total_wait_ns).into(),
            vm,
        )?;
        d.set_item("max_wait_ns", vm.ctx.new_int(stats.max_wait_ns).into(), vm)?;
        d.set_item("poll_loops", vm.ctx.new_int(stats.poll_loops).into(), vm)?;
        d.set_item(
            "attached_seen",
            vm.ctx.new_int(stats.attached_seen).into(),
            vm,
        )?;
        d.set_item(
            "forced_parks",
            vm.ctx.new_int(stats.forced_parks).into(),
            vm,
        )?;
        d.set_item(
            "suspend_notifications",
            vm.ctx.new_int(stats.suspend_notifications).into(),
            vm,
        )?;
        d.set_item(
            "attach_wait_yields",
            vm.ctx.new_int(stats.attach_wait_yields).into(),
            vm,
        )?;
        d.set_item(
            "suspend_wait_yields",
            vm.ctx.new_int(stats.suspend_wait_yields).into(),
            vm,
        )?;
        d.set_item(
            "world_stopped",
            vm.ctx.new_bool(stats.world_stopped).into(),
            vm,
        )?;
        Ok(d)
    }

    #[cfg(all(unix, feature = "threading"))]
    #[pyfunction]
    fn _stop_the_world_reset_stats(vm: &VirtualMachine) {
        vm.state.stop_the_world.reset_stats();
    }

    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    #[pyattr]
    const _NAME_MAXLEN: usize = host_thread::NAME_MAXLEN;

    /// Truncate a Windows thread name to `_NAME_MAXLEN` UTF-16 code units,
    /// dropping a trailing non-BMP character that would not fit as a pair.
    #[cfg(windows)]
    fn truncate_thread_name_wide(name: &crate::common::wtf8::Wtf8) -> Vec<u16> {
        let encoded: Vec<u16> = name.encode_wide().collect();
        let mut units = Vec::new();
        let mut i = 0;
        while i < encoded.len() {
            let unit = encoded[i];
            if unit == 0 {
                break;
            }
            let width = match encoded.get(i + 1) {
                Some(&lo)
                    if (0xD800..=0xDBFF).contains(&unit) && (0xDC00..=0xDFFF).contains(&lo) =>
                {
                    2
                }
                _ => 1,
            };
            if units.len() + width > host_thread::NAME_MAXLEN {
                break;
            }
            units.extend_from_slice(&encoded[i..i + width]);
            i += width;
        }
        units.push(0);
        units
    }

    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    #[derive(FromArgs)]
    struct SetNameArgs {
        #[pyarg(any)]
        name: PyStrRef,
    }

    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    #[pyfunction]
    fn set_name(SetNameArgs { name }: SetNameArgs, vm: &VirtualMachine) -> PyResult<()> {
        #[cfg(windows)]
        {
            let units = truncate_thread_name_wide(name.as_wtf8());
            host_thread::set_current_thread_name_wide(&units).map_err(|e| e.to_pyexception(vm))?;
        }
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let os_name = vm.state.codec_registry.encode_text(
                name,
                "utf-8",
                Some(vm.ctx.new_utf8_str("replace")),
                vm,
            )?;
            host_thread::set_current_thread_name_bytes(os_name.as_bytes());
        }
        Ok(())
    }

    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    #[pyfunction]
    fn _get_name(vm: &VirtualMachine) -> PyResult {
        #[cfg(windows)]
        {
            let units =
                host_thread::current_thread_name_wide().map_err(|e| e.to_pyexception(vm))?;
            Ok(vm.ctx.new_str(Wtf8Buf::from_wide(&units)).into())
        }
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let bytes =
                host_thread::current_thread_name(host_thread::NAME_MAXLEN.saturating_add(1))
                    .map_err(|e| e.to_pyexception(vm))?;
            let os = unsafe { std::ffi::OsString::from_encoded_bytes_unchecked(bytes) };
            Ok(vm.fsdecode(os)?.into())
        }
    }

    /// Get OS-level thread ID (pthread_self on Unix, GetCurrentThreadId on Windows)
    /// This is important for fork compatibility - the ID must remain stable after fork
    fn current_thread_id() -> u64 {
        cfg_select! {
            any(unix, windows) => host_thread::current_thread_id(),
            _ => thread_to_rust_id(&thread::current()),
        }
    }

    /// Convert Rust thread to ID (used when no native thread id exists)
    #[cfg(not(any(unix, windows)))]
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
        #[allow(clippy::unnecessary_cast)]
        {
            // On Unix, use pthread ID from the handle
            use std::os::unix::thread::JoinHandleExt;
            handle.as_pthread_t() as _
        }

        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            host_thread::thread_id_from_handle(handle.as_raw_handle())
        }
        #[cfg(not(any(unix, windows)))]
        {
            thread_to_rust_id(handle.thread())
        }
    }

    #[pyfunction]
    const fn allocate_lock() -> Lock {
        Lock { mu: RawMutex::INIT }
    }

    struct StartNewThreadArgs {
        func_obj: PyObjectRef,
        args_obj: PyObjectRef,
        func: ArgCallable,
        args: PyTupleRef,
        kwargs: Option<PyDictRef>,
    }

    impl FromArgs for StartNewThreadArgs {
        const PARAMS: Option<&'static [Param]> = Some(&[
            Param::positional_only("function"),
            Param::positional_only("args"),
            Param {
                name: "kwargs",
                kind: ParamKind::PositionalOnly,
                default: Some(DefaultRepr::Raw("{}")),
            },
        ]);

        fn from_args(vm: &VirtualMachine, f_args: &mut FuncArgs) -> Result<Self, ArgumentError> {
            let func_obj = f_args.take_positional().ok_or(ArgumentError::TooFewArgs)?;
            let args_obj = f_args.take_positional().ok_or(ArgumentError::TooFewArgs)?;
            let kwargs_obj = f_args.take_positional();

            if func_obj.to_callable().is_none() {
                return Err(vm.new_type_error("first arg must be callable").into());
            }

            if !args_obj.fast_isinstance(vm.ctx.types.tuple_type) {
                return Err(vm.new_type_error("2nd arg must be a tuple").into());
            }

            if kwargs_obj
                .as_ref()
                .is_some_and(|obj| !obj.fast_isinstance(vm.ctx.types.dict_type))
            {
                return Err(vm
                    .new_type_error("optional 3rd arg must be a dictionary")
                    .into());
            }

            let func: ArgCallable = func_obj.clone().try_into_value(vm)?;
            let args: PyTupleRef = args_obj.clone().try_into_value(vm)?;
            let kwargs: Option<PyDictRef> =
                kwargs_obj.map(|obj| obj.try_into_value(vm)).transpose()?;
            Ok(Self {
                func_obj,
                args_obj,
                func,
                args,
                kwargs,
            })
        }
    }

    #[pyfunction]
    fn start_new_thread(parsed: StartNewThreadArgs, vm: &VirtualMachine) -> PyResult<u64> {
        let StartNewThreadArgs {
            func_obj,
            args_obj,
            func,
            args,
            kwargs,
        } = parsed;

        vm.audit("_thread.start_new_thread", || {
            (
                func_obj,
                args_obj,
                kwargs
                    .as_ref()
                    .map_or_else(|| vm.ctx.none(), |k| k.clone().into()),
            )
        })?;

        if !vm.state.allow_threads() {
            return Err(vm.new_runtime_error(
                "thread is not supported for isolated subinterpreters".to_owned(),
            ));
        }
        if vm
            .state
            .finalizing
            .load(core::sync::atomic::Ordering::Acquire)
        {
            return Err(vm.new_exception_msg(
                vm.ctx.exceptions.python_finalization_error.to_owned(),
                "can't create new thread at interpreter shutdown"
                    .to_owned()
                    .into(),
            ));
        }

        let args = FuncArgs::new(
            args.as_slice().to_vec(),
            kwargs
                .map_or_else(
                    || Ok(Default::default()),
                    |k| {
                        k.to_attributes(vm, |vm| Err(vm.new_type_error("keywords must be strings")))
                    },
                )?
                .into_iter()
                .map(|(k, v)| (k.as_str().to_owned(), v))
                .collect::<KwArgs>(),
        );
        let thread_builder = apply_thread_stack_size(thread::Builder::new(), vm);
        thread_builder
            .spawn(
                vm.new_thread()
                    .make_spawn_func(move |vm| run_thread(func, args, vm)),
            )
            .map(|handle| thread_to_id(&handle))
            .map_err(|_err| vm.new_runtime_error("can't start new thread"))
    }

    fn report_unraisable_thread_exception(
        exc: PyBaseExceptionRef,
        func: &ArgCallable,
        vm: &VirtualMachine,
    ) {
        let msg = func.as_ref().repr(vm).ok().map(|repr| {
            let mut message = Wtf8Buf::from("Exception ignored in thread started by ");
            message.push_wtf8(repr.as_wtf8());
            vm.ctx.new_str(message)
        });
        vm.run_unraisable_with_message(exc, msg, vm.ctx.none());
    }

    fn run_thread(func: ArgCallable, args: FuncArgs, vm: &VirtualMachine) {
        // Increment thread count when thread actually starts executing
        vm.state.thread_count.fetch_add(1);

        // Inner scope: drop `func` (and its Python refs) before the thread
        // slot is torn down below. Otherwise the parameter `func` would drop
        // at end-of-function, after cleanup_current_thread_frames has cleared
        // CURRENT_THREAD_SLOT, and a weakref callback fired during that drop
        // would panic in push_thread_frame.
        {
            let func = func;
            if let Err(exc) = func.invoke(args, vm)
                && !exc.fast_isinstance(vm.ctx.exceptions.system_exit)
            {
                report_unraisable_thread_exception(exc, &func, vm);
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
        vm.clear_async_exception();
        // Clean up frame tracking
        crate::vm::thread::cleanup_current_thread_frames(vm);
        vm.state.thread_count.fetch_sub(1);
    }

    /// Default stack size for Python threads in **debug builds only**, where
    /// Rust stack frames are substantially larger than in release. Rust's
    /// `std::thread::Builder` otherwise defaults to 2 MB, which is too small
    /// for the call chains the Python stdlib runs on helper threads in debug
    /// (e.g. the SSL test server, see #7941). Release builds keep the prior
    /// behavior — leave the stack size unset and let Rust's std default apply
    /// — to avoid oversized virtual stack mappings when many threads spawn.
    #[cfg(debug_assertions)]
    const DEFAULT_THREAD_STACK_SIZE: usize = 8 * 1024 * 1024;

    /// Configure a `thread::Builder` with the stack size to use for a new
    /// Python thread. Release builds use the value set via
    /// `threading.stack_size(N)` when the user has provided one (non-zero) and
    /// otherwise leave the builder unmodified (Rust's std default applies).
    ///
    /// Debug builds take [`DEFAULT_THREAD_STACK_SIZE`] as a floor rather than
    /// only as a default: an unoptimized `ExecutingFrame::run` reserves around
    /// eighty kilobytes of stack where an optimized one reserves under a
    /// thousand, so a size that holds a Python call chain in release holds
    /// three of its frames here — starting a thread at all needs six. The
    /// value `threading.stack_size()` reports is untouched.
    fn apply_thread_stack_size(
        thread_builder: thread::Builder,
        vm: &VirtualMachine,
    ) -> thread::Builder {
        let configured = vm.state.stacksize.load();
        #[cfg(debug_assertions)]
        {
            thread_builder.stack_size(configured.max(DEFAULT_THREAD_STACK_SIZE))
        }
        #[cfg(not(debug_assertions))]
        {
            if configured == 0 {
                thread_builder
            } else {
                thread_builder.stack_size(configured)
            }
        }
    }

    /// Clean up thread-local data for the current thread.
    /// This triggers __del__ on objects stored in thread-local variables.
    fn cleanup_thread_local_data() {
        // Move all guards out before dropping them. A local dict's __del__ may
        // re-enter thread-local access and borrow LOCAL_GUARDS again.
        let _guards = LOCAL_GUARDS.take();
    }

    #[cfg(all(not(target_arch = "wasm32"), feature = "host_env"))]
    #[derive(FromArgs)]
    struct InterruptMainArgs {
        // The signal enum's repr is not valid syntax in a text signature.
        #[pyarg(positional, optional, py_default = "signal.SIGINT")]
        signum: OptionalArg<SignalNum>,
    }

    #[cfg(all(not(target_arch = "wasm32"), feature = "host_env"))]
    #[pyfunction]
    fn interrupt_main(args: InterruptMainArgs) -> PyResult<()> {
        let sig = args.signum.unwrap_or(SignalNum::SIGINT);
        crate::signal::set_interrupt_ex(sig)
    }

    #[pyfunction]
    fn exit(vm: &VirtualMachine) -> PyResult {
        Err(vm.new_system_exit(vec![].into()))
    }

    thread_local!(static SENTINELS: RefCell<Vec<PyRef<Lock>>> = const { RefCell::new(Vec::new()) });

    #[pyfunction]
    fn _set_sentinel(vm: &VirtualMachine) -> PyRef<Lock> {
        let lock = Lock { mu: RawMutex::INIT }.into_ref(&vm.ctx);
        SENTINELS.with_borrow_mut(|sentinels| sentinels.push(lock.clone()));
        lock
    }

    #[derive(FromArgs)]
    struct StackSizeArgs {
        #[pyarg(positional, default = 0)]
        size: ArgIndex,
    }

    #[pyfunction]
    fn stack_size(args: StackSizeArgs, vm: &VirtualMachine) -> PyResult<usize> {
        const MIN_SIZE: usize = PY_OS_MIN_STACK_SIZE + SYSTEM_PAGE_SIZE;
        let StackSizeArgs { size } = args;

        let Ok(size) = size.as_ref().try_to_primitive(vm) else {
            return Err(vm.new_value_error(format!("size must be at least {MIN_SIZE} bytes")));
        };

        if size != 0 && size < MIN_SIZE {
            return Err(vm.new_value_error(format!("size must be at least {MIN_SIZE} bytes")));
        }

        Ok(vm.state.stacksize.swap(size))
    }

    #[pyfunction]
    fn _count(vm: &VirtualMachine) -> usize {
        vm.state.thread_count.load()
    }

    #[pyfunction]
    fn daemon_threads_allowed(vm: &VirtualMachine) -> bool {
        vm.state.allow_daemon_threads()
    }

    // Registry for non-daemon threads that need to be joined at shutdown
    pub(crate) type ShutdownEntry = (
        Weak<parking_lot::Mutex<ThreadHandleInner>>,
        Weak<(parking_lot::Mutex<bool>, parking_lot::Condvar)>,
    );

    #[pyfunction]
    pub(crate) fn _shutdown(vm: &VirtualMachine) {
        // Wait for all non-daemon threads to finish
        let current_ident = get_ident();

        loop {
            // Find a thread that's not finished and not the current thread
            let handle_to_join = {
                let mut handles = vm.state.shutdown_handles.lock();
                // Drop expired entries, but retain the current thread's entry:
                // another thread may need to wait for its teardown.
                handles.retain(|(inner_weak, done_event_weak): &ShutdownEntry| {
                    inner_weak.upgrade().is_some() && done_event_weak.upgrade().is_some()
                });

                // Find first unfinished handle
                handles
                    .iter()
                    .find_map(|(inner_weak, done_event_weak): &ShutdownEntry| {
                        let inner = inner_weak.upgrade()?;
                        let done_event = done_event_weak.upgrade()?;
                        let guard = inner.lock();
                        if guard.ident == current_ident {
                            None
                        } else {
                            Some((inner.clone(), done_event))
                        }
                    })
            };

            match handle_to_join {
                Some((inner, done_event)) => {
                    // The starter publishes Running after the child reports its
                    // identity. Let it finish instead of reporting an unraisable
                    // "thread not started" during that window.
                    let starting = inner.lock().state == ThreadHandleState::Starting;
                    if starting {
                        vm.allow_threads(std::thread::yield_now);
                        continue;
                    }
                    if let Err(exc) = ThreadHandle::join_internal(&inner, &done_event, None, vm) {
                        vm.run_unraisable(
                            exc,
                            Some(
                                "Exception ignored while joining a thread in _thread._shutdown()"
                                    .to_owned(),
                            ),
                            vm.ctx.none(),
                        );
                        return;
                    }
                }
                None => break, // No more threads to wait on
            }
        }
    }

    // The caller stops the world before using this to decide finalization.
    pub(crate) fn has_pending_shutdown_threads(vm: &VirtualMachine) -> bool {
        let current_ident = get_ident();
        vm.state
            .shutdown_handles
            .lock()
            .iter()
            .any(|(inner, event)| {
                inner.upgrade().is_some_and(|inner| {
                    event.upgrade().is_some() && inner.lock().ident != current_ident
                })
            })
    }

    /// Add a non-daemon thread handle to the shutdown registry
    fn add_to_shutdown_handles(
        vm: &VirtualMachine,
        inner: &Arc<parking_lot::Mutex<ThreadHandleInner>>,
        done_event: &Arc<(parking_lot::Mutex<bool>, parking_lot::Condvar)>,
    ) {
        let mut handles = vm.state.shutdown_handles.lock();
        handles.push((Arc::downgrade(inner), Arc::downgrade(done_event)));
    }

    fn remove_from_shutdown_handles(
        vm: &VirtualMachine,
        inner: &Arc<parking_lot::Mutex<ThreadHandleInner>>,
        done_event: &Arc<(parking_lot::Mutex<bool>, parking_lot::Condvar)>,
    ) {
        remove_from_shutdown_handles_for_state(&vm.state, inner, done_event);
    }

    fn remove_from_shutdown_handles_for_state(
        state: &crate::vm::PyGlobalState,
        inner: &Arc<parking_lot::Mutex<ThreadHandleInner>>,
        done_event: &Arc<(parking_lot::Mutex<bool>, parking_lot::Condvar)>,
    ) {
        let mut handles = state.shutdown_handles.lock();
        handles.retain(|(inner_weak, done_event_weak): &ShutdownEntry| {
            let Some(registered_inner) = inner_weak.upgrade() else {
                return false;
            };
            let Some(registered_done_event) = done_event_weak.upgrade() else {
                return false;
            };
            !(Arc::ptr_eq(&registered_inner, inner)
                && Arc::ptr_eq(&registered_done_event, done_event))
        });
    }

    // Keep the shutdown entry until the VM has detached and been dropped.
    // This guard also signals joiners if the worker unwinds.
    struct WorkerCompletion {
        vm: Option<crate::vm::thread::ThreadedVirtualMachine>,
        state: PyRc<crate::vm::PyGlobalState>,
        inner: Arc<parking_lot::Mutex<ThreadHandleInner>>,
        done_event: Arc<(parking_lot::Mutex<bool>, parking_lot::Condvar)>,
    }

    impl Drop for WorkerCompletion {
        fn drop(&mut self) {
            let state = self.state.clone();
            let inner = self.inner.clone();
            let done_event = self.done_event.clone();
            // Publish completion even if dropping the VM unwinds.
            let _publish = scopeguard::guard((), move |_| {
                inner.lock().state = ThreadHandleState::Done;
                remove_from_shutdown_handles_for_state(&state, &inner, &done_event);
                let (lock, cvar) = &*done_event;
                *lock.lock() = true;
                cvar.notify_all();
            });
            drop(self.vm.take());
        }
    }

    #[derive(FromArgs)]
    struct MakeThreadHandleArgs {
        // An `int` only; the range is checked separately so it stays an OverflowError.
        #[pyarg(positional, error_msg = "ident must be an integer")]
        ident: PyIntRef,
    }

    #[pyfunction]
    fn _make_thread_handle(
        args: MakeThreadHandleArgs,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<ThreadHandle>> {
        let ident = args.ident.try_to_primitive::<u64>(vm)?;
        let handle = ThreadHandle::new(vm);
        {
            let mut inner = handle.inner.lock();
            inner.ident = ident;
            inner.state = ThreadHandleState::Running;
        }
        Ok(handle.into_ref(&vm.ctx))
    }

    #[pyfunction]
    fn _get_main_thread_ident(vm: &VirtualMachine) -> u64 {
        vm.state.main_thread_ident.load()
    }

    #[pyfunction]
    fn _is_main_interpreter(vm: &VirtualMachine) -> bool {
        vm.state.is_main_interpreter()
    }

    /// Initialize the main thread ident. Should be called once at interpreter startup.
    pub(crate) fn init_main_thread_ident(vm: &VirtualMachine) {
        let ident = get_ident();
        vm.state.main_thread_ident.store(ident);
    }

    // This allows threading.py to import _excepthook and _ExceptHookArgs from _thread
    #[pystruct_sequence_data]
    struct ExceptHookArgsData {
        exc_type: crate::PyObjectRef,
        exc_value: crate::PyObjectRef,
        exc_traceback: crate::PyObjectRef,
        thread: crate::PyObjectRef,
    }

    #[pyattr]
    #[pystruct_sequence(
        name = "_ExceptHookArgs",
        module = "_thread",
        data = "ExceptHookArgsData"
    )]
    struct PyExceptHookArgs;

    #[pyclass(with(PyStructSequence))]
    impl PyExceptHookArgs {
        #[extend_class]
        fn extend_pyclass(ctx: &crate::vm::Context, class: &'static Py<crate::builtins::PyType>) {
            // (type, (sequence, dict)). Set before the trait installs its default.
            const EXCEPT_HOOK_ARGS_REDUCE: crate::function::PyMethodDef =
                crate::function::PyMethodDef::new_const(
                    "__reduce__",
                    |zelf: crate::PyRef<crate::builtins::PyTuple>,
                     vm: &VirtualMachine|
                     -> PyTupleRef {
                        vm.new_tuple((
                            zelf.class().to_owned(),
                            (
                                vm.ctx.new_tuple(zelf.as_slice().to_vec()),
                                vm.ctx.new_dict(),
                            ),
                        ))
                    },
                    crate::function::PyMethodFlags::METHOD,
                    crate::function::ItemDoc::static_text("__reduce__($self, /)\n--\n\n"),
                );
            class.set_attr(
                ctx.intern_str("__reduce__"),
                EXCEPT_HOOK_ARGS_REDUCE.to_proper_method(class, ctx),
            );
        }
    }

    #[pyfunction]
    fn _excepthook(args: crate::PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        // Type check: args must be _ExceptHookArgs
        let args = args.downcast::<PyExceptHookArgs>().map_err(|_| {
            vm.new_type_error("_thread.excepthook argument type must be ExceptHookArgs")
        })?;
        let fields = args.0.as_slice();
        let (Some(exc_type), Some(exc_value), Some(exc_traceback), Some(thread)) =
            (fields.first(), fields.get(1), fields.get(2), fields.get(3))
        else {
            return Err(
                vm.new_type_error("_thread.excepthook argument type must be ExceptHookArgs")
            );
        };
        let exc_type = exc_type.clone();
        let exc_value = exc_value.clone();
        let exc_traceback = exc_traceback.clone();
        let thread = thread.clone();

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
                .map(|s| s.as_wtf8().to_owned())
        } else {
            None
        };
        let name = thread_name.unwrap_or_else(|| Wtf8Buf::from(format!("{}", get_ident())));

        let _ = vm.call_method(&file, "write", (format!("Exception in thread {name}:\n"),));

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
        static LOCAL_GUARDS: RefCell<Vec<LocalGuard>> = const { RefCell::new(Vec::new()) };
    }

    // Guard that removes thread-local data when dropped
    struct LocalGuard {
        local: Weak<LocalData>,
        thread_id: u64,
    }

    impl Drop for LocalGuard {
        fn drop(&mut self) {
            if let Some(local_data) = self.local.upgrade() {
                // Remove from map while holding the lock, but drop the value
                // outside the lock to prevent deadlock if __del__ accesses _local
                let _removed = local_data.state.lock().dicts.remove(&self.thread_id);
            }
        }
    }

    struct LocalState {
        init_args: FuncArgs,
        dicts: std::collections::HashMap<u64, PyDictRef>,
    }

    unsafe impl Traverse for LocalState {
        fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
            self.init_args.traverse(tracer_fn);
            #[allow(clippy::iter_over_hash_type)]
            for dict in self.dicts.values() {
                dict.traverse(tracer_fn);
            }
        }

        fn clear_refs(&mut self, out: &mut Vec<crate::PyObjectRef>) {
            out.append(&mut self.init_args.args);
            out.extend(self.init_args.kwargs.drain(..).map(|(_, value)| value));
            out.extend(self.dicts.drain().map(|(_, dict)| dict.into()));
        }
    }

    // Shared data structure for Local
    struct LocalData {
        state: PyMutex<LocalState>,
    }

    impl fmt::Debug for LocalData {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("LocalData").finish_non_exhaustive()
        }
    }

    #[pyattr]
    #[pyclass(module = "_thread", name = "_local", traverse = "manual")]
    #[derive(Debug, PyPayload)]
    struct Local {
        inner: Arc<LocalData>,
    }

    unsafe impl Traverse for Local {
        fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
            self.inner.state.traverse(tracer_fn);
        }

        fn clear_refs(&mut self, out: &mut Vec<crate::PyObjectRef>) {
            if let Some(mut state) = self.inner.state.try_lock() {
                state.clear_refs(out);
            }
        }
    }

    impl Constructor for Local {
        type Args = ();

        fn py_new(_cls: &Py<PyType>, _args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            Err(vm.new_type_error("use slot_new"))
        }
    }

    #[pyclass(with(Constructor, GetAttr, SetAttr), flags(BASETYPE))]
    impl Local {
        fn custom_init(cls: &Py<PyType>, vm: &VirtualMachine) -> Option<crate::types::InitFunc> {
            let cls_init = cls.slots.init.load()?;
            let object_init = vm
                .ctx
                .types
                .object_type
                .slots
                .init
                .load()
                .map(|init| crate::types::fn_addr(init));
            (Some(crate::types::fn_addr(cls_init)) != object_init).then_some(cls_init)
        }

        fn create_dict(&self, vm: &VirtualMachine) -> (PyDictRef, bool) {
            let thread_id = current_thread_id();

            // Fast path: check if dict exists under lock
            let value = self.inner.state.lock().dicts.get(&thread_id).cloned();
            if let Some(dict) = value {
                return (dict, false);
            }

            // Slow path: allocate dict outside lock to reduce lock hold time
            let new_dict = vm.ctx.new_dict();

            // Insert with double-check to handle races
            let mut state = self.inner.state.lock();
            use std::collections::hash_map::Entry;
            let (dict, need_guard) = match state.dicts.entry(thread_id) {
                Entry::Occupied(e) => (e.get().clone(), false),
                Entry::Vacant(e) => {
                    e.insert(new_dict.clone());
                    (new_dict, true)
                }
            };
            drop(state); // Release lock before TLS access

            // Register cleanup guard only if we inserted a new entry
            if need_guard {
                let guard = LocalGuard {
                    local: Arc::downgrade(&self.inner),
                    thread_id,
                };
                LOCAL_GUARDS.with(|guards| {
                    guards.borrow_mut().push(guard);
                });
            }

            (dict, need_guard)
        }

        fn remove_current_dict(&self) {
            let thread_id = current_thread_id();
            let guard = LOCAL_GUARDS.with(|guards| {
                let mut guards = guards.borrow_mut();
                guards
                    .iter()
                    .rposition(|guard| {
                        guard.thread_id == thread_id
                            && guard.local.as_ptr() == Arc::as_ptr(&self.inner)
                    })
                    .map(|position| guards.remove(position))
            });

            if let Some(guard) = guard {
                drop(guard);
            } else {
                let _removed = self.inner.state.lock().dicts.remove(&thread_id);
            }
        }

        fn l_dict(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyDictRef> {
            let (dict, created) = zelf.create_dict(vm);
            if !created {
                return Ok(dict);
            }

            let Some(init) = Self::custom_init(zelf.class(), vm) else {
                return Ok(dict);
            };
            let init_args = zelf.inner.state.lock().init_args.clone();
            if let Err(err) = init(zelf.as_object(), init_args, vm) {
                zelf.remove_current_dict();
                return Err(err);
            }

            Ok(dict)
        }

        #[pygetset]
        fn __dict__(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyDictRef> {
            Self::l_dict(&zelf, vm)
        }

        #[pyslot]
        fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            if !args.is_empty() && Self::custom_init(&cls, vm).is_none() {
                return Err(vm.new_type_error("Initialization arguments are not supported"));
            }

            let zelf = Self {
                inner: Arc::new(LocalData {
                    state: PyMutex::new(LocalState {
                        init_args: args,
                        dicts: std::collections::HashMap::new(),
                    }),
                }),
            }
            .into_ref_with_type(vm, cls)?;

            // type.__call__ invokes __init__ after __new__. Create this thread's
            // dict first so assignments made by __init__ cannot recursively
            // initialize the same local object.
            zelf.create_dict(vm);
            Ok(zelf.into())
        }
    }

    impl GetAttr for Local {
        fn getattro(zelf: &Py<Self>, attr: &Py<PyStr>, vm: &VirtualMachine) -> PyResult {
            let l_dict = Self::l_dict(zelf, vm)?;
            if attr.as_bytes() == b"__dict__" {
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
            if attr.as_bytes() == b"__dict__" {
                Err(vm.new_attribute_error(format!(
                    "{} attribute '__dict__' is read-only",
                    zelf.class().name()
                )))
            } else {
                let dict = Self::l_dict(zelf, vm)?;
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
    pub(crate) type HandleEntry = (
        Weak<parking_lot::Mutex<ThreadHandleInner>>,
        Weak<(parking_lot::Mutex<bool>, parking_lot::Condvar)>,
    );

    // Re-export type from vm::thread for PyGlobalState
    pub(crate) use crate::vm::thread::CurrentFrameSlot;

    /// Get all threads' current (top) frames. Used by sys._current_frames().
    pub(crate) fn get_all_current_frames(vm: &VirtualMachine) -> Vec<(u64, FrameObjectRef)> {
        // unix: read each thread's published top frame under stop-the-world so
        // the owning thread is parked at a safepoint and cannot pop or free the
        // frame while we take a strong reference. Request stop-the-world before
        // the registry lock to avoid deadlocking a thread parking mid-registry.
        //
        // For the current thread, use TLS CURRENT_FRAME directly because
        // stack-allocated frames only update TLS (not top_frame).
        #[cfg(unix)]
        {
            use core::sync::atomic::Ordering;
            let current_ident = get_ident();
            vm.state.stop_the_world.stop_the_world(&vm.state);
            scopeguard::defer! { vm.state.stop_the_world.start_the_world(&vm.state); }
            let registry = vm.state.thread_frames.lock();
            registry
                .iter()
                .filter_map(|(id, slot)| {
                    if *id == current_ident {
                        // Current thread: materialize from TLS chain
                        crate::frame::current_thread_frame_materialize(vm).map(|frame| (*id, frame))
                    } else {
                        // Other threads: try top_frame first (FrameObject),
                        // fall back to top_iframe (may be a stack-allocated frame).
                        let top = slot.top_frame.load(Ordering::Relaxed);
                        if let Some(p) = core::ptr::NonNull::new(top) {
                            // SAFETY: world stopped -> the owning thread is parked
                            // with this frame on its chain, so it is alive.
                            let py = unsafe { p.as_ref() };
                            Some((*id, py.to_owned()))
                        } else {
                            // Stack-allocated frame: materialize from top_iframe.
                            // SAFETY: world stopped -> owning thread is parked.
                            let iframe_ptr = slot.top_iframe.load(Ordering::Relaxed)
                                as *const crate::frame::InterpreterFrame;
                            if !iframe_ptr.is_null() {
                                let iframe = unsafe { &*iframe_ptr };
                                // SAFETY: world stopped -> owning thread parked.
                                let fo = unsafe { iframe.materialize_detached_chain(vm) };
                                Some((*id, fo))
                            } else {
                                None
                            }
                        }
                    }
                })
                .collect()
        }
        #[cfg(not(unix))]
        {
            use core::sync::atomic::Ordering;
            let current_ident = get_ident();
            vm.state.stop_the_world.stop_the_world(&vm.state);
            scopeguard::defer! { vm.state.stop_the_world.start_the_world(&vm.state); }
            let registry = vm.state.thread_frames.lock();
            registry
                .iter()
                .filter_map(|(id, slot)| {
                    if *id == current_ident {
                        // Current thread: materialize from TLS chain
                        crate::frame::current_thread_frame_materialize(vm).map(|frame| (*id, frame))
                    } else {
                        // Other threads: use top_iframe to include
                        // stack-allocated frames. Materialize the entire
                        // chain and link retained_back so f_back works.
                        // SAFETY: world stopped -> owning thread is parked.
                        let iframe_ptr = slot.top_iframe.load(Ordering::Relaxed)
                            as *const crate::frame::InterpreterFrame;
                        if !iframe_ptr.is_null() {
                            let iframe = unsafe { &*iframe_ptr };
                            // SAFETY: world stopped -> owning thread parked.
                            let fo = unsafe { iframe.materialize_detached_chain(vm) };
                            Some((*id, fo))
                        } else {
                            // Fall back to frames stack for FrameObject-only path
                            let frames = slot.frames.lock();
                            frames
                                .last()
                                .map(|fp| (*id, unsafe { fp.as_ref() }.to_owned()))
                        }
                    }
                })
                .collect()
        }
    }

    /// Called after fork() in child process to mark all other threads as done.
    /// This prevents join() from hanging on threads that don't exist in the child.
    ///
    /// Precondition: `reinit_locks_after_fork()` has already been called, so all
    /// parking_lot-based locks in VmState are in unlocked state.
    #[cfg(all(unix, feature = "threading", feature = "host_env"))]
    pub(crate) fn after_fork_child(vm: &VirtualMachine) {
        let current_ident = get_ident();

        // Update main thread ident - after fork, the current thread becomes the main thread
        vm.state.main_thread_ident.store(current_ident);

        // Reinitialize frame slot for current thread.
        // Locks are already reinit'd, so lock() is safe.
        crate::vm::thread::reinit_frame_slot_after_fork(vm);

        // Clean up thread handles. All VmState locks were reinit'd to unlocked,
        // so lock() won't deadlock. Per-thread Arc<Mutex<ThreadHandleInner>>
        // locks are also reinit'd below before use.
        {
            let mut handles = vm.state.thread_handles.lock();
            handles.retain(|(inner_weak, done_event_weak): &HandleEntry| {
                let Some(inner) = inner_weak.upgrade() else {
                    return false;
                };
                let Some(done_event) = done_event_weak.upgrade() else {
                    return false;
                };

                // SAFETY: only the forking thread survives in the child.
                unsafe { reinit_handle_after_fork(&inner, &done_event, current_ident) };

                true
            });
        }

        // Every shutdown entry comes from ThreadHandle::new and is also in
        // thread_handles, so its completion event was already reset above.
        // Clean up shutdown_handles.
        {
            let mut handles = vm.state.shutdown_handles.lock();
            handles.retain(|(inner_weak, done_event_weak): &ShutdownEntry| {
                let Some(inner) = inner_weak.upgrade() else {
                    return false;
                };
                let Some(done_event) = done_event_weak.upgrade() else {
                    return false;
                };

                reinit_parking_lot_mutex(&inner);
                let mut inner_guard = inner.lock();

                if inner_guard.ident == current_ident {
                    return true;
                }
                if inner_guard.state == ThreadHandleState::NotStarted {
                    return true;
                }

                inner_guard.state = ThreadHandleState::Done;
                drop(inner_guard);

                let (lock, cvar) = &*done_event;
                reinit_parking_lot_mutex(lock);
                reinit_parking_lot_condvar(cvar);
                *lock.lock() = true;

                false
            });
        }
    }

    /// Repair a copied handle before any child thread cleanup or join.
    ///
    /// Unlike CPython's atomic completion event, ours includes a mutex that a
    /// parent joiner can hold while detached. Even the surviving thread's
    /// completion mutex and condvar must lose that dead owner's/waiter's state.
    /// Its completion value remains unchanged until the thread really exits.
    ///
    /// # Safety
    /// Only call in the single-threaded post-fork child, with no live guards or
    /// waiters on these locks.
    #[cfg(all(unix, feature = "host_env"))]
    unsafe fn reinit_handle_after_fork(
        inner: &parking_lot::Mutex<ThreadHandleInner>,
        done_event: &(parking_lot::Mutex<bool>, parking_lot::Condvar),
        current_ident: u64,
    ) {
        reinit_parking_lot_mutex(inner);
        let (lock, cvar) = done_event;
        reinit_parking_lot_mutex(lock);
        reinit_parking_lot_condvar(cvar);
        let mut inner_guard = inner.lock();
        if inner_guard.ident == current_ident || inner_guard.state == ThreadHandleState::NotStarted
        {
            return;
        }

        inner_guard.state = ThreadHandleState::Done;
        // The OS thread did not survive. Do not pthread_detach a copied handle.
        if let Some(handle) = inner_guard.join_handle.take() {
            core::mem::forget(handle);
        }
        drop(inner_guard);
        // Do not notify waiters: none survived the fork.
        *lock.lock() = true;
    }

    /// Take a thread handle's completion mutex, detaching first.
    ///
    /// A joiner holds this mutex across its `allow_threads` wait, so it can
    /// still hold it when stop-the-world stops it. An attached thread that
    /// blocked on it would never reach a safepoint, so the stop could never
    /// complete and the holder would never be resumed to release it.
    fn lock_done<'a>(
        lock: &'a parking_lot::Mutex<bool>,
        vm: &VirtualMachine,
    ) -> parking_lot::MutexGuard<'a, bool> {
        vm.allow_threads(|| lock.lock())
    }

    /// Reset a parking_lot::Mutex to unlocked state after fork.
    #[cfg(all(unix, feature = "host_env"))]
    fn reinit_parking_lot_mutex<T: ?Sized>(mutex: &parking_lot::Mutex<T>) {
        unsafe { rustpython_common::lock::zero_reinit_after_fork(mutex.raw()) };
    }

    /// Reset a parking_lot::Condvar after fork.
    ///
    /// `notify_all()` would unpark waiters that did not survive the fork.
    #[cfg(all(unix, feature = "host_env"))]
    fn reinit_parking_lot_condvar(cvar: &parking_lot::Condvar) {
        unsafe { rustpython_common::lock::zero_reinit_after_fork(cvar) };
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

    // _ThreadHandle - handle for joinable threads
    #[pyattr]
    #[pyclass(module = "_thread", name = "_ThreadHandle")]
    #[derive(Debug, PyPayload)]
    struct ThreadHandle {
        inner: Arc<parking_lot::Mutex<ThreadHandleInner>>,
        // Event to signal thread completion (for timed join support)
        done_event: Arc<(parking_lot::Mutex<bool>, parking_lot::Condvar)>,
    }

    impl ThreadHandle {
        fn new(vm: &VirtualMachine) -> Self {
            let inner = Arc::new(parking_lot::Mutex::new(ThreadHandleInner {
                state: ThreadHandleState::NotStarted,
                ident: 0,
                join_handle: None,
                joining: false,
                joined: false,
            }));
            let done_event =
                Arc::new((parking_lot::Mutex::new(false), parking_lot::Condvar::new()));

            // Register in global registry for fork cleanup
            vm.state
                .thread_handles
                .lock()
                .push((Arc::downgrade(&inner), Arc::downgrade(&done_event)));

            Self { inner, done_event }
        }

        fn join_wait_slice(timeout: Duration, elapsed: Duration) -> Duration {
            timeout.saturating_sub(elapsed).min(Duration::from_micros(
                pytime::THREAD_TIMEOUT_MAX_MICROSECONDS as u64,
            ))
        }

        fn join_internal(
            inner: &Arc<parking_lot::Mutex<ThreadHandleInner>>,
            done_event: &Arc<(parking_lot::Mutex<bool>, parking_lot::Condvar)>,
            timeout_duration: Option<Duration>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            Self::check_started(inner, vm)?;

            let started = std::time::Instant::now();

            // Wait for thread completion using Condvar (supports timeout)
            // Loop to handle spurious wakeups
            let (lock, cvar) = &**done_event;
            let mut done = lock_done(lock, vm);

            // ThreadHandle_join semantics: self-join/finalizing checks
            // apply only while target thread has not reported it is exiting yet.
            if !*done {
                let inner_guard = inner.lock();
                let current_ident = get_ident();
                if inner_guard.ident == current_ident
                    && inner_guard.state == ThreadHandleState::Running
                {
                    return Err(vm.new_runtime_error("Cannot join current thread"));
                }
                if vm
                    .state
                    .finalizing
                    .load(core::sync::atomic::Ordering::Acquire)
                {
                    return Err(vm.new_exception_msg(
                        vm.ctx.exceptions.python_finalization_error.to_owned(),
                        "cannot join thread at interpreter shutdown"
                            .to_owned()
                            .into(),
                    ));
                }
            }

            while !*done {
                if let Some(timeout) = timeout_duration {
                    let wait = Self::join_wait_slice(timeout, started.elapsed());
                    if wait.is_zero() {
                        return Ok(());
                    }
                    // Keep each Windows wait below INFINITE and retain the
                    // original timeout budget after a slice or spurious wakeup.
                    vm.allow_threads(|| cvar.wait_for(&mut done, wait));
                } else {
                    // Infinite wait
                    vm.allow_threads(|| cvar.wait(&mut done));
                }
            }
            drop(done);

            // Thread is done, now perform cleanup
            let join_handle = {
                let mut inner_guard = inner.lock();

                // If already joined, return immediately (idempotent)
                if inner_guard.joined {
                    return Ok(());
                }

                // If another thread is already joining, wait for them to finish
                if inner_guard.joining {
                    drop(inner_guard);
                    // Wait on done_event
                    let (lock, cvar) = &**done_event;
                    let mut done = lock_done(lock, vm);
                    while !*done {
                        vm.allow_threads(|| cvar.wait(&mut done));
                    }
                    return Ok(());
                }

                // Mark that we're joining
                inner_guard.joining = true;

                // Take the join handle if available
                inner_guard.join_handle.take()
            };

            // Perform the actual join outside the lock
            if let Some(handle) = join_handle {
                // Ignore the result - panics in spawned threads are already handled
                let _ = vm.allow_threads(|| handle.join());
            }

            // Mark as joined and clear joining flag
            {
                let mut inner_guard = inner.lock();
                inner_guard.joined = true;
                inner_guard.joining = false;
            }

            Ok(())
        }

        fn check_started(
            inner: &Arc<parking_lot::Mutex<ThreadHandleInner>>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let state = inner.lock().state;
            if matches!(
                state,
                ThreadHandleState::NotStarted | ThreadHandleState::Starting
            ) {
                return Err(vm.new_runtime_error("thread not started"));
            }
            Ok(())
        }

        fn set_done_internal(
            inner: &Arc<parking_lot::Mutex<ThreadHandleInner>>,
            done_event: &Arc<(parking_lot::Mutex<bool>, parking_lot::Condvar)>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            Self::check_started(inner, vm)?;
            {
                let mut inner_guard = inner.lock();
                inner_guard.state = ThreadHandleState::Done;
                // _set_done() detach path. Dropping the JoinHandle
                // detaches the underlying Rust thread.
                inner_guard.join_handle = None;
                inner_guard.joining = false;
                inner_guard.joined = true;
            }
            remove_from_shutdown_handles(vm, inner, done_event);

            let (lock, cvar) = &**done_event;
            *lock_done(lock, vm) = true;
            cvar.notify_all();
            Ok(())
        }

        fn parse_join_timeout(
            timeout_obj: Option<crate::PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<Option<Duration>> {
            let Some(timeout_obj) = timeout_obj else {
                return Ok(None);
            };
            let nanoseconds = pytime::from_seconds_object(&timeout_obj, pytime::Round::Up, vm)?;
            Ok((nanoseconds >= 0).then(|| Duration::from_nanos(nanoseconds as u64)))
        }
    }

    #[pyclass(with(Representable))]
    impl Py<ThreadHandle> {
        #[pygetset]
        fn ident(&self) -> u64 {
            self.inner.lock().ident
        }

        #[pymethod]
        fn is_done(&self, vm: &VirtualMachine) -> PyResult<bool> {
            // If completion was observed, perform one-time join cleanup
            // before returning True.
            let done = {
                let (lock, _) = &*self.done_event;
                *lock_done(lock, vm)
            };
            if !done {
                return Ok(false);
            }
            ThreadHandle::join_internal(&self.inner, &self.done_event, Some(Duration::ZERO), vm)?;
            Ok(true)
        }

        #[pymethod]
        fn _set_done(&self, vm: &VirtualMachine) -> PyResult<()> {
            ThreadHandle::set_done_internal(&self.inner, &self.done_event, vm)
        }

        #[pymethod]
        fn join(&self, mut f_args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
            if !f_args.kwargs.is_empty() {
                return Err(vm.new_type_error("_ThreadHandle.join() takes no keyword arguments"));
            }
            let given = f_args.args.len();
            if given > 1 {
                return Err(
                    vm.new_type_error(format!("join() takes at most 1 argument ({given} given)"))
                );
            }
            let timeout = f_args.take_positional().filter(|obj| !vm.is_none(obj));
            let timeout_duration = ThreadHandle::parse_join_timeout(timeout, vm)?;
            ThreadHandle::join_internal(&self.inner, &self.done_event, timeout_duration, vm)
        }

        #[pyslot]
        fn slot_new(cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            ThreadHandle::new(vm)
                .into_ref_with_type(vm, cls)
                .map(Into::into)
        }
    }

    impl Representable for ThreadHandle {
        fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            let ident = zelf.inner.lock().ident;
            Ok(format!(
                "<{} object: ident={ident}>",
                zelf.class().slot_name()
            ))
        }
    }

    struct StartJoinableThreadArgs {
        function_obj: PyObjectRef,
        function: ArgCallable,
        handle: Option<PyRef<ThreadHandle>>,
        daemon: bool,
    }

    impl FromArgs for StartJoinableThreadArgs {
        // PyArg_ParseTupleAndKeywords converts daemon before checking unknown names.
        const BINDING_PARAMS: Option<&'static [Param]> = None;

        const PARAMS: Option<&'static [Param]> = Some(&[
            Param::positional_or_keyword("function"),
            Param {
                name: "handle",
                kind: ParamKind::PositionalOrKeyword,
                default: Some(DefaultRepr::None),
            },
            Param {
                name: "daemon",
                kind: ParamKind::PositionalOrKeyword,
                default: Some(DefaultRepr::Bool(true)),
            },
        ]);

        fn from_args(vm: &VirtualMachine, f_args: &mut FuncArgs) -> Result<Self, ArgumentError> {
            let given = f_args.args.len() + f_args.kwargs.len();
            if given > 3 {
                return Err(vm
                    .new_type_error(format!(
                        "start_joinable_thread() takes at most 3 arguments ({given} given)"
                    ))
                    .into());
            }

            let function_pos = f_args.take_positional();
            let function_kw = f_args.take_keyword("function");
            if function_pos.is_some() && function_kw.is_some() {
                return Err(vm
                    .new_type_error(
                        "argument for start_joinable_thread() given by name ('function') and position (1)",
                    )
                    .into());
            }
            let Some(function_obj) = function_pos.or(function_kw) else {
                return Err(vm
                    .new_type_error(
                        "start_joinable_thread() missing required argument 'function' (pos 1)",
                    )
                    .into());
            };

            let handle_pos = f_args.take_positional();
            let handle_kw = f_args.take_keyword("handle");
            if handle_pos.is_some() && handle_kw.is_some() {
                return Err(vm
                    .new_type_error(
                        "argument for start_joinable_thread() given by name ('handle') and position (2)",
                    )
                    .into());
            }
            let handle_obj = handle_pos.or(handle_kw);

            let daemon_pos = f_args.take_positional();
            let daemon_kw = f_args.take_keyword("daemon");
            if daemon_pos.is_some() && daemon_kw.is_some() {
                return Err(vm
                    .new_type_error(
                        "argument for start_joinable_thread() given by name ('daemon') and position (3)",
                    )
                    .into());
            }
            let daemon = daemon_pos
                .or(daemon_kw)
                .map_or(Ok(true), |obj| obj.try_to_bool(vm))?;

            // Required argument errors are raised before unknown keyword errors
            // when `function` is missing.
            if let Some(unexpected) = f_args.kwargs.keys().next() {
                let suggestion = ["function", "handle", "daemon"]
                    .iter()
                    .filter_map(|candidate| {
                        let max_distance = (unexpected.len() + candidate.len() + 3) * MOVE_COST / 6;
                        let distance = levenshtein_distance(
                            unexpected.as_bytes(),
                            candidate.as_bytes(),
                            max_distance,
                        );
                        (distance <= max_distance).then_some((distance, *candidate))
                    })
                    .min_by_key(|(distance, _)| *distance)
                    .map(|(_, candidate)| candidate);

                let msg_suffix =
                    suggestion.map_or_else(String::new, |s| format!(". Did you mean '{s}'?"));
                let msg = format!(
                    "start_joinable_thread() got an unexpected keyword argument '{unexpected}'{msg_suffix}"
                );
                return Err(vm.new_type_error(msg).into());
            }

            if function_obj.to_callable().is_none() {
                return Err(vm.new_type_error("thread function must be callable").into());
            }
            let function: ArgCallable = function_obj.clone().try_into_value(vm)?;

            let thread_handle_type = ThreadHandle::class(&vm.ctx);
            let handle = if let Some(handle_obj) = handle_obj {
                if vm.is_none(&handle_obj) {
                    None
                } else if !handle_obj.class().is(thread_handle_type) {
                    return Err(vm.new_type_error("'handle' must be a _ThreadHandle").into());
                } else {
                    Some(
                        handle_obj
                            .downcast::<ThreadHandle>()
                            .map_err(|_| vm.new_type_error("'handle' must be a _ThreadHandle"))?,
                    )
                }
            } else {
                None
            };
            Ok(Self {
                function_obj,
                function,
                handle,
                daemon,
            })
        }
    }

    #[pyfunction]
    fn start_joinable_thread(
        parsed: StartJoinableThreadArgs,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<ThreadHandle>> {
        let StartJoinableThreadArgs {
            function_obj,
            function,
            handle,
            daemon,
        } = parsed;

        vm.audit("_thread.start_joinable_thread", || {
            (
                function_obj,
                i32::from(daemon),
                handle
                    .as_ref()
                    .map_or_else(|| vm.ctx.none(), |h| h.clone().into()),
            )
        })?;

        if !vm.state.allow_threads() {
            return Err(vm.new_runtime_error(
                "thread is not supported for isolated subinterpreters".to_owned(),
            ));
        }
        if vm
            .state
            .finalizing
            .load(core::sync::atomic::Ordering::Acquire)
        {
            return Err(vm.new_exception_msg(
                vm.ctx.exceptions.python_finalization_error.to_owned(),
                "can't create new thread at interpreter shutdown"
                    .to_owned()
                    .into(),
            ));
        }

        let handle = match handle {
            Some(h) => h,
            None => ThreadHandle::new(vm).into_ref(&vm.ctx),
        };

        // Must only start once (ThreadHandle_start).
        {
            let mut inner = handle.inner.lock();
            if inner.state != ThreadHandleState::NotStarted {
                return Err(vm.new_runtime_error("thread already started"));
            }
            inner.state = ThreadHandleState::Starting;
            inner.ident = 0;
            inner.join_handle = None;
            inner.joining = false;
            inner.joined = false;
        }
        // Starting a handle always resets the completion event.
        {
            let (done_lock, _) = &*handle.done_event;
            *lock_done(done_lock, vm) = false;
        }

        // Add non-daemon threads to shutdown registry so _shutdown() will wait for them
        if !daemon {
            add_to_shutdown_handles(vm, &handle.inner, &handle.done_event);
        }

        let func = function;
        let handle_clone = handle.clone();
        let inner_clone = handle.inner.clone();
        let done_event_clone = handle.done_event.clone();
        // Use std::sync (pthread-based) instead of parking_lot for these
        // events so they remain fork-safe without the parking_lot_core patch.
        let started_event = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
        let started_event_clone = Arc::clone(&started_event);
        let handle_ready_event =
            Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
        let handle_ready_event_clone = Arc::clone(&handle_ready_event);

        let thread_builder = apply_thread_stack_size(thread::Builder::new(), vm);
        let worker_vm = vm.new_thread();
        let worker_state = vm.state.clone();

        let join_handle = thread_builder
            .spawn(move || {
                let completion = WorkerCompletion {
                    vm: Some(worker_vm),
                    state: worker_state,
                    inner: inner_clone.clone(),
                    done_event: done_event_clone,
                };
                completion.vm.as_ref().unwrap().run(move |vm| {
                    // Publish ident for the parent starter thread.
                    {
                        inner_clone.lock().ident = get_ident();
                    }
                    {
                        let (started_lock, started_cvar) = &*started_event_clone;
                        *started_lock.lock().unwrap() = true;
                        started_cvar.notify_all();
                    }
                    // Don't execute the target function until parent marks the
                    // handle as running. Detach while blocked so a concurrent
                    // stop-the-world (e.g. a GC on another thread) can park this
                    // thread instead of stalling waiting for it to reach a
                    // safepoint it will not reach until released.
                    {
                        let (ready_lock, ready_cvar) = &*handle_ready_event_clone;
                        vm.allow_threads(|| {
                            let mut ready = ready_lock.lock().unwrap();
                            while !*ready {
                                // Short timeout so we stay responsive to STW requests.
                                let (guard, _) = ready_cvar
                                    .wait_timeout(ready, core::time::Duration::from_millis(1))
                                    .unwrap();
                                ready = guard;
                            }
                        });
                    }

                    // Ensure cleanup happens even if the function panics.
                    let vm_state = vm.state.clone();
                    scopeguard::defer! {
                        // Handle sentinels
                        for lock in SENTINELS.take() {
                            if lock.mu.is_locked() {
                                unsafe { lock.mu.unlock() };
                            }
                        }

                        // Clean up thread-local data while VM context is still active
                        cleanup_thread_local_data();
                        vm.clear_async_exception();

                        // Clean up frame tracking
                        crate::vm::thread::cleanup_current_thread_frames(vm);

                        vm_state.thread_count.fetch_sub(1);
                    }

                    // Increment thread count when thread actually starts executing
                    vm_state.thread_count.fetch_add(1);

                    // Inner scope: drop `func` (and its Python refs) before the
                    // outer scopeguard::defer tears down the thread slot. As a
                    // `move` closure capture, `func` would otherwise drop after
                    // all locals (including the scopeguard `_guard`), and a
                    // weakref callback fired during that drop would panic in
                    // push_thread_frame.
                    {
                        let func = func;
                        // Run the function
                        if let Err(exc) = func.invoke((), vm)
                            && !exc.fast_isinstance(vm.ctx.exceptions.system_exit)
                        {
                            report_unraisable_thread_exception(exc, &func, vm);
                        }
                    }
                });
            })
            .map_err(|_err| {
                // force_done + remove_from_shutdown_handles on start failure.
                {
                    let mut inner = handle.inner.lock();
                    inner.state = ThreadHandleState::Done;
                    inner.join_handle = None;
                    inner.joining = false;
                    inner.joined = true;
                }
                {
                    let (done_lock, done_cvar) = &*handle.done_event;
                    *lock_done(done_lock, vm) = true;
                    done_cvar.notify_all();
                }
                if !daemon {
                    remove_from_shutdown_handles(vm, &handle.inner, &handle.done_event);
                }
                vm.new_runtime_error("can't start new thread")
            })?;

        // Wait until the new thread has reported its ident. Detach while
        // waiting so a concurrent stop-the-world (e.g. a GC on another thread)
        // can park this thread instead of stalling on it: the child may park
        // itself at startup while the world is stopped and cannot report until
        // released, so the waiter must be parkable too.
        {
            let (started_lock, started_cvar) = &*started_event;
            vm.allow_threads(|| {
                let mut started = started_lock.lock().unwrap();
                while !*started {
                    let (guard, _) = started_cvar
                        .wait_timeout(started, core::time::Duration::from_millis(1))
                        .unwrap();
                    started = guard;
                }
            });
        }

        // Mark the handle running in the parent thread (like CPython's
        // ThreadHandle_start sets THREAD_HANDLE_RUNNING after spawn succeeds).
        {
            let mut inner = handle.inner.lock();
            inner.join_handle = Some(join_handle);
            inner.state = ThreadHandleState::Running;
        }

        // Unblock the started thread once handle state is fully published.
        {
            let (ready_lock, ready_cvar) = &*handle_ready_event;
            *ready_lock.lock().unwrap() = true;
            ready_cvar.notify_all();
        }

        Ok(handle_clone)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::{Interpreter, class::PyClassImpl, protocol::PyNumberMethods, types::AsNumber};

        #[test]
        fn join_wait_slices_preserve_the_timeout_budget() {
            let limit = Duration::from_micros(pytime::THREAD_TIMEOUT_MAX_MICROSECONDS as u64);
            let tail = Duration::from_nanos(1);
            let timeout = limit + tail;
            assert_eq!(
                ThreadHandle::join_wait_slice(timeout, Duration::ZERO),
                limit
            );
            assert_eq!(ThreadHandle::join_wait_slice(timeout, limit), tail);
            assert_eq!(
                ThreadHandle::join_wait_slice(timeout, timeout),
                Duration::ZERO
            );
            assert_eq!(
                ThreadHandle::join_wait_slice(timeout, timeout + tail),
                Duration::ZERO
            );
            #[cfg(windows)]
            assert!(limit.as_millis() < u128::from(u32::MAX));
        }

        #[pyclass(name = "AcquireConversion", module = false)]
        #[derive(Debug, PyPayload)]
        struct AcquireConversion {
            calls: Arc<PyMutex<Vec<&'static str>>>,
            fail_truth: bool,
        }

        #[pyclass(with(AsNumber))]
        impl AcquireConversion {}

        impl AsNumber for AcquireConversion {
            fn as_number() -> &'static PyNumberMethods {
                static METHODS: PyNumberMethods = PyNumberMethods {
                    boolean: Some(|number, vm| {
                        let value = number.obj.downcast_ref::<AcquireConversion>().unwrap();
                        value.calls.lock().push("bool");
                        if value.fail_truth {
                            Err(vm.new_value_error("truth conversion failed"))
                        } else {
                            Ok(true)
                        }
                    }),
                    index: Some(|number, vm| {
                        let value = number.obj.downcast_ref::<AcquireConversion>().unwrap();
                        value.calls.lock().push("index");
                        Ok(vm.ctx.new_int(0).into())
                    }),
                    float: Some(|number, vm| {
                        let value = number.obj.downcast_ref::<AcquireConversion>().unwrap();
                        value.calls.lock().push("float");
                        Err(vm.new_value_error("index must take precedence"))
                    }),
                    ..PyNumberMethods::NOT_IMPLEMENTED
                };
                &METHODS
            }
        }

        #[test]
        fn acquire_defaults_preserve_signature_and_missing_values() {
            let params = AcquireArgs::PARAMS.unwrap();
            assert_eq!(params.len(), 2);
            for (param, name, default) in [
                (&params[0], "blocking", "True"),
                (&params[1], "timeout", "-1"),
            ] {
                assert_eq!(param.name, name);
                assert!(matches!(param.kind, ParamKind::PositionalOrKeyword));
                assert!(matches!(param.default, Some(DefaultRepr::Raw(value)) if value == default));
            }
            Interpreter::without_stdlib(Default::default()).enter(|vm| {
                let args = FuncArgs::default().bind::<AcquireArgs>(vm).unwrap();
                assert!(args.blocking.is_missing());
                assert!(args.timeout.is_missing());
                assert_eq!(validate_acquire_args(&args, vm).unwrap(), (true, None));
                let args = AcquireArgs {
                    timeout: OptionalArg::Present(vm.ctx.none()),
                    ..default_acquire_args()
                };
                let error = validate_acquire_args(&args, vm).unwrap_err();
                assert!(error.fast_isinstance(vm.ctx.exceptions.type_error));
            });
        }

        #[test]
        fn acquire_blocking_accepts_general_truthiness() {
            Interpreter::without_stdlib(Default::default()).enter(|vm| {
                for (value, expected) in [
                    (vm.ctx.none(), false),
                    (vm.ctx.new_list(vec![]).into(), false),
                    (vm.ctx.new_list(vec![vm.ctx.none()]).into(), true),
                    (vm.ctx.new_str("").into(), false),
                    (vm.ctx.new_str("blocking").into(), true),
                ] {
                    let args = FuncArgs::from(vec![value]).bind::<AcquireArgs>(vm).unwrap();
                    assert_eq!(validate_acquire_args(&args, vm).unwrap(), (expected, None));
                }
            });
        }

        #[test]
        fn acquire_converts_once_in_parameter_order() {
            Interpreter::without_stdlib(Default::default()).enter(|vm| {
                let _class = AcquireConversion::make_static_type();
                for fail_truth in [false, true] {
                    let calls = Arc::new(PyMutex::new(Vec::new()));
                    let value: PyObjectRef = AcquireConversion {
                        calls: calls.clone(),
                        fail_truth,
                    }
                    .into_ref(&vm.ctx)
                    .into();
                    let args = FuncArgs::new(
                        vec![],
                        [("timeout", value.clone()), ("blocking", value)]
                            .into_iter()
                            .collect::<KwArgs>(),
                    )
                    .bind::<AcquireArgs>(vm)
                    .unwrap();
                    assert!(calls.lock().is_empty());
                    let result = validate_acquire_args(&args, vm);
                    if fail_truth {
                        assert!(
                            result
                                .unwrap_err()
                                .fast_isinstance(vm.ctx.exceptions.value_error)
                        );
                        assert_eq!(&*calls.lock(), &["bool"]);
                    } else {
                        assert_eq!(result.unwrap(), (true, Some(Duration::ZERO)));
                        assert_eq!(&*calls.lock(), &["bool", "index"]);
                    }
                }
            });
        }

        #[test]
        fn acquire_binding_errors_do_not_run_converters() {
            Interpreter::without_stdlib(Default::default()).enter(|vm| {
                let _class = AcquireConversion::make_static_type();
                let calls = Arc::new(PyMutex::new(Vec::new()));
                let value: PyObjectRef = AcquireConversion {
                    calls: calls.clone(),
                    fail_truth: false,
                }
                .into_ref(&vm.ctx)
                .into();
                for args in [
                    FuncArgs::from(vec![value.clone(); 3]),
                    FuncArgs::new(
                        vec![value.clone()],
                        core::iter::once(("blocking", value.clone())).collect::<KwArgs>(),
                    ),
                    FuncArgs::new(
                        vec![value.clone()],
                        core::iter::once(("unexpected", value)).collect::<KwArgs>(),
                    ),
                ] {
                    let error = args.bind::<AcquireArgs>(vm).unwrap_err();
                    assert!(error.fast_isinstance(vm.ctx.exceptions.type_error));
                    assert!(calls.lock().is_empty());
                }
            });
        }

        #[test]
        fn acquire_timeout_uses_rounded_signed_nanoseconds() {
            Interpreter::without_stdlib(Default::default()).enter(|vm| {
                for (seconds, expected) in [
                    (-1.0, None),
                    (-1.0 + f64::EPSILON / 2.0, None),
                    (0.0, Some(Duration::ZERO)),
                    (-0.0, Some(Duration::ZERO)),
                    (1e-300, Some(Duration::from_nanos(1))),
                    (
                        TIMEOUT_MAX + 0.1,
                        Some(Duration::from_nanos(
                            ((TIMEOUT_MAX + 0.1) * 1_000_000_000.0).ceil() as u64,
                        )),
                    ),
                ] {
                    let args = AcquireArgs {
                        timeout: OptionalArg::Present(vm.ctx.new_float(seconds).into()),
                        ..default_acquire_args()
                    };
                    assert_eq!(validate_acquire_args(&args, vm).unwrap(), (true, expected));
                }
                for seconds in [-1.0, -1.0 + f64::EPSILON / 2.0] {
                    let args = AcquireArgs {
                        blocking: OptionalArg::Present(vm.ctx.none()),
                        timeout: OptionalArg::Present(vm.ctx.new_float(seconds).into()),
                    };
                    assert_eq!(validate_acquire_args(&args, vm).unwrap(), (false, None));
                }
                let args = AcquireArgs {
                    blocking: OptionalArg::Present(vm.ctx.none()),
                    timeout: OptionalArg::Present(vm.ctx.new_int(0).into()),
                };
                let error = validate_acquire_args(&args, vm).unwrap_err();
                assert!(error.fast_isinstance(vm.ctx.exceptions.value_error));
            });
        }

        #[test]
        fn acquire_timeout_errors_precede_nonblocking_restriction() {
            Interpreter::without_stdlib(Default::default()).enter(|vm| {
                for (timeout, expected) in [
                    (vm.ctx.none(), vm.ctx.exceptions.type_error),
                    (
                        vm.ctx.new_float(f64::NAN).into(),
                        vm.ctx.exceptions.value_error,
                    ),
                    (
                        vm.ctx.new_float(f64::INFINITY).into(),
                        vm.ctx.exceptions.overflow_error,
                    ),
                    (
                        vm.ctx.new_int(i128::MAX).into(),
                        vm.ctx.exceptions.overflow_error,
                    ),
                ] {
                    let args = AcquireArgs {
                        blocking: OptionalArg::Present(vm.ctx.none()),
                        timeout: OptionalArg::Present(timeout),
                    };
                    let error = validate_acquire_args(&args, vm).unwrap_err();
                    assert!(error.fast_isinstance(expected));
                    if error.fast_isinstance(vm.ctx.exceptions.value_error) {
                        let message = error.get_arg(0).unwrap();
                        assert_eq!(
                            message.downcast_ref::<PyStr>().unwrap().as_wtf8().as_str(),
                            Ok("Invalid value NaN (not a number)"),
                        );
                    }
                }
                for blocking in [false, true] {
                    for seconds in [-1e-300, -2.0] {
                        let args = AcquireArgs {
                            blocking: OptionalArg::Present(vm.ctx.new_bool(blocking).into()),
                            timeout: OptionalArg::Present(vm.ctx.new_float(seconds).into()),
                        };
                        let error = validate_acquire_args(&args, vm).unwrap_err();
                        assert!(error.fast_isinstance(vm.ctx.exceptions.value_error));
                        let message = error.get_arg(0).unwrap();
                        assert_eq!(
                            message.downcast_ref::<PyStr>().unwrap().as_wtf8().as_str(),
                            Ok(if blocking {
                                "timeout value must be a non-negative number"
                            } else {
                                "can't specify a timeout for a non-blocking call"
                            }),
                        );
                    }
                }
            });
        }

        /// Exercise only the native completion-state invariant, without Python
        /// execution or actual fork in the multithreaded Rust test harness.
        #[test]
        #[cfg(all(unix, feature = "host_env"))]
        fn fork_reinitializes_surviving_handle_completion_lock() {
            for (ident, state, completed, expected_state, expected_completed) in [
                (
                    7,
                    ThreadHandleState::Running,
                    false,
                    ThreadHandleState::Running,
                    false,
                ),
                (
                    0,
                    ThreadHandleState::NotStarted,
                    false,
                    ThreadHandleState::NotStarted,
                    false,
                ),
                (
                    8,
                    ThreadHandleState::Running,
                    false,
                    ThreadHandleState::Done,
                    true,
                ),
                (
                    8,
                    ThreadHandleState::Done,
                    true,
                    ThreadHandleState::Done,
                    true,
                ),
            ] {
                let inner = parking_lot::Mutex::new(ThreadHandleInner {
                    state,
                    ident,
                    join_handle: None,
                    joining: false,
                    joined: false,
                });
                let done_event = (
                    parking_lot::Mutex::new(completed),
                    parking_lot::Condvar::new(),
                );
                // Model the bytes inherited from a parent joiner that held the
                // mutex at fork. No guard remains live and no other thread can
                // access these exclusively owned fixtures.
                core::mem::forget(done_event.0.lock());
                assert!(done_event.0.try_lock().is_none());
                // SAFETY: these private fixtures have no live guards/waiters,
                // exactly as in a child after its parent lock owner vanished.
                unsafe { reinit_handle_after_fork(&inner, &done_event, 7) };
                let completed = done_event
                    .0
                    .try_lock()
                    .expect("copied completion lock must be unlocked");
                assert_eq!(*completed, expected_completed);
                let inner = inner.lock();
                assert_eq!(inner.state, expected_state);
                assert_eq!(inner.ident, ident);
            }
        }

        /// Regression test for #7941: a Python thread started without an
        /// explicit `threading.stack_size()` must not run on Rust's 2 MiB
        /// std default in debug builds, where the call chains the stdlib
        /// runs on helper threads (e.g. the SSL test server) overflowed it.
        #[test]
        #[cfg(all(debug_assertions, any(target_os = "linux", target_os = "macos")))]
        fn default_python_thread_stack_size_debug() {
            Interpreter::without_stdlib(Default::default()).enter(|vm| {
                assert_eq!(vm.state.stacksize.load(), 0);
                let builder = apply_thread_stack_size(thread::Builder::new(), vm);
                let stack_size = builder
                    .spawn(current_thread_stack_size)
                    .expect("failed to spawn thread")
                    .join()
                    .expect("thread panicked");
                assert!(
                    stack_size >= DEFAULT_THREAD_STACK_SIZE,
                    "Python thread stack size is {stack_size} bytes, expected at least {DEFAULT_THREAD_STACK_SIZE}"
                );
            });
        }

        /// A size small enough for CPython's frames is not small enough for an
        /// unoptimized build's: `test_threading` asks for 256 KiB, which holds
        /// three of them where starting a thread needs six. The size the
        /// request set is still what `threading.stack_size()` answers with.
        #[test]
        #[cfg(all(debug_assertions, any(target_os = "linux", target_os = "macos")))]
        fn explicit_python_thread_stack_size_is_a_floor_debug() {
            const REQUESTED: usize = 256 * 1024;

            Interpreter::without_stdlib(Default::default()).enter(|vm| {
                vm.state.stacksize.store(REQUESTED);
                let builder = apply_thread_stack_size(thread::Builder::new(), vm);
                let stack_size = builder
                    .spawn(current_thread_stack_size)
                    .expect("failed to spawn thread")
                    .join()
                    .expect("thread panicked");
                assert!(
                    stack_size >= DEFAULT_THREAD_STACK_SIZE,
                    "Python thread stack size is {stack_size} bytes, expected at least {DEFAULT_THREAD_STACK_SIZE}"
                );
                assert_eq!(vm.state.stacksize.load(), REQUESTED);
            });
        }

        #[cfg(all(debug_assertions, target_os = "linux"))]
        fn current_thread_stack_size() -> usize {
            use libc::{
                pthread_attr_destroy, pthread_attr_getstacksize, pthread_attr_t,
                pthread_getattr_np, pthread_self,
            };
            let mut attr: pthread_attr_t = unsafe { core::mem::zeroed() };
            unsafe {
                assert_eq!(pthread_getattr_np(pthread_self(), &mut attr), 0);
                let mut size = 0;
                assert_eq!(pthread_attr_getstacksize(&attr, &mut size), 0);
                pthread_attr_destroy(&mut attr);
                size
            }
        }

        #[cfg(all(debug_assertions, target_os = "macos"))]
        fn current_thread_stack_size() -> usize {
            unsafe { libc::pthread_get_stacksize_np(libc::pthread_self()) }
        }
    }
}

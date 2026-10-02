//! Scoped access to interpreter-owned objects.
//!
//! A `PyHandle` can be retained by the host or sent to another thread. Bind it
//! to its interpreter to access Python. Bound values never expose raw payloads.
//!
//! Native integration uses explicitly unsafe entry points. Their contract covers
//! the entire lifetime of raw `VirtualMachine`, `Context`, `PyObjectRef` and
//! `PyRef` values obtained through them:
//!
//! - Access and release private Python objects only while attached to their owner.
//! - Do not store foreign Python references in an interpreter's object graph.
//! - Do not access raw references, payloads or borrowed storage across detachment
//!   or a nested entry into another interpreter. Native-only detached work must
//!   not execute Python destructors when dropping its captures.
//! - Retain host values using `PyHandle`; raw references must not escape the entry.
//! - Native callbacks, including initialization hooks and buffer release, obey
//!   the same rules. Process-wide definitions contain no mutable Python state.
//!
//! During bootstrap only the interpreter implementation may access the shared
//! native definitions before an owner is attached.
//!
//! ```compile_fail
//! let interpreter = rustpython_vm::Interpreter::without_stdlib(Default::default());
//! let escaped = interpreter.enter(|vm| vm.new_int(1).unwrap());
//! ```
//!
//! ```compile_fail
//! let interpreter = rustpython_vm::Interpreter::without_stdlib(Default::default());
//! interpreter.enter(|vm| {
//!     let value = vm.new_int(1).unwrap();
//!     vm.detach(|| value.to_i64()).unwrap();
//! });
//! ```
//!
//! Raw getters and native callbacks cannot bypass this boundary in safe Rust:
//!
//! ```compile_fail
//! let context = rustpython_vm::Context::genesis_unchecked();
//! let object = context.new_list(Vec::new());
//! ```
//!
//! ```compile_fail
//! use rustpython_vm::class::StaticType;
//! let class = rustpython_vm::builtins::PyList::static_type();
//! ```
//!
//! ```compile_fail
//! rustpython_vm::vm::thread::with_current_vm_unchecked(|vm| vm.ctx.none());
//! ```
//!
//! ```compile_fail
//! rustpython_vm::Interpreter::builder(Default::default()).init_hook(|vm| {
//!     let _object = vm.ctx.new_list(Vec::new());
//! });
//! ```
//!
//! ```compile_fail
//! rustpython_vm::vm::thread::update_thread_exception(None);
//! ```
//!
//! ```compile_fail
//! rustpython_vm::vm::runtime::destroy_owned_interpreter(0);
//! ```

use crate::{
    PyObject, PyObjectRef, PyResult, TryFromBorrowedObject, VirtualMachine,
    builtins::PyBaseException,
    common::rc::PyRc,
    vm::{roots::RootLease, thread},
};
use core::{fmt, marker::PhantomData};

/// An opaque persistent root in one interpreter. Dropping the last clone queues
/// release on the owner; it never runs Python on the dropping native thread.
pub struct PyHandle<T = PyObject> {
    lease: PyRc<RootLease>,
    kind: PhantomData<fn() -> T>,
}

impl<T> Clone for PyHandle<T> {
    fn clone(&self) -> Self {
        Self {
            lease: self.lease.clone(),
            kind: PhantomData,
        }
    }
}

impl<T> fmt::Debug for PyHandle<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PyHandle")
            .field("interpreter", &self.lease.owner())
            .finish_non_exhaustive()
    }
}

impl<T> PyHandle<T> {
    #[must_use]
    pub fn interpreter_id(&self) -> i64 {
        self.lease.owner()
    }

    /// Obtain an owner-local reference for a native callback.
    ///
    /// # Safety
    /// Release the returned reference before leaving this owner's attachment.
    /// Follow [`crate::Interpreter::enter_unchecked`]'s contract.
    pub unsafe fn to_object_unchecked(&self, vm: &VirtualMachine) -> Result<PyObjectRef> {
        let vm = Vm::new(vm);
        vm.check()?;
        if self.interpreter_id() != vm.raw.state.interpreter_id {
            return Err(Error::WrongInterpreter);
        }
        self.lease.object().ok_or(Error::InterpreterClosed)
    }
}

impl PyHandle {
    /// Transfer a native callback's owned reference into an opaque host root.
    ///
    /// # Safety
    /// The object must be owned by the currently attached interpreter, and its
    /// reachable graph must obey the native ownership contract of this module.
    pub unsafe fn from_object_unchecked(object: PyObjectRef, vm: &VirtualMachine) -> Result<Self> {
        Vm::new(vm).root(object)
    }
}

impl PyHandle<PyBaseException> {
    pub(crate) fn exception(
        &self,
        vm: &VirtualMachine,
    ) -> Option<crate::builtins::PyBaseExceptionRef> {
        if self.interpreter_id() != vm.state.interpreter_id {
            return Some(vm.new_runtime_error("exception belongs to another interpreter"));
        }
        self.lease
            .object()
            .and_then(|object| object.downcast().ok())
    }
}

/// Access failures contain no unscoped Python references.
#[derive(Debug)]
pub enum Error {
    WrongInterpreter,
    InterpreterClosed,
    ForkedOwner,
    NotAttached,
    Python(PyHandle<PyBaseException>),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::WrongInterpreter => "object belongs to another interpreter",
            Self::InterpreterClosed => "interpreter has been closed",
            Self::ForkedOwner => "VM owner was inactive or on another thread at fork",
            Self::NotAttached => "this interpreter scope is not currently attached",
            Self::Python(_) => "Python raised an exception",
        })
    }
}

impl core::error::Error for Error {}

pub type Result<T> = core::result::Result<T, Error>;

/// A fresh, invariant scope for one entry into Python. Neither this token nor a
/// bound object can cross native threads, outlive the entry, or be used while
/// the thread is detached or running a nested entry into another VM.
#[derive(Clone, Copy)]
pub struct Vm<'vm> {
    raw: &'vm VirtualMachine,
    scope: PhantomData<&'vm mut &'vm ()>,
    thread: PhantomData<alloc::rc::Rc<()>>,
}

pub struct Bound<'vm, T = PyObject> {
    handle: PyHandle<T>,
    vm: Vm<'vm>,
}

impl<T> Clone for Bound<'_, T> {
    fn clone(&self) -> Self {
        Self {
            handle: self.handle.clone(),
            vm: self.vm,
        }
    }
}

impl<T> fmt::Debug for Bound<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.handle.fmt(f)
    }
}

impl<'vm> Vm<'vm> {
    pub(crate) fn new(raw: &'vm VirtualMachine) -> Self {
        Self {
            raw,
            scope: PhantomData,
            thread: PhantomData,
        }
    }

    fn check(self) -> Result<()> {
        if !self.raw.owner_is_valid() {
            return Err(Error::ForkedOwner);
        }
        if self
            .raw
            .state
            .closed
            .load(core::sync::atomic::Ordering::Acquire)
        {
            return Err(Error::InterpreterClosed);
        }
        if thread::is_current_attached(self.raw) {
            if self.raw.state.roots.is_closed() {
                return Err(Error::InterpreterClosed);
            }
            self.raw.state.roots.drain_pending();
            Ok(())
        } else {
            Err(Error::NotAttached)
        }
    }

    fn root<T>(self, object: PyObjectRef) -> Result<PyHandle<T>> {
        self.check()?;
        if object
            .gc_handle()
            .heap()
            .and_then(|heap| heap.owner)
            .is_some_and(|owner| owner != self.raw.state.interpreter_id)
        {
            return Err(Error::WrongInterpreter);
        }
        self.raw
            .state
            .roots
            .insert_object(object)
            .map(|lease| PyHandle {
                lease,
                kind: PhantomData,
            })
            .ok_or(Error::InterpreterClosed)
    }

    fn exception(self, exception: crate::builtins::PyBaseExceptionRef) -> Error {
        match self.root(exception.into()) {
            Ok(handle) => Error::Python(handle),
            Err(error) => error,
        }
    }

    fn value(self, result: PyResult) -> Result<Bound<'vm>> {
        let object = result.map_err(|exception| self.exception(exception))?;
        Ok(Bound {
            handle: self.root(object)?,
            vm: self,
        })
    }

    pub fn bind<T>(self, handle: &PyHandle<T>) -> Result<Bound<'vm, T>> {
        self.check()?;
        if handle.lease.owner() != self.raw.state.interpreter_id {
            return Err(Error::WrongInterpreter);
        }
        let object = handle.lease.object().ok_or(Error::InterpreterClosed)?;
        drop(object);
        Ok(Bound {
            handle: handle.clone(),
            vm: self,
        })
    }

    pub fn none(self) -> Result<Bound<'vm>> {
        self.check()?;
        self.value(Ok(self.raw.ctx.none()))
    }

    pub fn new_int(self, value: i64) -> Result<Bound<'vm>> {
        self.check()?;
        self.value(Ok(self.raw.ctx.new_int(value).into()))
    }

    pub fn new_str(self, value: &str) -> Result<Bound<'vm>> {
        self.check()?;
        self.value(Ok(self.raw.ctx.new_str(value).into()))
    }

    pub fn new_bytes(self, value: Vec<u8>) -> Result<Bound<'vm>> {
        self.check()?;
        self.value(Ok(self.raw.ctx.new_bytes(value).into()))
    }

    pub fn new_list(self, values: &[Bound<'vm>]) -> Result<Bound<'vm>> {
        self.check()?;
        let objects = values
            .iter()
            .map(|value| value.object_for(self))
            .collect::<Result<_>>()?;
        self.value(Ok(self.raw.ctx.new_list(objects).into()))
    }

    pub fn import(self, name: &str) -> Result<Bound<'vm>> {
        self.check()?;
        self.value(self.raw.import(&self.raw.ctx.new_str(name), 0))
    }

    /// Execute in the interpreter's persistent `__main__` namespace.
    #[cfg(feature = "rustpython-compiler")]
    pub fn exec(self, source: &str) -> Result<()> {
        self.run(source, crate::compiler::Mode::Exec).map(drop)
    }

    #[cfg(feature = "rustpython-compiler")]
    pub fn eval(self, source: &str) -> Result<Bound<'vm>> {
        self.run(source, crate::compiler::Mode::Eval)
    }

    #[cfg(feature = "rustpython-compiler")]
    fn run(self, source: &str, mode: crate::compiler::Mode) -> Result<Bound<'vm>> {
        self.check()?;
        let result = (|| {
            let globals = self.raw.main_namespace()?;
            let scope = crate::scope::Scope::with_builtins(None, globals, self.raw);
            let code = self
                .raw
                .compile(source, mode, "<embedded>")
                .map_err(|error| error.into_pyexception(self.raw, Some(source)))?;
            self.raw.run_code_obj(code, scope)
        })();
        self.value(result)
    }

    /// Run a host wait without blocking collection. Scoped VM and object tokens
    /// are not Send/Sync, so the callback cannot capture and use them detached.
    pub fn detach<R: Send>(self, f: impl FnOnce() -> R + Send) -> Result<R> {
        self.check()?;
        Ok(self.raw.allow_threads(f))
    }

    /// Release handles dropped by host threads since the preceding drain.
    pub fn drain_releases(self) -> Result<()> {
        self.check()?;
        self.raw.state.roots.drain_pending();
        Ok(())
    }
}

impl<'vm, T> Bound<'vm, T> {
    fn object_for(&self, vm: Vm<'vm>) -> Result<PyObjectRef> {
        vm.check()?;
        if self.handle.lease.owner() != vm.raw.state.interpreter_id {
            return Err(Error::WrongInterpreter);
        }
        self.handle.lease.object().ok_or(Error::InterpreterClosed)
    }

    /// Keep this object beyond the current entry without exposing its payload.
    #[must_use]
    pub fn unbind(self) -> PyHandle<T> {
        self.handle
    }

    pub fn get_attr(&self, name: &str) -> Result<Bound<'vm>> {
        let object = self.object_for(self.vm)?;
        self.vm
            .value(object.get_attr(&self.vm.raw.ctx.new_str(name), self.vm.raw))
    }

    pub fn set_attr<U>(&self, name: &str, value: &Bound<'vm, U>) -> Result<()> {
        let object = self.object_for(self.vm)?;
        let value = value.object_for(self.vm)?;
        object
            .set_attr(&self.vm.raw.ctx.new_str(name), value, self.vm.raw)
            .map_err(|error| self.vm.exception(error))
    }

    pub fn call(&self, args: &[Bound<'vm>]) -> Result<Bound<'vm>> {
        let object = self.object_for(self.vm)?;
        let args = args
            .iter()
            .map(|arg| arg.object_for(self.vm))
            .collect::<Result<Vec<_>>>()?;
        self.vm
            .value(object.call(crate::function::FuncArgs::from(args), self.vm.raw))
    }

    pub fn repr(&self) -> Result<String> {
        let object = self.object_for(self.vm)?;
        object
            .repr(self.vm.raw)
            .map(|value| value.as_wtf8().to_string_lossy().into_owned())
            .map_err(|error| self.vm.exception(error))
    }

    pub fn to_string(&self) -> Result<String> {
        let object = self.object_for(self.vm)?;
        object
            .str(self.vm.raw)
            .map(|value| value.as_wtf8().to_string_lossy().into_owned())
            .map_err(|error| self.vm.exception(error))
    }

    pub fn next(&self) -> Result<Option<Bound<'vm>>> {
        let object = self.object_for(self.vm)?;
        let result = crate::protocol::PyIter::new(object)
            .next(self.vm.raw)
            .map_err(|error| self.vm.exception(error))?;
        match result {
            crate::protocol::PyIterReturn::Return(value) => self.vm.value(Ok(value)).map(Some),
            crate::protocol::PyIterReturn::StopIteration(_) => Ok(None),
        }
    }

    pub fn to_i64(&self) -> Result<i64> {
        let object = self.object_for(self.vm)?;
        i64::try_from_borrowed_object(self.vm.raw, &object)
            .map_err(|error| self.vm.exception(error))
    }

    pub fn to_bool(&self) -> Result<bool> {
        let object = self.object_for(self.vm)?;
        object
            .try_to_bool(self.vm.raw)
            .map_err(|error| self.vm.exception(error))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Interpreter;

    #[test]
    fn shutdown_retires_all_root_groups_after_native_panics() {
        use alloc::sync::Arc;
        use core::sync::atomic::{AtomicUsize, Ordering};

        struct PanicDrop(Arc<AtomicUsize>);
        impl Drop for PanicDrop {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::Relaxed);
                panic!("root destructor");
            }
        }
        struct CacheKey;

        let panics = Arc::new(AtomicUsize::new(0));
        let interpreter = Interpreter::without_stdlib(Default::default());
        let (state, lease) = interpreter.enter_raw(|vm| {
            let lease = vm.state.roots.insert_resource(PanicDrop(panics.clone()));
            vm.__replace_native::<CacheKey, _>(PanicDrop(panics.clone()));
            (vm.state.clone(), lease)
        });
        let result = std::panic::catch_unwind(core::panic::AssertUnwindSafe(|| drop(interpreter)));
        assert!(result.is_err());
        assert_eq!(panics.load(Ordering::Relaxed), 2);
        assert!(state.roots.is_closed());
        assert!(state.gc.garbage_list().is_none());
        assert!(state.gc.callbacks_list().is_none());
        drop(lease);
        #[cfg(feature = "threading")]
        std::thread::spawn(move || drop(state)).join().unwrap();
    }

    #[test]
    fn shutdown_releases_native_resources_before_disabling_callbacks() {
        use alloc::sync::Arc;
        use core::sync::atomic::{AtomicUsize, Ordering};

        unsafe extern "C" fn release(object: *mut crate::PyObject) {
            let capsule = unsafe { &*object }
                .downcast_ref::<crate::builtins::PyCapsule>()
                .unwrap();
            let released = unsafe { Box::from_raw(capsule.pointer().cast::<Arc<AtomicUsize>>()) };
            assert!(thread::try_with_attached_vm(|_| ()).is_some());
            released.fetch_add(1, Ordering::Relaxed);
        }

        struct CacheKey;
        let released = Arc::new(AtomicUsize::new(0));
        let interpreter = Interpreter::without_stdlib(Default::default());
        let retained_state = interpreter.enter_raw(|vm| {
            let capsule = || -> PyObjectRef {
                vm.ctx
                    .new_capsule(
                        Box::into_raw(Box::new(released.clone())).cast(),
                        None,
                        Some(release),
                    )
                    .into()
            };
            vm.__replace_native::<CacheKey, _>(capsule());
            vm.ctx
                .types
                .list_type
                .set_str_attr("_shutdown_resource", capsule(), &vm.ctx);
            let len = vm.callable_cache.len.get().unwrap();
            len.set_attr("__module__", capsule(), vm).unwrap();
            let signum = unsafe { crate::signal::SignalNum::new_unchecked(2) };
            vm.signal_handlers.get().unwrap().borrow_mut()[signum] = Some(capsule());
            vm.state
                .warnings
                .once_registry()
                .set_item("resource", capsule(), vm)
                .unwrap();
            #[cfg(feature = "threading")]
            drop(
                thread::current_thread_slot()
                    .unwrap()
                    .replace_profile(capsule()),
            );
            vm.state
                .gc
                .garbage_list()
                .unwrap()
                .borrow_vec_mut()
                .push(capsule());
            vm.state.clone()
        });
        drop(interpreter);
        assert_eq!(
            released.load(Ordering::Relaxed),
            if cfg!(feature = "threading") { 7 } else { 6 }
        );
        drop(retained_state);
    }

    #[cfg(feature = "threading")]
    #[test]
    fn busy_finalize_preserves_the_owner_and_can_be_retried() {
        let interpreter = Interpreter::without_stdlib(Default::default());
        let worker = interpreter.new_thread();
        let value = interpreter.enter(|vm| vm.new_int(42).unwrap().unbind());
        let busy = interpreter.finalize().unwrap_err();
        busy.interpreter().enter(|vm| {
            assert_eq!(vm.bind(&value).unwrap().to_i64().unwrap(), 42);
        });
        drop(worker);
        assert_eq!(busy.retry().unwrap(), 0);
        drop(value);
    }

    #[cfg(feature = "threading")]
    #[test]
    fn scoped_shutdown_joins_python_workers_after_native_owners_leave() {
        use alloc::sync::Arc;
        use core::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Mutex;

        let interpreter = Interpreter::without_stdlib(Default::default());
        let native_worker = interpreter.new_thread();
        let pending = Arc::new(Mutex::new(Some(
            interpreter.enter_raw(VirtualMachine::new_python_thread),
        )));
        let joined = Arc::new(AtomicBool::new(false));
        interpreter.enter_raw(|vm| {
            let pending = pending.clone();
            let joined = joined.clone();
            let shutdown = vm.new_function("_shutdown", move |vm: &VirtualMachine| {
                let worker = pending.lock().unwrap().take().unwrap();
                let parent = std::thread::spawn(move || {
                    worker.run_raw(|vm| {
                        let child = vm.new_python_thread();
                        let child = std::thread::spawn(move || {
                            child.run(|vm| {
                                assert_eq!(vm.new_int(42).unwrap().to_i64().unwrap(), 42);
                            });
                        });
                        vm.allow_threads(|| child.join().unwrap());
                    });
                });
                vm.allow_threads(|| parent.join().unwrap());
                joined.store(true, Ordering::Release);
            });
            let attrs = vm.ctx.new_dict();
            attrs.set_item("_shutdown", shutdown.into(), vm).unwrap();
            let module = vm.new_module("threading", attrs, None);
            vm.sys_module
                .get_attr("modules", vm)
                .unwrap()
                .set_item("threading", module.into(), vm)
                .unwrap();
        });
        let busy = interpreter.run(|_| Ok(())).unwrap_err();
        assert!(!joined.load(Ordering::Acquire));
        busy.interpreter().enter(|vm| {
            assert_eq!(vm.new_int(7).unwrap().to_i64().unwrap(), 7);
        });
        drop(native_worker);
        let result = busy.retry();
        // On a regression, release the pending worker before asserting so
        // its capture cannot retain the interpreter through the module.
        drop(pending.lock().unwrap().take());
        assert_eq!(result.unwrap(), 0);
        assert!(joined.load(Ordering::Acquire));
    }

    #[cfg(feature = "threading")]
    #[test]
    fn last_worker_closes_roots_before_foreign_state_snapshot_is_released() {
        let interpreter = Interpreter::without_stdlib(Default::default());
        let state = interpreter.enter_raw(|vm| vm.state.clone());
        let worker = interpreter.new_thread();
        drop(interpreter);
        assert!(!state.roots.is_closed());
        let other = Interpreter::without_stdlib(Default::default());
        other.enter(|outer| {
            drop(worker);
            assert!(state.roots.is_closed());
            assert_eq!(outer.new_int(7).unwrap().to_i64().unwrap(), 7);
        });
        std::thread::spawn(move || drop(state)).join().unwrap();
    }

    #[cfg(feature = "threading")]
    #[test]
    fn cancelled_worker_after_native_finalize_never_enters_shutdown_wait() {
        let interpreter = Interpreter::without_stdlib(Default::default());
        let worker = interpreter.new_thread();
        let _ = interpreter.finalize_raw(None);
        worker.run(|vm| {
            assert!(matches!(vm.new_int(1), Err(Error::InterpreterClosed)));
        });
        std::thread::spawn(move || drop(worker)).join().unwrap();
    }

    #[test]
    fn bootstrap_panic_retires_roots_in_their_owner() {
        use alloc::sync::Arc;
        use core::sync::atomic::{AtomicI64, Ordering};

        struct Probe(Arc<AtomicI64>);
        impl Drop for Probe {
            fn drop(&mut self) {
                self.0.store(
                    thread::try_with_attached_vm(|vm| vm.state.interpreter_id).unwrap_or(-1),
                    Ordering::Release,
                );
            }
        }

        let released_by = Arc::new(AtomicI64::new(-1));
        let expected_owner = Arc::new(AtomicI64::new(-2));
        let retained = PyRc::new(std::sync::Mutex::new(None));
        let release = released_by.clone();
        let expected = expected_owner.clone();
        let root = retained.clone();
        let failed = std::panic::catch_unwind(move || {
            // SAFETY: the hook retains only native integers and owner roots.
            unsafe {
                Interpreter::builder(Default::default()).init_hook(move |vm| {
                    expected.store(vm.state.interpreter_id, Ordering::Release);
                    *root.lock().unwrap() = vm.state.roots.insert_resource(Probe(release));
                    panic!("bootstrap rollback");
                })
            }
            .build()
        });
        assert!(failed.is_err());
        assert_eq!(
            released_by.load(Ordering::Acquire),
            expected_owner.load(Ordering::Acquire)
        );
        drop(retained);
    }

    #[test]
    fn handles_bind_only_to_the_owner_and_nested_entries_invalidate_outer_tokens() {
        let first = Interpreter::without_stdlib(Default::default());
        let second = first.create_subinterpreter();
        let handle = first.enter(|vm| vm.new_int(42).unwrap().unbind());
        second.enter(|vm| {
            assert!(matches!(vm.bind(&handle), Err(Error::WrongInterpreter)));
        });
        first.enter(|vm| {
            let value = vm.bind(&handle).unwrap();
            second.enter(|_| {
                assert!(matches!(value.to_i64(), Err(Error::NotAttached)));
                assert!(matches!(vm.new_int(1), Err(Error::NotAttached)));
            });
            assert_eq!(value.to_i64().unwrap(), 42);
        });
        #[cfg(feature = "threading")]
        {
            let thread = first.new_thread();
            drop(first);
            thread.run_raw(|raw| {
                assert_eq!(Vm::new(raw).bind(&handle).unwrap().to_i64().unwrap(), 42);
            });
        }
        drop(handle);
    }

    #[cfg(all(feature = "threading", feature = "rustpython-compiler"))]
    #[test]
    fn foreign_handle_drop_defers_finalizers_until_owner_entry() {
        let interpreter = Interpreter::without_stdlib(Default::default());
        let handle = interpreter.enter(|vm| {
            vm.exec("events = []\nclass C:\n def __del__(self): events.append(1)\n")
                .unwrap();
            vm.eval("C()").unwrap().unbind()
        });
        let other = interpreter.create_subinterpreter();
        other.enter(|_| std::thread::spawn(move || drop(handle)).join().unwrap());
        interpreter.enter(|vm| {
            assert_eq!(vm.eval("len(events)").unwrap().to_i64().unwrap(), 1);
        });
    }

    #[cfg(all(feature = "threading", feature = "rustpython-compiler"))]
    #[test]
    fn host_thread_exit_releases_locals_in_the_owner_and_unlocks_sentinels() {
        let interpreter = Interpreter::without_stdlib(Default::default());
        interpreter.enter(|vm| {
            vm.exec("import _thread\nevents = []\nlocal = _thread._local()\nclass C:\n def __del__(self): events.append(1)\n").unwrap();
        });
        let thread = interpreter.new_thread();
        std::thread::spawn(move || thread.run_raw(|raw| {
            Vm::new(raw).exec("local.value = C()\nsentinel = _thread._set_sentinel()\nsentinel.acquire()\n").unwrap();
        })).join().unwrap();
        interpreter.enter(|vm| {
            assert_eq!(vm.eval("len(events)").unwrap().to_i64().unwrap(), 1);
            assert!(!vm.eval("sentinel.locked()").unwrap().to_bool().unwrap());
        });
    }

    #[cfg(all(feature = "threading", feature = "rustpython-compiler"))]
    #[test]
    fn local_initialization_failure_is_retired_during_a_root_release() {
        let interpreter = Interpreter::without_stdlib(Default::default());
        interpreter.enter(|vm| {
            vm.exec("import _thread\ncalls = []\nfail = False\nclass Local(_thread._local):\n def __init__(self):\n  calls.append(1)\n  if fail: raise ValueError\nclass C:\n def __del__(self):\n  for _ in range(2):\n   try: local.value\n   except ValueError: pass\n").unwrap();
        });
        let thread = interpreter.new_thread();
        std::thread::spawn(move || {
            thread.run_raw(|raw| {
                Vm::new(raw).exec("local = Local()\n").unwrap();
            })
        })
        .join()
        .unwrap();
        let handle = interpreter.enter(|vm| {
            vm.exec("fail = True\n").unwrap();
            vm.eval("C()").unwrap().unbind()
        });
        drop(handle);
        interpreter.enter(|vm| {
            assert_eq!(vm.eval("len(calls)").unwrap().to_i64().unwrap(), 3);
        });
    }

    #[cfg(feature = "rustpython-compiler")]
    #[test]
    fn exceptions_remain_bound_and_functions_accept_bound_arguments() {
        let interpreter = Interpreter::without_stdlib(Default::default());
        let error = interpreter.enter(|vm| {
            let function = vm.eval("lambda x: x + 1").unwrap();
            let value = vm.new_int(41).unwrap();
            assert_eq!(function.call(&[value]).unwrap().to_i64().unwrap(), 42);
            vm.eval("1 / 0").unwrap_err()
        });
        let Error::Python(exception) = error else {
            panic!("expected Python exception")
        };
        interpreter.enter(|vm| {
            assert!(
                vm.bind(&exception)
                    .unwrap()
                    .repr()
                    .unwrap()
                    .contains("ZeroDivisionError")
            );
        });
    }
}

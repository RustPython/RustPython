/// Implementation of the _thread module
use crate::function::PyFuncArgs;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{PyClassImpl, PyObjectRef, PyRef, PyResult, PyValue};
use crate::vm::VirtualMachine;

use parking_lot::{
    lock_api::{GetThreadId, RawMutex as RawMutexT, RawMutexTimed},
    RawMutex, RawThreadId,
};
use std::cell::Cell;
use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

#[cfg(not(target_os = "windows"))]
const PY_TIMEOUT_MAX: isize = std::isize::MAX;

#[cfg(target_os = "windows")]
const PY_TIMEOUT_MAX: isize = 0xffffffff * 1_000_000;

const TIMEOUT_MAX: f64 = (PY_TIMEOUT_MAX / 1_000_000_000) as f64;

#[pyimpl]
trait LockProtocol: PyValue {
    type RawMutex: RawMutexT + RawMutexTimed<Duration = Duration>;
    fn mutex(&self) -> &Self::RawMutex;

    #[pymethod]
    #[pymethod(name = "acquire_lock")]
    #[pymethod(name = "__enter__")]
    #[allow(clippy::float_cmp, clippy::match_bool)]
    fn acquire(&self, args: AcquireArgs, vm: &VirtualMachine) -> PyResult<bool> {
        let mu = self.mutex();
        match args.waitflag {
            true if args.timeout == -1.0 => {
                mu.lock();
                Ok(true)
            }
            true if args.timeout < 0.0 => {
                Err(vm.new_value_error("timeout value must be positive".to_owned()))
            }
            true => Ok(mu.try_lock_for(Duration::from_secs_f64(args.timeout))),
            false if args.timeout != -1.0 => {
                Err(vm
                    .new_value_error("can't specify a timeout for a non-blocking call".to_owned()))
            }
            false => Ok(mu.try_lock()),
        }
    }
    #[pymethod]
    #[pymethod(name = "release_lock")]
    fn release(&self) {
        self.mutex().unlock()
    }

    #[pymethod(magic)]
    fn exit(&self, _args: PyFuncArgs) {
        self.release()
    }
}
#[derive(FromArgs)]
struct AcquireArgs {
    #[pyarg(positional_or_keyword, default = "true")]
    waitflag: bool,
    #[pyarg(positional_or_keyword, default = "-1.0")]
    timeout: f64,
}

#[pyclass(name = "lock")]
struct PyLock {
    mu: RawMutex,
}

impl PyValue for PyLock {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("_thread", "LockType")
    }
}

impl fmt::Debug for PyLock {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.pad("PyLock")
    }
}

impl LockProtocol for PyLock {
    type RawMutex = RawMutex;
    fn mutex(&self) -> &RawMutex {
        &self.mu
    }
}

#[pyimpl(with(LockProtocol))]
impl PyLock {
    // TODO: locked(), might require something to change in parking_lot
}

// Copied from lock_api
// TODO: open a PR to make this public in lock_api
struct RawReentrantMutex<R, G> {
    owner: AtomicUsize,
    lock_count: Cell<usize>,
    mutex: R,
    get_thread_id: G,
}

impl<R: RawMutexT, G: GetThreadId> RawReentrantMutex<R, G> {
    #[inline]
    fn lock_internal<F: FnOnce() -> bool>(&self, try_lock: F) -> bool {
        let id = self.get_thread_id.nonzero_thread_id().get();
        if self.owner.load(Ordering::Relaxed) == id {
            self.lock_count.set(
                self.lock_count
                    .get()
                    .checked_add(1)
                    .expect("ReentrantMutex lock count overflow"),
            );
        } else {
            if !try_lock() {
                return false;
            }
            self.owner.store(id, Ordering::Relaxed);
            debug_assert_eq!(self.lock_count.get(), 0);
            self.lock_count.set(1);
        }
        true
    }
}

unsafe impl<R: RawMutexT + Send, G: GetThreadId + Send> Send for RawReentrantMutex<R, G> {}
unsafe impl<R: RawMutexT + Sync, G: GetThreadId + Sync> Sync for RawReentrantMutex<R, G> {}

unsafe impl<R: RawMutexT, G: GetThreadId> RawMutexT for RawReentrantMutex<R, G> {
    const INIT: Self = RawReentrantMutex {
        owner: AtomicUsize::new(0),
        lock_count: Cell::new(0),
        mutex: R::INIT,
        get_thread_id: G::INIT,
    };

    type GuardMarker = R::GuardMarker;

    #[inline]
    fn lock(&self) {
        self.lock_internal(|| {
            self.mutex.lock();
            true
        });
    }

    #[inline]
    fn try_lock(&self) -> bool {
        self.lock_internal(|| self.mutex.try_lock())
    }

    #[inline]
    fn unlock(&self) {
        let lock_count = self.lock_count.get() - 1;
        self.lock_count.set(lock_count);
        if lock_count == 0 {
            self.owner.store(0, Ordering::Relaxed);
            self.mutex.unlock();
        }
    }
}

unsafe impl<R: RawMutexTimed, G: GetThreadId> RawMutexTimed for RawReentrantMutex<R, G> {
    type Instant = R::Instant;
    type Duration = R::Duration;
    #[inline]
    fn try_lock_until(&self, timeout: R::Instant) -> bool {
        self.lock_internal(|| self.mutex.try_lock_until(timeout))
    }

    #[inline]
    fn try_lock_for(&self, timeout: R::Duration) -> bool {
        self.lock_internal(|| self.mutex.try_lock_for(timeout))
    }
}

type RawRMutex = RawReentrantMutex<RawMutex, RawThreadId>;
#[pyclass(name = "RLock")]
struct PyRLock {
    mu: RawRMutex,
}

impl PyValue for PyRLock {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("_thread", "RLock")
    }
}

impl fmt::Debug for PyRLock {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.pad("PyRLock")
    }
}

impl LockProtocol for PyRLock {
    type RawMutex = RawRMutex;
    fn mutex(&self) -> &Self::RawMutex {
        &self.mu
    }
}

#[pyimpl(with(LockProtocol))]
impl PyRLock {
    #[pyslot]
    fn tp_new(cls: PyClassRef, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        PyRLock {
            mu: RawRMutex::INIT,
        }
        .into_ref_with_type(vm, cls)
    }
}

fn get_ident() -> u64 {
    let id = std::thread::current().id();
    // TODO: use id.as_u64() once it's stable, until then, ThreadId is just a wrapper
    // around NonZeroU64, so this is safe
    unsafe { std::mem::transmute(id) }
}

fn allocate_lock() -> PyLock {
    PyLock { mu: RawMutex::INIT }
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    py_module!(vm, "_thread", {
        "RLock" => PyRLock::make_class(ctx),
        "LockType" => PyLock::make_class(ctx),
        "get_ident" => ctx.new_function(get_ident),
        "allocate_lock" => ctx.new_function(allocate_lock),
        "TIMEOUT_MAX" => ctx.new_float(TIMEOUT_MAX),
    })
}

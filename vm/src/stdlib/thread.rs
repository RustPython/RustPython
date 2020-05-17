/// Implementation of the _thread module
use crate::function::PyFuncArgs;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{PyClassImpl, PyObjectRef, PyRef, PyResult, PyValue};
use crate::vm::VirtualMachine;

use parking_lot::{
    lock_api::{RawMutex as RawMutexT, RawMutexTimed, RawReentrantMutex},
    RawMutex, RawThreadId,
};
use std::fmt;
use std::time::Duration;

#[cfg(not(target_os = "windows"))]
const PY_TIMEOUT_MAX: isize = std::isize::MAX;

#[cfg(target_os = "windows")]
const PY_TIMEOUT_MAX: isize = 0xffffffff * 1_000_000;

const TIMEOUT_MAX: f64 = (PY_TIMEOUT_MAX / 1_000_000_000) as f64;

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

#[pyimpl]
impl PyLock {
    #[pymethod]
    #[pymethod(name = "acquire_lock")]
    #[pymethod(name = "__enter__")]
    #[allow(clippy::float_cmp, clippy::match_bool)]
    fn acquire(&self, args: AcquireArgs, vm: &VirtualMachine) -> PyResult<bool> {
        match args.waitflag {
            true if args.timeout == -1.0 => {
                self.mu.lock();
                Ok(true)
            }
            true if args.timeout < 0.0 => {
                Err(vm.new_value_error("timeout value must be positive".to_owned()))
            }
            true => Ok(self.mu.try_lock_for(Duration::from_secs_f64(args.timeout))),
            false if args.timeout != -1.0 => {
                Err(vm
                    .new_value_error("can't specify a timeout for a non-blocking call".to_owned()))
            }
            false => Ok(self.mu.try_lock()),
        }
    }
    #[pymethod]
    #[pymethod(name = "release_lock")]
    fn release(&self) {
        self.mu.unlock()
    }

    #[pymethod(magic)]
    fn exit(&self, _args: PyFuncArgs) {
        self.release()
    }

    #[pymethod]
    fn locked(&self) -> bool {
        self.mu.is_locked()
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

#[pyimpl]
impl PyRLock {
    #[pyslot]
    fn tp_new(cls: PyClassRef, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        PyRLock {
            mu: RawRMutex::INIT,
        }
        .into_ref_with_type(vm, cls)
    }

    #[pymethod]
    #[pymethod(name = "acquire_lock")]
    #[pymethod(name = "__enter__")]
    #[allow(clippy::float_cmp, clippy::match_bool)]
    fn acquire(&self, args: AcquireArgs, vm: &VirtualMachine) -> PyResult<bool> {
        match args.waitflag {
            true if args.timeout == -1.0 => {
                self.mu.lock();
                Ok(true)
            }
            true if args.timeout < 0.0 => {
                Err(vm.new_value_error("timeout value must be positive".to_owned()))
            }
            true => Ok(self.mu.try_lock_for(Duration::from_secs_f64(args.timeout))),
            false if args.timeout != -1.0 => {
                Err(vm
                    .new_value_error("can't specify a timeout for a non-blocking call".to_owned()))
            }
            false => Ok(self.mu.try_lock()),
        }
    }
    #[pymethod]
    #[pymethod(name = "release_lock")]
    fn release(&self) {
        self.mu.unlock()
    }

    #[pymethod(magic)]
    fn exit(&self, _args: PyFuncArgs) {
        self.release()
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

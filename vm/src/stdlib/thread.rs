/// Implementation of the _thread module
use crate::exceptions;
use crate::function::{Args, KwArgs, OptionalArg, PyFuncArgs};
use crate::obj::objdict::PyDictRef;
use crate::obj::objtuple::PyTupleRef;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{Either, PyCallable, PyClassImpl, PyObjectRef, PyRef, PyResult, PyValue};
use crate::vm::VirtualMachine;

use parking_lot::{
    lock_api::{RawMutex as RawMutexT, RawMutexTimed, RawReentrantMutex},
    RawMutex, RawThreadId,
};
use std::cell::RefCell;
use std::io::Write;
use std::time::Duration;
use std::{fmt, thread};

#[cfg(not(target_os = "windows"))]
const PY_TIMEOUT_MAX: isize = std::isize::MAX;

#[cfg(target_os = "windows")]
const PY_TIMEOUT_MAX: isize = 0xffffffff * 1_000_000;

const TIMEOUT_MAX: f64 = (PY_TIMEOUT_MAX / 1_000_000_000) as f64;

#[derive(FromArgs)]
struct AcquireArgs {
    #[pyarg(positional_or_keyword, default = "true")]
    waitflag: bool,
    #[pyarg(positional_or_keyword, default = "Either::A(-1.0)")]
    timeout: Either<f64, isize>,
}

macro_rules! acquire_lock_impl {
    ($mu:expr, $args:expr, $vm:expr) => {{
        let (mu, args, vm) = ($mu, $args, $vm);
        let timeout = match args.timeout {
            Either::A(f) => f,
            Either::B(i) => i as f64,
        };
        match args.waitflag {
            true if timeout == -1.0 => {
                mu.lock();
                Ok(true)
            }
            true if timeout < 0.0 => {
                Err(vm.new_value_error("timeout value must be positive".to_owned()))
            }
            true => {
                // TODO: respect TIMEOUT_MAX here
                Ok(mu.try_lock_for(Duration::from_secs_f64(timeout)))
            }
            false if timeout != -1.0 => {
                Err(vm
                    .new_value_error("can't specify a timeout for a non-blocking call".to_owned()))
            }
            false => Ok(mu.try_lock()),
        }
    }};
}

#[pyclass(name = "lock")]
struct PyLock {
    mu: RawMutex,
}
type PyLockRef = PyRef<PyLock>;

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
        acquire_lock_impl!(&self.mu, args, vm)
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
        acquire_lock_impl!(&self.mu, args, vm)
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

fn thread_get_ident() -> u64 {
    thread_to_id(&thread::current())
}

fn thread_to_id(t: &thread::Thread) -> u64 {
    // TODO: use id.as_u64() once it's stable, until then, ThreadId is just a wrapper
    // around NonZeroU64, so this is safe
    unsafe { std::mem::transmute(t.id()) }
}

fn thread_allocate_lock() -> PyLock {
    PyLock { mu: RawMutex::INIT }
}

fn thread_start_new_thread(
    func: PyCallable,
    args: PyTupleRef,
    kwargs: OptionalArg<PyDictRef>,
    vm: &VirtualMachine,
) -> PyResult<u64> {
    let thread_vm = vm.new_thread();
    let mut thread_builder = thread::Builder::new();
    let stacksize = vm.state.stacksize.load();
    if stacksize != 0 {
        thread_builder = thread_builder.stack_size(stacksize);
    }
    let res = thread_builder.spawn(move || {
        let vm = &thread_vm;
        let args = Args::from(args.as_slice().to_owned());
        let kwargs = KwArgs::from(kwargs.map_or_else(Default::default, |k| k.to_attributes()));
        if let Err(exc) = func.invoke(PyFuncArgs::from((args, kwargs)), vm) {
            // TODO: sys.unraisablehook
            let stderr = std::io::stderr();
            let mut stderr = stderr.lock();
            let repr = vm.to_repr(&func.into_object()).ok();
            let repr = repr
                .as_ref()
                .map_or("<object repr() failed>", |s| s.as_str());
            writeln!(stderr, "Exception ignored in thread started by: {}", repr)
                .and_then(|()| exceptions::write_exception(&mut stderr, vm, &exc))
                .ok();
        }
        SENTINELS.with(|sents| {
            for lock in sents.replace(Default::default()) {
                lock.release()
            }
        })
    });
    res.map(|handle| thread_to_id(&handle.thread()))
        .map_err(|err| super::os::convert_io_error(vm, err))
}

thread_local!(static SENTINELS: RefCell<Vec<PyLockRef>> = RefCell::default());

fn thread_set_sentinel(vm: &VirtualMachine) -> PyLockRef {
    let lock = PyLock { mu: RawMutex::INIT }.into_ref(vm);
    SENTINELS.with(|sents| sents.borrow_mut().push(lock.clone()));
    lock
}

fn thread_stack_size(size: OptionalArg<usize>, vm: &VirtualMachine) -> usize {
    let size = size.unwrap_or(0);
    // TODO: do validation on this to make sure it's not too small
    vm.state.stacksize.swap(size)
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    py_module!(vm, "_thread", {
        "RLock" => PyRLock::make_class(ctx),
        "LockType" => PyLock::make_class(ctx),
        "get_ident" => ctx.new_function(thread_get_ident),
        "allocate_lock" => ctx.new_function(thread_allocate_lock),
        "start_new_thread" => ctx.new_function(thread_start_new_thread),
        "_set_sentinel" => ctx.new_function(thread_set_sentinel),
        "stack_size" => ctx.new_function(thread_stack_size),
        "error" => ctx.exceptions.runtime_error.clone(),
        "TIMEOUT_MAX" => ctx.new_float(TIMEOUT_MAX),
    })
}

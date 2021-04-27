use crate::builtins::dict::PyDictRef;
use crate::builtins::pystr::PyStrRef;
use crate::builtins::pytype::PyTypeRef;
use crate::builtins::tuple::PyTupleRef;
/// Implementation of the _thread module
use crate::exceptions::{self, IntoPyException};
use crate::function::{FuncArgs, KwArgs, OptionalArg};
use crate::py_io;
use crate::pyobject::{
    BorrowValue, Either, IdProtocol, ItemProtocol, PyCallable, PyClassImpl, PyObjectRef, PyRef,
    PyResult, PyValue, StaticType, TypeProtocol,
};
use crate::slots::{SlotGetattro, SlotSetattro};
use crate::VirtualMachine;

use parking_lot::{
    lock_api::{RawMutex as RawMutexT, RawMutexTimed, RawReentrantMutex},
    RawMutex, RawThreadId,
};
use thread_local::ThreadLocal;

use std::cell::RefCell;
use std::io::Write;
use std::time::Duration;
use std::{fmt, thread};

// PY_TIMEOUT_MAX is a value in microseconds
#[cfg(not(target_os = "windows"))]
const PY_TIMEOUT_MAX: i64 = i64::MAX / 1_000;

#[cfg(target_os = "windows")]
const PY_TIMEOUT_MAX: i64 = 0xffffffff * 1_000;

// this is a value in seconds
const TIMEOUT_MAX: f64 = (PY_TIMEOUT_MAX / 1_000_000) as f64;

#[derive(FromArgs)]
struct AcquireArgs {
    #[pyarg(any, default = "true")]
    blocking: bool,
    #[pyarg(any, default = "Either::A(-1.0)")]
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
                let micros = timeout * 1_000_000.0;
                let nanos = timeout * 1_000_000_000.0;
                if micros > PY_TIMEOUT_MAX as f64 || nanos < 0.0 || !nanos.is_finite() {
                    return Err(vm.new_overflow_error(
                        "timestamp too large to convert to Rust Duration".to_owned(),
                    ));
                }

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
macro_rules! repr_lock_impl {
    ($zelf:expr) => {{
        let status = if $zelf.mu.is_locked() {
            "locked"
        } else {
            "unlocked"
        };
        format!(
            "<{} {} object at {:#x}>",
            status,
            $zelf.class().name,
            $zelf.get_id()
        )
    }};
}

#[pyclass(module = "thread", name = "lock")]
struct PyLock {
    mu: RawMutex,
}
type PyLockRef = PyRef<PyLock>;

impl PyValue for PyLock {
    fn class(_vm: &VirtualMachine) -> &PyTypeRef {
        Self::static_type()
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
    fn release(&self, vm: &VirtualMachine) -> PyResult<()> {
        if !self.mu.is_locked() {
            return Err(vm.new_runtime_error("release unlocked lock".to_owned()));
        }
        unsafe { self.mu.unlock() };
        Ok(())
    }

    #[pymethod(magic)]
    fn exit(&self, _args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
        self.release(vm)
    }

    #[pymethod]
    fn locked(&self) -> bool {
        self.mu.is_locked()
    }

    #[pymethod(magic)]
    fn repr(zelf: PyRef<Self>) -> String {
        repr_lock_impl!(zelf)
    }
}

pub type RawRMutex = RawReentrantMutex<RawMutex, RawThreadId>;
#[pyclass(module = "thread", name = "RLock")]
struct PyRLock {
    mu: RawRMutex,
}

impl PyValue for PyRLock {
    fn class(_vm: &VirtualMachine) -> &PyTypeRef {
        Self::static_type()
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
    fn tp_new(cls: PyTypeRef, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
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
    fn release(&self, vm: &VirtualMachine) -> PyResult<()> {
        if !self.mu.is_locked() {
            return Err(vm.new_runtime_error("release unlocked lock".to_owned()));
        }
        unsafe { self.mu.unlock() };
        Ok(())
    }

    #[pymethod]
    fn _is_owned(&self) -> bool {
        self.mu.is_owned_by_current_thread()
    }

    #[pymethod(magic)]
    fn exit(&self, _args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
        self.release(vm)
    }

    #[pymethod(magic)]
    fn repr(zelf: PyRef<Self>) -> String {
        repr_lock_impl!(zelf)
    }
}

fn _thread_get_ident() -> u64 {
    thread_to_id(&thread::current())
}

fn thread_to_id(t: &thread::Thread) -> u64 {
    // TODO: use id.as_u64() once it's stable, until then, ThreadId is just a wrapper
    // around NonZeroU64, so this is safe
    unsafe { std::mem::transmute(t.id()) }
}

fn _thread_allocate_lock() -> PyLock {
    PyLock { mu: RawMutex::INIT }
}

fn _thread_start_new_thread(
    func: PyCallable,
    args: PyTupleRef,
    kwargs: OptionalArg<PyDictRef>,
    vm: &VirtualMachine,
) -> PyResult<u64> {
    let args = FuncArgs::new(
        args.borrow_value().to_owned(),
        kwargs
            .map_or_else(Default::default, |k| k.to_attributes())
            .into_iter()
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
            thread_to_id(&handle.thread())
        })
        .map_err(|err| err.into_pyexception(vm))
}

fn run_thread(func: PyCallable, args: FuncArgs, vm: &VirtualMachine) {
    if let Err(exc) = func.invoke(args, vm) {
        // TODO: sys.unraisablehook
        let stderr = std::io::stderr();
        let mut stderr = py_io::IoWriter(stderr.lock());
        let repr = vm.to_repr(&func.into_object()).ok();
        let repr = repr
            .as_ref()
            .map_or("<object repr() failed>", |s| s.borrow_value());
        writeln!(*stderr, "Exception ignored in thread started by: {}", repr)
            .and_then(|()| exceptions::write_exception(&mut stderr, vm, &exc))
            .ok();
    }
    SENTINELS.with(|sents| {
        for lock in sents.replace(Default::default()) {
            if lock.mu.is_locked() {
                unsafe { lock.mu.unlock() };
            }
        }
    });
    vm.state.thread_count.fetch_sub(1);
}

thread_local!(static SENTINELS: RefCell<Vec<PyLockRef>> = RefCell::default());

fn _thread_set_sentinel(vm: &VirtualMachine) -> PyLockRef {
    let lock = PyLock { mu: RawMutex::INIT }.into_ref(vm);
    SENTINELS.with(|sents| sents.borrow_mut().push(lock.clone()));
    lock
}

fn _thread_stack_size(size: OptionalArg<usize>, vm: &VirtualMachine) -> usize {
    let size = size.unwrap_or(0);
    // TODO: do validation on this to make sure it's not too small
    vm.state.stacksize.swap(size)
}

fn _thread_count(vm: &VirtualMachine) -> usize {
    vm.state.thread_count.load()
}

#[pyclass(module = "thread", name = "_local")]
#[derive(Debug)]
struct PyLocal {
    data: ThreadLocal<PyDictRef>,
}

impl PyValue for PyLocal {
    fn class(_vm: &VirtualMachine) -> &PyTypeRef {
        Self::static_type()
    }
}

#[pyimpl(with(SlotGetattro, SlotSetattro), flags(BASETYPE))]
impl PyLocal {
    fn ldict(&self, vm: &VirtualMachine) -> PyDictRef {
        self.data.get_or(|| vm.ctx.new_dict()).clone()
    }

    #[pyslot]
    fn tp_new(cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        PyLocal {
            data: ThreadLocal::new(),
        }
        .into_ref_with_type(vm, cls)
    }
}

impl SlotGetattro for PyLocal {
    fn getattro(zelf: PyRef<Self>, attr: PyStrRef, vm: &VirtualMachine) -> PyResult {
        let ldict = zelf.ldict(vm);
        if attr.borrow_value() == "__dict__" {
            Ok(ldict.into_object())
        } else {
            let zelf = zelf.into_object();
            vm.generic_getattribute_opt(zelf.clone(), attr.clone(), Some(ldict))?
                .ok_or_else(|| {
                    vm.new_attribute_error(format!("{} has no attribute '{}'", zelf, attr))
                })
        }
    }
}

impl SlotSetattro for PyLocal {
    fn setattro(
        zelf: &PyRef<Self>,
        attr: PyStrRef,
        value: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        if attr.borrow_value() == "__dict__" {
            Err(vm.new_attribute_error(format!(
                "{} attribute '__dict__' is read-only",
                zelf.as_object()
            )))
        } else {
            let dict = zelf.ldict(vm);
            if let Some(value) = value {
                dict.set_item(attr, value, vm)?;
            } else {
                dict.del_item(attr, vm)?;
            }
            Ok(())
        }
    }
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    py_module!(vm, "_thread", {
        "RLock" => PyRLock::make_class(ctx),
        "LockType" => PyLock::make_class(ctx),
        "_local" => PyLocal::make_class(ctx),
        "get_ident" => named_function!(ctx, _thread, get_ident),
        "allocate_lock" => named_function!(ctx, _thread, allocate_lock),
        "start_new_thread" => named_function!(ctx, _thread, start_new_thread),
        "_set_sentinel" => named_function!(ctx, _thread, set_sentinel),
        "stack_size" => named_function!(ctx, _thread, stack_size),
        "_count" => named_function!(ctx, _thread, count),
        "error" => ctx.exceptions.runtime_error.clone(),
        "TIMEOUT_MAX" => ctx.new_float(TIMEOUT_MAX),
    })
}

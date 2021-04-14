//! Implement virtual machine to run instructions.
//!
//! See also:
//!   https://github.com/ProgVal/pythonvm-rust/blob/master/src/processor/mod.rs
//!

use std::cell::{Cell, Ref, RefCell};
use std::collections::{HashMap, HashSet};
use std::fmt;

use crossbeam_utils::atomic::AtomicCell;
use num_traits::{Signed, ToPrimitive};

use crate::builtins::code::{self, PyCode, PyCodeRef};
use crate::builtins::dict::PyDictRef;
use crate::builtins::int::{PyInt, PyIntRef};
use crate::builtins::list::PyList;
use crate::builtins::module::{self, PyModule};
use crate::builtins::object;
use crate::builtins::pybool;
use crate::builtins::pystr::{PyStr, PyStrRef};
use crate::builtins::pytype::PyTypeRef;
use crate::builtins::tuple::{PyTuple, PyTupleTyped};
use crate::common::{hash::HashSecret, lock::PyMutex, rc::PyRc};
#[cfg(feature = "rustpython-compiler")]
use crate::compile::{self, CompileError, CompileErrorType, CompileOpts};
use crate::exceptions::{self, PyBaseException, PyBaseExceptionRef};
use crate::frame::{ExecutionResult, Frame, FrameRef};
use crate::function::{FuncArgs, IntoFuncArgs};
use crate::pyobject::{
    BorrowValue, Either, IdProtocol, IntoPyObject, ItemProtocol, PyArithmaticValue, PyContext,
    PyLease, PyMethod, PyObject, PyObjectRef, PyRef, PyRefExact, PyResult, PyValue, TryFromObject,
    TryIntoRef, TypeProtocol,
};
use crate::scope::Scope;
use crate::slots::PyComparisonOp;
use crate::{builtins, bytecode, frozen, import, iterator, stdlib, sysmodule};

// use objects::ects;

// Objects are live when they are on stack, or referenced by a name (for now)

/// Top level container of a python virtual machine. In theory you could
/// create more instances of this struct and have them operate fully isolated.
pub struct VirtualMachine {
    pub builtins: PyObjectRef,
    pub sys_module: PyObjectRef,
    pub ctx: PyRc<PyContext>,
    pub frames: RefCell<Vec<FrameRef>>,
    pub wasm_id: Option<String>,
    exceptions: RefCell<ExceptionStack>,
    pub import_func: PyObjectRef,
    pub profile_func: RefCell<PyObjectRef>,
    pub trace_func: RefCell<PyObjectRef>,
    pub use_tracing: Cell<bool>,
    pub recursion_limit: Cell<usize>,
    pub signal_handlers: Option<Box<RefCell<[Option<PyObjectRef>; NSIG]>>>,
    pub repr_guards: RefCell<HashSet<usize>>,
    pub state: PyRc<PyGlobalState>,
    pub initialized: bool,
}

#[derive(Debug, Default)]
struct ExceptionStack {
    exc: Option<PyBaseExceptionRef>,
    prev: Option<Box<ExceptionStack>>,
}

pub(crate) mod thread {
    use super::{PyObjectRef, TypeProtocol, VirtualMachine};
    use itertools::Itertools;
    use std::cell::RefCell;
    use std::ptr::NonNull;
    use std::thread_local;

    thread_local! {
        pub(super) static VM_STACK: RefCell<Vec<NonNull<VirtualMachine>>> = Vec::with_capacity(1).into();
    }

    pub fn enter_vm<R>(vm: &VirtualMachine, f: impl FnOnce() -> R) -> R {
        VM_STACK.with(|vms| {
            vms.borrow_mut().push(vm.into());
            let ret = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
            vms.borrow_mut().pop();
            ret.unwrap_or_else(|e| std::panic::resume_unwind(e))
        })
    }

    pub fn with_vm<F, R>(obj: &PyObjectRef, f: F) -> Option<R>
    where
        F: Fn(&VirtualMachine) -> R,
    {
        let vm_owns_obj = |intp: NonNull<VirtualMachine>| {
            // SAFETY: all references in VM_STACK should be valid
            let vm = unsafe { intp.as_ref() };
            obj.isinstance(&vm.ctx.types.object_type)
        };
        VM_STACK.with(|vms| {
            let intp = match vms.borrow().iter().copied().exactly_one() {
                Ok(x) => {
                    debug_assert!(vm_owns_obj(x));
                    x
                }
                Err(mut others) => others.find(|x| vm_owns_obj(*x))?,
            };
            // SAFETY: all references in VM_STACK should be valid, and should not be changed or moved
            // at least until this function returns and the stack unwinds to an enter_vm() call
            let vm = unsafe { intp.as_ref() };
            Some(f(vm))
        })
    }
}

pub struct PyGlobalState {
    pub settings: PySettings,
    pub stdlib_inits: HashMap<String, stdlib::StdlibInitFunc, ahash::RandomState>,
    pub frozen: HashMap<String, code::FrozenModule, ahash::RandomState>,
    pub stacksize: AtomicCell<usize>,
    pub thread_count: AtomicCell<usize>,
    pub hash_secret: HashSecret,
    pub atexit_funcs: PyMutex<Vec<(PyObjectRef, FuncArgs)>>,
}

pub const NSIG: usize = 64;

#[derive(Copy, Clone, PartialEq, Eq)]
pub enum InitParameter {
    Internal,
    External,
}

/// Struct containing all kind of settings for the python vm.
pub struct PySettings {
    /// -d command line switch
    pub debug: bool,

    /// -i
    pub inspect: bool,

    /// -i, with no script
    pub interactive: bool,

    /// -O optimization switch counter
    pub optimize: u8,

    /// -s
    pub no_user_site: bool,

    /// -S
    pub no_site: bool,

    /// -E
    pub ignore_environment: bool,

    /// verbosity level (-v switch)
    pub verbose: u8,

    /// -q
    pub quiet: bool,

    /// -B
    pub dont_write_bytecode: bool,

    /// -b
    pub bytes_warning: u64,

    /// -Xfoo[=bar]
    pub xopts: Vec<(String, Option<String>)>,

    /// -I
    pub isolated: bool,

    /// -Xdev
    pub dev_mode: bool,

    /// -Wfoo
    pub warnopts: Vec<String>,

    /// Environment PYTHONPATH and RUSTPYTHONPATH:
    pub path_list: Vec<String>,

    /// sys.argv
    pub argv: Vec<String>,

    /// PYTHONHASHSEED=x
    pub hash_seed: Option<u32>,

    /// -u, PYTHONUNBUFFERED=x
    // TODO: use this; can TextIOWrapper even work with a non-buffered?
    pub stdio_unbuffered: bool,
}

/// Trace events for sys.settrace and sys.setprofile.
enum TraceEvent {
    Call,
    Return,
}

impl fmt::Display for TraceEvent {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use TraceEvent::*;
        match self {
            Call => write!(f, "call"),
            Return => write!(f, "return"),
        }
    }
}

/// Sensible default settings.
impl Default for PySettings {
    fn default() -> Self {
        PySettings {
            debug: false,
            inspect: false,
            interactive: false,
            optimize: 0,
            no_user_site: false,
            no_site: false,
            ignore_environment: false,
            verbose: 0,
            quiet: false,
            dont_write_bytecode: false,
            bytes_warning: 0,
            xopts: vec![],
            isolated: false,
            dev_mode: false,
            warnopts: vec![],
            path_list: vec![],
            argv: vec![],
            hash_seed: None,
            stdio_unbuffered: false,
        }
    }
}

impl VirtualMachine {
    /// Create a new `VirtualMachine` structure.
    fn new(settings: PySettings) -> VirtualMachine {
        flame_guard!("new VirtualMachine");
        let ctx = PyContext::new();

        // make a new module without access to the vm; doesn't
        // set __spec__, __loader__, etc. attributes
        let new_module =
            |dict| PyObject::new(PyModule {}, ctx.types.module_type.clone(), Some(dict));

        // Hard-core modules:
        let builtins_dict = ctx.new_dict();
        let builtins = new_module(builtins_dict.clone());
        let sysmod_dict = ctx.new_dict();
        let sysmod = new_module(sysmod_dict.clone());

        let import_func = ctx.none();
        let profile_func = RefCell::new(ctx.none());
        let trace_func = RefCell::new(ctx.none());
        // hack to get around const array repeat expressions, rust issue #79270
        const NONE: Option<PyObjectRef> = None;
        let signal_handlers = RefCell::new([NONE; NSIG]);

        let stdlib_inits = stdlib::get_module_inits();

        let hash_secret = match settings.hash_seed {
            Some(seed) => HashSecret::new(seed),
            None => rand::random(),
        };

        let mut vm = VirtualMachine {
            builtins,
            sys_module: sysmod,
            ctx: PyRc::new(ctx),
            frames: RefCell::new(vec![]),
            wasm_id: None,
            exceptions: RefCell::default(),
            import_func,
            profile_func,
            trace_func,
            use_tracing: Cell::new(false),
            recursion_limit: Cell::new(if cfg!(debug_assertions) { 256 } else { 512 }),
            signal_handlers: Some(Box::new(signal_handlers)),
            repr_guards: RefCell::default(),
            state: PyRc::new(PyGlobalState {
                settings,
                stdlib_inits,
                frozen: HashMap::default(),
                stacksize: AtomicCell::new(0),
                thread_count: AtomicCell::new(0),
                hash_secret,
                atexit_funcs: PyMutex::default(),
            }),
            initialized: false,
        };

        let frozen = frozen::map_frozen(&vm, frozen::get_module_inits()).collect();
        PyRc::get_mut(&mut vm.state).unwrap().frozen = frozen;

        module::init_module_dict(
            &vm,
            &builtins_dict,
            vm.ctx.new_str("builtins"),
            vm.ctx.none(),
        );
        module::init_module_dict(&vm, &sysmod_dict, vm.ctx.new_str("sys"), vm.ctx.none());
        vm
    }

    fn initialize(&mut self, initialize_parameter: InitParameter) {
        flame_guard!("init VirtualMachine");

        if self.initialized {
            panic!("Double Initialize Error");
        }

        builtins::make_module(self, self.builtins.clone());
        sysmodule::make_module(self, self.sys_module.clone(), self.builtins.clone());

        let mut inner_init = || -> PyResult<()> {
            #[cfg(not(target_arch = "wasm32"))]
            import::import_builtin(self, "_signal")?;

            #[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
            {
                // this isn't fully compatible with CPython; it imports "io" and sets
                // builtins.open to io.OpenWrapper, but this is easier, since it doesn't
                // require the Python stdlib to be present
                let io = import::import_builtin(self, "_io")?;
                let set_stdio = |name, fd, mode: &str| {
                    let stdio = crate::stdlib::io::open(
                        self.ctx.new_int(fd),
                        Some(mode),
                        Default::default(),
                        self,
                    )?;
                    self.set_attr(
                        &self.sys_module,
                        format!("__{}__", name), // e.g. __stdin__
                        stdio.clone(),
                    )?;
                    self.set_attr(&self.sys_module, name, stdio)?;
                    Ok(())
                };
                set_stdio("stdin", 0, "r")?;
                set_stdio("stdout", 1, "w")?;
                set_stdio("stderr", 2, "w")?;

                let io_open = self.get_attribute(io, "open")?;
                self.set_attr(&self.builtins, "open", io_open)?;
            }

            import::init_importlib(self, initialize_parameter)?;

            Ok(())
        };

        let res = inner_init();

        self.expect_pyresult(res, "initializiation failed");

        self.initialized = true;
    }

    /// Can only be used in the initialization closure passed to [`Interpreter::new_with_init`]
    pub fn add_native_module(&mut self, name: String, module: stdlib::StdlibInitFunc) {
        let state = PyRc::get_mut(&mut self.state)
            .expect("can't add_native_module when there are multiple threads");
        state.stdlib_inits.insert(name, module);
    }

    /// Can only be used in the initialization closure passed to [`Interpreter::new_with_init`]
    pub fn add_frozen<I>(&mut self, frozen: I)
    where
        I: IntoIterator<Item = (String, bytecode::FrozenModule)>,
    {
        let frozen = frozen::map_frozen(self, frozen).collect::<Vec<_>>();
        let state = PyRc::get_mut(&mut self.state)
            .expect("can't add_frozen when there are multiple threads");
        state.frozen.extend(frozen);
    }

    /// Start a new thread with access to the same interpreter.
    ///
    /// # Note
    ///
    /// If you return a `PyObjectRef` (or a type that contains one) from `F`, and don't `join()`
    /// on the thread, there is a possibility that that thread will panic as `PyObjectRef`'s `Drop`
    /// implementation tries to run the `__del__` destructor of a python object but finds that it's
    /// not in the context of any vm.
    #[cfg(feature = "threading")]
    pub fn start_thread<F, R>(&self, f: F) -> std::thread::JoinHandle<R>
    where
        F: FnOnce(&VirtualMachine) -> R,
        F: Send + 'static,
        R: Send + 'static,
    {
        let thread = self.new_thread();
        std::thread::spawn(|| thread.run(f))
    }

    /// Create a new VM thread that can be passed to a function like [`std::thread::spawn`]
    /// to use the same interpreter on a different thread. Note that if you just want to
    /// use this with `thread::spawn`, you can use
    /// [`vm.start_thread()`](`VirtualMachine::start_thread`) as a convenience.
    ///
    /// # Usage
    ///
    /// ```
    /// # rustpython_vm::Interpreter::default().enter(|vm| {
    /// use std::thread::Builder;
    /// let handle = Builder::new()
    ///     .name("my thread :)".into())
    ///     .spawn(vm.new_thread().make_spawn_func(|vm| vm.ctx.none()))
    ///     .expect("couldn't spawn thread");
    /// let returned_obj = handle.join().expect("thread panicked");
    /// assert!(vm.is_none(&returned_obj));
    /// # })
    /// ```
    ///
    /// Note: this function is safe, but running the returned PyThread in the same
    /// thread context (i.e. with the same thread-local storage) doesn't have any
    /// specific guaranteed behavior.
    #[cfg(feature = "threading")]
    pub fn new_thread(&self) -> PyThread {
        let thread_vm = VirtualMachine {
            builtins: self.builtins.clone(),
            sys_module: self.sys_module.clone(),
            ctx: self.ctx.clone(),
            frames: RefCell::new(vec![]),
            wasm_id: self.wasm_id.clone(),
            exceptions: RefCell::default(),
            import_func: self.import_func.clone(),
            profile_func: RefCell::new(self.ctx.none()),
            trace_func: RefCell::new(self.ctx.none()),
            use_tracing: Cell::new(false),
            recursion_limit: self.recursion_limit.clone(),
            signal_handlers: None,
            repr_guards: RefCell::default(),
            state: self.state.clone(),
            initialized: self.initialized,
        };
        PyThread { thread_vm }
    }

    pub fn run_atexit_funcs(&self) -> PyResult<()> {
        let mut last_exc = None;
        for (func, args) in self.state.atexit_funcs.lock().drain(..).rev() {
            if let Err(e) = self.invoke(&func, args) {
                last_exc = Some(e.clone());
                if !e.isinstance(&self.ctx.exceptions.system_exit) {
                    writeln!(sysmodule::PyStderr(self), "Error in atexit._run_exitfuncs:");
                    exceptions::print_exception(self, e);
                }
            }
        }
        match last_exc {
            None => Ok(()),
            Some(e) => Err(e),
        }
    }

    pub fn run_code_obj(&self, code: PyCodeRef, scope: Scope) -> PyResult {
        let frame =
            Frame::new(code, scope, self.builtins.dict().unwrap(), &[], self).into_ref(self);
        self.run_frame_full(frame)
    }

    pub fn run_frame_full(&self, frame: FrameRef) -> PyResult {
        match self.run_frame(frame)? {
            ExecutionResult::Return(value) => Ok(value),
            _ => panic!("Got unexpected result from function"),
        }
    }

    pub fn with_frame<R, F: FnOnce(FrameRef) -> PyResult<R>>(
        &self,
        frame: FrameRef,
        f: F,
    ) -> PyResult<R> {
        self.check_recursive_call("")?;
        self.frames.borrow_mut().push(frame.clone());
        let result = f(frame);
        // defer dec frame
        let _popped = self.frames.borrow_mut().pop();
        result
    }

    pub fn run_frame(&self, frame: FrameRef) -> PyResult<ExecutionResult> {
        self.with_frame(frame, |f| f.run(self))
    }

    fn check_recursive_call(&self, _where: &str) -> PyResult<()> {
        if self.frames.borrow().len() > self.recursion_limit.get() {
            Err(self.new_recursion_error(format!("maximum recursion depth exceeded {}", _where)))
        } else {
            Ok(())
        }
    }

    pub fn current_frame(&self) -> Option<Ref<FrameRef>> {
        let frames = self.frames.borrow();
        if frames.is_empty() {
            None
        } else {
            Some(Ref::map(self.frames.borrow(), |frames| {
                frames.last().unwrap()
            }))
        }
    }

    pub fn current_locals(&self) -> PyResult<PyDictRef> {
        self.current_frame()
            .expect("called current_locals but no frames on the stack")
            .locals(self)
    }

    pub fn current_globals(&self) -> Ref<PyDictRef> {
        let frame = self
            .current_frame()
            .expect("called current_globals but no frames on the stack");
        Ref::map(frame, |f| &f.globals)
    }

    pub fn try_class(&self, module: &str, class: &str) -> PyResult<PyTypeRef> {
        let class = self
            .get_attribute(self.import(module, None, 0)?, class)?
            .downcast()
            .expect("not a class");
        Ok(class)
    }

    pub fn class(&self, module: &str, class: &str) -> PyTypeRef {
        let module = self
            .import(module, None, 0)
            .unwrap_or_else(|_| panic!("unable to import {}", module));
        let class = self
            .get_attribute(module.clone(), class)
            .unwrap_or_else(|_| panic!("module {} has no class {}", module, class));
        class.downcast().expect("not a class")
    }

    /// Create a new python object
    pub fn new_pyobj<T: IntoPyObject>(&self, value: T) -> PyObjectRef {
        value.into_pyobject(self)
    }

    pub fn new_code_object(&self, code: impl code::IntoCodeObject) -> PyCodeRef {
        self.ctx.new_code_object(code.into_codeobj(self))
    }

    pub fn new_module(&self, name: &str, dict: PyDictRef) -> PyObjectRef {
        module::init_module_dict(
            self,
            &dict,
            self.new_pyobj(name.to_owned()),
            self.ctx.none(),
        );
        PyObject::new(PyModule {}, self.ctx.types.module_type.clone(), Some(dict))
    }

    /// Instantiate an exception with arguments.
    /// This function should only be used with builtin exception types; if a user-defined exception
    /// type is passed in, it may not be fully initialized; try using [`exceptions::invoke`](invoke)
    /// or [`exceptions::ExceptionCtor`](ctor) instead.
    ///
    /// [invoke]: rustpython_vm::exceptions::invoke
    /// [ctor]: rustpython_vm::exceptions::ExceptionCtor
    pub fn new_exception(&self, exc_type: PyTypeRef, args: Vec<PyObjectRef>) -> PyBaseExceptionRef {
        // TODO: add repr of args into logging?
        vm_trace!("New exception created: {}", exc_type.name);

        PyRef::new_ref(
            PyBaseException::new(args, self),
            exc_type,
            Some(self.ctx.new_dict()),
        )
    }

    /// Instantiate an exception with no arguments.
    /// This function should only be used with builtin exception types; if a user-defined exception
    /// type is passed in, it may not be fully initialized; try using [`exceptions::invoke`](invoke)
    /// or [`exceptions::ExceptionCtor`](ctor) instead.
    ///
    /// [invoke]: rustpython_vm::exceptions::invoke
    /// [ctor]: rustpython_vm::exceptions::ExceptionCtor
    pub fn new_exception_empty(&self, exc_type: PyTypeRef) -> PyBaseExceptionRef {
        self.new_exception(exc_type, vec![])
    }

    /// Instantiate an exception with `msg` as the only argument.
    /// This function should only be used with builtin exception types; if a user-defined exception
    /// type is passed in, it may not be fully initialized; try using [`exceptions::invoke`](invoke)
    /// or [`exceptions::ExceptionCtor`](ctor) instead.
    ///
    /// [invoke]: rustpython_vm::exceptions::invoke
    /// [ctor]: rustpython_vm::exceptions::ExceptionCtor
    pub fn new_exception_msg(&self, exc_type: PyTypeRef, msg: String) -> PyBaseExceptionRef {
        self.new_exception(exc_type, vec![self.ctx.new_str(msg)])
    }

    pub fn new_lookup_error(&self, msg: String) -> PyBaseExceptionRef {
        let lookup_error = self.ctx.exceptions.lookup_error.clone();
        self.new_exception_msg(lookup_error, msg)
    }

    pub fn new_attribute_error(&self, msg: String) -> PyBaseExceptionRef {
        let attribute_error = self.ctx.exceptions.attribute_error.clone();
        self.new_exception_msg(attribute_error, msg)
    }

    pub fn new_type_error(&self, msg: String) -> PyBaseExceptionRef {
        let type_error = self.ctx.exceptions.type_error.clone();
        self.new_exception_msg(type_error, msg)
    }

    pub fn new_name_error(&self, msg: String) -> PyBaseExceptionRef {
        let name_error = self.ctx.exceptions.name_error.clone();
        self.new_exception_msg(name_error, msg)
    }

    pub fn new_unsupported_binop_error(
        &self,
        a: &PyObjectRef,
        b: &PyObjectRef,
        op: &str,
    ) -> PyBaseExceptionRef {
        self.new_type_error(format!(
            "Unsupported operand types for '{}': '{}' and '{}'",
            op,
            a.class().name,
            b.class().name
        ))
    }

    pub fn new_unsupported_ternop_error(
        &self,
        a: &PyObjectRef,
        b: &PyObjectRef,
        c: &PyObjectRef,
        op: &str,
    ) -> PyBaseExceptionRef {
        self.new_type_error(format!(
            "Unsupported operand types for '{}': '{}', '{}', and '{}'",
            op,
            a.class().name,
            b.class().name,
            c.class().name
        ))
    }

    pub fn new_os_error(&self, msg: String) -> PyBaseExceptionRef {
        let os_error = self.ctx.exceptions.os_error.clone();
        self.new_exception_msg(os_error, msg)
    }

    pub fn new_unicode_decode_error(&self, msg: String) -> PyBaseExceptionRef {
        let unicode_decode_error = self.ctx.exceptions.unicode_decode_error.clone();
        self.new_exception_msg(unicode_decode_error, msg)
    }

    pub fn new_unicode_encode_error(&self, msg: String) -> PyBaseExceptionRef {
        let unicode_encode_error = self.ctx.exceptions.unicode_encode_error.clone();
        self.new_exception_msg(unicode_encode_error, msg)
    }

    /// Create a new python ValueError object. Useful for raising errors from
    /// python functions implemented in rust.
    pub fn new_value_error(&self, msg: String) -> PyBaseExceptionRef {
        let value_error = self.ctx.exceptions.value_error.clone();
        self.new_exception_msg(value_error, msg)
    }

    pub fn new_buffer_error(&self, msg: String) -> PyBaseExceptionRef {
        let buffer_error = self.ctx.exceptions.buffer_error.clone();
        self.new_exception_msg(buffer_error, msg)
    }

    pub fn new_key_error(&self, obj: PyObjectRef) -> PyBaseExceptionRef {
        let key_error = self.ctx.exceptions.key_error.clone();
        self.new_exception(key_error, vec![obj])
    }

    pub fn new_index_error(&self, msg: String) -> PyBaseExceptionRef {
        let index_error = self.ctx.exceptions.index_error.clone();
        self.new_exception_msg(index_error, msg)
    }

    pub fn new_not_implemented_error(&self, msg: String) -> PyBaseExceptionRef {
        let not_implemented_error = self.ctx.exceptions.not_implemented_error.clone();
        self.new_exception_msg(not_implemented_error, msg)
    }

    pub fn new_recursion_error(&self, msg: String) -> PyBaseExceptionRef {
        let recursion_error = self.ctx.exceptions.recursion_error.clone();
        self.new_exception_msg(recursion_error, msg)
    }

    pub fn new_zero_division_error(&self, msg: String) -> PyBaseExceptionRef {
        let zero_division_error = self.ctx.exceptions.zero_division_error.clone();
        self.new_exception_msg(zero_division_error, msg)
    }

    pub fn new_overflow_error(&self, msg: String) -> PyBaseExceptionRef {
        let overflow_error = self.ctx.exceptions.overflow_error.clone();
        self.new_exception_msg(overflow_error, msg)
    }

    #[cfg(feature = "rustpython-compiler")]
    pub fn new_syntax_error(&self, error: &CompileError) -> PyBaseExceptionRef {
        let syntax_error_type = match &error.error {
            CompileErrorType::Parse(p) if p.is_indentation_error() => {
                self.ctx.exceptions.indentation_error.clone()
            }
            CompileErrorType::Parse(p) if p.is_tab_error() => self.ctx.exceptions.tab_error.clone(),
            _ => self.ctx.exceptions.syntax_error.clone(),
        };
        let syntax_error = self.new_exception_msg(syntax_error_type, error.to_string());
        let lineno = self.ctx.new_int(error.location.row());
        let offset = self.ctx.new_int(error.location.column());
        self.set_attr(syntax_error.as_object(), "lineno", lineno)
            .unwrap();
        self.set_attr(syntax_error.as_object(), "offset", offset)
            .unwrap();
        self.set_attr(
            syntax_error.as_object(),
            "text",
            error.statement.clone().into_pyobject(self),
        )
        .unwrap();
        self.set_attr(
            syntax_error.as_object(),
            "filename",
            self.ctx.new_str(error.source_path.clone()),
        )
        .unwrap();
        syntax_error
    }

    pub fn new_import_error(
        &self,
        msg: String,
        name: impl TryIntoRef<PyStr>,
    ) -> PyBaseExceptionRef {
        let import_error = self.ctx.exceptions.import_error.clone();
        let exc = self.new_exception_msg(import_error, msg);
        self.set_attr(exc.as_object(), "name", name.try_into_ref(self).unwrap())
            .unwrap();
        exc
    }

    pub fn new_runtime_error(&self, msg: String) -> PyBaseExceptionRef {
        let runtime_error = self.ctx.exceptions.runtime_error.clone();
        self.new_exception_msg(runtime_error, msg)
    }

    pub fn new_stop_iteration(&self) -> PyBaseExceptionRef {
        let stop_iteration_type = self.ctx.exceptions.stop_iteration.clone();
        self.new_exception_empty(stop_iteration_type)
    }

    // TODO: #[track_caller] when stabilized
    fn _py_panic_failed(&self, exc: PyBaseExceptionRef, msg: &str) -> ! {
        #[cfg(not(all(target_arch = "wasm32", not(target_os = "wasi"))))]
        {
            let show_backtrace = std::env::var_os("RUST_BACKTRACE").map_or(false, |v| &v != "0");
            let after = if show_backtrace {
                exceptions::print_exception(self, exc);
                "exception backtrace above"
            } else {
                "run with RUST_BACKTRACE=1 to see Python backtrace"
            };
            panic!("{}; {}", msg, after)
        }
        #[cfg(all(target_arch = "wasm32", not(target_os = "wasi")))]
        {
            use wasm_bindgen::prelude::*;
            #[wasm_bindgen]
            extern "C" {
                #[wasm_bindgen(js_namespace = console)]
                fn error(s: &str);
            }
            let mut s = Vec::<u8>::new();
            exceptions::write_exception(&mut s, self, &exc).unwrap();
            error(std::str::from_utf8(&s).unwrap());
            panic!("{}; exception backtrace above", msg)
        }
    }
    pub fn unwrap_pyresult<T>(&self, result: PyResult<T>) -> T {
        result.unwrap_or_else(|exc| {
            self._py_panic_failed(exc, "called `vm.unwrap_pyresult()` on an `Err` value")
        })
    }
    pub fn expect_pyresult<T>(&self, result: PyResult<T>, msg: &str) -> T {
        result.unwrap_or_else(|exc| self._py_panic_failed(exc, msg))
    }

    pub fn new_scope_with_builtins(&self) -> Scope {
        Scope::with_builtins(None, self.ctx.new_dict(), self)
    }

    /// Test whether a python object is `None`.
    pub fn is_none(&self, obj: &PyObjectRef) -> bool {
        obj.is(&self.ctx.none)
    }
    pub fn option_if_none(&self, obj: PyObjectRef) -> Option<PyObjectRef> {
        if self.is_none(&obj) {
            None
        } else {
            Some(obj)
        }
    }
    pub fn unwrap_or_none(&self, obj: Option<PyObjectRef>) -> PyObjectRef {
        obj.unwrap_or_else(|| self.ctx.none())
    }

    // Container of the virtual machine state:
    pub fn to_str(&self, obj: &PyObjectRef) -> PyResult<PyStrRef> {
        if obj.class().is(&self.ctx.types.str_type) {
            Ok(obj.clone().downcast().unwrap())
        } else {
            let s = self.call_special_method(obj.clone(), "__str__", ())?;
            PyStrRef::try_from_object(self, s)
        }
    }

    pub fn to_pystr<'a, T: Into<&'a PyObjectRef>>(&'a self, obj: T) -> PyResult<String> {
        let py_str_obj = self.to_str(obj.into())?;
        Ok(py_str_obj.borrow_value().to_owned())
    }

    pub fn to_repr(&self, obj: &PyObjectRef) -> PyResult<PyStrRef> {
        let repr = self.call_special_method(obj.clone(), "__repr__", ())?;
        PyStrRef::try_from_object(self, repr)
    }

    pub fn to_index_opt(&self, obj: PyObjectRef) -> Option<PyResult<PyIntRef>> {
        match obj.downcast() {
            Ok(val) => Some(Ok(val)),
            Err(obj) => self.get_method(obj, "__index__").map(|index| {
                // TODO: returning strict subclasses of int in __index__ is deprecated
                self.invoke(&index?, ())?.downcast().map_err(|bad| {
                    self.new_type_error(format!(
                        "__index__ returned non-int (type {})",
                        bad.class().name
                    ))
                })
            }),
        }
    }
    pub fn to_index(&self, obj: &PyObjectRef) -> PyResult<PyIntRef> {
        self.to_index_opt(obj.clone()).unwrap_or_else(|| {
            Err(self.new_type_error(format!(
                "'{}' object cannot be interpreted as an integer",
                obj.class().name
            )))
        })
    }

    #[inline]
    pub fn import(
        &self,
        module: impl TryIntoRef<PyStr>,
        from_list: Option<PyTupleTyped<PyStrRef>>,
        level: usize,
    ) -> PyResult {
        self._import_inner(module.try_into_ref(self)?, from_list, level)
    }

    fn _import_inner(
        &self,
        module: PyStrRef,
        from_list: Option<PyTupleTyped<PyStrRef>>,
        level: usize,
    ) -> PyResult {
        // if the import inputs seem weird, e.g a package import or something, rather than just
        // a straight `import ident`
        let weird = module.borrow_value().contains('.')
            || level != 0
            || from_list
                .as_ref()
                .map_or(false, |x| !x.borrow_value().is_empty());

        let cached_module = if weird {
            None
        } else {
            let sys_modules = self.get_attribute(self.sys_module.clone(), "modules")?;
            sys_modules.get_item(module.clone(), self).ok()
        };

        match cached_module {
            Some(cached_module) => {
                if self.is_none(&cached_module) {
                    Err(self.new_import_error(
                        format!("import of {} halted; None in sys.modules", module),
                        module,
                    ))
                } else {
                    Ok(cached_module)
                }
            }
            None => {
                let import_func = self
                    .get_attribute(self.builtins.clone(), "__import__")
                    .map_err(|_| {
                        self.new_import_error("__import__ not found".to_owned(), module.clone())
                    })?;

                let (locals, globals) = if let Some(frame) = self.current_frame() {
                    (Some(frame.locals.clone()), Some(frame.globals.clone()))
                } else {
                    (None, None)
                };
                let from_list = match from_list {
                    Some(tup) => tup.into_pyobject(self),
                    None => self.ctx.new_tuple(vec![]),
                };
                self.invoke(&import_func, (module, globals, locals, from_list, level))
                    .map_err(|exc| import::remove_importlib_frames(self, &exc))
            }
        }
    }

    /// Determines if `obj` is an instance of `cls`, either directly, indirectly or virtually via
    /// the __instancecheck__ magic method.
    pub fn isinstance(&self, obj: &PyObjectRef, cls: &PyTypeRef) -> PyResult<bool> {
        // cpython first does an exact check on the type, although documentation doesn't state that
        // https://github.com/python/cpython/blob/a24107b04c1277e3c1105f98aff5bfa3a98b33a0/Objects/abstract.c#L2408
        if obj.class().is(cls) {
            Ok(true)
        } else {
            let ret = self.call_special_method(
                cls.as_object().clone(),
                "__instancecheck__",
                (obj.clone(),),
            )?;
            pybool::boolval(self, ret)
        }
    }

    /// Determines if `subclass` is a subclass of `cls`, either directly, indirectly or virtually
    /// via the __subclasscheck__ magic method.
    pub fn issubclass(&self, subclass: &PyTypeRef, cls: &PyTypeRef) -> PyResult<bool> {
        let ret = self.call_special_method(
            cls.as_object().clone(),
            "__subclasscheck__",
            (subclass.clone(),),
        )?;
        pybool::boolval(self, ret)
    }

    pub fn call_get_descriptor_specific(
        &self,
        descr: PyObjectRef,
        obj: Option<PyObjectRef>,
        cls: Option<PyObjectRef>,
    ) -> Result<PyResult, PyObjectRef> {
        let descr_get = descr.class().mro_find_map(|cls| cls.slots.descr_get.load());
        match descr_get {
            Some(descr_get) => Ok(descr_get(descr, obj, cls, self)),
            None => Err(descr),
        }
    }

    pub fn call_get_descriptor(
        &self,
        descr: PyObjectRef,
        obj: PyObjectRef,
    ) -> Result<PyResult, PyObjectRef> {
        let cls = obj.clone_class().into_object();
        self.call_get_descriptor_specific(descr, Some(obj), Some(cls))
    }

    pub fn call_if_get_descriptor(&self, attr: PyObjectRef, obj: PyObjectRef) -> PyResult {
        self.call_get_descriptor(attr, obj).unwrap_or_else(Ok)
    }

    #[inline]
    pub fn call_method<T>(&self, obj: &PyObjectRef, method_name: &str, args: T) -> PyResult
    where
        T: IntoFuncArgs,
    {
        flame_guard!(format!("call_method({:?})", method_name));

        PyMethod::get(obj.clone(), PyStr::from(method_name).into_ref(self), self)?
            .invoke(args, self)
    }

    #[inline]
    pub(crate) fn get_special_method(
        &self,
        obj: PyObjectRef,
        method: &str,
    ) -> PyResult<Result<PyMethod, PyObjectRef>> {
        PyMethod::get_special(obj, method, self)
    }

    /// NOT PUBLIC API
    #[doc(hidden)]
    pub fn call_special_method(
        &self,
        obj: PyObjectRef,
        method: &str,
        args: impl IntoFuncArgs,
    ) -> PyResult {
        self.get_special_method(obj, method)?
            .map_err(|_obj| self.new_attribute_error(method.to_owned()))?
            .invoke(args, self)
    }

    fn _invoke(&self, callable: &PyObjectRef, args: FuncArgs) -> PyResult {
        vm_trace!("Invoke: {:?} {:?}", callable, args);
        let slot_call = callable.class().mro_find_map(|cls| cls.slots.call.load());
        match slot_call {
            Some(slot_call) => {
                self.trace_event(TraceEvent::Call)?;
                let result = slot_call(callable, args, self);
                self.trace_event(TraceEvent::Return)?;
                result
            }
            None => Err(self.new_type_error(format!(
                "'{}' object is not callable",
                callable.class().name
            ))),
        }
    }

    #[inline]
    pub fn invoke<T>(&self, func_ref: &PyObjectRef, args: T) -> PyResult
    where
        T: IntoFuncArgs,
    {
        self._invoke(func_ref, args.into_args(self))
    }

    /// Call registered trace function.
    #[inline]
    fn trace_event(&self, event: TraceEvent) -> PyResult<()> {
        if self.use_tracing.get() {
            self._trace_event_inner(event)
        } else {
            Ok(())
        }
    }
    fn _trace_event_inner(&self, event: TraceEvent) -> PyResult<()> {
        let trace_func = self.trace_func.borrow().clone();
        let profile_func = self.profile_func.borrow().clone();
        if self.is_none(&trace_func) && self.is_none(&profile_func) {
            return Ok(());
        }

        let frame_ref = self.current_frame();
        if frame_ref.is_none() {
            return Ok(());
        }

        let frame = frame_ref.unwrap().as_object().clone();
        let event = self.ctx.new_str(event.to_string());
        let args = vec![frame, event, self.ctx.none()];

        // temporarily disable tracing, during the call to the
        // tracing function itself.
        if !self.is_none(&trace_func) {
            self.use_tracing.set(false);
            let res = self.invoke(&trace_func, args.clone());
            self.use_tracing.set(true);
            res?;
        }

        if !self.is_none(&profile_func) {
            self.use_tracing.set(false);
            let res = self.invoke(&profile_func, args);
            self.use_tracing.set(true);
            res?;
        }
        Ok(())
    }

    pub fn extract_elements_func<T, F>(&self, value: &PyObjectRef, func: F) -> PyResult<Vec<T>>
    where
        F: Fn(PyObjectRef) -> PyResult<T>,
    {
        // Extract elements from item, if possible:
        let cls = value.class();
        if cls.is(&self.ctx.types.tuple_type) {
            value
                .payload::<PyTuple>()
                .unwrap()
                .borrow_value()
                .iter()
                .map(|obj| func(obj.clone()))
                .collect()
        } else if cls.is(&self.ctx.types.list_type) {
            value
                .payload::<PyList>()
                .unwrap()
                .borrow_value()
                .iter()
                .map(|obj| func(obj.clone()))
                .collect()
        } else {
            let iter = iterator::get_iter(self, value.clone())?;
            iterator::try_map(self, &iter, |obj| func(obj))
        }
    }

    pub fn extract_elements<T: TryFromObject>(&self, value: &PyObjectRef) -> PyResult<Vec<T>> {
        self.extract_elements_func(value, |obj| T::try_from_object(self, obj))
    }

    pub fn map_iterable_object<F, R>(
        &self,
        obj: &PyObjectRef,
        mut f: F,
    ) -> PyResult<PyResult<Vec<R>>>
    where
        F: FnMut(PyObjectRef) -> PyResult<R>,
    {
        match_class!(match obj {
            ref l @ PyList => {
                let mut i: usize = 0;
                let mut results = Vec::with_capacity(l.borrow_value().len());
                loop {
                    let elem = {
                        let elements = &*l.borrow_value();
                        if i >= elements.len() {
                            results.shrink_to_fit();
                            return Ok(Ok(results));
                        } else {
                            elements[i].clone()
                        }
                        // free the lock
                    };
                    match f(elem) {
                        Ok(result) => results.push(result),
                        Err(err) => return Ok(Err(err)),
                    }
                    i += 1;
                }
            }
            ref t @ PyTuple => Ok(t.borrow_value().iter().cloned().map(f).collect()),
            // TODO: put internal iterable type
            obj => {
                let iter = iterator::get_iter(self, obj.clone())?;
                Ok(iterator::try_map(self, &iter, f))
            }
        })
    }

    // get_attribute should be used for full attribute access (usually from user code).
    #[cfg_attr(feature = "flame-it", flame("VirtualMachine"))]
    pub fn get_attribute<T>(&self, obj: PyObjectRef, attr_name: T) -> PyResult
    where
        T: TryIntoRef<PyStr>,
    {
        let attr_name = attr_name.try_into_ref(self)?;
        vm_trace!("vm.__getattribute__: {:?} {:?}", obj, attr_name);
        let getattro = obj
            .class()
            .mro_find_map(|cls| cls.slots.getattro.load())
            .unwrap();
        getattro(obj, attr_name, self)
    }

    pub fn get_attribute_opt<T>(
        &self,
        obj: PyObjectRef,
        attr_name: T,
    ) -> PyResult<Option<PyObjectRef>>
    where
        T: TryIntoRef<PyStr>,
    {
        match self.get_attribute(obj, attr_name) {
            Ok(attr) => Ok(Some(attr)),
            Err(e) if e.isinstance(&self.ctx.exceptions.attribute_error) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn call_set_attr(
        &self,
        obj: &PyObjectRef,
        attr_name: PyStrRef,
        attr_value: Option<PyObjectRef>,
    ) -> PyResult<()> {
        let setattro = {
            let cls = obj.class();
            cls.mro_find_map(|cls| cls.slots.setattro.load())
                .ok_or_else(|| {
                    let assign = attr_value.is_some();
                    let has_getattr = cls.mro_find_map(|cls| cls.slots.getattro.load()).is_some();
                    self.new_type_error(format!(
                        "'{}' object has {} attributes ({} {})",
                        cls.name,
                        if has_getattr { "only read-only" } else { "no" },
                        if assign { "assign to" } else { "del" },
                        attr_name
                    ))
                })?
        };
        setattro(obj, attr_name, attr_value, self)
    }

    pub fn set_attr<K, V>(&self, obj: &PyObjectRef, attr_name: K, attr_value: V) -> PyResult<()>
    where
        K: TryIntoRef<PyStr>,
        V: Into<PyObjectRef>,
    {
        let attr_name = attr_name.try_into_ref(self)?;
        self.call_set_attr(obj, attr_name, Some(attr_value.into()))
    }

    pub fn del_attr(&self, obj: &PyObjectRef, attr_name: impl TryIntoRef<PyStr>) -> PyResult<()> {
        let attr_name = attr_name.try_into_ref(self)?;
        self.call_set_attr(obj, attr_name, None)
    }

    // get_method should be used for internal access to magic methods (by-passing
    // the full getattribute look-up.
    pub fn get_method_or_type_error<F>(
        &self,
        obj: PyObjectRef,
        method_name: &str,
        err_msg: F,
    ) -> PyResult
    where
        F: FnOnce() -> String,
    {
        match obj.get_class_attr(method_name) {
            Some(method) => self.call_if_get_descriptor(method, obj),
            None => Err(self.new_type_error(err_msg())),
        }
    }

    // TODO: remove + transfer over to get_special_method
    pub(crate) fn get_method(&self, obj: PyObjectRef, method_name: &str) -> Option<PyResult> {
        let method = obj.get_class_attr(method_name)?;
        Some(self.call_if_get_descriptor(method, obj))
    }

    /// Calls a method on `obj` passing `arg`, if the method exists.
    ///
    /// Otherwise, or if the result is the special `NotImplemented` built-in constant,
    /// calls `unsupported` to determine fallback value.
    pub fn call_or_unsupported<F>(
        &self,
        obj: &PyObjectRef,
        arg: &PyObjectRef,
        method: &str,
        unsupported: F,
    ) -> PyResult
    where
        F: Fn(&VirtualMachine, &PyObjectRef, &PyObjectRef) -> PyResult,
    {
        if let Some(method_or_err) = self.get_method(obj.clone(), method) {
            let method = method_or_err?;
            let result = self.invoke(&method, (arg.clone(),))?;
            if let PyArithmaticValue::Implemented(x) = PyArithmaticValue::from_object(self, result)
            {
                return Ok(x);
            }
        }
        unsupported(self, obj, arg)
    }

    /// Calls a method, falling back to its reflection with the operands
    /// reversed, and then to the value provided by `unsupported`.
    ///
    /// For example: the following:
    ///
    /// `call_or_reflection(lhs, rhs, "__and__", "__rand__", unsupported)`
    ///
    /// 1. Calls `__and__` with `lhs` and `rhs`.
    /// 2. If above is not implemented, calls `__rand__` with `rhs` and `lhs`.
    /// 3. If above is not implemented, invokes `unsupported` for the result.
    pub fn call_or_reflection(
        &self,
        lhs: &PyObjectRef,
        rhs: &PyObjectRef,
        default: &str,
        reflection: &str,
        unsupported: fn(&VirtualMachine, &PyObjectRef, &PyObjectRef) -> PyResult,
    ) -> PyResult {
        // Try to call the default method
        self.call_or_unsupported(lhs, rhs, default, move |vm, lhs, rhs| {
            // Try to call the reflection method
            // don't call reflection method if operands are of the same type
            if !lhs.class().is(&rhs.class()) {
                vm.call_or_unsupported(rhs, lhs, reflection, |_, rhs, lhs| {
                    // switch them around again
                    unsupported(vm, lhs, rhs)
                })
            } else {
                unsupported(vm, lhs, rhs)
            }
        })
    }

    pub fn generic_getattribute(&self, obj: PyObjectRef, name: PyStrRef) -> PyResult {
        self.generic_getattribute_opt(obj.clone(), name.clone(), None)?
            .ok_or_else(|| self.new_attribute_error(format!("{} has no attribute '{}'", obj, name)))
    }

    /// CPython _PyObject_GenericGetAttrWithDict
    pub fn generic_getattribute_opt(
        &self,
        obj: PyObjectRef,
        name_str: PyStrRef,
        dict: Option<PyDictRef>,
    ) -> PyResult<Option<PyObjectRef>> {
        let name = name_str.borrow_value();
        let obj_cls = obj.class();
        let cls_attr = match obj_cls.get_attr(name) {
            Some(descr) => {
                let descr_cls = descr.class();
                let descr_get = descr_cls.mro_find_map(|cls| cls.slots.descr_get.load());
                if let Some(descr_get) = descr_get {
                    if descr_cls
                        .mro_find_map(|cls| cls.slots.descr_set.load())
                        .is_some()
                    {
                        drop(descr_cls);
                        let cls = PyLease::into_pyref(obj_cls).into_object();
                        return descr_get(descr, Some(obj), Some(cls), self).map(Some);
                    }
                }
                drop(descr_cls);
                Some((descr, descr_get))
            }
            None => None,
        };

        let dict = dict.or_else(|| obj.dict());

        let attr = if let Some(dict) = dict {
            dict.get_item_option(name, self)?
        } else {
            None
        };

        if let Some(obj_attr) = attr {
            Ok(Some(obj_attr))
        } else if let Some((attr, descr_get)) = cls_attr {
            match descr_get {
                Some(descr_get) => {
                    let cls = PyLease::into_pyref(obj_cls).into_object();
                    descr_get(attr, Some(obj), Some(cls), self).map(Some)
                }
                None => Ok(Some(attr)),
            }
        } else if let Some(getter) = obj_cls.get_attr("__getattr__") {
            drop(obj_cls);
            self.invoke(&getter, (obj, name_str)).map(Some)
        } else {
            Ok(None)
        }
    }

    pub fn is_callable(&self, obj: &PyObjectRef) -> bool {
        obj.class()
            .mro_find_map(|cls| cls.slots.call.load())
            .is_some()
    }

    #[inline]
    /// Checks for triggered signals and calls the appropriate handlers. A no-op on
    /// platforms where signals are not supported.
    pub fn check_signals(&self) -> PyResult<()> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            crate::stdlib::signal::check_signals(self)
        }
        #[cfg(target_arch = "wasm32")]
        {
            Ok(())
        }
    }

    /// Returns a basic CompileOpts instance with options accurate to the vm. Used
    /// as the CompileOpts for `vm.compile()`.
    #[cfg(feature = "rustpython-compiler")]
    pub fn compile_opts(&self) -> CompileOpts {
        CompileOpts {
            optimize: self.state.settings.optimize,
        }
    }

    #[cfg(feature = "rustpython-compiler")]
    pub fn compile(
        &self,
        source: &str,
        mode: compile::Mode,
        source_path: String,
    ) -> Result<PyCodeRef, CompileError> {
        self.compile_with_opts(source, mode, source_path, self.compile_opts())
    }

    #[cfg(feature = "rustpython-compiler")]
    pub fn compile_with_opts(
        &self,
        source: &str,
        mode: compile::Mode,
        source_path: String,
        opts: CompileOpts,
    ) -> Result<PyCodeRef, CompileError> {
        compile::compile(source, mode, source_path, opts)
            .map(|code| PyCode::new(self.map_codeobj(code)).into_ref(self))
    }

    fn call_codec_func(
        &self,
        func: &str,
        obj: PyObjectRef,
        encoding: Option<PyStrRef>,
        errors: Option<PyStrRef>,
    ) -> PyResult {
        let codecsmodule = self.import("_codecs", None, 0)?;
        let func = self.get_attribute(codecsmodule, func)?;
        let mut args = vec![obj, encoding.into_pyobject(self)];
        if let Some(errors) = errors {
            args.push(errors.into_object());
        }
        self.invoke(&func, args)
    }

    pub fn decode(
        &self,
        obj: PyObjectRef,
        encoding: Option<PyStrRef>,
        errors: Option<PyStrRef>,
    ) -> PyResult {
        self.call_codec_func("decode", obj, encoding, errors)
    }

    pub fn encode(
        &self,
        obj: PyObjectRef,
        encoding: Option<PyStrRef>,
        errors: Option<PyStrRef>,
    ) -> PyResult {
        self.call_codec_func("encode", obj, encoding, errors)
    }

    pub fn _sub(&self, a: &PyObjectRef, b: &PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__sub__", "__rsub__", |vm, a, b| {
            Err(vm.new_unsupported_binop_error(a, b, "-"))
        })
    }

    pub fn _isub(&self, a: &PyObjectRef, b: &PyObjectRef) -> PyResult {
        self.call_or_unsupported(a, b, "__isub__", |vm, a, b| {
            vm.call_or_reflection(a, b, "__sub__", "__rsub__", |vm, a, b| {
                Err(vm.new_unsupported_binop_error(a, b, "-="))
            })
        })
    }

    pub fn _add(&self, a: &PyObjectRef, b: &PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__add__", "__radd__", |vm, a, b| {
            Err(vm.new_unsupported_binop_error(a, b, "+"))
        })
    }

    pub fn _iadd(&self, a: &PyObjectRef, b: &PyObjectRef) -> PyResult {
        self.call_or_unsupported(a, b, "__iadd__", |vm, a, b| {
            vm.call_or_reflection(a, b, "__add__", "__radd__", |vm, a, b| {
                Err(vm.new_unsupported_binop_error(a, b, "+="))
            })
        })
    }

    pub fn _mul(&self, a: &PyObjectRef, b: &PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__mul__", "__rmul__", |vm, a, b| {
            Err(vm.new_unsupported_binop_error(a, b, "*"))
        })
    }

    pub fn _imul(&self, a: &PyObjectRef, b: &PyObjectRef) -> PyResult {
        self.call_or_unsupported(a, b, "__imul__", |vm, a, b| {
            vm.call_or_reflection(a, b, "__mul__", "__rmul__", |vm, a, b| {
                Err(vm.new_unsupported_binop_error(a, b, "*="))
            })
        })
    }

    pub fn _matmul(&self, a: &PyObjectRef, b: &PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__matmul__", "__rmatmul__", |vm, a, b| {
            Err(vm.new_unsupported_binop_error(a, b, "@"))
        })
    }

    pub fn _imatmul(&self, a: &PyObjectRef, b: &PyObjectRef) -> PyResult {
        self.call_or_unsupported(a, b, "__imatmul__", |vm, a, b| {
            vm.call_or_reflection(a, b, "__matmul__", "__rmatmul__", |vm, a, b| {
                Err(vm.new_unsupported_binop_error(a, b, "@="))
            })
        })
    }

    pub fn _truediv(&self, a: &PyObjectRef, b: &PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__truediv__", "__rtruediv__", |vm, a, b| {
            Err(vm.new_unsupported_binop_error(a, b, "/"))
        })
    }

    pub fn _itruediv(&self, a: &PyObjectRef, b: &PyObjectRef) -> PyResult {
        self.call_or_unsupported(a, b, "__itruediv__", |vm, a, b| {
            vm.call_or_reflection(a, b, "__truediv__", "__rtruediv__", |vm, a, b| {
                Err(vm.new_unsupported_binop_error(a, b, "/="))
            })
        })
    }

    pub fn _floordiv(&self, a: &PyObjectRef, b: &PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__floordiv__", "__rfloordiv__", |vm, a, b| {
            Err(vm.new_unsupported_binop_error(a, b, "//"))
        })
    }

    pub fn _ifloordiv(&self, a: &PyObjectRef, b: &PyObjectRef) -> PyResult {
        self.call_or_unsupported(a, b, "__ifloordiv__", |vm, a, b| {
            vm.call_or_reflection(a, b, "__floordiv__", "__rfloordiv__", |vm, a, b| {
                Err(vm.new_unsupported_binop_error(a, b, "//="))
            })
        })
    }

    pub fn _pow(&self, a: &PyObjectRef, b: &PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__pow__", "__rpow__", |vm, a, b| {
            Err(vm.new_unsupported_binop_error(a, b, "**"))
        })
    }

    pub fn _ipow(&self, a: &PyObjectRef, b: &PyObjectRef) -> PyResult {
        self.call_or_unsupported(a, b, "__ipow__", |vm, a, b| {
            vm.call_or_reflection(a, b, "__pow__", "__rpow__", |vm, a, b| {
                Err(vm.new_unsupported_binop_error(a, b, "**="))
            })
        })
    }

    pub fn _mod(&self, a: &PyObjectRef, b: &PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__mod__", "__rmod__", |vm, a, b| {
            Err(vm.new_unsupported_binop_error(a, b, "%"))
        })
    }

    pub fn _imod(&self, a: &PyObjectRef, b: &PyObjectRef) -> PyResult {
        self.call_or_unsupported(a, b, "__imod__", |vm, a, b| {
            vm.call_or_reflection(a, b, "__mod__", "__rmod__", |vm, a, b| {
                Err(vm.new_unsupported_binop_error(a, b, "%="))
            })
        })
    }

    pub fn _divmod(&self, a: &PyObjectRef, b: &PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__divmod__", "__rdivmod__", |vm, a, b| {
            Err(vm.new_unsupported_binop_error(a, b, "divmod"))
        })
    }

    pub fn _lshift(&self, a: &PyObjectRef, b: &PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__lshift__", "__rlshift__", |vm, a, b| {
            Err(vm.new_unsupported_binop_error(a, b, "<<"))
        })
    }

    pub fn _ilshift(&self, a: &PyObjectRef, b: &PyObjectRef) -> PyResult {
        self.call_or_unsupported(a, b, "__ilshift__", |vm, a, b| {
            vm.call_or_reflection(a, b, "__lshift__", "__rlshift__", |vm, a, b| {
                Err(vm.new_unsupported_binop_error(a, b, "<<="))
            })
        })
    }

    pub fn _rshift(&self, a: &PyObjectRef, b: &PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__rshift__", "__rrshift__", |vm, a, b| {
            Err(vm.new_unsupported_binop_error(a, b, ">>"))
        })
    }

    pub fn _irshift(&self, a: &PyObjectRef, b: &PyObjectRef) -> PyResult {
        self.call_or_unsupported(a, b, "__irshift__", |vm, a, b| {
            vm.call_or_reflection(a, b, "__rshift__", "__rrshift__", |vm, a, b| {
                Err(vm.new_unsupported_binop_error(a, b, ">>="))
            })
        })
    }

    pub fn _xor(&self, a: &PyObjectRef, b: &PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__xor__", "__rxor__", |vm, a, b| {
            Err(vm.new_unsupported_binop_error(a, b, "^"))
        })
    }

    pub fn _ixor(&self, a: &PyObjectRef, b: &PyObjectRef) -> PyResult {
        self.call_or_unsupported(a, b, "__ixor__", |vm, a, b| {
            vm.call_or_reflection(a, b, "__xor__", "__rxor__", |vm, a, b| {
                Err(vm.new_unsupported_binop_error(a, b, "^="))
            })
        })
    }

    pub fn _or(&self, a: &PyObjectRef, b: &PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__or__", "__ror__", |vm, a, b| {
            Err(vm.new_unsupported_binop_error(a, b, "|"))
        })
    }

    pub fn _ior(&self, a: &PyObjectRef, b: &PyObjectRef) -> PyResult {
        self.call_or_unsupported(a, b, "__ior__", |vm, a, b| {
            vm.call_or_reflection(a, b, "__or__", "__ror__", |vm, a, b| {
                Err(vm.new_unsupported_binop_error(a, b, "|="))
            })
        })
    }

    pub fn _and(&self, a: &PyObjectRef, b: &PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__and__", "__rand__", |vm, a, b| {
            Err(vm.new_unsupported_binop_error(a, b, "&"))
        })
    }

    pub fn _iand(&self, a: &PyObjectRef, b: &PyObjectRef) -> PyResult {
        self.call_or_unsupported(a, b, "__iand__", |vm, a, b| {
            vm.call_or_reflection(a, b, "__and__", "__rand__", |vm, a, b| {
                Err(vm.new_unsupported_binop_error(a, b, "&="))
            })
        })
    }

    // Perform a comparison, raising TypeError when the requested comparison
    // operator is not supported.
    // see: CPython PyObject_RichCompare
    fn _cmp(
        &self,
        v: &PyObjectRef,
        w: &PyObjectRef,
        op: PyComparisonOp,
    ) -> PyResult<Either<PyObjectRef, bool>> {
        let swapped = op.swapped();
        // TODO: _Py_EnterRecursiveCall(tstate, " in comparison")

        let call_cmp = |obj: &PyObjectRef, other, op| {
            let cmp = obj
                .class()
                .mro_find_map(|cls| cls.slots.cmp.load())
                .unwrap();
            Ok(match cmp(obj, other, op, self)? {
                Either::A(obj) => PyArithmaticValue::from_object(self, obj).map(Either::A),
                Either::B(arithmatic) => arithmatic.map(Either::B),
            })
        };

        let mut checked_reverse_op = false;
        let is_strict_subclass = {
            let v_class = v.class();
            let w_class = w.class();
            !v_class.is(&w_class) && w_class.issubclass(&v_class)
        };
        if is_strict_subclass {
            let res = call_cmp(w, v, swapped)?;
            checked_reverse_op = true;
            if let PyArithmaticValue::Implemented(x) = res {
                return Ok(x);
            }
        }
        if let PyArithmaticValue::Implemented(x) = call_cmp(v, w, op)? {
            return Ok(x);
        }
        if !checked_reverse_op {
            let res = call_cmp(w, v, swapped)?;
            if let PyArithmaticValue::Implemented(x) = res {
                return Ok(x);
            }
        }
        match op {
            PyComparisonOp::Eq => Ok(Either::B(v.is(&w))),
            PyComparisonOp::Ne => Ok(Either::B(!v.is(&w))),
            _ => Err(self.new_unsupported_binop_error(v, w, op.operator_token())),
        }
        // TODO: _Py_LeaveRecursiveCall(tstate);
    }

    pub fn bool_cmp(&self, a: &PyObjectRef, b: &PyObjectRef, op: PyComparisonOp) -> PyResult<bool> {
        match self._cmp(a, b, op)? {
            Either::A(obj) => pybool::boolval(self, obj),
            Either::B(b) => Ok(b),
        }
    }

    pub fn obj_cmp(&self, a: PyObjectRef, b: PyObjectRef, op: PyComparisonOp) -> PyResult {
        self._cmp(&a, &b, op).map(|res| res.into_pyobject(self))
    }

    pub fn _hash(&self, obj: &PyObjectRef) -> PyResult<rustpython_common::hash::PyHash> {
        let hash = obj
            .class()
            .mro_find_map(|cls| cls.slots.hash.load())
            .unwrap(); // hash always exist
        hash(&obj, self)
    }

    pub fn obj_len_opt(&self, obj: &PyObjectRef) -> Option<PyResult<usize>> {
        self.get_special_method(obj.clone(), "__len__")
            .map(Result::ok)
            .transpose()
            .map(|meth| {
                let len = meth?.invoke((), self)?;
                let len = len
                    .payload_if_subclass::<PyInt>(self)
                    .ok_or_else(|| {
                        self.new_type_error(format!(
                            "'{}' object cannot be interpreted as an integer",
                            len.class().name
                        ))
                    })?
                    .borrow_value();
                if len.is_negative() {
                    return Err(self.new_value_error("__len__() should return >= 0".to_owned()));
                }
                let len = len.to_isize().ok_or_else(|| {
                    self.new_overflow_error(
                        "cannot fit 'int' into an index-sized integer".to_owned(),
                    )
                })?;
                Ok(len as usize)
            })
    }

    pub fn obj_len(&self, obj: &PyObjectRef) -> PyResult<usize> {
        self.obj_len_opt(obj).unwrap_or_else(|| {
            Err(self.new_type_error(format!(
                "object of type '{}' has no len()",
                obj.class().name
            )))
        })
    }

    // https://docs.python.org/3/reference/expressions.html#membership-test-operations
    fn _membership_iter_search(&self, haystack: PyObjectRef, needle: PyObjectRef) -> PyResult {
        let iter = iterator::get_iter(self, haystack)?;
        loop {
            if let Some(element) = iterator::get_next_object(self, &iter)? {
                if self.bool_eq(&needle, &element)? {
                    return Ok(self.ctx.new_bool(true));
                } else {
                    continue;
                }
            } else {
                return Ok(self.ctx.new_bool(false));
            }
        }
    }

    pub fn _membership(&self, haystack: PyObjectRef, needle: PyObjectRef) -> PyResult {
        match PyMethod::get_special(haystack, "__contains__", self)? {
            Ok(method) => method.invoke((needle,), self),
            Err(haystack) => self._membership_iter_search(haystack, needle),
        }
    }

    pub(crate) fn push_exception(&self, exc: Option<PyBaseExceptionRef>) {
        let mut excs = self.exceptions.borrow_mut();
        let prev = std::mem::take(&mut *excs);
        excs.prev = Some(Box::new(prev));
        excs.exc = exc
    }

    pub(crate) fn pop_exception(&self) -> Option<PyBaseExceptionRef> {
        let mut excs = self.exceptions.borrow_mut();
        let cur = std::mem::take(&mut *excs);
        *excs = *cur.prev.expect("pop_exception() without nested exc stack");
        cur.exc
    }

    pub(crate) fn take_exception(&self) -> Option<PyBaseExceptionRef> {
        self.exceptions.borrow_mut().exc.take()
    }

    pub(crate) fn current_exception(&self) -> Option<PyBaseExceptionRef> {
        self.exceptions.borrow().exc.clone()
    }

    pub(crate) fn set_exception(&self, exc: Option<PyBaseExceptionRef>) {
        // don't be holding the refcell guard while __del__ is called
        let prev = std::mem::replace(&mut self.exceptions.borrow_mut().exc, exc);
        drop(prev);
    }

    pub(crate) fn contextualize_exception(&self, exception: &PyBaseExceptionRef) {
        if let Some(context_exc) = self.topmost_exception() {
            if !context_exc.is(exception) {
                let mut o = context_exc.clone();
                while let Some(context) = o.context() {
                    if context.is(&exception) {
                        o.set_context(None);
                        break;
                    }
                    o = context;
                }
                exception.set_context(Some(context_exc))
            }
        }
    }

    pub(crate) fn topmost_exception(&self) -> Option<PyBaseExceptionRef> {
        let excs = self.exceptions.borrow();
        let mut cur = &*excs;
        loop {
            if let Some(exc) = &cur.exc {
                return Some(exc.clone());
            }
            cur = cur.prev.as_deref()?;
        }
    }

    pub fn bool_eq(&self, a: &PyObjectRef, b: &PyObjectRef) -> PyResult<bool> {
        self.bool_cmp(a, b, PyComparisonOp::Eq)
    }

    pub fn identical_or_equal(&self, a: &PyObjectRef, b: &PyObjectRef) -> PyResult<bool> {
        if a.is(b) {
            Ok(true)
        } else {
            self.bool_eq(a, b)
        }
    }

    pub fn bool_seq_lt(&self, a: &PyObjectRef, b: &PyObjectRef) -> PyResult<Option<bool>> {
        let value = if self.bool_cmp(a, b, PyComparisonOp::Lt)? {
            Some(true)
        } else if !self.bool_eq(a, b)? {
            Some(false)
        } else {
            None
        };
        Ok(value)
    }

    pub fn bool_seq_gt(&self, a: &PyObjectRef, b: &PyObjectRef) -> PyResult<Option<bool>> {
        let value = if self.bool_cmp(a, b, PyComparisonOp::Gt)? {
            Some(true)
        } else if !self.bool_eq(a, b)? {
            Some(false)
        } else {
            None
        };
        Ok(value)
    }

    pub fn map_codeobj(&self, code: bytecode::CodeObject) -> code::CodeObject {
        code.map_bag(&code::PyObjBag(self))
    }

    pub fn intern_string<S: Internable>(&self, s: S) -> PyStrRef {
        let (s, ()) = self
            .ctx
            .string_cache
            .setdefault_entry(self, s, || ())
            .expect("string_cache lookup should never error");
        s.downcast()
            .expect("only strings should be in string_cache")
    }

    #[doc(hidden)]
    pub fn __module_set_attr(
        &self,
        module: &PyObjectRef,
        attr_name: impl TryIntoRef<PyStr>,
        attr_value: impl Into<PyObjectRef>,
    ) -> PyResult<()> {
        let val = attr_value.into();
        object::setattr(module, attr_name.try_into_ref(self)?, Some(val), self)
    }
}

mod sealed {
    use super::*;
    pub trait SealedInternable {}
    impl SealedInternable for String {}
    impl SealedInternable for &str {}
    impl SealedInternable for PyRefExact<PyStr> {}
}
/// A sealed marker trait for `DictKey` types that always become an exact instance of `str`
pub trait Internable: sealed::SealedInternable + crate::dictdatatype::DictKey {}
impl Internable for String {}
impl Internable for &str {}
impl Internable for PyRefExact<PyStr> {}

pub struct ReprGuard<'vm> {
    vm: &'vm VirtualMachine,
    id: usize,
}

/// A guard to protect repr methods from recursion into itself,
impl<'vm> ReprGuard<'vm> {
    /// Returns None if the guard against 'obj' is still held otherwise returns the guard. The guard
    /// which is released if dropped.
    pub fn enter(vm: &'vm VirtualMachine, obj: &PyObjectRef) -> Option<Self> {
        let mut guards = vm.repr_guards.borrow_mut();

        // Should this be a flag on the obj itself? putting it in a global variable for now until it
        // decided the form of the PyObject. https://github.com/RustPython/RustPython/issues/371
        let id = obj.get_id();
        if guards.contains(&id) {
            return None;
        }
        guards.insert(id);
        Some(ReprGuard { vm, id })
    }
}

impl<'vm> Drop for ReprGuard<'vm> {
    fn drop(&mut self) {
        self.vm.repr_guards.borrow_mut().remove(&self.id);
    }
}

pub struct Interpreter {
    vm: VirtualMachine,
}

impl Interpreter {
    pub fn new(settings: PySettings, init: InitParameter) -> Self {
        Self::new_with_init(settings, |_| init)
    }

    pub fn new_with_init<F>(settings: PySettings, init: F) -> Self
    where
        F: FnOnce(&mut VirtualMachine) -> InitParameter,
    {
        let mut vm = VirtualMachine::new(settings);
        let init = init(&mut vm);
        vm.initialize(init);
        Self { vm }
    }

    pub fn enter<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&VirtualMachine) -> R,
    {
        thread::enter_vm(&self.vm, || f(&self.vm))
    }

    // TODO: interpreter shutdown
    // pub fn run<F>(self, f: F)
    // where
    //     F: FnOnce(&VirtualMachine),
    // {
    //     self.enter(f);
    //     self.shutdown();
    // }

    // pub fn shutdown(self) {}
}

impl Default for Interpreter {
    fn default() -> Self {
        Self::new(PySettings::default(), InitParameter::External)
    }
}

#[must_use = "PyThread does nothing unless you move it to another thread and call .run()"]
#[cfg(feature = "threading")]
pub struct PyThread {
    thread_vm: VirtualMachine,
}

#[cfg(feature = "threading")]
impl PyThread {
    /// Create a `FnOnce()` that can easily be passed to a function like [`std::thread::Builder::spawn`]
    ///
    /// # Note
    ///
    /// If you return a `PyObjectRef` (or a type that contains one) from `F`, and don't `join()`
    /// on the thread this `FnOnce` runs in, there is a possibility that that thread will panic
    /// as `PyObjectRef`'s `Drop` implementation tries to run the `__del__` destructor of a
    /// Python object but finds that it's not in the context of any vm.
    pub fn make_spawn_func<F, R>(self, f: F) -> impl FnOnce() -> R
    where
        F: FnOnce(&VirtualMachine) -> R,
    {
        move || self.run(f)
    }

    /// Run a function in this thread context
    ///
    /// # Note
    ///
    /// If you return a `PyObjectRef` (or a type that contains one) from `F`, and don't return the object
    /// to the parent thread and then `join()` on the `JoinHandle` (or similar), there is a possibility that
    /// the current thread will panic as `PyObjectRef`'s `Drop` implementation tries to run the `__del__`
    /// destructor of a python object but finds that it's not in the context of any vm.
    pub fn run<F, R>(self, f: F) -> R
    where
        F: FnOnce(&VirtualMachine) -> R,
    {
        let vm = &self.thread_vm;
        thread::enter_vm(vm, || f(vm))
    }
}

#[cfg(test)]
mod tests {
    use super::Interpreter;
    use crate::builtins::{int, pystr};
    use num_bigint::ToBigInt;

    #[test]
    fn test_add_py_integers() {
        Interpreter::default().enter(|vm| {
            let a = vm.ctx.new_int(33_i32);
            let b = vm.ctx.new_int(12_i32);
            let res = vm._add(&a, &b).unwrap();
            let value = int::get_value(&res);
            assert_eq!(*value, 45_i32.to_bigint().unwrap());
        })
    }

    #[test]
    fn test_multiply_str() {
        Interpreter::default().enter(|vm| {
            let a = vm.ctx.new_str(String::from("Hello "));
            let b = vm.ctx.new_int(4_i32);
            let res = vm._mul(&a, &b).unwrap();
            let value = pystr::borrow_value(&res);
            assert_eq!(value, String::from("Hello Hello Hello Hello "))
        })
    }
}

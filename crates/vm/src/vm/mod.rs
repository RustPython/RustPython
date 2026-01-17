//! Implement virtual machine to run instructions.
//!
//! See also:
//!   <https://github.com/ProgVal/pythonvm-rust/blob/master/src/processor/mod.rs>

#[cfg(feature = "rustpython-compiler")]
mod compile;
mod context;
mod interpreter;
mod method;
#[cfg(feature = "rustpython-compiler")]
mod python_run;
mod setting;
pub mod thread;
mod vm_new;
mod vm_object;
mod vm_ops;

use crate::{
    AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult,
    builtins::{
        PyBaseExceptionRef, PyDict, PyDictRef, PyInt, PyList, PyModule, PyStr, PyStrInterned,
        PyStrRef, PyTypeRef,
        code::PyCode,
        dict::{PyDictItems, PyDictKeys, PyDictValues},
        pystr::AsPyStr,
        tuple::PyTuple,
    },
    codecs::CodecsRegistry,
    common::{hash::HashSecret, lock::PyMutex, rc::PyRc},
    convert::ToPyObject,
    exceptions::types::PyBaseException,
    frame::{ExecutionResult, Frame, FrameRef},
    frozen::FrozenModule,
    function::{ArgMapping, FuncArgs, PySetterValue},
    import,
    protocol::PyIterIter,
    scope::Scope,
    signal, stdlib,
    warn::WarningsState,
};
use alloc::borrow::Cow;
use core::{
    cell::{Cell, Ref, RefCell},
    sync::atomic::AtomicBool,
};
use crossbeam_utils::atomic::AtomicCell;
#[cfg(unix)]
use nix::{
    sys::signal::{SaFlags, SigAction, SigSet, Signal::SIGINT, kill, sigaction},
    unistd::getpid,
};
use std::{
    collections::{HashMap, HashSet},
    ffi::{OsStr, OsString},
};

pub use context::Context;
pub use interpreter::Interpreter;
pub(crate) use method::PyMethod;
pub use setting::{CheckHashPycsMode, Paths, PyConfig, Settings};

pub const MAX_MEMORY_SIZE: usize = isize::MAX as usize;

// Objects are live when they are on stack, or referenced by a name (for now)

/// Top level container of a python virtual machine. In theory you could
/// create more instances of this struct and have them operate fully isolated.
///
/// To construct this, please refer to the [`Interpreter`]
pub struct VirtualMachine {
    pub builtins: PyRef<PyModule>,
    pub sys_module: PyRef<PyModule>,
    pub ctx: PyRc<Context>,
    pub frames: RefCell<Vec<FrameRef>>,
    pub wasm_id: Option<String>,
    exceptions: RefCell<ExceptionStack>,
    pub import_func: PyObjectRef,
    pub profile_func: RefCell<PyObjectRef>,
    pub trace_func: RefCell<PyObjectRef>,
    pub use_tracing: Cell<bool>,
    pub recursion_limit: Cell<usize>,
    pub(crate) signal_handlers: Option<Box<RefCell<[Option<PyObjectRef>; signal::NSIG]>>>,
    pub(crate) signal_rx: Option<signal::UserSignalReceiver>,
    pub repr_guards: RefCell<HashSet<usize>>,
    pub state: PyRc<PyGlobalState>,
    pub initialized: bool,
    recursion_depth: Cell<usize>,
    /// Async generator firstiter hook (per-thread, set via sys.set_asyncgen_hooks)
    pub async_gen_firstiter: RefCell<Option<PyObjectRef>>,
    /// Async generator finalizer hook (per-thread, set via sys.set_asyncgen_hooks)
    pub async_gen_finalizer: RefCell<Option<PyObjectRef>>,
}

#[derive(Debug, Default)]
struct ExceptionStack {
    exc: Option<PyBaseExceptionRef>,
    prev: Option<Box<ExceptionStack>>,
}

pub struct PyGlobalState {
    pub config: PyConfig,
    pub module_inits: stdlib::StdlibMap,
    pub frozen: HashMap<&'static str, FrozenModule, ahash::RandomState>,
    pub stacksize: AtomicCell<usize>,
    pub thread_count: AtomicCell<usize>,
    pub hash_secret: HashSecret,
    pub atexit_funcs: PyMutex<Vec<(PyObjectRef, FuncArgs)>>,
    pub codec_registry: CodecsRegistry,
    pub finalizing: AtomicBool,
    pub warnings: WarningsState,
    pub override_frozen_modules: AtomicCell<isize>,
    pub before_forkers: PyMutex<Vec<PyObjectRef>>,
    pub after_forkers_child: PyMutex<Vec<PyObjectRef>>,
    pub after_forkers_parent: PyMutex<Vec<PyObjectRef>>,
    pub int_max_str_digits: AtomicCell<usize>,
    pub switch_interval: AtomicCell<f64>,
    /// Global trace function for all threads (set by sys._settraceallthreads)
    pub global_trace_func: PyMutex<Option<PyObjectRef>>,
    /// Global profile function for all threads (set by sys._setprofileallthreads)
    pub global_profile_func: PyMutex<Option<PyObjectRef>>,
    /// Main thread identifier (pthread_self on Unix)
    #[cfg(feature = "threading")]
    pub main_thread_ident: AtomicCell<u64>,
    /// Registry of all threads' current frames for sys._current_frames()
    #[cfg(feature = "threading")]
    pub thread_frames: parking_lot::Mutex<HashMap<u64, stdlib::thread::CurrentFrameSlot>>,
    /// Registry of all ThreadHandles for fork cleanup
    #[cfg(feature = "threading")]
    pub thread_handles: parking_lot::Mutex<Vec<stdlib::thread::HandleEntry>>,
    /// Registry for non-daemon threads that need to be joined at shutdown
    #[cfg(feature = "threading")]
    pub shutdown_handles: parking_lot::Mutex<Vec<stdlib::thread::ShutdownEntry>>,
}

pub fn process_hash_secret_seed() -> u32 {
    use std::sync::OnceLock;
    static SEED: OnceLock<u32> = OnceLock::new();
    // os_random is expensive, but this is only ever called once
    *SEED.get_or_init(|| u32::from_ne_bytes(rustpython_common::rand::os_random()))
}

impl VirtualMachine {
    /// Create a new `VirtualMachine` structure.
    fn new(config: PyConfig, ctx: PyRc<Context>) -> Self {
        flame_guard!("new VirtualMachine");

        // make a new module without access to the vm; doesn't
        // set __spec__, __loader__, etc. attributes
        let new_module = |def| {
            PyRef::new_ref(
                PyModule::from_def(def),
                ctx.types.module_type.to_owned(),
                Some(ctx.new_dict()),
            )
        };

        // Hard-core modules:
        let builtins = new_module(stdlib::builtins::__module_def(&ctx));
        let sys_module = new_module(stdlib::sys::__module_def(&ctx));

        let import_func = ctx.none();
        let profile_func = RefCell::new(ctx.none());
        let trace_func = RefCell::new(ctx.none());
        let signal_handlers = Some(Box::new(
            // putting it in a const optimizes better, prevents linear initialization of the array
            const { RefCell::new([const { None }; signal::NSIG]) },
        ));

        let module_inits = stdlib::get_module_inits();

        let seed = match config.settings.hash_seed {
            Some(seed) => seed,
            None => process_hash_secret_seed(),
        };
        let hash_secret = HashSecret::new(seed);

        let codec_registry = CodecsRegistry::new(&ctx);

        let warnings = WarningsState::init_state(&ctx);

        let int_max_str_digits = AtomicCell::new(match config.settings.int_max_str_digits {
            -1 => 4300,
            other => other,
        } as usize);
        let mut vm = Self {
            builtins,
            sys_module,
            ctx,
            frames: RefCell::new(vec![]),
            wasm_id: None,
            exceptions: RefCell::default(),
            import_func,
            profile_func,
            trace_func,
            use_tracing: Cell::new(false),
            recursion_limit: Cell::new(if cfg!(debug_assertions) { 256 } else { 1000 }),
            signal_handlers,
            signal_rx: None,
            repr_guards: RefCell::default(),
            state: PyRc::new(PyGlobalState {
                config,
                module_inits,
                frozen: HashMap::default(),
                stacksize: AtomicCell::new(0),
                thread_count: AtomicCell::new(0),
                hash_secret,
                atexit_funcs: PyMutex::default(),
                codec_registry,
                finalizing: AtomicBool::new(false),
                warnings,
                override_frozen_modules: AtomicCell::new(0),
                before_forkers: PyMutex::default(),
                after_forkers_child: PyMutex::default(),
                after_forkers_parent: PyMutex::default(),
                int_max_str_digits,
                switch_interval: AtomicCell::new(0.005),
                global_trace_func: PyMutex::default(),
                global_profile_func: PyMutex::default(),
                #[cfg(feature = "threading")]
                main_thread_ident: AtomicCell::new(0),
                #[cfg(feature = "threading")]
                thread_frames: parking_lot::Mutex::new(HashMap::new()),
                #[cfg(feature = "threading")]
                thread_handles: parking_lot::Mutex::new(Vec::new()),
                #[cfg(feature = "threading")]
                shutdown_handles: parking_lot::Mutex::new(Vec::new()),
            }),
            initialized: false,
            recursion_depth: Cell::new(0),
            async_gen_firstiter: RefCell::new(None),
            async_gen_finalizer: RefCell::new(None),
        };

        if vm.state.hash_secret.hash_str("")
            != vm
                .ctx
                .interned_str("")
                .expect("empty str must be interned")
                .hash(&vm)
        {
            panic!("Interpreters in same process must share the hash seed");
        }

        let frozen = core_frozen_inits().collect();
        PyRc::get_mut(&mut vm.state).unwrap().frozen = frozen;

        vm.builtins.init_dict(
            vm.ctx.intern_str("builtins"),
            Some(vm.ctx.intern_str(stdlib::builtins::DOC.unwrap()).to_owned()),
            &vm,
        );
        vm.sys_module.init_dict(
            vm.ctx.intern_str("sys"),
            Some(vm.ctx.intern_str(stdlib::sys::DOC.unwrap()).to_owned()),
            &vm,
        );
        // let name = vm.sys_module.get_attr("__name__", &vm).unwrap();
        vm
    }

    /// set up the encodings search function
    /// init_importlib must be called before this call
    #[cfg(feature = "encodings")]
    fn import_encodings(&mut self) -> PyResult<()> {
        self.import("encodings", 0).map_err(|import_err| {
            let rustpythonpath_env = std::env::var("RUSTPYTHONPATH").ok();
            let pythonpath_env = std::env::var("PYTHONPATH").ok();
            let env_set = rustpythonpath_env.as_ref().is_some() || pythonpath_env.as_ref().is_some();
            let path_contains_env = self.state.config.paths.module_search_paths.iter().any(|s| {
                Some(s.as_str()) == rustpythonpath_env.as_deref() || Some(s.as_str()) == pythonpath_env.as_deref()
            });

            let guide_message = if cfg!(feature = "freeze-stdlib") {
                "`rustpython_pylib` maybe not set while using `freeze-stdlib` feature. Try using `rustpython::InterpreterConfig::init_stdlib` or manually call `vm.add_frozen(rustpython_pylib::FROZEN_STDLIB)` in `rustpython_vm::Interpreter::with_init`."
            } else if !env_set {
                "Neither RUSTPYTHONPATH nor PYTHONPATH is set. Try setting one of them to the stdlib directory."
            } else if path_contains_env {
                "RUSTPYTHONPATH or PYTHONPATH is set, but it doesn't contain the encodings library. If you are customizing the RustPython vm/interpreter, try adding the stdlib directory to the path. If you are developing the RustPython interpreter, it might be a bug during development."
            } else {
                "RUSTPYTHONPATH or PYTHONPATH is set, but it wasn't loaded to `PyConfig::paths::module_search_paths`. If you are going to customize the RustPython vm/interpreter, those environment variables are not loaded in the Settings struct by default. Please try creating a customized instance of the Settings struct. If you are developing the RustPython interpreter, it might be a bug during development."
            };

            let mut msg = format!(
                "RustPython could not import the encodings module. It usually means something went wrong. Please carefully read the following messages and follow the steps.\n\
                \n\
                {guide_message}");
            if !cfg!(feature = "freeze-stdlib") {
                msg += "\n\
                If you don't have access to a consistent external environment (e.g. targeting wasm, embedding \
                    rustpython in another application), try enabling the `freeze-stdlib` feature.\n\
                If this is intended and you want to exclude the encodings module from your interpreter, please remove the `encodings` feature from `rustpython-vm` crate.";
            }

            let err = self.new_runtime_error(msg);
            err.set___cause__(Some(import_err));
            err
        })?;
        Ok(())
    }

    fn import_ascii_utf8_encodings(&mut self) -> PyResult<()> {
        import::import_frozen(self, "codecs")?;

        // Use dotted names when freeze-stdlib is enabled (modules come from Lib/encodings/),
        // otherwise use underscored names (modules come from core_modules/).
        let (ascii_module_name, utf8_module_name) = if cfg!(feature = "freeze-stdlib") {
            ("encodings.ascii", "encodings.utf_8")
        } else {
            ("encodings_ascii", "encodings_utf_8")
        };

        // Register ascii encoding
        let ascii_module = import::import_frozen(self, ascii_module_name)?;
        let getregentry = ascii_module.get_attr("getregentry", self)?;
        let codec_info = getregentry.call((), self)?;
        self.state
            .codec_registry
            .register_manual("ascii", codec_info.try_into_value(self)?)?;

        // Register utf-8 encoding
        let utf8_module = import::import_frozen(self, utf8_module_name)?;
        let getregentry = utf8_module.get_attr("getregentry", self)?;
        let codec_info = getregentry.call((), self)?;
        self.state
            .codec_registry
            .register_manual("utf-8", codec_info.try_into_value(self)?)?;
        Ok(())
    }

    fn initialize(&mut self) {
        flame_guard!("init VirtualMachine");

        if self.initialized {
            panic!("Double Initialize Error");
        }

        // Initialize main thread ident before any threading operations
        #[cfg(feature = "threading")]
        stdlib::thread::init_main_thread_ident(self);

        stdlib::builtins::init_module(self, &self.builtins);
        stdlib::sys::init_module(self, &self.sys_module, &self.builtins);

        let mut essential_init = || -> PyResult {
            import::import_builtin(self, "_typing")?;
            #[cfg(not(target_arch = "wasm32"))]
            import::import_builtin(self, "_signal")?;
            #[cfg(any(feature = "parser", feature = "compiler"))]
            import::import_builtin(self, "_ast")?;
            #[cfg(not(feature = "threading"))]
            import::import_frozen(self, "_thread")?;
            let importlib = import::init_importlib_base(self)?;
            self.import_ascii_utf8_encodings()?;

            #[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
            {
                let io = import::import_builtin(self, "_io")?;
                #[cfg(feature = "stdio")]
                let make_stdio = |name, fd, write| {
                    let buffered_stdio = self.state.config.settings.buffered_stdio;
                    let unbuffered = write && !buffered_stdio;
                    let buf = crate::stdlib::io::open(
                        self.ctx.new_int(fd).into(),
                        Some(if write { "wb" } else { "rb" }),
                        crate::stdlib::io::OpenArgs {
                            buffering: if unbuffered { 0 } else { -1 },
                            ..Default::default()
                        },
                        self,
                    )?;
                    let raw = if unbuffered {
                        buf.clone()
                    } else {
                        buf.get_attr("raw", self)?
                    };
                    raw.set_attr("name", self.ctx.new_str(format!("<{name}>")), self)?;
                    let isatty = self.call_method(&raw, "isatty", ())?.is_true(self)?;
                    let write_through = !buffered_stdio;
                    let line_buffering = buffered_stdio && (isatty || fd == 2);

                    let newline = if cfg!(windows) { None } else { Some("\n") };
                    let encoding = self.state.config.settings.stdio_encoding.as_deref();
                    // stderr always uses backslashreplace (ignores stdio_errors)
                    let errors = if fd == 2 {
                        Some("backslashreplace")
                    } else {
                        self.state.config.settings.stdio_errors.as_deref()
                    };

                    let stdio = self.call_method(
                        &io,
                        "TextIOWrapper",
                        (
                            buf,
                            encoding,
                            errors,
                            newline,
                            line_buffering,
                            write_through,
                        ),
                    )?;
                    let mode = if write { "w" } else { "r" };
                    stdio.set_attr("mode", self.ctx.new_str(mode), self)?;
                    Ok(stdio)
                };
                #[cfg(not(feature = "stdio"))]
                let make_stdio =
                    |_name, _fd, _write| Ok(crate::builtins::PyNone.into_pyobject(self));

                let set_stdio = |name, fd, write| {
                    let stdio = make_stdio(name, fd, write)?;
                    let dunder_name = self.ctx.intern_str(format!("__{name}__"));
                    self.sys_module.set_attr(
                        dunder_name, // e.g. __stdin__
                        stdio.clone(),
                        self,
                    )?;
                    self.sys_module.set_attr(name, stdio, self)?;
                    Ok(())
                };
                set_stdio("stdin", 0, false)?;
                set_stdio("stdout", 1, true)?;
                set_stdio("stderr", 2, true)?;

                let io_open = io.get_attr("open", self)?;
                self.builtins.set_attr("open", io_open, self)?;
            }

            Ok(importlib)
        };

        let res = essential_init();
        let importlib = self.expect_pyresult(res, "essential initialization failed");

        if self.state.config.settings.allow_external_library
            && cfg!(feature = "rustpython-compiler")
            && let Err(e) = import::init_importlib_package(self, importlib)
        {
            eprintln!(
                "importlib initialization failed. This is critical for many complicated packages."
            );
            self.print_exception(e);
        }

        let _expect_stdlib = cfg!(feature = "freeze-stdlib")
            || !self.state.config.paths.module_search_paths.is_empty();

        #[cfg(feature = "encodings")]
        if _expect_stdlib {
            if let Err(e) = self.import_encodings() {
                eprintln!(
                    "encodings initialization failed. Only utf-8 encoding will be supported."
                );
                self.print_exception(e);
            }
        } else {
            // Here may not be the best place to give general `path_list` advice,
            // but bare rustpython_vm::VirtualMachine users skipped proper settings must hit here while properly setup vm never enters here.
            eprintln!(
                "feature `encodings` is enabled but `paths.module_search_paths` is empty. \
                Please add the library path to `settings.path_list`. If you intended to disable the entire standard library (including the `encodings` feature), please also make sure to disable the `encodings` feature.\n\
                Tip: You may also want to add `\"\"` to `settings.path_list` in order to enable importing from the current working directory."
            );
        }

        self.initialized = true;
    }

    fn state_mut(&mut self) -> &mut PyGlobalState {
        PyRc::get_mut(&mut self.state)
            .expect("there should not be multiple threads while a user has a mut ref to a vm")
    }

    /// Can only be used in the initialization closure passed to [`Interpreter::with_init`]
    pub fn add_native_module<S>(&mut self, name: S, module: stdlib::StdlibInitFunc)
    where
        S: Into<Cow<'static, str>>,
    {
        self.state_mut().module_inits.insert(name.into(), module);
    }

    pub fn add_native_modules<I>(&mut self, iter: I)
    where
        I: IntoIterator<Item = (Cow<'static, str>, stdlib::StdlibInitFunc)>,
    {
        self.state_mut().module_inits.extend(iter);
    }

    /// Can only be used in the initialization closure passed to [`Interpreter::with_init`]
    pub fn add_frozen<I>(&mut self, frozen: I)
    where
        I: IntoIterator<Item = (&'static str, FrozenModule)>,
    {
        self.state_mut().frozen.extend(frozen);
    }

    /// Set the custom signal channel for the interpreter
    pub fn set_user_signal_channel(&mut self, signal_rx: signal::UserSignalReceiver) {
        self.signal_rx = Some(signal_rx);
    }

    /// Execute Python bytecode (`.pyc`) from an in-memory buffer.
    ///
    /// When the RustPython CLI is available, `.pyc` files are normally executed by
    /// invoking `rustpython <input>.pyc`. This method provides an alternative for
    /// environments where the binary is unavailable or file I/O is restricted
    /// (e.g. WASM).
    ///
    /// ## Preparing a `.pyc` file
    ///
    /// First, compile a Python source file into bytecode:
    ///
    /// ```sh
    /// # Generate a .pyc file
    /// $ rustpython -m py_compile <input>.py
    /// ```
    ///
    /// ## Running the bytecode
    ///
    /// Load the resulting `.pyc` file into memory and execute it using the VM:
    ///
    /// ```no_run
    /// use rustpython_vm::Interpreter;
    /// Interpreter::without_stdlib(Default::default()).enter(|vm| {
    ///     let bytes = std::fs::read("__pycache__/<input>.rustpython-314.pyc").unwrap();
    ///     let main_scope = vm.new_scope_with_main().unwrap();
    ///     vm.run_pyc_bytes(&bytes, main_scope);
    /// });
    /// ```
    pub fn run_pyc_bytes(&self, pyc_bytes: &[u8], scope: Scope) -> PyResult<()> {
        let code = PyCode::from_pyc(pyc_bytes, Some("<pyc_bytes>"), None, None, self)?;
        self.with_simple_run("<source>", |_module_dict| {
            self.run_code_obj(code, scope)?;
            Ok(())
        })
    }

    pub fn run_code_obj(&self, code: PyRef<PyCode>, scope: Scope) -> PyResult {
        use crate::builtins::PyFunction;

        // Create a function object for module code, similar to CPython's PyEval_EvalCode
        let func = PyFunction::new(code.clone(), scope.globals.clone(), self)?;
        let func_obj = func.into_ref(&self.ctx).into();

        let frame = Frame::new(code, scope, self.builtins.dict(), &[], Some(func_obj), self)
            .into_ref(&self.ctx);
        self.run_frame(frame)
    }

    #[cold]
    pub fn run_unraisable(&self, e: PyBaseExceptionRef, msg: Option<String>, object: PyObjectRef) {
        // During interpreter finalization, sys.unraisablehook may not be available,
        // but we still need to report exceptions (especially from atexit callbacks).
        // Write directly to stderr like PyErr_FormatUnraisable.
        if self
            .state
            .finalizing
            .load(std::sync::atomic::Ordering::Acquire)
        {
            self.write_unraisable_to_stderr(&e, msg.as_deref(), &object);
            return;
        }

        let sys_module = self.import("sys", 0).unwrap();
        let unraisablehook = sys_module.get_attr("unraisablehook", self).unwrap();

        let exc_type = e.class().to_owned();
        let exc_traceback = e.__traceback__().to_pyobject(self); // TODO: actual traceback
        let exc_value = e.into();
        let args = stdlib::sys::UnraisableHookArgsData {
            exc_type,
            exc_value,
            exc_traceback,
            err_msg: self.new_pyobj(msg),
            object,
        };
        if let Err(e) = unraisablehook.call((args,), self) {
            println!("{}", e.as_object().repr(self).unwrap());
        }
    }

    /// Write unraisable exception to stderr during finalization.
    /// Similar to _PyErr_WriteUnraisableDefaultHook in CPython.
    fn write_unraisable_to_stderr(
        &self,
        e: &PyBaseExceptionRef,
        msg: Option<&str>,
        object: &PyObjectRef,
    ) {
        // Get stderr once and reuse it
        let stderr = crate::stdlib::sys::get_stderr(self).ok();

        let write_to_stderr = |s: &str, stderr: &Option<PyObjectRef>, vm: &VirtualMachine| {
            if let Some(stderr) = stderr {
                let _ = vm.call_method(stderr, "write", (s.to_owned(),));
            } else {
                eprint!("{}", s);
            }
        };

        // Format: "Exception ignored {msg} {object_repr}\n"
        if let Some(msg) = msg {
            write_to_stderr(&format!("Exception ignored {}", msg), &stderr, self);
        } else {
            write_to_stderr("Exception ignored in: ", &stderr, self);
        }

        if let Ok(repr) = object.repr(self) {
            write_to_stderr(&format!("{}\n", repr.as_str()), &stderr, self);
        } else {
            write_to_stderr("<object repr failed>\n", &stderr, self);
        }

        // Write exception type and message
        let exc_type_name = e.class().name();
        if let Ok(exc_str) = e.as_object().str(self) {
            let exc_str = exc_str.as_str();
            if exc_str.is_empty() {
                write_to_stderr(&format!("{}\n", exc_type_name), &stderr, self);
            } else {
                write_to_stderr(&format!("{}: {}\n", exc_type_name, exc_str), &stderr, self);
            }
        } else {
            write_to_stderr(&format!("{}\n", exc_type_name), &stderr, self);
        }

        // Flush stderr to ensure output is visible
        if let Some(ref stderr) = stderr {
            let _ = self.call_method(stderr, "flush", ());
        }
    }

    #[inline(always)]
    pub fn run_frame(&self, frame: FrameRef) -> PyResult {
        match self.with_frame(frame, |f| f.run(self))? {
            ExecutionResult::Return(value) => Ok(value),
            _ => panic!("Got unexpected result from function"),
        }
    }

    /// Run `run` with main scope.
    fn with_simple_run(
        &self,
        path: &str,
        run: impl FnOnce(&Py<PyDict>) -> PyResult<()>,
    ) -> PyResult<()> {
        let sys_modules = self.sys_module.get_attr(identifier!(self, modules), self)?;
        let main_module = sys_modules.get_item(identifier!(self, __main__), self)?;
        let module_dict = main_module.dict().expect("main module must have __dict__");

        // Track whether we set __file__ (for cleanup)
        let set_file_name = !module_dict.contains_key(identifier!(self, __file__), self);
        if set_file_name {
            module_dict.set_item(
                identifier!(self, __file__),
                self.ctx.new_str(path).into(),
                self,
            )?;
            module_dict.set_item(identifier!(self, __cached__), self.ctx.none(), self)?;
        }

        let result = run(&module_dict);

        self.flush_io();

        // Cleanup __file__ and __cached__ after execution
        if set_file_name {
            let _ = module_dict.del_item(identifier!(self, __file__), self);
            let _ = module_dict.del_item(identifier!(self, __cached__), self);
        }

        result
    }

    /// flush_io
    ///
    /// Flush stdout and stderr. Errors are silently ignored.
    fn flush_io(&self) {
        if let Ok(stdout) = self.sys_module.get_attr("stdout", self) {
            let _ = self.call_method(&stdout, identifier!(self, flush).as_str(), ());
        }
        if let Ok(stderr) = self.sys_module.get_attr("stderr", self) {
            let _ = self.call_method(&stderr, identifier!(self, flush).as_str(), ());
        }
    }

    pub fn current_recursion_depth(&self) -> usize {
        self.recursion_depth.get()
    }

    /// Used to run the body of a (possibly) recursive function. It will raise a
    /// RecursionError if recursive functions are nested far too many times,
    /// preventing a stack overflow.
    pub fn with_recursion<R, F: FnOnce() -> PyResult<R>>(&self, _where: &str, f: F) -> PyResult<R> {
        self.check_recursive_call(_where)?;
        self.recursion_depth.set(self.recursion_depth.get() + 1);
        let result = f();
        self.recursion_depth.set(self.recursion_depth.get() - 1);
        result
    }

    pub fn with_frame<R, F: FnOnce(FrameRef) -> PyResult<R>>(
        &self,
        frame: FrameRef,
        f: F,
    ) -> PyResult<R> {
        self.with_recursion("", || {
            self.frames.borrow_mut().push(frame.clone());
            // Update the current frame slot for sys._current_frames()
            #[cfg(feature = "threading")]
            crate::vm::thread::update_current_frame(Some(frame.clone()));
            // Push a new exception context for frame isolation
            // Each frame starts with no active exception (None)
            // This prevents exceptions from leaking between function calls
            self.push_exception(None);
            let result = f(frame);
            // Pop the exception context - restores caller's exception state
            self.pop_exception();
            // defer dec frame
            let _popped = self.frames.borrow_mut().pop();
            // Update the frame slot to the new top frame (or None if empty)
            #[cfg(feature = "threading")]
            crate::vm::thread::update_current_frame(self.frames.borrow().last().cloned());
            result
        })
    }

    /// Returns a basic CompileOpts instance with options accurate to the vm. Used
    /// as the CompileOpts for `vm.compile()`.
    #[cfg(feature = "rustpython-codegen")]
    pub fn compile_opts(&self) -> crate::compiler::CompileOpts {
        crate::compiler::CompileOpts {
            optimize: self.state.config.settings.optimize,
            debug_ranges: self.state.config.settings.code_debug_ranges,
        }
    }

    // To be called right before raising the recursion depth.
    fn check_recursive_call(&self, _where: &str) -> PyResult<()> {
        if self.recursion_depth.get() >= self.recursion_limit.get() {
            Err(self.new_recursion_error(format!("maximum recursion depth exceeded {_where}")))
        } else {
            Ok(())
        }
    }

    pub fn current_frame(&self) -> Option<Ref<'_, FrameRef>> {
        let frames = self.frames.borrow();
        if frames.is_empty() {
            None
        } else {
            Some(Ref::map(self.frames.borrow(), |frames| {
                frames.last().unwrap()
            }))
        }
    }

    pub fn current_locals(&self) -> PyResult<ArgMapping> {
        self.current_frame()
            .expect("called current_locals but no frames on the stack")
            .locals(self)
    }

    pub fn current_globals(&self) -> Ref<'_, PyDictRef> {
        let frame = self
            .current_frame()
            .expect("called current_globals but no frames on the stack");
        Ref::map(frame, |f| &f.globals)
    }

    pub fn try_class(&self, module: &'static str, class: &'static str) -> PyResult<PyTypeRef> {
        let class = self
            .import(module, 0)?
            .get_attr(class, self)?
            .downcast()
            .expect("not a class");
        Ok(class)
    }

    pub fn class(&self, module: &'static str, class: &'static str) -> PyTypeRef {
        let module = self
            .import(module, 0)
            .unwrap_or_else(|_| panic!("unable to import {module}"));

        let class = module
            .get_attr(class, self)
            .unwrap_or_else(|_| panic!("module {module:?} has no class {class}"));
        class.downcast().expect("not a class")
    }

    /// Call Python __import__ function without from_list.
    /// Roughly equivalent to `import module_name` or `import top.submodule`.
    ///
    /// See also [`VirtualMachine::import_from`] for more advanced import.
    /// See also [`rustpython_vm::import::import_source`] and other primitive import functions.
    #[inline]
    pub fn import<'a>(&self, module_name: impl AsPyStr<'a>, level: usize) -> PyResult {
        let module_name = module_name.as_pystr(&self.ctx);
        let from_list = self.ctx.empty_tuple_typed();
        self.import_inner(module_name, from_list, level)
    }

    /// Call Python __import__ function caller with from_list.
    /// Roughly equivalent to `from module_name import item1, item2` or `from top.submodule import item1, item2`
    #[inline]
    pub fn import_from<'a>(
        &self,
        module_name: impl AsPyStr<'a>,
        from_list: &Py<PyTuple<PyStrRef>>,
        level: usize,
    ) -> PyResult {
        let module_name = module_name.as_pystr(&self.ctx);
        self.import_inner(module_name, from_list, level)
    }

    fn import_inner(
        &self,
        module: &Py<PyStr>,
        from_list: &Py<PyTuple<PyStrRef>>,
        level: usize,
    ) -> PyResult {
        // if the import inputs seem weird, e.g a package import or something, rather than just
        // a straight `import ident`
        let weird = module.as_str().contains('.') || level != 0 || !from_list.is_empty();

        let cached_module = if weird {
            None
        } else {
            let sys_modules = self.sys_module.get_attr("modules", self)?;
            sys_modules.get_item(module, self).ok()
        };

        match cached_module {
            Some(cached_module) => {
                if self.is_none(&cached_module) {
                    Err(self.new_import_error(
                        format!("import of {module} halted; None in sys.modules"),
                        module.to_owned(),
                    ))
                } else {
                    Ok(cached_module)
                }
            }
            None => {
                let import_func = self
                    .builtins
                    .get_attr(identifier!(self, __import__), self)
                    .map_err(|_| {
                        self.new_import_error("__import__ not found", module.to_owned())
                    })?;

                let (locals, globals) = if let Some(frame) = self.current_frame() {
                    (Some(frame.locals.clone()), Some(frame.globals.clone()))
                } else {
                    (None, None)
                };
                let from_list: PyObjectRef = from_list.to_owned().into();
                import_func
                    .call((module.to_owned(), globals, locals, from_list, level), self)
                    .inspect_err(|exc| import::remove_importlib_frames(self, exc))
            }
        }
    }

    pub fn extract_elements_with<T, F>(&self, value: &PyObject, func: F) -> PyResult<Vec<T>>
    where
        F: Fn(PyObjectRef) -> PyResult<T>,
    {
        // Extract elements from item, if possible:
        let cls = value.class();
        let list_borrow;
        let slice = if cls.is(self.ctx.types.tuple_type) {
            value.downcast_ref::<PyTuple>().unwrap().as_slice()
        } else if cls.is(self.ctx.types.list_type) {
            list_borrow = value.downcast_ref::<PyList>().unwrap().borrow_vec();
            &list_borrow
        } else if cls.is(self.ctx.types.dict_keys_type) {
            // Atomic snapshot of dict keys - prevents race condition during iteration
            let keys = value.downcast_ref::<PyDictKeys>().unwrap().dict.keys_vec();
            return keys.into_iter().map(func).collect();
        } else if cls.is(self.ctx.types.dict_values_type) {
            // Atomic snapshot of dict values - prevents race condition during iteration
            let values = value
                .downcast_ref::<PyDictValues>()
                .unwrap()
                .dict
                .values_vec();
            return values.into_iter().map(func).collect();
        } else if cls.is(self.ctx.types.dict_items_type) {
            // Atomic snapshot of dict items - prevents race condition during iteration
            let items = value
                .downcast_ref::<PyDictItems>()
                .unwrap()
                .dict
                .items_vec();
            return items
                .into_iter()
                .map(|(k, v)| func(self.ctx.new_tuple(vec![k, v]).into()))
                .collect();
        } else {
            return self.map_py_iter(value, func);
        };
        slice.iter().map(|obj| func(obj.clone())).collect()
    }

    pub fn map_iterable_object<F, R>(&self, obj: &PyObject, mut f: F) -> PyResult<PyResult<Vec<R>>>
    where
        F: FnMut(PyObjectRef) -> PyResult<R>,
    {
        match_class!(match obj {
            ref l @ PyList => {
                let mut i: usize = 0;
                let mut results = Vec::with_capacity(l.borrow_vec().len());
                loop {
                    let elem = {
                        let elements = &*l.borrow_vec();
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
            ref t @ PyTuple => Ok(t.iter().cloned().map(f).collect()),
            // TODO: put internal iterable type
            obj => {
                Ok(self.map_py_iter(obj, f))
            }
        })
    }

    fn map_py_iter<F, R>(&self, value: &PyObject, mut f: F) -> PyResult<Vec<R>>
    where
        F: FnMut(PyObjectRef) -> PyResult<R>,
    {
        let iter = value.to_owned().get_iter(self)?;
        let cap = match self.length_hint_opt(value.to_owned()) {
            Err(e) if e.class().is(self.ctx.exceptions.runtime_error) => return Err(e),
            Ok(Some(value)) => Some(value),
            // Use a power of 2 as a default capacity.
            _ => None,
        };
        // TODO: fix extend to do this check (?), see test_extend in Lib/test/list_tests.py,
        // https://github.com/python/cpython/blob/v3.9.0/Objects/listobject.c#L922-L928
        if let Some(cap) = cap
            && cap >= isize::MAX as usize
        {
            return Ok(Vec::new());
        }

        let mut results = PyIterIter::new(self, iter.as_ref(), cap)
            .map(|element| f(element?))
            .collect::<PyResult<Vec<_>>>()?;
        results.shrink_to_fit();
        Ok(results)
    }

    pub fn get_attribute_opt<'a>(
        &self,
        obj: PyObjectRef,
        attr_name: impl AsPyStr<'a>,
    ) -> PyResult<Option<PyObjectRef>> {
        let attr_name = attr_name.as_pystr(&self.ctx);
        match obj.get_attr_inner(attr_name, self) {
            Ok(attr) => Ok(Some(attr)),
            Err(e) if e.fast_isinstance(self.ctx.exceptions.attribute_error) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn set_attribute_error_context(
        &self,
        exc: &Py<PyBaseException>,
        obj: PyObjectRef,
        name: PyStrRef,
    ) {
        if exc.class().is(self.ctx.exceptions.attribute_error) {
            let exc = exc.as_object();
            exc.set_attr("name", name, self).unwrap();
            exc.set_attr("obj", obj, self).unwrap();
        }
    }

    // get_method should be used for internal access to magic methods (by-passing
    // the full getattribute look-up.
    pub fn get_method_or_type_error<F>(
        &self,
        obj: PyObjectRef,
        method_name: &'static PyStrInterned,
        err_msg: F,
    ) -> PyResult
    where
        F: FnOnce() -> String,
    {
        let method = obj
            .class()
            .get_attr(method_name)
            .ok_or_else(|| self.new_type_error(err_msg()))?;
        self.call_if_get_descriptor(&method, obj)
    }

    // TODO: remove + transfer over to get_special_method
    pub(crate) fn get_method(
        &self,
        obj: PyObjectRef,
        method_name: &'static PyStrInterned,
    ) -> Option<PyResult> {
        let method = obj.get_class_attr(method_name)?;
        Some(self.call_if_get_descriptor(&method, obj))
    }

    pub(crate) fn get_str_method(&self, obj: PyObjectRef, method_name: &str) -> Option<PyResult> {
        let method_name = self.ctx.interned_str(method_name)?;
        self.get_method(obj, method_name)
    }

    #[inline]
    /// Checks for triggered signals and calls the appropriate handlers. A no-op on
    /// platforms where signals are not supported.
    pub fn check_signals(&self) -> PyResult<()> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            crate::signal::check_signals(self)
        }
        #[cfg(target_arch = "wasm32")]
        {
            Ok(())
        }
    }

    pub(crate) fn push_exception(&self, exc: Option<PyBaseExceptionRef>) {
        let mut excs = self.exceptions.borrow_mut();
        let prev = core::mem::take(&mut *excs);
        excs.prev = Some(Box::new(prev));
        excs.exc = exc
    }

    pub(crate) fn pop_exception(&self) -> Option<PyBaseExceptionRef> {
        let mut excs = self.exceptions.borrow_mut();
        let cur = core::mem::take(&mut *excs);
        *excs = *cur.prev.expect("pop_exception() without nested exc stack");
        cur.exc
    }

    pub(crate) fn current_exception(&self) -> Option<PyBaseExceptionRef> {
        self.exceptions.borrow().exc.clone()
    }

    pub(crate) fn set_exception(&self, exc: Option<PyBaseExceptionRef>) {
        // don't be holding the RefCell guard while __del__ is called
        let prev = core::mem::replace(&mut self.exceptions.borrow_mut().exc, exc);
        drop(prev);
    }

    pub(crate) fn contextualize_exception(&self, exception: &Py<PyBaseException>) {
        if let Some(context_exc) = self.topmost_exception()
            && !context_exc.is(exception)
        {
            // Traverse the context chain to find `exception` and break cycles
            // Uses Floyd's cycle detection: o moves every step, slow_o every other step
            let mut o = context_exc.clone();
            let mut slow_o = context_exc.clone();
            let mut slow_update_toggle = false;
            while let Some(context) = o.__context__() {
                if context.is(exception) {
                    o.set___context__(None);
                    break;
                }
                o = context;
                if o.is(&slow_o) {
                    // Pre-existing cycle detected - all exceptions on the path were visited
                    break;
                }
                if slow_update_toggle && let Some(slow_context) = slow_o.__context__() {
                    slow_o = slow_context;
                }
                slow_update_toggle = !slow_update_toggle;
            }
            exception.set___context__(Some(context_exc))
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

    pub fn handle_exit_exception(&self, exc: PyBaseExceptionRef) -> u32 {
        if exc.fast_isinstance(self.ctx.exceptions.system_exit) {
            let args = exc.args();
            let msg = match args.as_slice() {
                [] => return 0,
                [arg] => match_class!(match arg {
                    ref i @ PyInt => {
                        use num_traits::cast::ToPrimitive;
                        // Try u32 first, then i32 (for negative values), else -1 for overflow
                        let code = i
                            .as_bigint()
                            .to_u32()
                            .or_else(|| i.as_bigint().to_i32().map(|v| v as u32))
                            .unwrap_or(-1i32 as u32);
                        return code;
                    }
                    arg => {
                        if self.is_none(arg) {
                            return 0;
                        } else {
                            arg.str(self).ok()
                        }
                    }
                }),
                _ => args.as_object().repr(self).ok(),
            };
            if let Some(msg) = msg {
                // Write using Python's write() to use stderr's error handler (backslashreplace)
                if let Ok(stderr) = stdlib::sys::get_stderr(self) {
                    let _ = self.call_method(&stderr, "write", (msg,));
                    let _ = self.call_method(&stderr, "write", ("\n",));
                }
            }
            1
        } else if exc.fast_isinstance(self.ctx.exceptions.keyboard_interrupt) {
            #[allow(clippy::if_same_then_else)]
            {
                self.print_exception(exc);
                #[cfg(unix)]
                {
                    let action = SigAction::new(
                        nix::sys::signal::SigHandler::SigDfl,
                        SaFlags::SA_ONSTACK,
                        SigSet::empty(),
                    );
                    let result = unsafe { sigaction(SIGINT, &action) };
                    if result.is_ok() {
                        self.flush_std();
                        kill(getpid(), SIGINT).expect("Expect to be killed.");
                    }

                    (libc::SIGINT as u32) + 128
                }
                #[cfg(windows)]
                {
                    // STATUS_CONTROL_C_EXIT - same as CPython
                    0xC000013A
                }
                #[cfg(not(any(unix, windows)))]
                {
                    1
                }
            }
        } else {
            self.print_exception(exc);
            1
        }
    }

    #[doc(hidden)]
    pub fn __module_set_attr(
        &self,
        module: &Py<PyModule>,
        attr_name: &'static PyStrInterned,
        attr_value: impl Into<PyObjectRef>,
    ) -> PyResult<()> {
        let val = attr_value.into();
        module
            .as_object()
            .generic_setattr(attr_name, PySetterValue::Assign(val), self)
    }

    pub fn insert_sys_path(&self, obj: PyObjectRef) -> PyResult<()> {
        let sys_path = self.sys_module.get_attr("path", self).unwrap();
        self.call_method(&sys_path, "insert", (0, obj))?;
        Ok(())
    }

    pub fn run_module(&self, module: &str) -> PyResult<()> {
        let runpy = self.import("runpy", 0)?;
        let run_module_as_main = runpy.get_attr("_run_module_as_main", self)?;
        run_module_as_main.call((module,), self)?;
        Ok(())
    }

    pub fn fs_encoding(&self) -> &'static PyStrInterned {
        identifier!(self, utf_8)
    }

    pub fn fs_encode_errors(&self) -> &'static PyStrInterned {
        if cfg!(windows) {
            identifier!(self, surrogatepass)
        } else {
            identifier!(self, surrogateescape)
        }
    }

    pub fn fsdecode(&self, s: impl Into<OsString>) -> PyStrRef {
        match s.into().into_string() {
            Ok(s) => self.ctx.new_str(s),
            Err(s) => {
                let bytes = self.ctx.new_bytes(s.into_encoded_bytes());
                let errors = self.fs_encode_errors().to_owned();
                let res = self.state.codec_registry.decode_text(
                    bytes.into(),
                    "utf-8",
                    Some(errors),
                    self,
                );
                self.expect_pyresult(res, "fsdecode should be lossless and never fail")
            }
        }
    }

    pub fn fsencode<'a>(&self, s: &'a Py<PyStr>) -> PyResult<Cow<'a, OsStr>> {
        if cfg!(windows) || s.is_utf8() {
            // XXX: this is sketchy on windows; it's not guaranteed that the
            //      OsStr encoding will always be compatible with WTF-8.
            let s = unsafe { OsStr::from_encoded_bytes_unchecked(s.as_bytes()) };
            return Ok(Cow::Borrowed(s));
        }
        let errors = self.fs_encode_errors().to_owned();
        let bytes = self
            .state
            .codec_registry
            .encode_text(s.to_owned(), "utf-8", Some(errors), self)?
            .to_vec();
        // XXX: this is sketchy on windows; it's not guaranteed that the
        //      OsStr encoding will always be compatible with WTF-8.
        let s = unsafe { OsString::from_encoded_bytes_unchecked(bytes) };
        Ok(Cow::Owned(s))
    }
}

impl AsRef<Context> for VirtualMachine {
    fn as_ref(&self) -> &Context {
        &self.ctx
    }
}

/// Resolve frozen module alias to its original name.
/// Returns the original module name if an alias exists, otherwise returns the input name.
pub fn resolve_frozen_alias(name: &str) -> &str {
    match name {
        "_frozen_importlib" => "importlib._bootstrap",
        "_frozen_importlib_external" => "importlib._bootstrap_external",
        "encodings_ascii" => "encodings.ascii",
        "encodings_utf_8" => "encodings.utf_8",
        _ => name,
    }
}

fn core_frozen_inits() -> impl Iterator<Item = (&'static str, FrozenModule)> {
    let iter = core::iter::empty();
    macro_rules! ext_modules {
        ($iter:ident, $($t:tt)*) => {
            let $iter = $iter.chain(py_freeze!($($t)*));
        };
    }

    // keep as example but use file one now
    // ext_modules!(
    //     iter,
    //     source = "initialized = True; print(\"Hello world!\")\n",
    //     module_name = "__hello__",
    // );

    // Python modules that the vm calls into, but are not actually part of the stdlib. They could
    // in theory be implemented in Rust, but are easiest to do in Python for one reason or another.
    // Includes _importlib_bootstrap and _importlib_bootstrap_external
    ext_modules!(
        iter,
        dir = "./Lib/python_builtins",
        crate_name = "rustpython_compiler_core"
    );

    // core stdlib Python modules that the vm calls into, but are still used in Python
    // application code, e.g. copyreg
    // FIXME: Initializing core_modules here results duplicated frozen module generation for core_modules.
    // We need a way to initialize this modules for both `Interpreter::without_stdlib()` and `InterpreterConfig::new().init_stdlib().interpreter()`
    // #[cfg(not(feature = "freeze-stdlib"))]
    ext_modules!(
        iter,
        dir = "./Lib/core_modules",
        crate_name = "rustpython_compiler_core"
    );

    iter
}

#[test]
fn test_nested_frozen() {
    use rustpython_vm as vm;

    vm::Interpreter::with_init(Default::default(), |vm| {
        // vm.add_native_modules(rustpython_stdlib::get_module_inits());
        vm.add_frozen(rustpython_vm::py_freeze!(
            dir = "../../extra_tests/snippets"
        ));
    })
    .enter(|vm| {
        let scope = vm.new_scope_with_builtins();

        let source = "from dir_module.dir_module_inner import value2";
        let code_obj = vm
            .compile(source, vm::compiler::Mode::Exec, "<embedded>".to_owned())
            .map_err(|err| vm.new_syntax_error(&err, Some(source)))
            .unwrap();

        if let Err(e) = vm.run_code_obj(code_obj, scope) {
            vm.print_exception(e);
            panic!();
        }
    })
}

#[test]
fn frozen_origname_matches() {
    use rustpython_vm as vm;

    vm::Interpreter::with_init(Default::default(), |_vm| {}).enter(|vm| {
        let check = |name, expected| {
            let module = import::import_frozen(vm, name).unwrap();
            let origname: PyStrRef = module
                .get_attr("__origname__", vm)
                .unwrap()
                .try_into_value(vm)
                .unwrap();
            assert_eq!(origname.as_str(), expected);
        };

        check("_frozen_importlib", "importlib._bootstrap");
        check(
            "_frozen_importlib_external",
            "importlib._bootstrap_external",
        );
    });
}

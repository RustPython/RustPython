use super::{Context, PyConfig, PyGlobalState, VirtualMachine, setting::Settings, thread};
use crate::{
    PyResult, builtins, common::rc::PyRc, frozen::FrozenModule, getpath, py_freeze, stdlib::atexit,
    vm::PyBaseExceptionRef,
};
use alloc::collections::BTreeMap;
use core::sync::atomic::Ordering;

type InitFunc = Box<dyn FnOnce(&mut VirtualMachine)>;

/// Configuration builder for constructing an Interpreter.
///
/// This is the preferred way to configure and create an interpreter with custom modules.
/// Modules must be registered before the interpreter is built,
/// similar to CPython's `PyImport_AppendInittab` which must be called before `Py_Initialize`.
///
/// # Example
/// ```
/// use rustpython_vm::Interpreter;
///
/// let builder = Interpreter::builder(Default::default());
/// // In practice, add stdlib: builder.add_native_modules(&stdlib_module_defs(&builder.ctx))
/// let interp = builder.build();
/// ```
pub struct InterpreterBuilder {
    settings: Settings,
    pub ctx: PyRc<Context>,
    module_defs: Vec<&'static builtins::PyModuleDef>,
    frozen_modules: Vec<(&'static str, FrozenModule)>,
    init_hooks: Vec<InitFunc>,
}

/// Private helper to initialize a VM with settings, context, and custom initialization.
fn initialize_main_vm<F>(
    settings: Settings,
    ctx: PyRc<Context>,
    module_defs: Vec<&'static builtins::PyModuleDef>,
    frozen_modules: Vec<(&'static str, FrozenModule)>,
    init_hooks: Vec<InitFunc>,
    init: F,
) -> (VirtualMachine, PyRc<PyGlobalState>)
where
    F: FnOnce(&mut VirtualMachine),
{
    use crate::codecs::CodecsRegistry;
    use crate::common::hash::HashSecret;
    use crate::common::lock::PyMutex;
    use crate::warn::WarningsState;
    use core::sync::atomic::AtomicBool;
    use crossbeam_utils::atomic::AtomicCell;

    let paths = getpath::init_path_config(&settings);
    let config = PyConfig::new(settings, paths);

    crate::types::TypeZoo::extend(&ctx);
    crate::exceptions::ExceptionZoo::extend(&ctx);

    // Build module_defs map from builtin modules + additional modules
    let mut all_module_defs: BTreeMap<&'static str, &'static builtins::PyModuleDef> =
        crate::stdlib::builtin_module_defs(&ctx)
            .into_iter()
            .chain(module_defs)
            .map(|def| (def.name.as_str(), def))
            .collect();

    // Register sysconfigdata under platform-specific name as well
    if let Some(&sysconfigdata_def) = all_module_defs.get("_sysconfigdata") {
        use std::sync::OnceLock;
        static SYSCONFIGDATA_NAME: OnceLock<&'static str> = OnceLock::new();
        let leaked_name = *SYSCONFIGDATA_NAME.get_or_init(|| {
            let name = crate::stdlib::sys::sysconfigdata_name();
            Box::leak(name.into_boxed_str())
        });
        all_module_defs.insert(leaked_name, sysconfigdata_def);
    }

    // Create hash secret
    let seed = match config.settings.hash_seed {
        Some(seed) => seed,
        None => super::process_hash_secret_seed(),
    };
    let hash_secret = HashSecret::new(seed);

    // Create codec registry and warnings state
    let codec_registry = CodecsRegistry::new(&ctx);
    let warnings = WarningsState::init_state(&ctx);

    // Create int_max_str_digits
    let int_max_str_digits = AtomicCell::new(match config.settings.int_max_str_digits {
        -1 => 4300,
        other => other,
    } as usize);

    // Initialize frozen modules (core + user-provided)
    let mut frozen: std::collections::HashMap<&'static str, FrozenModule, ahash::RandomState> =
        core_frozen_inits().collect();
    frozen.extend(frozen_modules);

    // Create PyGlobalState
    let global_state = PyRc::new(PyGlobalState {
        config,
        module_defs: all_module_defs,
        frozen,
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
        thread_frames: parking_lot::Mutex::new(std::collections::HashMap::new()),
        #[cfg(feature = "threading")]
        thread_handles: parking_lot::Mutex::new(Vec::new()),
        #[cfg(feature = "threading")]
        shutdown_handles: parking_lot::Mutex::new(Vec::new()),
    });

    // Create VM with the global state
    // Note: Don't clone here - init_hooks need exclusive access to mutate state
    let mut vm = VirtualMachine::new(ctx, global_state);

    // Execute initialization hooks (can mutate vm.state)
    for hook in init_hooks {
        hook(&mut vm);
    }

    // Call custom init function (can mutate vm.state)
    init(&mut vm);

    vm.initialize();

    // Clone global_state for Interpreter after all initialization is done
    let global_state = vm.state.clone();
    (vm, global_state)
}

impl InterpreterBuilder {
    /// Create a new interpreter configuration with default settings.
    pub fn new() -> Self {
        Self {
            settings: Settings::default(),
            ctx: Context::genesis().clone(),
            module_defs: Vec::new(),
            frozen_modules: Vec::new(),
            init_hooks: Vec::new(),
        }
    }

    /// Set custom settings for the interpreter.
    ///
    /// If called multiple times, only the last settings will be used.
    pub fn settings(mut self, settings: Settings) -> Self {
        self.settings = settings;
        self
    }

    /// Add a single native module definition.
    ///
    /// # Example
    /// ```
    /// use rustpython_vm::{Interpreter, builtins::PyModuleDef};
    ///
    /// let builder = Interpreter::builder(Default::default());
    /// // Note: In practice, use module_def from your #[pymodule]
    /// // let def = mymodule::module_def(&builder.ctx);
    /// // let interp = builder.add_native_module(def).build();
    /// let interp = builder.build();
    /// ```
    pub fn add_native_module(self, def: &'static builtins::PyModuleDef) -> Self {
        self.add_native_modules(&[def])
    }

    /// Add multiple native module definitions.
    ///
    /// # Example
    /// ```
    /// use rustpython_vm::Interpreter;
    ///
    /// let builder = Interpreter::builder(Default::default());
    /// // In practice, use module_defs from rustpython_stdlib:
    /// // let defs = rustpython_stdlib::stdlib_module_defs(&builder.ctx);
    /// // let interp = builder.add_native_modules(&defs).build();
    /// let interp = builder.build();
    /// ```
    pub fn add_native_modules(mut self, defs: &[&'static builtins::PyModuleDef]) -> Self {
        self.module_defs.extend_from_slice(defs);
        self
    }

    /// Add a custom initialization hook.
    ///
    /// Hooks are executed in the order they are added during interpreter creation.
    /// This function will be called after modules are registered but before
    /// the VM is initialized, allowing for additional customization.
    ///
    /// # Example
    /// ```
    /// use rustpython_vm::Interpreter;
    ///
    /// let interp = Interpreter::builder(Default::default())
    ///     .init_hook(|vm| {
    ///         // Custom initialization
    ///     })
    ///     .build();
    /// ```
    pub fn init_hook<F>(mut self, init: F) -> Self
    where
        F: FnOnce(&mut VirtualMachine) + 'static,
    {
        self.init_hooks.push(Box::new(init));
        self
    }

    /// Add frozen modules to the interpreter.
    ///
    /// Frozen modules are Python modules compiled into the binary.
    /// This method accepts any iterator of (name, FrozenModule) pairs.
    ///
    /// # Example
    /// ```
    /// use rustpython_vm::Interpreter;
    ///
    /// let interp = Interpreter::builder(Default::default())
    ///     // In practice: .add_frozen_modules(rustpython_pylib::FROZEN_STDLIB)
    ///     .build();
    /// ```
    pub fn add_frozen_modules<I>(mut self, frozen: I) -> Self
    where
        I: IntoIterator<Item = (&'static str, FrozenModule)>,
    {
        self.frozen_modules.extend(frozen);
        self
    }

    /// Build the interpreter.
    ///
    /// This consumes the configuration and returns a fully initialized Interpreter.
    pub fn build(self) -> Interpreter {
        let (vm, global_state) = initialize_main_vm(
            self.settings,
            self.ctx,
            self.module_defs,
            self.frozen_modules,
            self.init_hooks,
            |_| {}, // No additional init needed
        );
        Interpreter { global_state, vm }
    }

    /// Alias for `build()` for compatibility with the `interpreter()` pattern.
    pub fn interpreter(self) -> Interpreter {
        self.build()
    }
}

impl Default for InterpreterBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// The general interface for the VM
///
/// # Examples
/// Runs a simple embedded hello world program.
/// ```
/// use rustpython_vm::Interpreter;
/// use rustpython_vm::compiler::Mode;
/// Interpreter::without_stdlib(Default::default()).enter(|vm| {
///     let scope = vm.new_scope_with_builtins();
///     let source = r#"print("Hello World!")"#;
///     let code_obj = vm.compile(
///             source,
///             Mode::Exec,
///             "<embedded>".to_owned(),
///     ).map_err(|err| vm.new_syntax_error(&err, Some(source))).unwrap();
///     vm.run_code_obj(code_obj, scope).unwrap();
/// });
/// ```
pub struct Interpreter {
    pub global_state: PyRc<PyGlobalState>,
    vm: VirtualMachine,
}

impl Interpreter {
    /// Create a new interpreter configuration builder.
    ///
    /// # Example
    /// ```
    /// use rustpython_vm::Interpreter;
    ///
    /// let builder = Interpreter::builder(Default::default());
    /// // In practice, add stdlib: builder.add_native_modules(&stdlib_module_defs(&builder.ctx))
    /// let interp = builder.build();
    /// ```
    pub fn builder(settings: Settings) -> InterpreterBuilder {
        InterpreterBuilder::new().settings(settings)
    }

    /// This is a bare unit to build up an interpreter without the standard library.
    /// To create an interpreter with the standard library with the `rustpython` crate, use `rustpython::InterpreterBuilder`.
    /// To create an interpreter without the `rustpython` crate, but only with `rustpython-vm`,
    /// try to build one from the source code of `InterpreterBuilder`. It will not be a one-liner but it also will not be too hard.
    pub fn without_stdlib(settings: Settings) -> Self {
        Self::with_init(settings, |_| {})
    }

    /// Create with initialize function taking mutable vm reference.
    ///
    /// Note: This is a legacy API. To add stdlib, use `Interpreter::builder()` instead.
    pub fn with_init<F>(settings: Settings, init: F) -> Self
    where
        F: FnOnce(&mut VirtualMachine),
    {
        let (vm, global_state) = initialize_main_vm(
            settings,
            Context::genesis().clone(),
            Vec::new(), // No module_defs
            Vec::new(), // No frozen_modules
            Vec::new(), // No init_hooks
            init,
        );
        Self { global_state, vm }
    }

    /// Run a function with the main virtual machine and return a PyResult of the result.
    ///
    /// To enter vm context multiple times or to avoid buffer/exception management, this function is preferred.
    /// `enter` is lightweight and it returns a python object in PyResult.
    /// You can stop or continue the execution multiple times by calling `enter`.
    ///
    /// To finalize the vm once all desired `enter`s are called, calling `finalize` will be helpful.
    ///
    /// See also [`Interpreter::run`] for managed way to run the interpreter.
    pub fn enter<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&VirtualMachine) -> R,
    {
        thread::enter_vm(&self.vm, || f(&self.vm))
    }

    /// Run [`Interpreter::enter`] and call [`VirtualMachine::expect_pyresult`] for the result.
    ///
    /// This function is useful when you want to expect a result from the function,
    /// but also print useful panic information when exception raised.
    ///
    /// See also [`Interpreter::enter`] and [`VirtualMachine::expect_pyresult`] for more information.
    pub fn enter_and_expect<F, R>(&self, f: F, msg: &str) -> R
    where
        F: FnOnce(&VirtualMachine) -> PyResult<R>,
    {
        self.enter(|vm| {
            let result = f(vm);
            vm.expect_pyresult(result, msg)
        })
    }

    /// Run a function with the main virtual machine and return exit code.
    ///
    /// To enter vm context only once and safely terminate the vm, this function is preferred.
    /// Unlike [`Interpreter::enter`], `run` calls finalize and returns exit code.
    /// You will not be able to obtain Python exception in this way.
    ///
    /// See [`Interpreter::finalize`] for the finalization steps.
    /// See also [`Interpreter::enter`] for pure function call to obtain Python exception.
    pub fn run<F>(self, f: F) -> u32
    where
        F: FnOnce(&VirtualMachine) -> PyResult<()>,
    {
        let res = self.enter(|vm| f(vm));
        self.finalize(res.err())
    }

    /// Finalize vm and turns an exception to exit code.
    ///
    /// Finalization steps (matching Py_FinalizeEx):
    /// 1. Flush stdout and stderr.
    /// 1. Handle exit exception and turn it to exit code.
    /// 1. Wait for thread shutdown (call threading._shutdown).
    /// 1. Mark vm as finalizing.
    /// 1. Run atexit exit functions.
    /// 1. Finalize modules (clear module dicts in reverse import order).
    /// 1. Mark vm as finalized.
    ///
    /// Note that calling `finalize` is not necessary by purpose though.
    pub fn finalize(self, exc: Option<PyBaseExceptionRef>) -> u32 {
        self.enter(|vm| {
            vm.flush_std();

            // See if any exception leaked out:
            let exit_code = if let Some(exc) = exc {
                vm.handle_exit_exception(exc)
            } else {
                0
            };

            // Wait for thread shutdown - call threading._shutdown() if available.
            // This waits for all non-daemon threads to complete.
            // threading module may not be imported, so ignore import errors.
            if let Ok(threading) = vm.import("threading", 0)
                && let Ok(shutdown) = threading.get_attr("_shutdown", vm)
                && let Err(e) = shutdown.call((), vm)
            {
                vm.run_unraisable(
                    e,
                    Some("Exception ignored in threading shutdown".to_owned()),
                    threading,
                );
            }

            // Mark as finalizing AFTER thread shutdown
            vm.state.finalizing.store(true, Ordering::Release);

            // Run atexit exit functions
            atexit::_run_exitfuncs(vm);

            // Finalize modules: clear module dicts in reverse import order
            vm.finalize_modules();

            vm.flush_std();

            exit_code
        })
    }
}

fn core_frozen_inits() -> impl Iterator<Item = (&'static str, FrozenModule)> {
    let iter = core::iter::empty();
    macro_rules! ext_modules {
        ($iter:ident, $($t:tt)*) => {
            let $iter = $iter.chain(py_freeze!($($t)*));
        };
    }

    // Python modules that the vm calls into, but are not actually part of the stdlib. They could
    // in theory be implemented in Rust, but are easiest to do in Python for one reason or another.
    // Includes _importlib_bootstrap and _importlib_bootstrap_external
    ext_modules!(
        iter,
        dir = "../../Lib/python_builtins",
        crate_name = "rustpython_compiler_core"
    );

    // core stdlib Python modules that the vm calls into, but are still used in Python
    // application code, e.g. copyreg
    // FIXME: Initializing core_modules here results duplicated frozen module generation for core_modules.
    // We need a way to initialize this modules for both `Interpreter::without_stdlib()` and `InterpreterBuilder::new().init_stdlib().interpreter()`
    // #[cfg(not(feature = "freeze-stdlib"))]
    ext_modules!(
        iter,
        dir = "../../Lib/core_modules",
        crate_name = "rustpython_compiler_core"
    );

    // Collect and add frozen module aliases for test modules
    let mut entries: Vec<_> = iter.collect();
    if let Some(hello_code) = entries
        .iter()
        .find(|(n, _)| *n == "__hello__")
        .map(|(_, m)| m.code)
    {
        entries.push((
            "__hello_alias__",
            FrozenModule {
                code: hello_code,
                package: false,
            },
        ));
        entries.push((
            "__phello_alias__",
            FrozenModule {
                code: hello_code,
                package: true,
            },
        ));
        entries.push((
            "__phello_alias__.spam",
            FrozenModule {
                code: hello_code,
                package: false,
            },
        ));
    }
    if let Some(code) = entries
        .iter()
        .find(|(n, _)| *n == "__phello__")
        .map(|(_, m)| m.code)
    {
        entries.push((
            "__phello__.__init__",
            FrozenModule {
                code,
                package: false,
            },
        ));
    }
    if let Some(code) = entries
        .iter()
        .find(|(n, _)| *n == "__phello__.ham")
        .map(|(_, m)| m.code)
    {
        entries.push((
            "__phello__.ham.__init__",
            FrozenModule {
                code,
                package: false,
            },
        ));
    }
    entries.into_iter()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        PyObjectRef,
        builtins::{PyStr, int},
    };
    use malachite_bigint::ToBigInt;

    #[test]
    fn test_add_py_integers() {
        Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let a: PyObjectRef = vm.ctx.new_int(33_i32).into();
            let b: PyObjectRef = vm.ctx.new_int(12_i32).into();
            let res = vm._add(&a, &b).unwrap();
            let value = int::get_value(&res);
            assert_eq!(*value, 45_i32.to_bigint().unwrap());
        })
    }

    #[test]
    fn test_multiply_str() {
        Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let a = vm.new_pyobj(crate::common::ascii!("Hello "));
            let b = vm.new_pyobj(4_i32);
            let res = vm._mul(&a, &b).unwrap();
            let value = res.downcast_ref::<PyStr>().unwrap();
            assert_eq!(value.as_str(), "Hello Hello Hello Hello ")
        })
    }
}

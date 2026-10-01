#[cfg(feature = "threading")]
use super::StopTheWorldState;
use super::{
    Context, PyConfig, PyGlobalState, VirtualMachine,
    owned::OwnedVm,
    runtime::{self, InterpreterWhence},
    setting::Settings,
    thread,
};
use crate::{
    PyResult, builtins, common::rc::PyRc, frozen::FrozenModule, getpath, py_freeze, stdlib::atexit,
    vm::PyBaseExceptionRef,
};
use alloc::collections::BTreeMap;
use core::sync::atomic::Ordering;

type InitFunc = Box<dyn FnOnce(&VirtualMachine)>;
type ConfigFunc = Box<dyn FnOnce(&mut PyConfig)>;

/// Exit code used when stdout/stderr flush fails during interpreter shutdown.
/// Matches CPython's behavior (see cpython/Python/pylifecycle.c).
const EXITCODE_FLUSH_FAILURE: u32 = 120;

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
/// // In practice, add stdlib: builder.add_native_modules(&stdlib_module_defs(unsafe { builder.context() }))
/// let interp = builder.build();
/// ```
pub struct InterpreterBuilder {
    settings: Settings,
    ctx: PyRc<Context>,
    module_defs: Vec<&'static builtins::PyModuleDef>,
    frozen_modules: Vec<(&'static str, FrozenModule)>,
    init_hooks: Vec<InitFunc>,
    config_hooks: Vec<ConfigFunc>,
}

/// Options for constructing a main or sub-interpreter VM.
struct InitializeVmOpts<'a> {
    settings: Settings,
    ctx: PyRc<Context>,
    module_defs: Vec<&'static builtins::PyModuleDef>,
    frozen_modules: Vec<(&'static str, FrozenModule)>,
    init_hooks: Vec<InitFunc>,
    config_hooks: Vec<ConfigFunc>,
    is_main: bool,
    whence: InterpreterWhence,
    /// When `Some`, reuse parent module_defs/frozen/config seeds for a subinterpreter.
    parent_state: Option<&'a PyGlobalState>,
    interp_config: runtime::InterpreterConfig,
}

/// Shared constructor for main and sub-interpreters.
fn initialize_vm<F>(opts: InitializeVmOpts<'_>, init: F) -> (OwnedVm, PyRc<PyGlobalState>)
where
    F: FnOnce(&VirtualMachine),
{
    let InitializeVmOpts {
        settings,
        ctx,
        module_defs,
        frozen_modules,
        init_hooks,
        config_hooks,
        is_main,
        whence,
        parent_state,
        interp_config,
    } = opts;
    use crate::codecs::CodecsRegistry;
    use crate::common::lock::PyMutex;
    use crate::warn::WarningsState;
    use core::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicUsize};
    use crossbeam_utils::atomic::AtomicCell;

    // Before any lock this interpreter's threads can contend on exists.
    #[cfg(feature = "threading")]
    thread::install_blocking_wait_hook();

    let (config, all_module_defs, frozen, int_max_str_digits) = if let Some(parent) = parent_state {
        // Subinterpreter: clone config and module tables from parent, fresh runtime state.
        let int_max_str_digits = AtomicCell::new(parent.int_max_str_digits.load());
        (
            parent.config.clone(),
            parent.module_defs.clone(),
            parent.frozen.clone(),
            int_max_str_digits,
        )
    } else {
        // First top-level interpreter wins. Later seeds are ignored.
        super::init_hash_secret(settings.hash_seed);
        let paths = getpath::init_path_config(&settings);
        let mut config = PyConfig::new(settings, paths);
        for hook in config_hooks {
            hook(&mut config);
        }

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

        let int_max_str_digits = AtomicCell::new(match config.settings.int_max_str_digits {
            -1 => 4300,
            other => other,
        } as usize);

        let mut frozen: std::collections::HashMap<
            &'static str,
            FrozenModule,
            rapidhash::quality::RandomState,
        > = core_frozen_inits().collect();
        frozen.extend(frozen_modules);

        (config, all_module_defs, frozen, int_max_str_digits)
    };

    // Per-interpreter ephemeral state (must not be shared across interpreters).
    #[cfg(all(unix, feature = "threading"))]
    let publication = super::fork::native_phase();
    let interpreter_id = runtime::alloc_interpreter_id();
    let gc = crate::gc_state::GcInterpreterState::new_for_interpreter(&ctx, interpreter_id);
    let allocation_owner = gc.allocation_scope();
    let codec_registry = CodecsRegistry::new(&ctx);
    let warnings = WarningsState::init_state(&ctx);

    let runtime_root_id = parent_state.map_or(interpreter_id, |parent| parent.runtime_root_id);

    // Process main OS thread identity is process-global; subinterpreters inherit
    // it from the parent so `is_main_thread()` stays correct when running on the
    // main OS thread under a subinterpreter.
    #[cfg(feature = "threading")]
    let main_thread_ident = AtomicCell::new(parent_state.map_or(0, |p| p.main_thread_ident.load()));

    let feature_flags = interp_config.feature_flags();
    let own_gil = interp_config.own_gil();

    // Create PyGlobalState (≈ PyInterpreterState)
    let global_state = PyRc::new(PyGlobalState {
        owners: AtomicUsize::new(0),
        owner_leases: super::owner_lease::OwnerLeases::default(),
        teardowns: AtomicUsize::new(0),
        closed: AtomicBool::new(false),
        admission_closed: AtomicBool::new(false),
        gc,
        interpreter_id,
        runtime_root_id,
        whence,
        is_main,
        config,
        module_defs: all_module_defs,
        frozen,
        stacksize: AtomicCell::new(0),
        thread_count: AtomicCell::new(0),
        atexit_funcs: PyMutex::default(),
        audit_hooks: PyMutex::default(),
        codec_registry,
        struct_format_cache: crate::buffer::FormatSpecCache::default(),
        abc_invalidation_counter: AtomicU64::new(0),
        native_cache: super::native_cache::NativeCache::default(),
        roots: PyRc::new(super::roots::Roots::new(interpreter_id)),
        native_types: super::native_types::NativeTypes::default(),
        finalizing: AtomicBool::new(false),
        #[cfg(feature = "threading")]
        finalizing_thread_ident: AtomicCell::new(0),
        warnings,
        override_frozen_modules: AtomicCell::new(0),
        before_forkers: PyMutex::default(),
        after_forkers_child: PyMutex::default(),
        after_forkers_parent: PyMutex::default(),
        int_max_str_digits,
        switch_interval: AtomicCell::new(0.005),
        global_trace_func: PyMutex::default(),
        global_profile_func: PyMutex::default(),
        type_cache: crate::builtins::type_::TypeCache::default(),
        type_mutex: crate::common::lock::PyDetachingRwLock::default(),
        #[cfg(feature = "threading")]
        main_thread_ident,
        #[cfg(feature = "threading")]
        thread_frames: parking_lot::Mutex::new(std::collections::HashMap::new()),
        #[cfg(feature = "threading")]
        thread_handles: parking_lot::Mutex::new(Vec::new()),
        #[cfg(feature = "threading")]
        shutdown_handles: parking_lot::Mutex::new(Vec::new()),
        monitoring: PyMutex::default(),
        monitoring_events: AtomicCell::new(0),
        instrumentation_version: AtomicU64::new(0),
        #[cfg(feature = "threading")]
        stop_the_world: StopTheWorldState::new(),
        feature_flags,
        own_gil,
        running_main: AtomicBool::new(false),
        ready: AtomicBool::new(false),
        id_refcount: AtomicI64::new(0),
        require_idref: AtomicBool::new(false),
    });

    // Create VM with the global state
    let vm = OwnedVm::new(VirtualMachine::new(ctx, global_state));

    // Keep the VM address stable and attached throughout user hooks as well as
    // Python initialization. Unwinding hooks release their locals before this
    // guard ends; OwnedVm then performs the remaining partial-state teardown.
    let vm_guard = thread::VmBootstrapGuard::new(&vm);
    drop(allocation_owner);
    // Hooks can execute Python and must be visible to GC on other threads.
    runtime::register_interpreter(&vm.state);
    #[cfg(all(unix, feature = "threading"))]
    drop(publication);
    let initialized = std::panic::catch_unwind(core::panic::AssertUnwindSafe(|| {
        // Runtime hooks receive shared access after attachment.
        for hook in init_hooks {
            hook(&vm);
        }

        // Run custom native initialization under the same shared entry.
        init(&vm);

        // Imports in initialize() execute Python under the bootstrap guard,
        // before the first ordinary enter_vm scope.
        vm.initialize();
        vm.state.ready.store(true, Ordering::Release);
    }));
    drop(vm_guard);
    if let Err(error) = initialized {
        drop(vm);
        std::panic::resume_unwind(error);
    }

    // Clone global_state for Interpreter after all initialization is done
    let global_state = vm.state.clone();
    (vm, global_state)
}

/// Bootstrap a subinterpreter from an already-entered parent VM.
fn create_subinterpreter_from_parent(
    parent: &VirtualMachine,
    config: runtime::InterpreterConfig,
) -> Result<Interpreter, &'static str> {
    config.check()?;
    // Suspend the caller's current VM attachment (if any) for the duration
    // of subinterpreter initialization. Nested bootstrap would otherwise
    // swap `CURRENT_THREAD_SLOT` to the new interpreter while leaving the
    // outer interpreter's attach state inconsistent. Always restore, even
    // if initialization panics.
    #[cfg(feature = "threading")]
    let _restore_parent = {
        let saved = thread::current_vm_is_set().then(thread::save_current_thread);
        scopeguard::guard(saved, |saved| {
            if let Some(saved) = saved {
                thread::restore_current_thread(saved);
            }
        })
    };

    let (vm, global_state) = initialize_vm(
        InitializeVmOpts {
            // settings unused when parent_state is Some
            settings: Settings::default(),
            ctx: parent.ctx.clone(),
            module_defs: Vec::new(),
            frozen_modules: Vec::new(),
            init_hooks: Vec::new(),
            config_hooks: Vec::new(),
            is_main: false,
            whence: InterpreterWhence::Stdlib,
            parent_state: Some(&parent.state),
            interp_config: config,
        },
        |_| {},
    );
    let interp = Interpreter { global_state, vm };
    // Every interpreter has a `__main__` module once it is initialized.
    interp.enter_raw(|vm| {
        let _ = vm.ensure_main_module();
    });
    Ok(interp)
}

impl InterpreterBuilder {
    /// Access the raw type context for defining native modules.
    ///
    /// # Safety
    /// Follow the native ownership contract in [`crate::embedding`]. Do not retain
    /// Python objects through this context outside their attached interpreter.
    #[must_use]
    pub unsafe fn context(&self) -> &Context {
        &self.ctx
    }

    /// Create a new interpreter configuration with default settings.
    #[must_use]
    pub fn new() -> Self {
        Self {
            settings: Settings::default(),
            ctx: Context::genesis().clone(),
            module_defs: Vec::new(),
            frozen_modules: Vec::new(),
            init_hooks: Vec::new(),
            config_hooks: Vec::new(),
        }
    }

    /// Configure Rust-owned settings and paths before creating runtime objects.
    #[must_use]
    pub fn configure(mut self, configure: impl FnOnce(&mut PyConfig) + 'static) -> Self {
        self.config_hooks.push(Box::new(configure));
        self
    }

    /// Set custom settings for the interpreter.
    ///
    /// If called multiple times, only the last settings will be used.
    #[must_use]
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
    /// // let def = mymodule::module_def(unsafe { builder.context() });
    /// // let interp = builder.add_native_module(def).build();
    /// let interp = builder.build();
    /// ```
    #[must_use]
    /// # Safety
    /// Native callbacks must obey the raw ownership contract in [`crate::embedding`],
    /// including owner-local access and destruction of all Python references.
    pub unsafe fn add_native_module(self, def: &'static builtins::PyModuleDef) -> Self {
        unsafe { self.add_native_modules(&[def]) }
    }

    /// Add multiple native module definitions.
    ///
    /// # Example
    /// ```
    /// use rustpython_vm::Interpreter;
    ///
    /// let builder = Interpreter::builder(Default::default());
    /// // In practice, use module_defs from rustpython_stdlib:
    /// // let defs = rustpython_stdlib::stdlib_module_defs(unsafe { builder.context() });
    /// // let interp = builder.add_native_modules(&defs).build();
    /// let interp = builder.build();
    /// ```
    #[must_use]
    /// # Safety
    /// Native callbacks must obey the raw ownership contract in [`crate::embedding`],
    /// including owner-local access and destruction of all Python references.
    pub unsafe fn add_native_modules(mut self, defs: &[&'static builtins::PyModuleDef]) -> Self {
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
    /// // SAFETY: this hook retains no Python references.
    /// let interp = unsafe { Interpreter::builder(Default::default())
    ///     .init_hook(|_vm| {}) }
    ///     .build();
    /// ```
    #[must_use]
    /// # Safety
    /// Native callbacks must obey the raw ownership contract in [`crate::embedding`],
    /// including owner-local access and destruction of all Python references.
    pub unsafe fn init_hook<F>(mut self, init: F) -> Self
    where
        F: FnOnce(&VirtualMachine) + 'static,
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
    #[must_use]
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
    #[must_use]
    pub fn build(self) -> Interpreter {
        let (vm, global_state) = initialize_vm(
            InitializeVmOpts {
                settings: self.settings,
                ctx: self.ctx,
                module_defs: self.module_defs,
                frozen_modules: self.frozen_modules,
                init_hooks: self.init_hooks,
                config_hooks: self.config_hooks,
                is_main: true,
                whence: InterpreterWhence::Runtime,
                parent_state: None,
                interp_config: runtime::InterpreterConfig::MAIN,
            },
            |_| {}, // No additional init needed
        );
        Interpreter { global_state, vm }
    }

    /// Alias for `build()` for compatibility with the `interpreter()` pattern.
    #[must_use]
    pub fn interpreter(self) -> Interpreter {
        self.build()
    }
}

impl Default for InterpreterBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// One isolated Python interpreter in the process (≈ CPython `PyInterpreterState` + main tstate).
///
/// Historically RustPython exposed a single process-level `Interpreter`. For PEP 734
/// (multiple interpreters / subinterpreters) this type is now the owned handle for
/// **one** interpreter. Use [`Interpreter::create_subinterpreter`] to create additional
/// isolated interpreters that share the process-wide type context but not modules or
/// `PyGlobalState`.
///
/// # Examples
/// Runs a simple embedded hello world program.
/// ```
/// use rustpython_vm::Interpreter;
/// Interpreter::without_stdlib(Default::default()).enter(|vm| {
///     vm.exec(r#"print("Hello World!")"#).unwrap();
/// });
/// ```
pub struct Interpreter {
    pub(crate) global_state: PyRc<PyGlobalState>,
    vm: OwnedVm,
}

/// Finalization was not started because native workers still own this runtime.
/// Drop or join them, then call [`Self::retry`].
pub struct FinalizeBusy {
    interpreter: Interpreter,
    result: crate::embedding::Result<()>,
}

impl core::fmt::Debug for FinalizeBusy {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FinalizeBusy")
            .field("interpreter", &self.interpreter.id())
            .finish_non_exhaustive()
    }
}

impl core::fmt::Display for FinalizeBusy {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("native workers still own the interpreter")
    }
}

impl core::error::Error for FinalizeBusy {}

impl FinalizeBusy {
    #[must_use]
    pub fn interpreter(&self) -> &Interpreter {
        &self.interpreter
    }

    pub fn retry(self) -> Result<u32, Self> {
        self.interpreter.finish_scoped(self.result)
    }

    #[must_use]
    pub fn into_interpreter(self) -> Interpreter {
        self.interpreter
    }
}

impl Interpreter {
    /// Create a new interpreter configuration builder.
    ///
    /// # Example
    /// ```
    /// use rustpython_vm::Interpreter;
    ///
    /// let builder = Interpreter::builder(Default::default());
    /// // In practice, add stdlib: builder.add_native_modules(&stdlib_module_defs(unsafe { builder.context() }))
    /// let interp = builder.build();
    /// ```
    #[must_use]
    pub fn builder(settings: Settings) -> InterpreterBuilder {
        InterpreterBuilder::new().settings(settings)
    }

    /// This is a bare unit to build up an interpreter without the standard library.
    /// To create an interpreter with the standard library with the `rustpython` crate, use `rustpython::InterpreterBuilder`.
    /// To create an interpreter without the `rustpython` crate, but only with `rustpython-vm`,
    /// try to build one from the source code of `InterpreterBuilder`. It will not be a one-liner but it also will not be too hard.
    #[must_use]
    pub fn without_stdlib(settings: Settings) -> Self {
        Self::builder(settings).build()
    }

    /// Create with initialize function taking mutable vm reference.
    ///
    /// Note: This is a legacy API. To add stdlib, use `Interpreter::builder()` instead.
    /// # Safety
    /// Native callbacks must obey the raw ownership contract in [`crate::embedding`],
    /// including owner-local access and destruction of all Python references.
    pub unsafe fn with_init<F>(settings: Settings, init: F) -> Self
    where
        F: FnOnce(&VirtualMachine),
    {
        let (vm, global_state) = initialize_vm(
            InitializeVmOpts {
                settings,
                ctx: Context::genesis().clone(),
                module_defs: Vec::new(),
                frozen_modules: Vec::new(),
                init_hooks: Vec::new(),
                config_hooks: Vec::new(),
                is_main: true,
                whence: InterpreterWhence::Runtime,
                parent_state: None,
                interp_config: runtime::InterpreterConfig::MAIN,
            },
            init,
        );
        Self { global_state, vm }
    }

    /// Process-global interpreter id (main is [`super::MAIN_INTERPRETER_ID`]).
    #[inline]
    #[must_use]
    pub fn id(&self) -> i64 {
        self.global_state.interpreter_id
    }

    /// Where this interpreter was created.
    #[inline]
    #[must_use]
    pub fn whence(&self) -> InterpreterWhence {
        self.global_state.whence
    }

    /// Whether this is a top-level interpreter rather than a subinterpreter.
    ///
    /// Every top-level interpreter answers `true`; for *the* process main, use
    /// [`Interpreter::is_process_main`].
    #[inline]
    #[must_use]
    pub fn is_main(&self) -> bool {
        self.global_state.is_main
    }

    /// Whether this is the PEP 734 process main interpreter (`get_main()`).
    ///
    /// Unlike [`Interpreter::is_main`], which is set for every top-level
    /// interpreter, this is true for only the single first-registered main.
    #[inline]
    #[must_use]
    pub fn is_process_main(&self) -> bool {
        runtime::main_interpreter_id() == Some(self.id())
    }

    /// Create a subinterpreter and hand ownership to the runtime, returning its
    /// id. The runtime keeps it alive until [`runtime::take_owned_interpreter`].
    ///
    /// This is the shape `_interpreters.create()` uses: Python receives an
    /// id, not an owned handle.
    #[cfg(feature = "threading")]
    #[must_use]
    pub fn create_owned_subinterpreter(&self) -> i64 {
        self.create_owned_subinterpreter_with_config(runtime::InterpreterConfig::ISOLATED)
            .expect("the isolated config is always valid")
    }

    /// Create a runtime-owned subinterpreter with the given PEP 734 config.
    #[cfg(feature = "threading")]
    pub fn create_owned_subinterpreter_with_config(
        &self,
        config: runtime::InterpreterConfig,
    ) -> Result<i64, &'static str> {
        Ok(runtime::store_owned_interpreter(
            self.create_subinterpreter_with_config(config)?,
        ))
    }

    /// Create an isolated subinterpreter sharing this interpreter's type context
    /// (`Context`) and module definitions, but with its own `sys.modules`,
    /// builtins module instance, thread registry, and stop-the-world state.
    ///
    /// May be called while the parent is entered (matching CPython, where
    /// `_interpreters.create()` runs under the main interpreter). When the
    /// calling thread is currently attached to a VM, that attachment is
    /// temporarily saved so the subinterpreter can bootstrap as an outermost
    /// enter (correct thread-slot / stop-the-world state).
    #[must_use]
    pub fn create_subinterpreter(&self) -> Self {
        self.create_subinterpreter_with_config(runtime::InterpreterConfig::ISOLATED)
            .expect("the isolated config is always valid")
    }

    /// Create a subinterpreter from a parent VM (the currently entered one).
    pub fn create_subinterpreter_from_vm(
        parent: &VirtualMachine,
        config: runtime::InterpreterConfig,
    ) -> Result<Self, &'static str> {
        create_subinterpreter_from_parent(parent, config)
    }

    /// Create a subinterpreter with an explicit PEP 734 / `PyInterpreterConfig`.
    pub fn create_subinterpreter_with_config(
        &self,
        config: runtime::InterpreterConfig,
    ) -> Result<Self, &'static str> {
        self.enter_raw(|vm| create_subinterpreter_from_parent(vm, config))
    }

    /// Spawn a new OS-thread VM that shares this interpreter's `sys` / builtins.
    #[cfg(feature = "threading")]
    pub fn new_thread(&self) -> thread::ThreadedVirtualMachine {
        self.enter_raw(VirtualMachine::new_thread)
    }

    #[cfg(feature = "threading")]
    pub(super) fn owner_is_valid(&self) -> bool {
        self.vm.owner_is_valid()
    }

    // Only the runtime directory uses an exclusively owned, idle anchor.
    // Entering it while holding that directory's mutex could run Python.
    #[cfg(feature = "threading")]
    pub(super) fn new_thread_from_idle(&self) -> Option<thread::ThreadedVirtualMachine> {
        (self.owner_is_valid() && !self.vm.state.admission_closed.load(Ordering::Acquire))
            .then(|| self.vm.new_thread())
    }

    /// Internal entry for the interpreter implementation.
    pub(crate) fn enter_raw<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&VirtualMachine) -> R,
    {
        thread::enter_vm(&self.vm, || f(&self.vm))
    }

    /// Enter with scoped object access. Only opaque handles and Rust values can
    /// leave the callback; bound objects are tied to this individual entry.
    pub fn enter<R>(&self, f: impl for<'vm> FnOnce(crate::embedding::Vm<'vm>) -> R) -> R {
        if !self.vm.owner_lease().is_valid() {
            return f(crate::embedding::Vm::new(&self.vm));
        }
        self.enter_raw(|vm| {
            vm.state.roots.drain_pending();
            scopeguard::defer! { vm.state.roots.drain_pending(); }
            f(crate::embedding::Vm::new(vm))
        })
    }

    /// Enter the native API without scoped object wrappers.
    ///
    /// # Safety
    /// Follow the ownership and attachment contract in [`crate::embedding`].
    /// Raw references must not escape, cross interpreters, or be accessed while
    /// detached. Use opaque handles for values retained by the host.
    pub unsafe fn enter_unchecked<R>(&self, f: impl FnOnce(&VirtualMachine) -> R) -> R {
        self.enter_raw(f)
    }

    /// Run Python with scoped access, then finalize the interpreter.
    pub fn run(
        self,
        f: impl for<'vm> FnOnce(crate::embedding::Vm<'vm>) -> crate::embedding::Result<()>,
    ) -> Result<u32, FinalizeBusy> {
        let result = self.enter(f);
        self.finish_scoped(result)
    }

    fn finish_scoped(self, result: crate::embedding::Result<()>) -> Result<u32, FinalizeBusy> {
        if !self.vm.owner_lease().is_valid() {
            // An inherited inactive handle cannot finalize a surviving owner.
            // Its Drop only releases its native VM storage.
            return Ok(0);
        }
        // Stale post-fork handles are not counted as owners, but their native
        // cleanup can still race finalization. Close admission under the same
        // lock that admits those cleanup entries.
        let Some(_finalization) = self
            .global_state
            .owner_leases
            .try_finalize(&self.global_state)
        else {
            return Err(FinalizeBusy {
                interpreter: self,
                result,
            });
        };
        // Local owners drop before the permit even if error conversion panics.
        let interpreter = self;
        let exception = interpreter.enter_raw(|vm| match result {
            Ok(()) => None,
            Err(crate::embedding::Error::Python(error)) => error.exception(vm),
            Err(error) => Some(vm.new_runtime_error(error.to_string())),
        });
        Ok(interpreter.finalize_raw(exception))
    }

    /// Finalize an interpreter after repeated scoped entries.
    pub fn finalize(self) -> Result<u32, FinalizeBusy> {
        self.finish_scoped(Ok(()))
    }

    /// Finalize with an exception returned by native code.
    ///
    /// # Safety
    /// The exception and every object reachable from it must belong to this
    /// interpreter. No raw reference may be accessed after finalization.
    #[must_use]
    pub unsafe fn finalize_unchecked(self, exc: Option<PyBaseExceptionRef>) -> u32 {
        self.finalize_raw(exc)
    }

    /// Run the raw native API and finalize the interpreter.
    ///
    /// # Safety
    /// The callback must obey [`Self::enter_unchecked`]'s contract.
    pub unsafe fn run_unchecked(self, f: impl FnOnce(&VirtualMachine) -> PyResult<()>) -> u32 {
        self.run_raw(f)
    }

    /// Run [`Interpreter::enter`] and call [`VirtualMachine::expect_pyresult`] for the result.
    ///
    /// This function is useful when you want to expect a result from the function,
    /// but also print useful panic information when exception raised.
    ///
    /// See also [`Interpreter::enter`] and [`VirtualMachine::expect_pyresult`] for more information.
    /// # Safety
    /// The callback must obey [`Self::enter_unchecked`]'s contract.
    pub unsafe fn enter_and_expect_unchecked<F, R>(&self, f: F, msg: &str) -> R
    where
        F: FnOnce(&VirtualMachine) -> PyResult<R>,
    {
        self.enter_raw(|vm| {
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
    pub(crate) fn run_raw<F>(self, f: F) -> u32
    where
        F: FnOnce(&VirtualMachine) -> PyResult<()>,
    {
        let res = self.enter_raw(|vm| f(vm));
        self.finalize_raw(res.err())
    }

    /// Finalize vm and turns an exception to exit code.
    ///
    /// Finalization steps (matching Py_FinalizeEx):
    /// 1. Flush stdout and stderr.
    /// 1. Handle exit exception and turn it to exit code.
    /// 1. Call threading._shutdown() to join non-daemon threads.
    /// 1. Run atexit exit functions.
    /// 1. Set finalizing flag and hang remaining daemon threads.
    /// 1. Forced GC collection pass (collect cycles while builtins are available).
    /// 1. Module finalization (finalize_modules).
    /// 1. Clear interpreter-owned cross-interpreter data.
    /// 1. Final stdout/stderr flush.
    ///
    /// Note that calling `finalize` is not necessary by purpose though.
    pub(crate) fn finalize_raw(self, exc: Option<PyBaseExceptionRef>) -> u32 {
        let finalization = self
            .global_state
            .owner_leases
            .enter_finalization(&self.global_state);
        // This local must drop before the permit, including on unwind.
        let interpreter = self;
        let result = interpreter.enter_raw(|vm| {
            let mut flush_status = vm.flush_std();

            // See if any exception leaked out:
            let exit_code = if let Some(exc) = exc {
                vm.handle_exit_exception(exc)
            } else {
                0
            };

            // Wait for thread shutdown - call threading._shutdown() if available.
            // This waits for all non-daemon threads to complete.
            // threading module may not be imported, so ignore import errors.
            #[cfg(feature = "threading")]
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

            // Run atexit handlers before setting finalizing flag.
            // This allows unraisable exceptions from atexit handlers to be reported.
            atexit::_run_exitfuncs(vm);

            // Clean up any lingering subinterpreters. This has to happen before
            // the finalizing flag is set, or else threads might get prematurely
            // blocked.
            #[cfg(feature = "threading")]
            finalize_subinterpreters(vm);

            // Running non-daemon threads may start children while shutdown
            // joins them. Close native admission only after that join phase.
            vm.state.admission_closed.store(true, Ordering::Release);

            // Remaining daemon threads hang in `_PyThreadState_HangThread` when
            // they next attach (or at the eval-breaker if they are already
            // running). Their `_ThreadHandle` stays not-done so `is_alive()`
            // is still true for `join()` during the GC that follows.
            #[cfg(feature = "threading")]
            {
                vm.state.stop_the_world.stop_the_world(&vm.state);
                vm.state
                    .finalizing_thread_ident
                    .store(crate::stdlib::_thread::get_ident());
            }
            vm.state.finalizing.store(true, Ordering::Release);
            #[cfg(feature = "threading")]
            {
                thread::set_other_threads_shutting_down(&vm.state);
                vm.state.stop_the_world.start_the_world(&vm.state);
                crate::signal::set_finalizing_bit();
            }

            // GC pass - collect cycles before module cleanup
            vm.state.gc.collect_force(2);

            // Module finalization: remove modules from sys.modules, GC collect
            // (while builtins is still available for __del__), then clear module dicts.
            vm.finalize_modules();

            // CPython clears low-level cross-interpreter container data from
            // _PyAtExit_Fini(), after Python atexit callbacks and module
            // finalization.  In particular, values sent by an atexit callback
            // must also become unbound when this interpreter goes away.
            let interpreter_id = vm.state.interpreter_id;
            crate::stdlib::_interpchannels::clear_interpreter(interpreter_id);
            crate::stdlib::_interpqueues::clear_interpreter(interpreter_id);

            vm.state.roots.close();
            vm.state.type_cache.close();
            vm.state.native_cache.close();
            vm.state.native_types.close();
            vm.state.gc.collect_force(2);

            if vm.flush_std() < 0 && flush_status == 0 {
                flush_status = -1;
            }

            vm.retire_execution_roots();
            #[cfg(feature = "threading")]
            thread::retire_interpreter_thread_roots(vm);
            // SAFETY: consuming finalization has ended this VM's user entries;
            // no reference into its execution caches remains in this closure.
            unsafe { vm.retire_cached_roots() };
            vm.state.retire_python_roots(&vm.ctx);
            vm.state.gc.collect_force(2);

            // Python execution has ended. Late native destructors may release
            // buffers or SQLite statements, but cannot reenter cleared services.
            thread::native_teardown(&vm.state, || {
                vm.state.closed.store(true, Ordering::Release);
                vm.state.close_python_roots();
            });

            // Match CPython: if exit_code is 0 and stdout flush failed, exit 120
            let exit_code = if exit_code == 0 && flush_status < 0 {
                EXITCODE_FLUSH_FAILURE
            } else {
                exit_code
            };

            // Daemon threads may still exist, so use the safe `process()`,
            // not `drain_all()`.
            #[cfg(feature = "threading")]
            {
                vm.state.gc.qsbr().process();
                crate::object::qsbr::shared().process();
            }

            exit_code
        });
        drop(interpreter);
        drop(finalization);
        result
    }
}

/// `finalize_subinterpreters`: destroy the subinterpreters the program left
/// behind, after telling the user they are still around.
#[cfg(feature = "threading")]
fn finalize_subinterpreters(vm: &VirtualMachine) {
    if !vm.state.is_main {
        return;
    }
    let root_id = vm.state.runtime_root_id;
    // Bail out if there are no subinterpreters left.
    if runtime::owned_interpreter_ids_for(root_id).is_empty() {
        return;
    }
    // Warn the user if they forgot to clean up subinterpreters.
    let message = vm
        .ctx
        .new_str("remaining subinterpreters; close them with Interpreter.close()");
    let _ = crate::warn::warn(
        message.into(),
        Some(vm.ctx.exceptions.runtime_warning.to_owned()),
        0,
        None,
        vm,
    );
    // A subinterpreter's finalizers may create another subinterpreter, so
    // re-read the owner table after each destruction like CPython does.
    while let Some(id) = runtime::owned_interpreter_ids_for(root_id)
        .into_iter()
        .next()
    {
        let _ = runtime::destroy_owned_interpreter(id);
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

    // Collect frozen module entries
    let mut entries: Vec<_> = iter.collect();

    // Add test module aliases
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
        entries.push((
            "__hello_only__",
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
        AsObject, PyObjectRef,
        builtins::{PyStr, int},
        vm::{MAIN_INTERPRETER_ID, runtime},
    };
    use malachite_bigint::ToBigInt;

    #[test]
    fn add_py_integers() {
        Interpreter::without_stdlib(Default::default()).enter_raw(|vm| {
            let a: PyObjectRef = vm.ctx.new_int(33_i32).into();
            let b: PyObjectRef = vm.ctx.new_int(12_i32).into();
            let res = vm._add(&a, &b).unwrap();
            let value = int::get_value(&res);
            assert_eq!(*value, 45_i32.to_bigint().unwrap());
        })
    }

    #[test]
    fn multiply_str() {
        Interpreter::without_stdlib(Default::default()).enter_raw(|vm| {
            let a = vm.new_pyobj(crate::common::ascii!("Hello "));
            let b = vm.new_pyobj(4_i32);
            let res = vm._mul(&a, &b).unwrap();
            let value = res.downcast_ref::<PyStr>().unwrap();
            assert_eq!(value.as_wtf8(), "Hello Hello Hello Hello ")
        })
    }

    /// Main interpreter is marked main with Runtime whence and is registered.
    #[test]
    fn main_interpreter_identity() {
        let main = Interpreter::without_stdlib(Default::default());
        assert!(main.is_main());
        assert_eq!(main.whence(), InterpreterWhence::Runtime);
        assert!(
            runtime::list_interpreters()
                .iter()
                .any(|info| info.id == main.id() && info.whence == InterpreterWhence::Runtime)
        );
        // When this is the sole sequential main in a quiet process, id is 0;
        // under parallel tests the id is still unique and registered.
        assert!(main.id() >= MAIN_INTERPRETER_ID);
    }

    /// Subinterpreters get distinct ids, Stdlib whence, and appear in the registry.
    #[test]
    fn create_subinterpreter_registers_distinct_ids() {
        let main = Interpreter::without_stdlib(Default::default());
        let sub1 = main.create_subinterpreter();
        let sub2 = main.create_subinterpreter();

        assert!(main.is_main());
        assert!(!sub1.is_main());
        assert!(!sub2.is_main());
        assert_eq!(sub1.whence(), InterpreterWhence::Stdlib);
        assert_eq!(sub2.whence(), InterpreterWhence::Stdlib);
        assert_ne!(main.id(), sub1.id());
        assert_ne!(main.id(), sub2.id());
        assert_ne!(sub1.id(), sub2.id());

        let ids: Vec<i64> = runtime::list_interpreters()
            .into_iter()
            .map(|i| i.id)
            .collect();
        assert!(ids.contains(&main.id()));
        assert!(ids.contains(&sub1.id()));
        assert!(ids.contains(&sub2.id()));
    }

    /// An interpreter stays looked-up-able until nothing holds its state.
    ///
    /// Dropping the handle is not the end of its life: `new_thread()` workers
    /// hold their own reference, and a collection in progress holds one for
    /// every live interpreter while the world is stopped. So the registry entry
    /// goes away eventually rather than at the drop.
    fn wait_until_unregistered(id: i64) {
        use core::time::Duration;
        use std::time::Instant;

        let deadline = Instant::now() + Duration::from_secs(30);
        while runtime::lookup_interpreter(id).is_some() {
            assert!(
                Instant::now() < deadline,
                "interpreter {id} still registered long after its last reference"
            );
            std::thread::yield_now();
        }
    }

    /// Independent heaps allow bootstrap while an unrelated owner is stopped.
    #[cfg(feature = "threading")]
    #[test]
    fn registering_does_not_wait_for_another_interpreters_stop() {
        use core::time::Duration;
        use std::sync::mpsc;

        let first = Interpreter::without_stdlib(Default::default());
        let (tx, rx) = mpsc::channel();
        let (worker, result) = first.enter_raw(|vm| {
            vm.state.stop_the_world.with_stopped(&vm.state, || {
                let worker = std::thread::spawn(move || {
                    let interp = Interpreter::without_stdlib(Default::default());
                    tx.send(interp.id()).expect("receiver is alive");
                    interp
                });
                let result = rx.recv_timeout(Duration::from_secs(5));
                (worker, result)
            })
        });
        let interp = worker.join().expect("worker did not panic");
        let id = result.expect("unrelated interpreter stop delayed bootstrap");
        assert!(runtime::lookup_interpreter(id).is_some());
        drop(interp);
        wait_until_unregistered(id);
    }

    /// Dropping a subinterpreter releases it; main remains.
    #[test]
    fn drop_subinterpreter_unregisters() {
        let main = Interpreter::without_stdlib(Default::default());
        let sub_id = {
            let sub = main.create_subinterpreter();
            let id = sub.id();
            assert!(runtime::lookup_interpreter(id).is_some());
            id
        };
        wait_until_unregistered(sub_id);
        assert!(runtime::lookup_interpreter(main.id()).is_some());
    }

    /// Each interpreter has its own `sys.modules` / builtins module instance.
    #[test]
    fn subinterpreters_isolate_modules() {
        let main = Interpreter::without_stdlib(Default::default());
        let sub = main.create_subinterpreter();

        let (main_sys_ptr, main_builtins_ptr, main_ctx_ptr, main_state_ptr) =
            main.enter_raw(|vm| {
                (
                    vm.sys_module.as_object() as *const _,
                    vm.builtins.as_object() as *const _,
                    PyRc::as_ptr(&vm.ctx),
                    PyRc::as_ptr(&vm.state),
                )
            });
        let (sub_sys_ptr, sub_builtins_ptr, sub_ctx_ptr, sub_state_ptr) = sub.enter_raw(|vm| {
            (
                vm.sys_module.as_object() as *const _,
                vm.builtins.as_object() as *const _,
                PyRc::as_ptr(&vm.ctx),
                PyRc::as_ptr(&vm.state),
            )
        });

        assert_ne!(main_sys_ptr, sub_sys_ptr);
        assert_ne!(main_builtins_ptr, sub_builtins_ptr);
        // Distinct per-interpreter state.
        assert_ne!(main_state_ptr, sub_state_ptr);
        // Shared process-wide type context (immortal / builtin types).
        assert_eq!(main_ctx_ptr, sub_ctx_ptr);
    }

    /// Mutations to interpreter-owned modules must not leak between interpreters.
    #[test]
    fn subinterpreters_behaviorally_isolate_builtins_and_sys_modules() {
        const PROBE: &str = "__rustpython_subinterpreter_isolation_probe__";

        let main = Interpreter::without_stdlib(Default::default());
        let sub = main.create_subinterpreter();

        main.enter_raw(|vm| {
            vm.builtins
                .set_attr(PROBE, vm.ctx.new_int(11_i32), vm)
                .unwrap();
            vm.sys_module
                .get_attr("modules", vm)
                .unwrap()
                .set_item(PROBE, vm.ctx.new_int(12_i32).into(), vm)
                .unwrap();
        });

        sub.enter_raw(|vm| {
            assert!(vm.builtins.get_attr(PROBE, vm).is_err());
            let modules = vm.sys_module.get_attr("modules", vm).unwrap();
            assert!(modules.get_item(PROBE, vm).is_err());

            vm.builtins
                .set_attr(PROBE, vm.ctx.new_int(21_i32), vm)
                .unwrap();
            modules
                .set_item(PROBE, vm.ctx.new_int(22_i32).into(), vm)
                .unwrap();
        });

        main.enter_raw(|vm| {
            let builtin_probe = vm.builtins.get_attr(PROBE, vm).unwrap();
            assert_eq!(*int::get_value(&builtin_probe), 11_i32.to_bigint().unwrap());

            let module_probe = vm
                .sys_module
                .get_attr("modules", vm)
                .unwrap()
                .get_item(PROBE, vm)
                .unwrap();
            assert_eq!(*int::get_value(&module_probe), 12_i32.to_bigint().unwrap());
        });
    }

    /// Creating a subinterpreter while the parent is entered must not corrupt
    /// the parent's current-VM / thread-slot state.
    #[test]
    fn create_subinterpreter_while_parent_entered() {
        let main = Interpreter::without_stdlib(Default::default());
        main.enter_raw(|vm| {
            let before = vm.state.interpreter_id;
            let sub = main.create_subinterpreter();
            assert_ne!(sub.id(), before);
            // Still the parent after create returns.
            assert_eq!(vm.state.interpreter_id, before);
            // Can still use the parent VM.
            let n: PyObjectRef = vm.ctx.new_int(7_i32).into();
            assert_eq!(int::get_value(&n), &7_i32.to_bigint().unwrap());
            // And the sub is independently usable after parent section.
            drop(sub);
        });
    }

    /// Sequential enter of main then sub on the same OS thread is safe.
    #[test]
    fn sequential_enter_main_and_sub() {
        let main = Interpreter::without_stdlib(Default::default());
        let sub = main.create_subinterpreter();

        main.enter_raw(|vm| {
            assert!(vm.state.is_main_interpreter());
            let a: PyObjectRef = vm.ctx.new_int(1_i32).into();
            let b: PyObjectRef = vm.ctx.new_int(2_i32).into();
            let res = vm._add(&a, &b).unwrap();
            assert_eq!(*int::get_value(&res), 3_i32.to_bigint().unwrap());
        });
        sub.enter_raw(|vm| {
            assert!(!vm.state.is_main_interpreter());
            let a: PyObjectRef = vm.ctx.new_int(10_i32).into();
            let b: PyObjectRef = vm.ctx.new_int(5_i32).into();
            let res = vm._mul(&a, &b).unwrap();
            assert_eq!(*int::get_value(&res), 50_i32.to_bigint().unwrap());
        });
        // Re-enter main after sub.
        main.enter_raw(|vm| {
            assert!(vm.state.is_main_interpreter());
        });
    }

    /// Concurrent use of main + subinterpreter on different OS threads.
    #[cfg(feature = "threading")]
    #[test]
    fn concurrent_main_and_subinterpreter_threads() {
        use alloc::sync::Arc;
        use core::sync::atomic::{AtomicUsize, Ordering};

        let main = Interpreter::without_stdlib(Default::default());
        let sub = main.create_subinterpreter();
        let counter = Arc::new(AtomicUsize::new(0));

        let c1 = Arc::clone(&counter);
        let h_main = main.enter_raw(|vm| {
            let thread_vm = vm.new_thread();
            let c = Arc::clone(&c1);
            std::thread::spawn(move || {
                thread_vm.run_raw(|vm| {
                    for _ in 0..100 {
                        let a: PyObjectRef = vm.ctx.new_int(1_i32).into();
                        let b: PyObjectRef = vm.ctx.new_int(1_i32).into();
                        let _ = vm._add(&a, &b).unwrap();
                        c.fetch_add(1, Ordering::Relaxed);
                    }
                    assert!(vm.state.is_main_interpreter());
                });
            })
        });

        let c2 = Arc::clone(&counter);
        let h_sub = sub.enter_raw(|vm| {
            let thread_vm = vm.new_thread();
            let c = Arc::clone(&c2);
            std::thread::spawn(move || {
                thread_vm.run_raw(|vm| {
                    for _ in 0..100 {
                        let a: PyObjectRef = vm.ctx.new_int(2_i32).into();
                        let b: PyObjectRef = vm.ctx.new_int(3_i32).into();
                        let _ = vm._mul(&a, &b).unwrap();
                        c.fetch_add(1, Ordering::Relaxed);
                    }
                    assert!(!vm.state.is_main_interpreter());
                });
            })
        });

        h_main.join().expect("main worker panicked");
        h_sub.join().expect("sub worker panicked");
        assert_eq!(counter.load(Ordering::Relaxed), 200);
    }

    /// Entering one interpreter must not serialize entry into another interpreter.
    #[cfg(feature = "threading")]
    #[test]
    fn main_and_subinterpreter_run_sections_overlap() {
        use alloc::sync::Arc;
        use core::time::Duration;
        use std::{
            sync::{Condvar, Mutex},
            time::Instant,
        };

        #[derive(Default)]
        struct OverlapState {
            entered: usize,
            release: bool,
        }

        let main = Interpreter::without_stdlib(Default::default());
        let sub = main.create_subinterpreter();
        let state = Arc::new((Mutex::new(OverlapState::default()), Condvar::new()));

        let spawn_worker = |interpreter: &Interpreter| {
            let state = Arc::clone(&state);
            interpreter.enter_raw(|vm| {
                let thread_vm = vm.new_thread();
                std::thread::spawn(move || {
                    thread_vm.run_raw(|vm| {
                        let a: PyObjectRef = vm.ctx.new_int(20_i32).into();
                        let b: PyObjectRef = vm.ctx.new_int(22_i32).into();
                        assert_eq!(
                            *int::get_value(&vm._add(&a, &b).unwrap()),
                            42_i32.to_bigint().unwrap()
                        );

                        let (lock, ready) = &*state;
                        {
                            let mut state = lock.lock().unwrap();
                            state.entered += 1;
                            ready.notify_all();
                        }
                        // Wait attached, but keep passing safepoints: a thread
                        // that blocks outright while attached never suspends,
                        // so a concurrent stop-the-world could not finish and
                        // the other worker could never attach.
                        loop {
                            vm.check_signals().unwrap();
                            let state = lock.lock().unwrap();
                            if state.release {
                                break;
                            }
                            let _ = ready.wait_timeout(state, Duration::from_millis(1)).unwrap();
                        }
                    });
                })
            })
        };

        let main_worker = spawn_worker(&main);
        let sub_worker = spawn_worker(&sub);

        let (lock, ready) = &*state;
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut state_guard = lock.lock().unwrap();
        while state_guard.entered < 2 {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            let (next, _) = ready.wait_timeout(state_guard, deadline - now).unwrap();
            state_guard = next;
        }
        let overlapped = state_guard.entered == 2;
        state_guard.release = true;
        ready.notify_all();
        drop(state_guard);

        main_worker.join().expect("main worker panicked");
        sub_worker.join().expect("subinterpreter worker panicked");
        assert!(
            overlapped,
            "main and subinterpreter run sections were serialized"
        );
    }

    /// A busy interpreter must not prevent another interpreter from making progress.
    #[cfg(feature = "threading")]
    #[test]
    fn busy_main_interpreter_does_not_block_subinterpreter() {
        use alloc::sync::Arc;
        use core::{
            sync::atomic::{AtomicBool, Ordering},
            time::Duration,
        };
        use std::time::Instant;

        let main = Interpreter::without_stdlib(Default::default());
        let sub = main.create_subinterpreter();
        let main_started = Arc::new(AtomicBool::new(false));
        let sub_finished = Arc::new(AtomicBool::new(false));

        let main_started_worker = Arc::clone(&main_started);
        let sub_finished_worker = Arc::clone(&sub_finished);
        let main_worker = main.enter_raw(|vm| {
            let thread_vm = vm.new_thread();
            std::thread::spawn(move || {
                thread_vm.run_raw(|vm| {
                    main_started_worker.store(true, Ordering::Release);
                    let deadline = Instant::now() + Duration::from_secs(30);
                    let mut operations = 0;
                    while !sub_finished_worker.load(Ordering::Acquire) && Instant::now() < deadline
                    {
                        let a: PyObjectRef = vm.ctx.new_int(20_i32).into();
                        let b: PyObjectRef = vm.ctx.new_int(22_i32).into();
                        let result = vm._add(&a, &b).unwrap();
                        assert_eq!(*int::get_value(&result), 42_i32.to_bigint().unwrap());
                        operations += 1;
                        // The protocol calls above never reach a safepoint on
                        // their own; a bytecode loop would. Without this, a
                        // concurrent stop-the-world could not finish while this
                        // thread stays attached.
                        vm.check_signals().unwrap();
                        std::thread::yield_now();
                    }
                    (sub_finished_worker.load(Ordering::Acquire), operations)
                })
            })
        });

        let main_started_worker = Arc::clone(&main_started);
        let sub_finished_worker = Arc::clone(&sub_finished);
        let sub_worker = sub.enter_raw(|vm| {
            let thread_vm = vm.new_thread();
            std::thread::spawn(move || {
                while !main_started_worker.load(Ordering::Acquire) {
                    std::thread::yield_now();
                }
                thread_vm.run_raw(|vm| {
                    let a: PyObjectRef = vm.ctx.new_int(6_i32).into();
                    let b: PyObjectRef = vm.ctx.new_int(7_i32).into();
                    let result = vm._mul(&a, &b).unwrap();
                    assert_eq!(*int::get_value(&result), 42_i32.to_bigint().unwrap());
                    sub_finished_worker.store(true, Ordering::Release);
                });
            })
        });

        let (sub_progressed_while_main_was_busy, main_operations) =
            main_worker.join().expect("main worker panicked");
        sub_worker.join().expect("subinterpreter worker panicked");

        assert!(main_operations > 0);
        assert!(
            sub_progressed_while_main_was_busy,
            "subinterpreter made no progress until the busy main interpreter exited"
        );
    }

    /// `new_thread` on a subinterpreter shares that subinterpreter's state, not main's.
    #[cfg(feature = "threading")]
    #[test]
    fn subinterpreter_new_thread_shares_sub_state() {
        let main = Interpreter::without_stdlib(Default::default());
        let sub = main.create_subinterpreter();
        let sub_id = sub.id();

        let handle = sub.enter_raw(|vm| {
            let thread_vm = vm.new_thread();
            std::thread::spawn(move || {
                thread_vm.run_raw(|vm| {
                    assert_eq!(vm.state.interpreter_id, sub_id);
                    assert!(!vm.state.is_main_interpreter());
                });
            })
        });
        handle.join().expect("thread panicked");
    }

    /// Multiple subinterpreters can each run bytecode via compile+exec.
    #[cfg(feature = "rustpython-compiler")]
    #[test]
    fn subinterpreter_runs_python_code() {
        use crate::compiler::Mode;

        let main = Interpreter::without_stdlib(Default::default());
        let sub = main.create_subinterpreter();

        sub.enter_raw(|vm| {
            let scope = vm.new_scope_with_builtins();
            let source = "x = 40 + 2\n";
            let code = vm
                .compile(source, Mode::Exec, "<sub>")
                .map_err(|err| err.into_pyexception(vm, Some(source)))
                .unwrap();
            vm.run_code_obj(code, scope.clone()).unwrap();
            let x = scope.globals.get_item("x", vm).unwrap();
            assert_eq!(*int::get_value(&x), 42_i32.to_bigint().unwrap());
        });
    }

    /// Subclassing a shared type records the subclass on an object every
    /// interpreter reaches, but only the interpreter that created it lists it.
    fn run(vm: &VirtualMachine, scope: &crate::scope::Scope, source: &str) {
        let code = vm
            .compile(source, crate::compiler::Mode::Exec, "<test>")
            .map_err(|err| err.into_pyexception(vm, Some(source)))
            .unwrap();
        vm.run_code_obj(code, scope.clone()).unwrap();
    }

    #[test]
    fn subinterpreter_subclasses_are_scoped_to_their_interpreter() {
        use crate::scope::Scope;

        fn lists_subclass(vm: &VirtualMachine, scope: &Scope, name: &str) -> bool {
            run(
                vm,
                scope,
                &format!("found = any(c.__name__ == {name:?} for c in int.__subclasses__())\n"),
            );
            let found = scope.globals.get_item("found", vm).unwrap();
            found.try_to_bool(vm).unwrap()
        }

        let main = Interpreter::without_stdlib(Default::default());
        let sub = main.create_subinterpreter();

        // The scopes are what keep the classes alive; a subclass list holds
        // only weak references, so both must outlive every assertion below.
        let main_scope = main.enter_raw(|vm| {
            let scope = vm.new_scope_with_builtins();
            run(vm, &scope, "class MainOnly(int): pass\n");
            scope
        });
        let sub_scope = sub.enter_raw(|vm| {
            let scope = vm.new_scope_with_builtins();
            run(vm, &scope, "class SubOnly(int): pass\n");
            scope
        });

        main.enter_raw(|vm| {
            assert!(lists_subclass(vm, &main_scope, "MainOnly"));
            assert!(!lists_subclass(vm, &main_scope, "SubOnly"));
            // A subclass built before either interpreter existed belongs to the
            // shared context, so it stays visible to both.
            assert!(lists_subclass(vm, &main_scope, "bool"));
        });
        sub.enter_raw(|vm| {
            assert!(lists_subclass(vm, &sub_scope, "SubOnly"));
            assert!(!lists_subclass(vm, &sub_scope, "MainOnly"));
            assert!(lists_subclass(vm, &sub_scope, "bool"));
        });

        main.enter_raw(|_| drop(main_scope));
        sub.enter_raw(|_| drop(sub_scope));
    }

    /// A cycle allocated in one interpreter is not the parent's to collect.
    #[test]
    fn collections_only_reach_the_collecting_interpreter() {
        use core::time::Duration;
        use std::time::Instant;

        const CYCLE: &str = "class Node:\n    pass\n\
                             a = Node()\n\
                             b = Node()\n\
                             a.other = b\n\
                             b.other = a\n\
                             del a\n\
                             del b\n";

        fn live_nodes(vm: &VirtualMachine) -> usize {
            vm.state
                .gc
                .get_objects(None)
                .iter()
                .filter(|obj| &*obj.class().name() == "Node")
                .count()
        }

        let main = Interpreter::without_stdlib(Default::default());
        let sub = main.create_subinterpreter();

        let sub_scope = sub.enter_raw(|vm| {
            let scope = vm.new_scope_with_builtins();
            run(vm, &scope, CYCLE);
            assert_eq!(live_nodes(vm), 2);
            scope
        });

        // A collection in the parent walks its own tracked objects and leaves
        // the sub's cycle where it is. Collections are serialized process-wide
        // by a `try_lock`, so one running elsewhere in the suite makes
        // `collect_force` a no-op; retry until this one gets to run. Each retry
        // waits outside `enter`, since a thread that is entered but not running
        // bytecode never reaches a safepoint, and the collection this is
        // waiting for cannot stop it.
        let deadline = Instant::now() + Duration::from_secs(30);
        while !main.enter_raw(|vm| vm.state.gc.collect_force(2).candidates > 0) {
            assert!(
                Instant::now() < deadline,
                "no collection ran in the parent interpreter"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        sub.enter_raw(|vm| assert_eq!(live_nodes(vm), 2));

        sub.enter_raw(|_| drop(sub_scope));
    }

    /// And it is not the parent's to enumerate either.
    #[test]
    fn get_objects_only_reports_the_calling_interpreter() {
        fn tracks_class(vm: &VirtualMachine, name: &str) -> bool {
            vm.state
                .gc
                .get_objects(None)
                .iter()
                .any(|obj| &*obj.class().name() == name)
        }

        let main = Interpreter::without_stdlib(Default::default());
        let sub = main.create_subinterpreter();

        let main_scope = main.enter_raw(|vm| {
            let scope = vm.new_scope_with_builtins();
            run(vm, &scope, "class MainNode:\n    pass\nkeep = MainNode()\n");
            scope
        });
        let sub_scope = sub.enter_raw(|vm| {
            let scope = vm.new_scope_with_builtins();
            run(vm, &scope, "class SubNode:\n    pass\nkeep = SubNode()\n");
            scope
        });

        main.enter_raw(|vm| {
            assert!(tracks_class(vm, "MainNode"));
            assert!(!tracks_class(vm, "SubNode"));
        });
        sub.enter_raw(|vm| {
            assert!(tracks_class(vm, "SubNode"));
            assert!(!tracks_class(vm, "MainNode"));
        });

        main.enter_raw(|_| drop(main_scope));
        sub.enter_raw(|_| drop(sub_scope));
    }

    /// The runtime can own a subinterpreter by id and hand it back on destroy.
    #[cfg(feature = "threading")]
    #[test]
    fn runtime_owned_interpreter_lifecycle() {
        let main = Interpreter::without_stdlib(Default::default());
        let sub = main.create_subinterpreter();
        let id = sub.id();

        assert_eq!(runtime::store_owned_interpreter(sub), id);
        assert!(runtime::is_owned_interpreter(id));
        assert!(runtime::lookup_interpreter(id).is_some());
        // The owned table is process-global and other tests store into it in
        // parallel, so only this entry's own membership is deterministic.
        assert!(runtime::owned_interpreter_count() >= 1);

        // Reclaiming removes ownership but keeps the interpreter alive while the
        // returned handle is held.
        let reclaimed = runtime::take_owned_interpreter(id).expect("owned by runtime");
        assert_eq!(reclaimed.id(), id);
        assert!(!runtime::is_owned_interpreter(id));
        assert!(runtime::lookup_interpreter(id).is_some());
        assert!(runtime::take_owned_interpreter(id).is_none());

        // Dropping the reclaimed handle releases it.
        drop(reclaimed);
        wait_until_unregistered(id);
    }

    /// `create_owned_subinterpreter` stores the sub and returns only its id.
    #[cfg(feature = "threading")]
    #[test]
    fn create_owned_subinterpreter_returns_id() {
        let main = Interpreter::without_stdlib(Default::default());
        let id = main.create_owned_subinterpreter();
        assert!(runtime::is_owned_interpreter(id));
        assert_ne!(id, main.id());

        let sub = runtime::take_owned_interpreter(id).expect("owned by runtime");
        assert_eq!(sub.id(), id);
        assert!(!sub.is_main());
    }

    /// Finalizing one embedded runtime must leave another runtime's owned
    /// subinterpreters alone.
    #[cfg(feature = "threading")]
    #[test]
    fn owned_subinterpreter_cleanup_is_scoped_to_its_runtime() {
        let main1 = Interpreter::without_stdlib(Default::default());
        let main2 = Interpreter::without_stdlib(Default::default());
        let sub1 = main1.create_owned_subinterpreter();
        let sub2 = main2.create_owned_subinterpreter();

        main1.enter_raw(finalize_subinterpreters);

        assert!(!runtime::is_owned_interpreter(sub1));
        assert!(runtime::is_owned_interpreter(sub2));

        let sub2 = runtime::take_owned_interpreter(sub2).expect("owned by second runtime");
        let _ = sub2.finalize_raw(None);
    }

    /// A collection must stop every interpreter, not just the collecting one:
    /// the generation lists are process-global, so the reachability walk reads
    /// objects owned by other interpreters while their threads would otherwise
    /// still be mutating them.
    #[cfg(all(feature = "threading", feature = "rustpython-compiler"))]
    #[test]
    fn gc_collect_is_safe_while_another_interpreter_runs() {
        use crate::compiler::Mode;
        use alloc::sync::Arc;
        use core::{
            sync::atomic::{AtomicBool, Ordering},
            time::Duration,
        };
        use std::time::Instant;

        // Each interpreter churns reference cycles so both contribute tracked
        // objects to the shared generation lists.
        const CHURN: &str = "\
for _ in range(40):
    a = {}
    b = {'peer': a}
    a['peer'] = b
";

        let main = Interpreter::without_stdlib(Default::default());
        let sub = main.create_subinterpreter();
        let stop = Arc::new(AtomicBool::new(false));

        let run_source = |vm: &VirtualMachine, source: &str| {
            let scope = vm.new_scope_with_builtins();
            let code = vm
                .compile(source, Mode::Exec, "<churn>")
                .map_err(|err| err.into_pyexception(vm, Some(source)))
                .unwrap();
            vm.run_code_obj(code, scope).unwrap();
        };

        // Subinterpreter thread: allocate cycles continuously.
        let stop_worker = Arc::clone(&stop);
        let churner = sub.enter_raw(|vm| {
            let thread_vm = vm.new_thread();
            std::thread::spawn(move || {
                thread_vm.run_raw(|vm| {
                    while !stop_worker.load(Ordering::Acquire) {
                        run_source(vm, CHURN);
                    }
                });
            })
        });

        // Main interpreter: force collections while the sub keeps mutating.
        main.enter_raw(|vm| {
            run_source(vm, CHURN);
            let deadline = Instant::now() + Duration::from_secs(2);
            let mut collections = 0;
            while Instant::now() < deadline && collections < 20 {
                vm.state.gc.collect_force(2);
                collections += 1;
            }
            assert!(collections > 0);
        });

        stop.store(true, Ordering::Release);
        churner.join().expect("churn worker panicked");
    }

    /// A thread entered in one interpreter can park another interpreter's
    /// threads. This is what makes a collection safe: the generation lists are
    /// process-global, so the collector must be able to stop every interpreter,
    /// not only its own.
    #[cfg(all(feature = "threading", feature = "rustpython-compiler"))]
    #[test]
    fn stop_the_world_parks_threads_of_another_interpreter() {
        use crate::compiler::Mode;
        use alloc::sync::Arc;
        use core::{
            sync::atomic::{AtomicBool, AtomicU64, Ordering},
            time::Duration,
        };

        let main = Interpreter::without_stdlib(Default::default());
        let sub = main.create_subinterpreter();
        let sub_state = sub.enter_raw(|vm| vm.state.clone());

        let progress = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));

        // Sub-interpreter worker: runs bytecode (so it reaches safepoints) and
        // reports progress every iteration.
        let progress_worker = Arc::clone(&progress);
        let stop_worker = Arc::clone(&stop);
        let worker = sub.enter_raw(|vm| {
            let thread_vm = vm.new_thread();
            std::thread::spawn(move || {
                thread_vm.run_raw(|vm| {
                    let source = "x = 1 + 1\n";
                    let code = vm
                        .compile(source, Mode::Exec, "<spin>")
                        .map_err(|err| err.into_pyexception(vm, Some(source)))
                        .unwrap();
                    while !stop_worker.load(Ordering::Acquire) {
                        let scope = vm.new_scope_with_builtins();
                        vm.run_code_obj(code.clone(), scope).unwrap();
                        progress_worker.fetch_add(1, Ordering::Release);
                    }
                });
            })
        });

        // Wait until the worker is actually running.
        while progress.load(Ordering::Acquire) == 0 {
            std::thread::yield_now();
        }

        main.enter_raw(|_vm| {
            // Stop the *subinterpreter* from a thread whose current interpreter
            // is main — the cross-interpreter stop a collection performs.
            sub_state.stop_the_world.stop_the_world(&sub_state);

            let parked_at = progress.load(Ordering::Acquire);
            std::thread::sleep(Duration::from_millis(50));
            assert_eq!(
                progress.load(Ordering::Acquire),
                parked_at,
                "subinterpreter thread kept running while its world was stopped"
            );

            sub_state.stop_the_world.start_the_world(&sub_state);
        });

        // After restart the worker makes progress again.
        let resumed_from = progress.load(Ordering::Acquire);
        while progress.load(Ordering::Acquire) == resumed_from {
            std::thread::yield_now();
        }

        stop.store(true, Ordering::Release);
        worker.join().expect("worker panicked");
    }

    /// `STOP_BIT` is process-wide and `start_the_world` in any interpreter
    /// clears it. A worker must still park from its own `stop_requested`.
    #[cfg(all(feature = "threading", feature = "rustpython-compiler"))]
    #[test]
    fn stop_the_world_parks_when_global_stop_bit_is_cleared() {
        use crate::compiler::Mode;
        use alloc::sync::Arc;
        use core::{
            sync::atomic::{AtomicBool, AtomicU64, Ordering},
            time::Duration,
        };

        let main = Interpreter::without_stdlib(Default::default());
        let sub = main.create_subinterpreter();
        let sub_state = sub.enter_raw(|vm| vm.state.clone());

        let progress = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let flip = Arc::new(AtomicBool::new(true));

        let progress_worker = Arc::clone(&progress);
        let stop_worker = Arc::clone(&stop);
        let worker = sub.enter_raw(|vm| {
            let thread_vm = vm.new_thread();
            std::thread::spawn(move || {
                thread_vm.run_raw(|vm| {
                    let source = "x = 1 + 1\n";
                    let code = vm
                        .compile(source, Mode::Exec, "<spin>")
                        .map_err(|err| err.into_pyexception(vm, Some(source)))
                        .unwrap();
                    while !stop_worker.load(Ordering::Acquire) {
                        let scope = vm.new_scope_with_builtins();
                        vm.run_code_obj(code.clone(), scope).unwrap();
                        progress_worker.fetch_add(1, Ordering::Release);
                    }
                });
            })
        });

        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while progress.load(Ordering::Acquire) == 0 {
            assert!(
                std::time::Instant::now() < deadline,
                "worker never started making progress"
            );
            std::thread::yield_now();
        }

        let flip_flag = Arc::clone(&flip);
        let flicker_cycles = Arc::new(AtomicU64::new(0));
        let flicker_cycles_thread = Arc::clone(&flicker_cycles);
        let flicker = std::thread::spawn(move || {
            while flip_flag.load(Ordering::Acquire) {
                crate::signal::set_stop_bit();
                crate::signal::clear_stop_bit();
                flicker_cycles_thread.fetch_add(1, Ordering::Release);
            }
        });

        while flicker_cycles.load(Ordering::Acquire) == 0 {
            assert!(
                std::time::Instant::now() < deadline,
                "STOP_BIT flicker never ran"
            );
            std::thread::yield_now();
        }

        // Stop from a thread of its own so a hang fails the test instead of
        // stalling the suite. Keep flickering until after the stop completes.
        let (tx, rx) = std::sync::mpsc::channel();
        let stopper = std::thread::spawn(move || {
            sub_state.stop_the_world.stop_the_world(&sub_state);
            let stopped = tx.send(());
            (sub_state, stopped)
        });

        let stopped = rx.recv_timeout(Duration::from_secs(10));

        let parked_at = progress.load(Ordering::Acquire);
        std::thread::sleep(Duration::from_millis(50));
        let parked_after = progress.load(Ordering::Acquire);

        // Release before joining: if the stop hung, stopping the worker lets
        // its slot go away so the stopper can finish.
        flip.store(false, Ordering::Release);
        if stopped.is_err() {
            stop.store(true, Ordering::Release);
        }
        flicker.join().expect("stop-bit flicker panicked");
        let (stop_state, sent) = stopper.join().expect("stopper panicked");
        if stopped.is_ok() {
            stop_state.stop_the_world.start_the_world(&stop_state);
        }
        sent.expect("send");
        assert!(
            stopped.is_ok(),
            "stop-the-world hung while STOP_BIT was flickering"
        );
        assert_eq!(
            parked_after, parked_at,
            "subinterpreter thread kept running after STOP_BIT was cleared"
        );

        let resumed_from = progress.load(Ordering::Acquire);
        let resume_deadline = std::time::Instant::now() + Duration::from_secs(10);
        while progress.load(Ordering::Acquire) == resumed_from {
            assert!(
                std::time::Instant::now() < resume_deadline,
                "worker never resumed after start_the_world"
            );
            std::thread::yield_now();
        }

        stop.store(true, Ordering::Release);
        worker.join().expect("worker panicked");
    }

    /// Per-thread `stop_requested` must trip the eval breaker on its own.
    /// Do not touch or assert the process-wide eval-breaker word: other tests
    /// in this process may be flickering `STOP_BIT` at the same time.
    #[cfg(feature = "threading")]
    #[test]
    fn eval_breaker_tripped_when_stop_requested() {
        let interp = Interpreter::without_stdlib(Default::default());
        interp.enter_raw(|vm| {
            crate::signal::clear_eval_breaker_for_test();
            assert!(
                crate::vm::thread::set_stop_requested_for_current_thread(true),
                "current thread has no stop_requested flag"
            );
            let tripped = vm.eval_breaker_tripped();
            crate::vm::thread::set_stop_requested_for_current_thread(false);
            assert!(tripped);
        });
    }

    /// Entering a subinterpreter from inside the parent's `enter` must attach
    /// the subinterpreter's thread slot (and detach the parent's). Otherwise the
    /// thread runs the sub's bytecode with a DETACHED slot, and a collector
    /// stopping that interpreter force-parks the slot and wrongly concludes the
    /// world is stopped while this thread keeps mutating objects.
    #[cfg(all(feature = "threading", feature = "rustpython-compiler"))]
    #[test]
    fn nested_enter_of_subinterpreter_is_stoppable() {
        use crate::compiler::Mode;
        use alloc::sync::Arc;
        use core::{
            sync::atomic::{AtomicBool, AtomicU64, Ordering},
            time::Duration,
        };

        let main = Interpreter::without_stdlib(Default::default());
        let sub = main.create_subinterpreter();
        let sub_state = sub.enter_raw(|vm| vm.state.clone());

        let progress = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));

        // Worker runs the SUB nested inside an active MAIN section.
        let progress_worker = Arc::clone(&progress);
        let stop_worker = Arc::clone(&stop);
        let main_vm = main.enter_raw(|vm| vm.new_thread());
        let sub_vm = sub.enter_raw(|vm| vm.new_thread());
        let worker = std::thread::spawn(move || {
            main_vm.run_raw(|_main| {
                sub_vm.run_raw(|vm| {
                    let source = "x = 1 + 1\n";
                    let code = vm
                        .compile(source, Mode::Exec, "<nested>")
                        .map_err(|err| err.into_pyexception(vm, Some(source)))
                        .unwrap();
                    while !stop_worker.load(Ordering::Acquire) {
                        let scope = vm.new_scope_with_builtins();
                        vm.run_code_obj(code.clone(), scope).unwrap();
                        progress_worker.fetch_add(1, Ordering::Release);
                    }
                });
            });
        });

        while progress.load(Ordering::Acquire) == 0 {
            std::thread::yield_now();
        }

        sub_state.stop_the_world.stop_the_world(&sub_state);
        let parked_at = progress.load(Ordering::Acquire);
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(
            progress.load(Ordering::Acquire),
            parked_at,
            "nested subinterpreter thread kept running while the sub's world was stopped"
        );
        sub_state.stop_the_world.start_the_world(&sub_state);

        let resumed_from = progress.load(Ordering::Acquire);
        while progress.load(Ordering::Acquire) == resumed_from {
            std::thread::yield_now();
        }

        stop.store(true, Ordering::Release);
        worker.join().expect("nested worker panicked");
    }

    /// A thread blocked on a detaching lock must not stall stop-the-world.
    ///
    /// Blocking on a lock reaches no safepoint, so an interpreter thread that
    /// waits while attached is a thread the world can never stop — and the
    /// lock it waits for is routinely one a stopped thread holds, which is the
    /// deadlock. The waiter therefore leaves its interpreter for the wait.
    #[cfg(feature = "threading")]
    #[test]
    fn a_thread_blocked_on_a_lock_does_not_stall_stop_the_world() {
        use super::super::thread::ThreadState;
        use crate::common::lock::PyDetachingRwLock;
        use alloc::sync::Arc;
        use core::{
            sync::atomic::{AtomicU64, Ordering},
            time::Duration,
        };

        let interp = Interpreter::without_stdlib(Default::default());
        let state = interp.enter_raw(|vm| vm.state.clone());

        let lock: Arc<PyDetachingRwLock<()>> = Arc::new(PyDetachingRwLock::new(()));
        // The worker's thread id, published from inside the interpreter. No
        // thread has id 0, so it doubles as "not registered yet".
        let worker_ident = Arc::new(AtomicU64::new(0));

        // Held for the whole test, so the worker below blocks and stays blocked.
        let held = lock.write();

        let worker_lock = Arc::clone(&lock);
        let published_ident = Arc::clone(&worker_ident);
        let worker = interp.enter_raw(|vm| {
            let thread_vm = vm.new_thread();
            std::thread::spawn(move || {
                thread_vm.run_raw(|_vm| {
                    published_ident.store(crate::stdlib::_thread::get_ident(), Ordering::Release);
                    let _read = worker_lock.read();
                });
            })
        });

        // Wait for the worker to have blocked, not merely to have been scheduled
        // to. It publishes its id while attached, so that slot reaching DETACHED
        // is the contended acquire leaving the interpreter — the state this test
        // is about. A sleep here would let the stop below complete with no
        // blocked waiter at all, and pass without testing anything.
        //
        // Bounded, so an acquire that never detaches fails the test instead of
        // hanging it, as the timeout on the stop below does.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let blocked_detached = |ident| {
            state.thread_frames.lock().get(&ident).is_some_and(|slot| {
                slot.state.load(Ordering::Acquire) == ThreadState::Detached as i32
            })
        };
        loop {
            match worker_ident.load(Ordering::Acquire) {
                ident if ident != 0 && blocked_detached(ident) => break,
                _ => assert!(
                    std::time::Instant::now() < deadline,
                    "the worker never detached for the contended acquire"
                ),
            }
            std::thread::yield_now();
        }

        // Stop from a thread of its own so that a stop that never completes
        // fails the test instead of hanging it.
        let (tx, rx) = std::sync::mpsc::channel();
        let stop_state = state;
        let stopper = std::thread::spawn(move || {
            stop_state.stop_the_world.stop_the_world(&stop_state);
            let stopped = tx.send(());
            stop_state.stop_the_world.start_the_world(&stop_state);
            stopped
        });

        let stopped = rx.recv_timeout(Duration::from_secs(10));

        // Release before any assertion: the worker has to finish for the
        // stopper to be joinable, and for the test to end at all.
        drop(held);
        assert!(
            stopped.is_ok(),
            "stop-the-world did not complete while a thread was blocked on a lock"
        );
        stopper.join().expect("stopper panicked").expect("send");
        worker.join().expect("worker panicked");
    }

    /// A callback reaching Python from inside a detached call waits for the
    /// world to start again.
    ///
    /// Detaching for a blocking call is what lets stop-the-world count this
    /// thread as parked. A callback that runs Python from in there — an SSL
    /// handshake reaching a Python `sni_callback`, say — would run on a thread
    /// the requester believes is stopped, so it has to attach first, and
    /// attaching while the world is stopped means waiting.
    #[cfg(feature = "threading")]
    #[test]
    fn a_callback_inside_a_detached_call_waits_for_the_world() {
        use alloc::sync::Arc;
        use core::{
            sync::atomic::{AtomicBool, Ordering},
            time::Duration,
        };

        let interp = Interpreter::without_stdlib(Default::default());
        let state = interp.enter_raw(|vm| vm.state.clone());

        let detached = Arc::new(AtomicBool::new(false));
        let ran = Arc::new(AtomicBool::new(false));
        let go = Arc::new(AtomicBool::new(false));

        let worker_detached = Arc::clone(&detached);
        let worker_ran = Arc::clone(&ran);
        let worker_go = Arc::clone(&go);
        let worker = interp.enter_raw(|vm| {
            let thread_vm = vm.new_thread();
            std::thread::spawn(move || {
                thread_vm.run_raw(|vm| {
                    vm.allow_threads(|| {
                        worker_detached.store(true, Ordering::Release);
                        // Spinning here is spinning *detached*, which is what a
                        // blocking call looks like to the requester: it marks
                        // this thread SUSPENDED and the stop completes.
                        while !worker_go.load(Ordering::Acquire) {
                            std::thread::yield_now();
                        }
                        vm.attach_for_callback(|| worker_ran.store(true, Ordering::Release));
                    });
                });
            })
        });

        while !detached.load(Ordering::Acquire) {
            std::thread::yield_now();
        }

        // Stop from a thread of its own so that a stop that never completes
        // fails the test instead of hanging it.
        let (stopped_tx, stopped_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let stop_state = state;
        let stopper = std::thread::spawn(move || {
            stop_state.stop_the_world.stop_the_world(&stop_state);
            stopped_tx.send(()).expect("send");
            release_rx.recv().expect("recv");
            stop_state.stop_the_world.start_the_world(&stop_state);
        });

        stopped_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("stop-the-world did not complete");

        // The world is stopped; turn the worker loose at its callback. It has
        // to park instead of running it, so the flag stays clear — give it the
        // time it needs to get there and fail to run.
        go.store(true, Ordering::Release);
        std::thread::sleep(Duration::from_millis(200));
        let ran_while_stopped = ran.load(Ordering::Acquire);

        // Release before asserting: the worker has to finish for the stopper to
        // be joinable, and for the test to end at all.
        release_tx.send(()).expect("send");
        stopper.join().expect("stopper panicked");
        worker.join().expect("worker panicked");

        assert!(
            !ran_while_stopped,
            "a callback ran Python while the world was stopped"
        );
        assert!(
            ran.load(Ordering::Acquire),
            "the callback never ran once the world started again"
        );
    }

    /// The process main id is recorded once and is stable across later creates.
    #[test]
    fn process_main_id_recorded_and_stable() {
        // At least one main exists by now (this one, if not an earlier test), so
        // `get_main()` is populated.
        let main = Interpreter::without_stdlib(Default::default());
        let recorded = runtime::main_interpreter_id().expect("a process main exists");

        // Recording is once-only: further interpreters do not displace it.
        let _sub = main.create_subinterpreter();
        let _main2 = Interpreter::without_stdlib(Default::default());
        assert_eq!(runtime::main_interpreter_id(), Some(recorded));
    }

    /// A second top-level interpreter must keep the first interpreter's hash
    /// secret. Interned strings cache that hash for the whole process.
    #[test]
    fn second_interpreter_reuses_process_hash_secret() {
        let settings = Settings {
            hash_seed: Some(7),
            ..Settings::default()
        };
        let first = Interpreter::without_stdlib(settings);
        first.enter_raw(|vm| {
            vm.ctx
                .intern_str("zz_hash_seed_regression")
                .to_object()
                .hash(vm)
                .unwrap();
        });

        let settings = Settings {
            hash_seed: Some(8),
            ..Settings::default()
        };
        let second = Interpreter::without_stdlib(settings);
        second.enter_raw(|vm| {
            let interned = vm.ctx.intern_str("zz_hash_seed_regression").to_object();
            let dict = vm.ctx.new_dict();
            dict.set_item(&*interned, vm.ctx.new_int(1).into(), vm)
                .unwrap();

            let scope = vm.new_scope_with_builtins();
            scope.globals.set_item("d", dict.into(), vm).unwrap();
            run(
                vm,
                &scope,
                "k = 'zz_hash_seed_' + 'regression'\nresult = k in d\n",
            );
            let result = scope.globals.get_item("result", vm).unwrap();
            assert!(result.try_to_bool(vm).unwrap());
        });
    }
}

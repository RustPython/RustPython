use rustpython_vm::{Interpreter, PyRef, Settings, VirtualMachine, builtins::PyModule};

pub type InitHook = Box<dyn FnOnce(&mut VirtualMachine)>;

/// The convenient way to create [rustpython_vm::Interpreter] with stdlib and other stuffs.
///
/// Basic usage:
/// ```
/// let interpreter = rustpython::InterpreterConfig::new()
///     .init_stdlib()
///     .interpreter();
/// ```
///
/// To override [rustpython_vm::Settings]:
/// ```
/// use rustpython_vm::Settings;
/// // Override your settings here.
/// let mut settings = Settings::default();
/// settings.debug = 1;
/// // You may want to add paths to `rustpython_vm::Settings::path_list` to allow import python libraries.
/// settings.path_list.push("Lib".to_owned());  // add standard library directory
/// settings.path_list.push("".to_owned());  // add current working directory
/// let interpreter = rustpython::InterpreterConfig::new()
///     .settings(settings)
///     .interpreter();
/// ```
///
/// To add native modules:
/// ```
/// use rustpython_vm::pymodule;
///
/// #[pymodule]
/// mod your_module {}
///
/// let interpreter = rustpython::InterpreterConfig::new()
///     .init_stdlib()
///     .add_native_module(
///         "your_module_name".to_owned(),
///         your_module::make_module,
///     )
///     .interpreter();
/// ```
#[derive(Default)]
pub struct InterpreterConfig {
    settings: Option<Settings>,
    init_hooks: Vec<InitHook>,
}

impl InterpreterConfig {
    /// Creates a new interpreter configuration with default settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds the interpreter with the current configuration.
    pub fn interpreter(self) -> Interpreter {
        let settings = self.settings.unwrap_or_default();
        Interpreter::with_init(settings, |vm| {
            for hook in self.init_hooks {
                hook(vm);
            }
        })
    }

    /// Sets custom settings for the interpreter.
    ///
    /// If called multiple times, only the last settings will be used.
    pub fn settings(mut self, settings: Settings) -> Self {
        self.settings = Some(settings);
        self
    }

    /// Adds a custom initialization hook.
    ///
    /// Hooks are executed in the order they are added during interpreter creation.
    pub fn init_hook(mut self, hook: InitHook) -> Self {
        self.init_hooks.push(hook);
        self
    }

    /// Adds a native module to the interpreter.
    pub fn add_native_module(
        self,
        name: String,
        make_module: fn(&VirtualMachine) -> PyRef<PyModule>,
    ) -> Self {
        self.init_hook(Box::new(move |vm| {
            vm.add_native_module(name, Box::new(make_module))
        }))
    }

    /// Initializes the Python standard library.
    ///
    /// Requires the `stdlib` feature to be enabled.
    #[cfg(feature = "stdlib")]
    pub fn init_stdlib(self) -> Self {
        self.init_hook(Box::new(init_stdlib))
    }
}

/// Initializes all standard library modules for the given VM.
#[cfg(feature = "stdlib")]
pub fn init_stdlib(vm: &mut VirtualMachine) {
    vm.add_native_modules(rustpython_stdlib::get_module_inits());

    #[cfg(feature = "freeze-stdlib")]
    setup_frozen_stdlib(vm);

    #[cfg(not(feature = "freeze-stdlib"))]
    setup_dynamic_stdlib(vm);
}

/// Setup frozen standard library (compiled into the binary)
#[cfg(all(feature = "stdlib", feature = "freeze-stdlib"))]
fn setup_frozen_stdlib(vm: &mut VirtualMachine) {
    use rustpython_vm::common::rc::PyRc;

    vm.add_frozen(rustpython_pylib::FROZEN_STDLIB);

    // Set stdlib_dir to the frozen stdlib path
    let state = PyRc::get_mut(&mut vm.state).unwrap();
    state.config.paths.stdlib_dir = Some(rustpython_pylib::LIB_PATH.to_owned());
}

/// Setup dynamic standard library loading from filesystem
#[cfg(all(feature = "stdlib", not(feature = "freeze-stdlib")))]
fn setup_dynamic_stdlib(vm: &mut VirtualMachine) {
    use rustpython_vm::common::rc::PyRc;

    let state = PyRc::get_mut(&mut vm.state).unwrap();
    let paths = collect_stdlib_paths();

    // Set stdlib_dir to the first stdlib path if available
    if let Some(first_path) = paths.first() {
        state.config.paths.stdlib_dir = Some(first_path.clone());
    }

    // Insert at the beginning so stdlib comes before user paths
    for path in paths.into_iter().rev() {
        state.config.paths.module_search_paths.insert(0, path);
    }
}

/// Collect standard library paths from build-time configuration
#[cfg(all(feature = "stdlib", not(feature = "freeze-stdlib")))]
fn collect_stdlib_paths() -> Vec<String> {
    // BUILDTIME_RUSTPYTHONPATH should be set when distributing
    if let Some(paths) = option_env!("BUILDTIME_RUSTPYTHONPATH") {
        crate::settings::split_paths(paths)
            .map(|path| path.into_os_string().into_string().unwrap())
            .collect()
    } else {
        #[cfg(feature = "rustpython-pylib")]
        {
            vec![rustpython_pylib::LIB_PATH.to_owned()]
        }
        #[cfg(not(feature = "rustpython-pylib"))]
        {
            vec![]
        }
    }
}

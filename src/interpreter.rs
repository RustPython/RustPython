use rustpython_vm::{Interpreter, PyRef, Settings, VirtualMachine, builtins::PyModule};

pub type InitHook = Box<dyn FnOnce(&mut VirtualMachine)>;

/// The convenient way to create [rustpython_vm::Interpreter] with stdlib and other components.
///
/// # Basic Usage
/// ```no_run
/// use rustpython::InterpreterConfig;
///
/// let interpreter = InterpreterConfig::new()
///     .init_stdlib()
///     .interpreter();
/// ```
///
/// # Override Settings
/// ```no_run
/// use rustpython_vm::Settings;
/// use rustpython::InterpreterConfig;
///
/// let mut settings = Settings::default();
/// settings.debug = 1;
/// // Add paths to allow importing Python libraries
/// settings.path_list.push("Lib".to_owned());  // standard library directory
/// settings.path_list.push("".to_owned());     // current working directory
///
/// let interpreter = InterpreterConfig::new()
///     .settings(settings)
///     .interpreter();
/// ```
///
/// # Add Native Modules
/// ```no_run
/// use rustpython::InterpreterConfig;
/// use rustpython_vm::{VirtualMachine, PyRef, builtins::PyModule};
///
/// fn make_custom_module(vm: &VirtualMachine) -> PyRef<PyModule> {
///     // Your module implementation
/// #   todo!()
/// }
///
/// let interpreter = InterpreterConfig::new()
///     .init_stdlib()
///     .add_native_module(
///         "your_module_name".to_owned(),
///         make_custom_module,
///     )
///     .interpreter();
/// ```
#[derive(Default)]
pub struct InterpreterConfig {
    settings: Option<Settings>,
    init_hooks: Vec<InitHook>,
}

impl InterpreterConfig {
    /// Create a new interpreter configuration with default settings
    pub fn new() -> Self {
        Self::default()
    }

    /// Build the interpreter with the current configuration
    ///
    /// # Panics
    /// May panic if initialization hooks encounter fatal errors
    pub fn interpreter(self) -> Interpreter {
        let settings = self.settings.unwrap_or_default();
        Interpreter::with_init(settings, |vm| {
            for hook in self.init_hooks {
                hook(vm);
            }
        })
    }

    /// Set custom settings for the interpreter
    ///
    /// If called multiple times, only the last settings will be used
    pub fn settings(mut self, settings: Settings) -> Self {
        self.settings = Some(settings);
        self
    }

    /// Add a custom initialization hook
    ///
    /// Hooks are executed in the order they are added during interpreter creation
    pub fn init_hook(mut self, hook: InitHook) -> Self {
        self.init_hooks.push(hook);
        self
    }

    /// Add a native module to the interpreter
    ///
    /// # Arguments
    /// * `name` - The module name that will be used for imports
    /// * `make_module` - Function that creates the module when called
    ///
    /// # Example
    /// ```no_run
    /// # use rustpython::InterpreterConfig;
    /// # use rustpython_vm::{VirtualMachine, PyRef, builtins::PyModule};
    /// # fn my_module(vm: &VirtualMachine) -> PyRef<PyModule> { todo!() }
    /// let interpreter = InterpreterConfig::new()
    ///     .add_native_module("mymodule".to_owned(), my_module)
    ///     .interpreter();
    /// ```
    pub fn add_native_module(
        self,
        name: String,
        make_module: fn(&VirtualMachine) -> PyRef<PyModule>,
    ) -> Self {
        self.init_hook(Box::new(move |vm| {
            vm.add_native_module(name, Box::new(make_module))
        }))
    }

    /// Initialize the Python standard library
    ///
    /// This adds all standard library modules to the interpreter.
    /// Requires the `stdlib` feature to be enabled at compile time.
    #[cfg(feature = "stdlib")]
    pub fn init_stdlib(self) -> Self {
        self.init_hook(Box::new(init_stdlib))
    }

    /// Initialize the Python standard library (no-op without stdlib feature)
    ///
    /// When the `stdlib` feature is not enabled, this method does nothing
    /// and prints a warning. Enable the `stdlib` feature to use the standard library.
    #[cfg(not(feature = "stdlib"))]
    pub fn init_stdlib(self) -> Self {
        eprintln!(
            "Warning: stdlib feature is not enabled. Standard library will not be available."
        );
        self
    }

    /// Convenience method to set the debug level
    ///
    /// # Example
    /// ```no_run
    /// # use rustpython::InterpreterConfig;
    /// let interpreter = InterpreterConfig::new()
    ///     .with_debug(1)
    ///     .interpreter();
    /// ```
    pub fn with_debug(mut self, level: u8) -> Self {
        self.settings.get_or_insert_with(Default::default).debug = level;
        self
    }

    /// Convenience method to add a single path to the module search paths
    ///
    /// # Example
    /// ```no_run
    /// # use rustpython::InterpreterConfig;
    /// let interpreter = InterpreterConfig::new()
    ///     .add_path("Lib")
    ///     .add_path(".")
    ///     .interpreter();
    /// ```
    pub fn add_path(mut self, path: impl Into<String>) -> Self {
        self.settings
            .get_or_insert_with(Default::default)
            .path_list
            .push(path.into());
        self
    }

    /// Add multiple paths to the module search paths at once
    ///
    /// # Example
    /// ```no_run
    /// # use rustpython::InterpreterConfig;
    /// let interpreter = InterpreterConfig::new()
    ///     .add_paths(vec!["Lib", ".", "custom_modules"])
    ///     .interpreter();
    /// ```
    pub fn add_paths<I, S>(mut self, paths: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let settings = self.settings.get_or_insert_with(Default::default);
        settings.path_list.extend(paths.into_iter().map(Into::into));
        self
    }
}

/// Initialize the standard library modules
///
/// This function sets up both native modules and handles frozen/dynamic stdlib loading
#[cfg(feature = "stdlib")]
pub fn init_stdlib(vm: &mut VirtualMachine) {
    vm.add_native_modules(rustpython_stdlib::get_module_inits());

    #[cfg(feature = "freeze-stdlib")]
    setup_frozen_stdlib(vm);

    #[cfg(not(feature = "freeze-stdlib"))]
    setup_dynamic_stdlib(vm);
}

/// Setup frozen standard library
///
/// Used when the stdlib is compiled into the binary
#[cfg(all(feature = "stdlib", feature = "freeze-stdlib"))]
fn setup_frozen_stdlib(vm: &mut VirtualMachine) {
    vm.add_frozen(rustpython_pylib::FROZEN_STDLIB);

    // FIXME: Remove this hack once sys._stdlib_dir is properly implemented
    // or _frozen_importlib doesn't depend on it anymore.
    // The assert ensures _stdlib_dir doesn't already exist before we set it
    assert!(vm.sys_module.get_attr("_stdlib_dir", vm).is_err());
    vm.sys_module
        .set_attr(
            "_stdlib_dir",
            vm.new_pyobj(rustpython_pylib::LIB_PATH.to_owned()),
            vm,
        )
        .unwrap();
}

/// Setup dynamic standard library loading from filesystem
///
/// Used when the stdlib is loaded from disk at runtime
#[cfg(all(feature = "stdlib", not(feature = "freeze-stdlib")))]
fn setup_dynamic_stdlib(vm: &mut VirtualMachine) {
    use rustpython_vm::common::rc::PyRc;

    let state = PyRc::get_mut(&mut vm.state).unwrap();

    let additional_paths = collect_stdlib_paths();

    // Insert at the beginning so stdlib comes before user paths
    for path in additional_paths.into_iter().rev() {
        state.config.paths.module_search_paths.insert(0, path);
    }
}

/// Collect standard library paths from build-time configuration
///
/// Checks BUILDTIME_RUSTPYTHONPATH environment variable or uses default pylib path
#[cfg(all(feature = "stdlib", not(feature = "freeze-stdlib")))]
fn collect_stdlib_paths() -> Vec<String> {
    let mut additional_paths = Vec::new();

    // BUILDTIME_RUSTPYTHONPATH should be set when distributing
    if let Some(paths) = option_env!("BUILDTIME_RUSTPYTHONPATH") {
        additional_paths.extend(crate::settings::split_paths(paths).map(|path| {
            path.into_os_string()
                .into_string()
                .unwrap_or_else(|_| panic!("BUILDTIME_RUSTPYTHONPATH isn't valid unicode"))
        }))
    } else {
        #[cfg(feature = "rustpython-pylib")]
        additional_paths.push(rustpython_pylib::LIB_PATH.to_owned())
    }

    additional_paths
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = InterpreterConfig::new();
        assert!(config.settings.is_none());
        assert!(config.init_hooks.is_empty());
    }

    #[test]
    fn test_with_debug() {
        let config = InterpreterConfig::new().with_debug(2);
        let settings = config.settings.unwrap();
        assert_eq!(settings.debug, 2);
    }

    #[test]
    fn test_add_single_path() {
        let config = InterpreterConfig::new().add_path("test/path");
        let settings = config.settings.unwrap();
        assert_eq!(settings.path_list.len(), 1);
        assert_eq!(settings.path_list[0], "test/path");
    }

    #[test]
    fn test_add_multiple_paths_sequential() {
        let config = InterpreterConfig::new().add_path("path1").add_path("path2");
        let settings = config.settings.unwrap();
        assert_eq!(settings.path_list.len(), 2);
    }

    #[test]
    fn test_add_paths_batch() {
        let paths = vec!["path1", "path2", "path3"];
        let config = InterpreterConfig::new().add_paths(paths);
        let settings = config.settings.unwrap();
        assert_eq!(settings.path_list.len(), 3);
    }
}

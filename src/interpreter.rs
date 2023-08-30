use rustpython_vm::{Interpreter, Settings, VirtualMachine};

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
/// settings.debug = true;
/// // You may want to add paths to `rustpython_vm::Settings::path_list` to allow import python libraries.
/// settings.path_list.push("".to_owned());  // add current working directory
/// let interpreter = rustpython::InterpreterConfig::new()
///     .settings(settings)
///     .interpreter();
/// ```
///
/// To add native modules:
/// ```compile_fail
/// let interpreter = rustpython::InterpreterConfig::new()
///     .init_stdlib()
///     .init_hook(Box::new(|vm| {
///         vm.add_native_module(
///             "your_module_name".to_owned(),
///             Box::new(your_module::make_module),
///         );
///     }))
///     .interpreter();
/// ```
#[derive(Default)]
pub struct InterpreterConfig {
    settings: Option<Settings>,
    init_hooks: Vec<InitHook>,
}

impl InterpreterConfig {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn interpreter(self) -> Interpreter {
        let settings = self.settings.unwrap_or_default();
        Interpreter::with_init(settings, |vm| {
            for hook in self.init_hooks {
                hook(vm);
            }
        })
    }

    pub fn settings(mut self, settings: Settings) -> Self {
        self.settings = Some(settings);
        self
    }
    pub fn init_hook(mut self, hook: InitHook) -> Self {
        self.init_hooks.push(hook);
        self
    }
    #[cfg(feature = "stdlib")]
    pub fn init_stdlib(self) -> Self {
        self.init_hook(Box::new(init_stdlib))
    }
}

#[cfg(feature = "stdlib")]
pub fn init_stdlib(vm: &mut VirtualMachine) {
    vm.add_native_modules(rustpython_stdlib::get_module_inits());

    // if we're on freeze-stdlib, the core stdlib modules will be included anyway
    #[cfg(feature = "freeze-stdlib")]
    vm.add_frozen(rustpython_pylib::FROZEN_STDLIB);

    #[cfg(not(feature = "freeze-stdlib"))]
    {
        use rustpython_vm::common::rc::PyRc;

        let state = PyRc::get_mut(&mut vm.state).unwrap();
        let settings = &mut state.settings;

        let path_list = std::mem::take(&mut settings.path_list);

        // BUILDTIME_RUSTPYTHONPATH should be set when distributing
        if let Some(paths) = option_env!("BUILDTIME_RUSTPYTHONPATH") {
            settings.path_list.extend(
                crate::settings::split_paths(paths)
                    .map(|path| path.into_os_string().into_string().unwrap()),
            )
        } else {
            #[cfg(feature = "rustpython-pylib")]
            settings
                .path_list
                .push(rustpython_pylib::LIB_PATH.to_owned())
        }

        settings.path_list.extend(path_list);
    }
}

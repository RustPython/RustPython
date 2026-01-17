use rustpython_vm::InterpreterBuilder;

/// Extension trait for InterpreterBuilder to add rustpython-specific functionality.
pub trait InterpreterBuilderExt {
    /// Initialize the Python standard library.
    ///
    /// Requires the `stdlib` feature to be enabled.
    #[cfg(feature = "stdlib")]
    fn init_stdlib(self) -> Self;
}

impl InterpreterBuilderExt for InterpreterBuilder {
    #[cfg(feature = "stdlib")]
    fn init_stdlib(self) -> Self {
        let defs = rustpython_stdlib::stdlib_module_defs(&self.ctx);
        let builder = self.add_native_modules(&defs);

        #[cfg(feature = "freeze-stdlib")]
        let builder = builder
            .add_frozen_modules(rustpython_pylib::FROZEN_STDLIB)
            .init_hook(set_frozen_stdlib_dir);

        #[cfg(not(feature = "freeze-stdlib"))]
        let builder = builder.init_hook(setup_dynamic_stdlib);

        builder
    }
}

/// Set stdlib_dir for frozen standard library
#[cfg(all(feature = "stdlib", feature = "freeze-stdlib"))]
fn set_frozen_stdlib_dir(vm: &mut crate::VirtualMachine) {
    use rustpython_vm::common::rc::PyRc;

    let state = PyRc::get_mut(&mut vm.state).unwrap();
    state.config.paths.stdlib_dir = Some(rustpython_pylib::LIB_PATH.to_owned());
}

/// Setup dynamic standard library loading from filesystem
#[cfg(all(feature = "stdlib", not(feature = "freeze-stdlib")))]
fn setup_dynamic_stdlib(vm: &mut crate::VirtualMachine) {
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

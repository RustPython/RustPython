use rustpython_vm::InterpreterBuilder;

/// Extension trait for InterpreterBuilder to add rustpython-specific functionality.
pub trait InterpreterBuilderExt {
    /// Initialize the Python standard library.
    ///
    /// Requires the `stdlib` feature to be enabled.
    #[cfg(feature = "stdlib")]
    #[must_use]
    fn init_stdlib(self) -> Self;
}

impl InterpreterBuilderExt for InterpreterBuilder {
    #[cfg(feature = "stdlib")]
    fn init_stdlib(self) -> Self {
        let defs = rustpython_stdlib::stdlib_module_defs(&self.ctx);
        let builder = self.add_native_modules(&defs);
        #[cfg(all(feature = "ssl-rustls-aws-lc", not(target_arch = "wasm32")))]
        let builder = builder.init_hook(install_default_tls_provider);

        cfg_select! {
            feature = "freeze-stdlib" => builder
                .add_frozen_modules(rustpython_pylib::FROZEN_STDLIB)
                .init_hook(set_frozen_stdlib_dir),
            _ => builder.init_hook(setup_dynamic_stdlib),
        }
    }
}

#[cfg(all(feature = "ssl-rustls-aws-lc", not(target_arch = "wasm32")))]
fn install_default_tls_provider(_vm: &mut crate::VirtualMachine) {
    use rustls::crypto::aws_lc_rs;
    use rustpython_stdlib::ssl::providers::CryptoExt;

    #[cfg(feature = "ssl-rustls-aws-lc-fips")]
    let (all_cipher_suites, all_kx_groups) = (None, None);
    #[cfg(not(feature = "ssl-rustls-aws-lc-fips"))]
    let (all_cipher_suites, all_kx_groups) = (
        Some(aws_lc_rs::ALL_CIPHER_SUITES),
        Some(aws_lc_rs::ALL_KX_GROUPS),
    );

    let ext = CryptoExt {
        all_cipher_suites,
        default_cipher_suites: Some(aws_lc_rs::DEFAULT_CIPHER_SUITES),
        all_kx_groups,
        any_supported_key: Some(aws_lc_rs::sign::any_supported_type),
        ticketer: aws_lc_rs::Ticketer::new,
    };
    let _ = CryptoExt::set_provider(aws_lc_rs::default_provider(), ext);
}

/// Set stdlib_dir for frozen standard library
#[cfg(all(feature = "stdlib", feature = "freeze-stdlib"))]
fn set_frozen_stdlib_dir(vm: &mut crate::VirtualMachine) {
    use rustpython_vm::common::rc::PyRc;

    let state = PyRc::get_mut(&mut vm.state).unwrap();
    if !state
        .config
        .settings
        .home
        .as_ref()
        .is_some_and(|home| !home.is_empty())
    {
        state.config.paths.stdlib_dir =
            Some(canonical_stdlib_path(rustpython_pylib::LIB_PATH.to_owned()));
    }
}

/// Setup dynamic standard library loading from filesystem
#[cfg(all(feature = "stdlib", not(feature = "freeze-stdlib")))]
fn setup_dynamic_stdlib(vm: &mut crate::VirtualMachine) {
    use rustpython_vm::common::rc::PyRc;

    let state = PyRc::get_mut(&mut vm.state).unwrap();
    // An explicit home must not silently fall back to the build-tree library.
    if state
        .config
        .settings
        .home
        .as_ref()
        .is_some_and(|home| !home.is_empty())
    {
        return;
    }
    let paths: Vec<String> = collect_stdlib_paths()
        .into_iter()
        .map(canonical_stdlib_path)
        .collect();

    // Set stdlib_dir to the first stdlib path if available
    if let Some(first_path) = paths.first() {
        state.config.paths.stdlib_dir = Some(first_path.clone());
    }

    // Insert at the beginning so stdlib comes before user paths
    for path in paths.into_iter().rev() {
        state.config.paths.module_search_paths.insert(0, path);
    }
}

/// Resolve the build-tree library link consistently for both backends.
#[cfg(feature = "stdlib")]
fn canonical_stdlib_path(path: String) -> String {
    std::fs::canonicalize(&path)
        .map(|canonical| {
            let value = canonical.to_string_lossy();
            #[cfg(windows)]
            if let Some(stripped) = value.strip_prefix(r"\\?\") {
                return stripped.to_owned();
            }
            value.into_owned()
        })
        .unwrap_or(path)
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
        vec![
            #[cfg(feature = "rustpython-pylib")]
            rustpython_pylib::LIB_PATH.to_owned(),
        ]
    }
}

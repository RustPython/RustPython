/// Struct containing all kind of settings for the python vm.
#[non_exhaustive]
pub struct Settings {
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

    /// -X warn_default_encoding, PYTHONWARNDEFAULTENCODING
    pub warn_default_encoding: bool,

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

    /// --check-hash-based-pycs
    pub check_hash_based_pycs: String,

    /// false for wasm. Not a command-line option
    pub allow_external_library: bool,
}

/// Sensible default settings.
impl Default for Settings {
    fn default() -> Self {
        Settings {
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
            warn_default_encoding: false,
            warnopts: vec![],
            path_list: vec![
                #[cfg(all(feature = "pylib", not(feature = "freeze-stdlib")))]
                rustpython_pylib::LIB_PATH.to_owned(),
            ],
            argv: vec![],
            hash_seed: None,
            stdio_unbuffered: false,
            check_hash_based_pycs: "default".to_owned(),
            allow_external_library: cfg!(feature = "importlib"),
        }
    }
}

#[cfg(feature = "flame-it")]
use std::ffi::OsString;

/// Struct containing all kind of settings for the python vm.
/// Mostly `PyConfig` in CPython.
#[non_exhaustive]
pub struct Settings {
    /// -I
    pub isolated: bool,

    // int use_environment
    /// -Xdev
    pub dev_mode: bool,

    /// Not set SIGINT handler(i.e. for embedded mode)
    pub install_signal_handlers: bool,

    /// PYTHONHASHSEED=x
    /// None means use_hash_seed = 0 in CPython
    pub hash_seed: Option<u32>,

    // int faulthandler;
    // int tracemalloc;
    // int perf_profiling;
    // int import_time;
    // int code_debug_ranges;
    // int show_ref_count;
    // int dump_refs;
    // wchar_t *dump_refs_file;
    // int malloc_stats;
    // wchar_t *filesystem_encoding;
    // wchar_t *filesystem_errors;
    // wchar_t *pycache_prefix;
    // int parse_argv;
    // PyWideStringList orig_argv;
    /// sys.argv
    pub argv: Vec<String>,

    /// -Xfoo[=bar]
    pub xoptions: Vec<(String, Option<String>)>,

    /// -Wfoo
    pub warnoptions: Vec<String>,

    /// -S
    pub import_site: bool,

    /// -b
    pub bytes_warning: u64,

    /// -X warn_default_encoding, PYTHONWARNDEFAULTENCODING
    pub warn_default_encoding: bool,

    /// -i
    pub inspect: bool,

    /// -i, with no script
    pub interactive: bool,

    // int optimization_level;
    // int parser_debug;
    /// -B
    pub write_bytecode: bool,

    /// verbosity level (-v switch)
    pub verbose: u8,

    /// -q
    pub quiet: bool,

    /// -s
    pub user_site_directory: bool,

    // int configure_c_stdio;
    /// -u, PYTHONUNBUFFERED=x
    pub buffered_stdio: bool,

    // wchar_t *stdio_encoding;
    pub utf8_mode: u8,
    // wchar_t *stdio_errors;
    /// --check-hash-based-pycs
    pub check_hash_pycs_mode: CheckHashPycsMode,

    // int use_frozen_modules;
    /// -P
    pub safe_path: bool,

    /// -X int_max_str_digits
    pub int_max_str_digits: i64,

    // /* --- Path configuration inputs ------------ */
    // int pathconfig_warnings;
    // wchar_t *program_name;
    /// Environment PYTHONPATH (and RUSTPYTHONPATH)
    pub path_list: Vec<String>,

    // wchar_t *home;
    // wchar_t *platlibdir;
    /// -d command line switch
    pub debug: u8,

    /// -O optimization switch counter
    pub optimize: u8,

    /// -E
    pub ignore_environment: bool,

    /// false for wasm. Not a command-line option
    pub allow_external_library: bool,

    #[cfg(feature = "flame-it")]
    pub profile_output: Option<OsString>,
    #[cfg(feature = "flame-it")]
    pub profile_format: Option<String>,
}

#[derive(Debug, Default, Copy, Clone, strum_macros::Display, strum_macros::EnumString)]
#[strum(serialize_all = "lowercase")]
pub enum CheckHashPycsMode {
    #[default]
    Default,
    Always,
    Never,
}

impl Settings {
    pub fn with_path(mut self, path: String) -> Self {
        self.path_list.push(path);
        self
    }
}

/// Sensible default settings.
impl Default for Settings {
    fn default() -> Self {
        Settings {
            debug: 0,
            inspect: false,
            interactive: false,
            optimize: 0,
            install_signal_handlers: true,
            user_site_directory: true,
            import_site: true,
            ignore_environment: false,
            verbose: 0,
            quiet: false,
            write_bytecode: true,
            safe_path: false,
            bytes_warning: 0,
            xoptions: vec![],
            isolated: false,
            dev_mode: false,
            warn_default_encoding: false,
            warnoptions: vec![],
            path_list: vec![],
            argv: vec![],
            hash_seed: None,
            buffered_stdio: true,
            check_hash_pycs_mode: CheckHashPycsMode::Default,
            allow_external_library: cfg!(feature = "importlib"),
            utf8_mode: 1,
            int_max_str_digits: 4300,
            #[cfg(feature = "flame-it")]
            profile_output: None,
            #[cfg(feature = "flame-it")]
            profile_format: None,
        }
    }
}

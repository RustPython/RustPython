use lexopt::Arg::*;
use lexopt::ValueExt;
use rustpython_vm::{Settings, vm::CheckHashPycsMode};
use std::str::FromStr;
use std::{cmp, env};

pub enum RunMode {
    Script(String),
    Command(String),
    Module(String),
    InstallPip(InstallPipMode),
    Repl,
}

pub enum InstallPipMode {
    /// Install pip using the ensurepip pip module. This has a higher chance of
    /// success, but may not install the latest version of pip.
    Ensurepip,
    /// Install pip using the get-pip.py script, which retrieves the latest pip version.
    /// This can be broken due to incompatibilities with cpython.
    GetPip,
}

impl FromStr for InstallPipMode {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "ensurepip" => Ok(Self::Ensurepip),
            "get-pip" => Ok(Self::GetPip),
            _ => Err("--install-pip takes ensurepip or get-pip as first argument"),
        }
    }
}

#[derive(Default)]
struct CliArgs {
    bytes_warning: u8,
    dont_write_bytecode: bool,
    debug: u8,
    ignore_environment: bool,
    inspect: bool,
    isolate: bool,
    optimize: u8,
    safe_path: bool,
    quiet: bool,
    random_hash_seed: bool,
    no_user_site: bool,
    no_site: bool,
    unbuffered: bool,
    verbose: u8,
    warning_control: Vec<String>,
    implementation_option: Vec<String>,
    check_hash_based_pycs: CheckHashPycsMode,

    #[cfg(feature = "flame-it")]
    profile_output: Option<std::ffi::OsString>,
    #[cfg(feature = "flame-it")]
    profile_format: Option<String>,
}

const USAGE_STRING: &str = "\
usage: {PROG} [option] ... [-c cmd | -m mod | file | -] [arg] ...
Options (and corresponding environment variables):
-b     : issue warnings about converting bytes/bytearray to str and comparing
         bytes/bytearray with str or bytes with int. (-bb: issue errors)
-B     : don't write .pyc files on import; also PYTHONDONTWRITEBYTECODE=x
-c cmd : program passed in as string (terminates option list)
-d     : turn on parser debugging output (for experts only, only works on
         debug builds); also PYTHONDEBUG=x
-E     : ignore PYTHON* environment variables (such as PYTHONPATH)
-h     : print this help message and exit (also -? or --help)
-i     : inspect interactively after running script; forces a prompt even
         if stdin does not appear to be a terminal; also PYTHONINSPECT=x
-I     : isolate Python from the user's environment (implies -E and -s)
-m mod : run library module as a script (terminates option list)
-O     : remove assert and __debug__-dependent statements; add .opt-1 before
         .pyc extension; also PYTHONOPTIMIZE=x
-OO    : do -O changes and also discard docstrings; add .opt-2 before
         .pyc extension
-P     : don't prepend a potentially unsafe path to sys.path; also
         PYTHONSAFEPATH
-q     : don't print version and copyright messages on interactive startup
-s     : don't add user site directory to sys.path; also PYTHONNOUSERSITE=x
-S     : don't imply 'import site' on initialization
-u     : force the stdout and stderr streams to be unbuffered;
         this option has no effect on stdin; also PYTHONUNBUFFERED=x
-v     : verbose (trace import statements); also PYTHONVERBOSE=x
         can be supplied multiple times to increase verbosity
-V     : print the Python version number and exit (also --version)
         when given twice, print more information about the build
-W arg : warning control; arg is action:message:category:module:lineno
         also PYTHONWARNINGS=arg
-x     : skip first line of source, allowing use of non-Unix forms of #!cmd
-X opt : set implementation-specific option
--check-hash-based-pycs always|default|never:
         control how Python invalidates hash-based .pyc files
--help-env: print help about Python environment variables and exit
--help-xoptions: print help about implementation-specific -X options and exit
--help-all: print complete help information and exit

RustPython extensions:


Arguments:
file   : program read from script file
-      : program read from stdin (default; interactive mode if a tty)
arg ...: arguments passed to program in sys.argv[1:]
";

fn parse_args() -> Result<(CliArgs, RunMode, Vec<String>), lexopt::Error> {
    let mut args = CliArgs::default();
    let mut parser = lexopt::Parser::from_env();
    fn argv(argv0: String, mut parser: lexopt::Parser) -> Result<Vec<String>, lexopt::Error> {
        std::iter::once(Ok(argv0))
            .chain(parser.raw_args()?.map(|arg| arg.string()))
            .collect()
    }
    while let Some(arg) = parser.next()? {
        match arg {
            Short('b') => args.bytes_warning += 1,
            Short('B') => args.dont_write_bytecode = true,
            Short('c') => {
                let cmd = parser.value()?.string()?;
                return Ok((args, RunMode::Command(cmd), argv("-c".to_owned(), parser)?));
            }
            Short('d') => args.debug += 1,
            Short('E') => args.ignore_environment = true,
            Short('h' | '?') | Long("help") => help(parser),
            Short('i') => args.inspect = true,
            Short('I') => args.isolate = true,
            Short('m') => {
                let module = parser.value()?.string()?;
                let argv = argv("PLACEHOLDER".to_owned(), parser)?;
                return Ok((args, RunMode::Module(module), argv));
            }
            Short('O') => args.optimize += 1,
            Short('P') => args.safe_path = true,
            Short('q') => args.quiet = true,
            Short('R') => args.random_hash_seed = true,
            Short('S') => args.no_site = true,
            Short('s') => args.no_user_site = true,
            Short('u') => args.unbuffered = true,
            Short('v') => args.verbose += 1,
            Short('V') | Long("version") => version(),
            Short('W') => args.warning_control.push(parser.value()?.string()?),
            // TODO: Short('x') =>
            Short('X') => args.implementation_option.push(parser.value()?.string()?),

            Long("check-hash-based-pycs") => {
                args.check_hash_based_pycs = parser.value()?.parse()?
            }

            // TODO: make these more specific
            Long("help-env") => help(parser),
            Long("help-xoptions") => help(parser),
            Long("help-all") => help(parser),

            #[cfg(feature = "flame-it")]
            Long("profile-output") => args.profile_output = Some(parser.value()?),
            #[cfg(feature = "flame-it")]
            Long("profile-format") => args.profile_format = Some(parser.value()?.string()?),

            Long("install-pip") => {
                let (mode, argv) = if let Some(val) = parser.optional_value() {
                    (val.parse()?, vec![val.string()?])
                } else if let Ok(argv0) = parser.value() {
                    let mode = argv0.parse()?;
                    (mode, argv(argv0.string()?, parser)?)
                } else {
                    (
                        InstallPipMode::Ensurepip,
                        ["ensurepip", "--upgrade", "--default-pip"]
                            .map(str::to_owned)
                            .into(),
                    )
                };
                return Ok((args, RunMode::InstallPip(mode), argv));
            }
            Value(script_name) => {
                let script_name = script_name.string()?;
                let mode = if script_name == "-" {
                    RunMode::Repl
                } else {
                    RunMode::Script(script_name.clone())
                };
                return Ok((args, mode, argv(script_name, parser)?));
            }
            _ => return Err(arg.unexpected()),
        }
    }
    Ok((args, RunMode::Repl, vec![]))
}

fn help(parser: lexopt::Parser) -> ! {
    let usage = USAGE_STRING.replace("{PROG}", parser.bin_name().unwrap_or("rustpython"));
    print!("{usage}");
    std::process::exit(0);
}

fn version() -> ! {
    println!("Python {}", rustpython_vm::version::get_version());
    std::process::exit(0);
}

/// Create settings by examining command line arguments and environment
/// variables.
pub fn parse_opts() -> Result<(Settings, RunMode), lexopt::Error> {
    let (args, mode, argv) = parse_args()?;

    let mut settings = Settings::default();
    settings.isolated = args.isolate;
    settings.ignore_environment = settings.isolated || args.ignore_environment;
    settings.bytes_warning = args.bytes_warning.into();
    settings.import_site = !args.no_site;

    let ignore_environment = settings.ignore_environment;

    if !ignore_environment {
        settings.path_list.extend(get_paths("RUSTPYTHONPATH"));
        settings.path_list.extend(get_paths("PYTHONPATH"));
    }

    // Now process command line flags:

    let get_env = |env| (!ignore_environment).then(|| env::var_os(env)).flatten();

    let env_count = |env| {
        get_env(env).filter(|v| !v.is_empty()).map_or(0, |val| {
            val.to_str().and_then(|v| v.parse::<u8>().ok()).unwrap_or(1)
        })
    };

    settings.optimize = cmp::max(args.optimize, env_count("PYTHONOPTIMIZE"));
    settings.verbose = cmp::max(args.verbose, env_count("PYTHONVERBOSE"));
    settings.debug = cmp::max(args.debug, env_count("PYTHONDEBUG"));

    let env_bool = |env| get_env(env).is_some_and(|v| !v.is_empty());

    settings.user_site_directory =
        !(settings.isolated || args.no_user_site || env_bool("PYTHONNOUSERSITE"));
    settings.quiet = args.quiet;
    settings.write_bytecode = !(args.dont_write_bytecode || env_bool("PYTHONDONTWRITEBYTECODE"));
    settings.safe_path = settings.isolated || args.safe_path || env_bool("PYTHONSAFEPATH");
    settings.inspect = args.inspect || env_bool("PYTHONINSPECT");
    settings.interactive = args.inspect;
    settings.buffered_stdio = !args.unbuffered;

    if let Some(val) = get_env("PYTHONINTMAXSTRDIGITS") {
        settings.int_max_str_digits = match val.to_str().and_then(|s| s.parse().ok()) {
            Some(digits @ (0 | 640..)) => digits,
            _ => {
                error!(
                    "Fatal Python error: config_init_int_max_str_digits: PYTHONINTMAXSTRDIGITS: invalid limit; must be >= 640 or 0 for unlimited.\nPython runtime state: preinitialized"
                );
                std::process::exit(1);
            }
        };
    }

    settings.check_hash_pycs_mode = args.check_hash_based_pycs;

    let xopts = args.implementation_option.into_iter().map(|s| {
        let (name, value) = match s.split_once('=') {
            Some((name, value)) => (name.to_owned(), Some(value)),
            None => (s, None),
        };
        match &*name {
            "dev" => settings.dev_mode = true,
            "faulthandler" => settings.faulthandler = true,
            "warn_default_encoding" => settings.warn_default_encoding = true,
            "no_sig_int" => settings.install_signal_handlers = false,
            "no_debug_ranges" => settings.code_debug_ranges = false,
            "int_max_str_digits" => {
                settings.int_max_str_digits = match value.unwrap().parse() {
                    Ok(digits) if digits == 0 || digits >= 640 => digits,
                    _ => {
                        error!(
                            "Fatal Python error: config_init_int_max_str_digits: \
                             -X int_max_str_digits: \
                             invalid limit; must be >= 640 or 0 for unlimited.\n\
                             Python runtime state: preinitialized"
                        );
                        std::process::exit(1);
                    }
                };
            }
            "thread_inherit_context" => {
                settings.thread_inherit_context = match value {
                    Some("1") => true,
                    Some("0") => false,
                    _ => {
                        error!(
                            "Fatal Python error: config_init_thread_inherit_context: \
                             -X thread_inherit_context=n: n is missing or invalid\n\
                             Python runtime state: preinitialized"
                        );
                        std::process::exit(1);
                    }
                };
            }
            _ => {}
        }
        (name, value.map(str::to_owned))
    });
    settings.xoptions.extend(xopts);

    settings.warn_default_encoding =
        settings.warn_default_encoding || env_bool("PYTHONWARNDEFAULTENCODING");
    settings.faulthandler = settings.faulthandler || env_bool("PYTHONFAULTHANDLER");
    if env_bool("PYTHONNODEBUGRANGES") {
        settings.code_debug_ranges = false;
    }
    if let Some(val) = get_env("PYTHON_THREAD_INHERIT_CONTEXT") {
        settings.thread_inherit_context = match val.to_str() {
            Some("1") => true,
            Some("0") => false,
            _ => {
                error!(
                    "Fatal Python error: config_init_thread_inherit_context: \
                     PYTHON_THREAD_INHERIT_CONTEXT=N: N is missing or invalid\n\
                     Python runtime state: preinitialized"
                );
                std::process::exit(1);
            }
        };
    }

    // Parse PYTHONIOENCODING=encoding[:errors]
    if let Some(val) = get_env("PYTHONIOENCODING")
        && let Some(val_str) = val.to_str()
        && !val_str.is_empty()
    {
        if let Some((enc, err)) = val_str.split_once(':') {
            if !enc.is_empty() {
                settings.stdio_encoding = Some(enc.to_owned());
            }
            if !err.is_empty() {
                settings.stdio_errors = Some(err.to_owned());
            }
        } else {
            settings.stdio_encoding = Some(val_str.to_owned());
        }
    }

    if settings.dev_mode {
        settings.warnoptions.push("default".to_owned());
        settings.faulthandler = true;
    }
    if settings.bytes_warning > 0 {
        let warn = if settings.bytes_warning > 1 {
            "error::BytesWarning"
        } else {
            "default::BytesWarning"
        };
        settings.warnoptions.push(warn.to_owned());
    }
    settings.warnoptions.extend(args.warning_control);

    settings.hash_seed = match (!args.random_hash_seed)
        .then(|| get_env("PYTHONHASHSEED"))
        .flatten()
    {
        Some(s) if s == "random" || s.is_empty() => None,
        Some(s) => {
            let seed = s.parse_with(|s| {
                s.parse::<u32>().map_err(|_| {
                    "Fatal Python init error: PYTHONHASHSEED must be \
                    \"random\" or an integer in range [0; 4294967295]"
                })
            })?;
            Some(seed)
        }
        None => None,
    };

    settings.argv = argv;

    #[cfg(feature = "flame-it")]
    {
        settings.profile_output = args.profile_output;
        settings.profile_format = args.profile_format;
    }

    Ok((settings, mode))
}

/// Helper function to retrieve a sequence of paths from an environment variable.
fn get_paths(env_variable_name: &str) -> impl Iterator<Item = String> + '_ {
    env::var_os(env_variable_name)
        .filter(|v| !v.is_empty())
        .into_iter()
        .flat_map(move |paths| {
            split_paths(&paths)
                .map(|path| {
                    path.into_os_string()
                        .into_string()
                        .unwrap_or_else(|_| panic!("{env_variable_name} isn't valid unicode"))
                })
                .collect::<Vec<_>>()
        })
}

#[cfg(not(target_os = "wasi"))]
pub(crate) use env::split_paths;
#[cfg(target_os = "wasi")]
pub(crate) fn split_paths<T: AsRef<std::ffi::OsStr> + ?Sized>(
    s: &T,
) -> impl Iterator<Item = std::path::PathBuf> + '_ {
    use std::os::wasi::ffi::OsStrExt;
    let s = s.as_ref().as_bytes();
    s.split(|b| *b == b':')
        .map(|x| std::ffi::OsStr::from_bytes(x).to_owned().into())
}

use clap::{
    builder::TypedValueParser,
    Arg,
    ArgAction::{Append, Count, Set, SetTrue},
    ArgMatches, Command,
};
use rustpython_vm::vm::{CheckHashPycsMode, Settings};
use std::env;

pub enum RunMode {
    ScriptInteractive(Option<String>, bool),
    Command(String),
    Module(String),
    InstallPip(InstallPipMode),
}

pub enum InstallPipMode {
    EnsurePip,
    GetPip,
}

pub fn opts_with_clap() -> (Settings, RunMode) {
    let app = Command::new("RustPython");
    let matches = parse_arguments(app);
    settings_from(&matches)
}

fn parse_arguments(app: Command) -> ArgMatches {
    let app = app
        .version(env!("CARGO_PKG_VERSION"))
        .author(env!("CARGO_PKG_AUTHORS"))
        .about("Rust implementation of the Python language")
        .override_usage("rustpython [OPTIONS] [-c CMD | -m MODULE | FILE] [PYARGS]...")
        .args_override_self(true)
        .arg(
            Arg::new("script")
                .required(false)
                .action(Append)
                .num_args(0..)
                .value_name("script, args")
                .trailing_var_arg(true),
        )
        .arg(
            Arg::new("c")
                .short('c')
                .action(Append)
                .allow_hyphen_values(true)
                .num_args(1..)
                .value_name("cmd, args")
                .help("run the given string as a program")
                .trailing_var_arg(true),
        )
        .arg(
            Arg::new("m")
                .short('m')
                .action(Append)
                .allow_hyphen_values(true)
                .num_args(1..)
                .value_name("module, args")
                .help("run library module as script")
                .trailing_var_arg(true),
        )
        .arg(
            Arg::new("install_pip")
                .long("install-pip")
                .action(Append)
                .allow_hyphen_values(true)
                .num_args(0..)
                .value_name("get-pip args")
                .help(
                    "install the pip package manager for rustpython; \
                        requires rustpython be build with the ssl feature enabled.",
                )
                .trailing_var_arg(true),
        )
        .arg(
            Arg::new("optimize")
                .short('O')
                .action(Count)
                .help("Optimize. Set __debug__ to false. Remove debug statements."),
        )
        .arg(
            Arg::new("verbose")
                .short('v')
                .action(Count)
                .help("Give the verbosity (can be applied multiple times)"),
        )
        .arg(
            Arg::new("debug")
                .short('d')
                .action(Count)
                .help("Debug the parser."),
        )
        .arg(
            Arg::new("quiet")
                .short('q')
                .action(SetTrue)
                .help("Be quiet at startup."),
        )
        .arg(
            Arg::new("inspect")
                .short('i')
                .action(SetTrue)
                .help("Inspect interactively after running the script."),
        )
        .arg(
            Arg::new("no-user-site")
                .short('s')
                .action(SetTrue)
                .help("don't add user site directory to sys.path."),
        )
        .arg(
            Arg::new("no-site")
                .short('S')
                .action(SetTrue)
                .help("don't imply 'import site' on initialization"),
        )
        .arg(
            Arg::new("dont-write-bytecode")
                .short('B')
                .action(SetTrue)
                .help("don't write .pyc files on import"),
        )
        .arg(
            Arg::new("safe-path")
                .short('P')
                .action(SetTrue)
                .help("don’t prepend a potentially unsafe path to sys.path"),
        )
        .arg(
            Arg::new("ignore-environment")
                .short('E')
                .action(SetTrue)
                .help("Ignore environment variables PYTHON* such as PYTHONPATH"),
        )
        .arg(
            Arg::new("isolate")
                .short('I')
                .action(SetTrue)
                .help("isolate Python from the user's environment (implies -E and -s)"),
        )
        .arg(
            Arg::new("implementation-option")
                .short('X')
                .action(Append)
                .num_args(1)
                .help("set implementation-specific option"),
        )
        .arg(
            Arg::new("warning-control")
                .short('W')
                .action(Append)
                .num_args(1)
                .help("warning control; arg is action:message:category:module:lineno"),
        )
        .arg(
            Arg::new("check-hash-based-pycs")
                .long("check-hash-based-pycs")
                .action(Set)
                .num_args(1)
                .value_parser(
                    clap::builder::PossibleValuesParser::new(["always", "default", "never"])
                        .map(|x| x.parse::<CheckHashPycsMode>().unwrap()),
                )
                .help("control how Python invalidates hash-based .pyc files"),
        )
        .arg(Arg::new("bytes-warning").short('b').action(Count).help(
            "issue warnings about using bytes where strings \
                are usually expected (-bb: issue errors)",
        ))
        .arg(Arg::new("unbuffered").short('u').action(SetTrue).help(
            "force the stdout and stderr streams to be unbuffered; \
                        this option has no effect on stdin; also PYTHONUNBUFFERED=x",
        ));
    #[cfg(feature = "flame-it")]
    let app = app
        .arg(
            Arg::new("profile_output")
                .long("profile-output")
                .action(Set)
                .help("the file to output the profiling information to"),
        )
        .arg(
            Arg::new("profile_format")
                .long("profile-format")
                .action(Set)
                .help("the profile format to output the profiling information in"),
        );
    app.get_matches()
}

/// Create settings by examining command line arguments and environment
/// variables.
fn settings_from(matches: &ArgMatches) -> (Settings, RunMode) {
    let mut settings = Settings::default();
    settings.isolated = matches.get_flag("isolate");
    let ignore_environment = settings.isolated || matches.get_flag("ignore-environment");
    settings.ignore_environment = ignore_environment;
    settings.interactive = !matches.contains_id("c")
        && !matches.contains_id("m")
        && (!matches.contains_id("script") || matches.get_flag("inspect"));
    settings.bytes_warning = matches.get_count("bytes-warning").into();
    settings.import_site = !matches.get_flag("no-site");

    if !ignore_environment {
        settings.path_list.extend(get_paths("RUSTPYTHONPATH"));
        settings.path_list.extend(get_paths("PYTHONPATH"));
    }

    // Now process command line flags:

    let count_flag = |arg, env| {
        let mut val = matches.get_count(arg);
        if !ignore_environment {
            if let Some(value) = get_env_var_value(env) {
                val = std::cmp::max(val, value);
            }
        }
        val
    };

    settings.optimize = count_flag("optimize", "PYTHONOPTIMIZE");
    settings.verbose = count_flag("verbose", "PYTHONVERBOSE");
    settings.debug = count_flag("debug", "PYTHONDEBUG");

    let bool_env_var = |env| !ignore_environment && env::var_os(env).is_some_and(|v| !v.is_empty());
    let bool_flag = |arg, env| matches.get_flag(arg) || bool_env_var(env);

    settings.user_site_directory =
        !(settings.isolated || bool_flag("no-user-site", "PYTHONNOUSERSITE"));
    settings.quiet = matches.get_flag("quiet");
    settings.write_bytecode = !bool_flag("dont-write-bytecode", "PYTHONDONTWRITEBYTECODE");
    settings.safe_path = settings.isolated || bool_flag("safe-path", "PYTHONSAFEPATH");
    settings.inspect = bool_flag("inspect", "PYTHONINSPECT");

    if !ignore_environment && env::var_os("PYTHONINTMAXSTRDIGITS").is_some() {
        settings.int_max_str_digits = match env::var("PYTHONINTMAXSTRDIGITS").unwrap().parse() {
            Ok(digits) if digits == 0 || digits >= 640 => digits,
            _ => {
                error!("Fatal Python error: config_init_int_max_str_digits: PYTHONINTMAXSTRDIGITS: invalid limit; must be >= 640 or 0 for unlimited.\nPython runtime state: preinitialized");
                std::process::exit(1);
            }
        };
    }

    settings.check_hash_pycs_mode = matches
        .get_one("check-hash-based-pycs")
        .copied()
        .unwrap_or_default();

    let xopts = matches
        .get_many::<String>("implementation-option")
        .unwrap_or_default()
        .map(|s| {
            let (name, value) = s.split_once('=').unzip();
            let name = name.unwrap_or(s);
            match name {
                "dev" => settings.dev_mode = true,
                "warn_default_encoding" => settings.warn_default_encoding = true,
                "no_sig_int" => settings.install_signal_handlers = false,
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
                _ => {}
            }
            (name.to_owned(), value.map(str::to_owned))
        });
    settings.xoptions.extend(xopts);

    settings.warn_default_encoding |= bool_env_var("PYTHONWARNDEFAULTENCODING");

    if settings.dev_mode {
        settings.warnoptions.push("default".to_owned())
    }
    if settings.bytes_warning > 0 {
        let warn = if settings.bytes_warning > 1 {
            "error::BytesWarning"
        } else {
            "default::BytesWarning"
        };
        settings.warnoptions.push(warn.to_owned());
    }
    if let Some(warnings) = matches.get_many::<String>("warning-control") {
        settings.warnoptions.extend(warnings.cloned());
    }

    // script having values even though -c/-m was passed would means that it was something like -mmodule foo bar
    let weird_script_args = matches
        .get_many::<String>("script")
        .unwrap_or_default()
        .cloned();

    let (mode, argv) = if let Some(mut cmd) = matches.get_many::<String>("c") {
        let command = cmd.next().expect("clap ensure this exists");
        let argv = std::iter::once("-c".to_owned())
            .chain(cmd.cloned())
            .chain(weird_script_args)
            .collect();
        (RunMode::Command(command.to_owned()), argv)
    } else if let Some(mut cmd) = matches.get_many::<String>("m") {
        let module = cmd.next().expect("clap ensure this exists");
        let argv = std::iter::once("PLACEHOLDER".to_owned())
            .chain(cmd.cloned())
            .chain(weird_script_args)
            .collect();
        (RunMode::Module(module.to_owned()), argv)
    } else if let Some(get_pip_args) = matches.get_many::<String>("install_pip") {
        settings.isolated = true;
        let mut args: Vec<_> = get_pip_args.cloned().collect();
        if args.is_empty() {
            args.extend(["ensurepip", "--upgrade", "--default-pip"].map(str::to_owned));
        }
        let mode = match &*args[0] {
            "ensurepip" => InstallPipMode::EnsurePip,
            "get-pip" => InstallPipMode::GetPip,
            _ => panic!("--install-pip takes ensurepip or get-pip as first argument"),
        };
        (RunMode::InstallPip(mode), args)
    } else if let Some(argv) = matches.get_many::<String>("script") {
        let argv: Vec<_> = argv.cloned().collect();
        let script = argv[0].clone();
        (
            RunMode::ScriptInteractive(Some(script), matches.get_flag("inspect")),
            argv,
        )
    } else {
        (RunMode::ScriptInteractive(None, true), vec!["".to_owned()])
    };

    let hash_seed = match env::var("PYTHONHASHSEED") {
        Ok(s) if s == "random" => Some(None),
        Ok(s) => s.parse::<u32>().ok().map(Some),
        Err(_) => Some(None),
    };
    settings.hash_seed = hash_seed.unwrap_or_else(|| {
        error!("Fatal Python init error: PYTHONHASHSEED must be \"random\" or an integer in range [0; 4294967295]");
        // TODO: Need to change to ExitCode or Termination
        std::process::exit(1)
    });

    settings.argv = argv;

    (settings, mode)
}

/// Get environment variable and turn it into integer.
fn get_env_var_value(name: &str) -> Option<u8> {
    env::var_os(name).filter(|v| !v.is_empty()).map(|value| {
        value
            .to_str()
            .and_then(|v| v.parse::<u8>().ok())
            .unwrap_or(1)
    })
}

/// Helper function to retrieve a sequence of paths from an environment variable.
fn get_paths(env_variable_name: &str) -> impl Iterator<Item = String> + '_ {
    env::var_os(env_variable_name)
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

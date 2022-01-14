use clap::{
    Arg,
    ArgAction::{Append, Count, SetTrue},
    ArgMatches, Command,
};
use rustpython_vm::Settings;
use std::{env, str::FromStr};

pub enum RunMode {
    ScriptInteractive(Option<String>, bool),
    Command(String),
    Module(String),
    InstallPip(String),
}

pub fn opts_with_clap() -> (Settings, RunMode) {
    let app = Command::new("RustPython");
    let matches = parse_arguments(app);
    settings_from(&matches)
}

fn parse_arguments(app: Command<'_>) -> ArgMatches {
    let app = app
        .trailing_var_arg(true)
        .version(env!("CARGO_PKG_VERSION"))
        .author(env!("CARGO_PKG_AUTHORS"))
        .about("Rust implementation of the Python language")
        .override_usage("rustpython [OPTIONS] [-c CMD | -m MODULE | FILE] [PYARGS]...")
        .arg(
            Arg::new("script")
                .required(false)
                .multiple_values(true)
                .value_name("script, args")
                .min_values(0),
        )
        .arg(
            Arg::new("c")
                .short('c')
                .takes_value(true)
                .allow_hyphen_values(true)
                .multiple_values(true)
                .value_name("cmd, args")
                .min_values(1)
                .help("run the given string as a program"),
        )
        .arg(
            Arg::new("m")
                .short('m')
                .takes_value(true)
                .allow_hyphen_values(true)
                .multiple_values(true)
                .value_name("module, args")
                .min_values(1)
                .help("run library module as script"),
        )
        .arg(
            Arg::new("install_pip")
                .long("install-pip")
                .takes_value(true)
                .allow_hyphen_values(true)
                .multiple_values(true)
                .value_name("get-pip args")
                .min_values(0)
                .help(
                    "install the pip package manager for rustpython; \
                        requires rustpython be build with the ssl feature enabled.",
                ),
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
                .action(SetTrue)
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
                .takes_value(true)
                .action(Append)
                .number_of_values(1)
                .help("set implementation-specific option"),
        )
        .arg(
            Arg::new("warning-control")
                .short('W')
                .takes_value(true)
                .action(Append)
                .number_of_values(1)
                .help("warning control; arg is action:message:category:module:lineno"),
        )
        .arg(
            Arg::new("check-hash-based-pycs")
                .long("check-hash-based-pycs")
                .takes_value(true)
                .number_of_values(1)
                .default_value("default")
                .help("always|default|never\ncontrol how Python invalidates hash-based .pyc files"),
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
                .takes_value(true)
                .help("the file to output the profiling information to"),
        )
        .arg(
            Arg::new("profile_format")
                .long("profile-format")
                .takes_value(true)
                .help("the profile format to output the profiling information in"),
        );
    app.get_matches()
}

/// Create settings by examining command line arguments and environment
/// variables.
fn settings_from(matches: &ArgMatches) -> (Settings, RunMode) {
    let mut settings = Settings::default();
    settings.isolated = matches.get_flag("isolate");
    settings.ignore_environment = matches.get_flag("ignore-environment");
    settings.interactive = !matches.contains_id("c")
        && !matches.contains_id("m")
        && (!matches.contains_id("script") || matches.get_flag("inspect"));
    settings.bytes_warning = matches.get_count("bytes-warning").into();
    settings.import_site = !matches.get_flag("no-site");

    let ignore_environment = settings.ignore_environment || settings.isolated;

    if !ignore_environment {
        settings.path_list.extend(get_paths("RUSTPYTHONPATH"));
        settings.path_list.extend(get_paths("PYTHONPATH"));
    }

    // Now process command line flags:
    if matches.get_flag("debug") || (!ignore_environment && env::var_os("PYTHONDEBUG").is_some()) {
        settings.debug = true;
    }

    if matches.get_flag("inspect")
        || (!ignore_environment && env::var_os("PYTHONINSPECT").is_some())
    {
        settings.inspect = true;
    }

    if matches.contains_id("optimize") {
        settings.optimize = matches.get_count("optimize").try_into().unwrap();
    } else if !ignore_environment {
        if let Ok(value) = get_env_var_value("PYTHONOPTIMIZE") {
            settings.optimize = value;
        }
    }

    if matches.contains_id("verbose") {
        settings.verbose = matches.get_count("verbose").try_into().unwrap();
    } else if !ignore_environment {
        if let Ok(value) = get_env_var_value("PYTHONVERBOSE") {
            settings.verbose = value;
        }
    }

    if matches.get_flag("no-user-site")
        || matches.get_flag("isolate")
        || (!ignore_environment && env::var_os("PYTHONNOUSERSITE").is_some())
    {
        settings.user_site_directory = false;
    }

    if matches.get_flag("quiet") {
        settings.quiet = true;
    }

    if matches.get_flag("dont-write-bytecode")
        || (!ignore_environment && env::var_os("PYTHONDONTWRITEBYTECODE").is_some())
    {
        settings.write_bytecode = false;
    }
    if !ignore_environment && env::var_os("PYTHONINTMAXSTRDIGITS").is_some() {
        settings.int_max_str_digits = match env::var("PYTHONINTMAXSTRDIGITS").unwrap().parse() {
            Ok(digits) if digits == 0 || digits >= 640 => digits,
            _ => {
                error!("Fatal Python error: config_init_int_max_str_digits: PYTHONINTMAXSTRDIGITS: invalid limit; must be >= 640 or 0 for unlimited.\nPython runtime state: preinitialized");
                std::process::exit(1);
            }
        };
    }

    if matches.get_flag("safe-path")
        || (!ignore_environment && env::var_os("PYTHONSAFEPATH").is_some())
    {
        settings.safe_path = true;
    }

    matches
        .get_one::<String>("check-hash-based-pycs")
        .unwrap()
        .clone_into(&mut settings.check_hash_pycs_mode);

    let mut dev_mode = false;
    let mut warn_default_encoding = false;
    if let Some(xopts) = matches.get_many::<String>("implementation-option") {
        settings.xoptions.extend(xopts.map(|s| {
            let mut parts = s.splitn(2, '=');
            let name = parts.next().unwrap().to_owned();
            let value = parts.next().map(ToOwned::to_owned);
            if name == "dev" {
                dev_mode = true
            }
            if name == "warn_default_encoding" {
                warn_default_encoding = true
            }
            if name == "no_sig_int" {
                settings.install_signal_handlers = false;
            }
            if name == "int_max_str_digits" {
                settings.int_max_str_digits = match value.as_ref().unwrap().parse() {
                    Ok(digits) if digits == 0 || digits >= 640 => digits,
                    _ => {

                    error!("Fatal Python error: config_init_int_max_str_digits: -X int_max_str_digits: invalid limit; must be >= 640 or 0 for unlimited.\nPython runtime state: preinitialized");
                    std::process::exit(1);
                    },
                };
            }
            (name, value)
        }));
    }
    settings.dev_mode = dev_mode;
    if warn_default_encoding
        || (!ignore_environment && env::var_os("PYTHONWARNDEFAULTENCODING").is_some())
    {
        settings.warn_default_encoding = true;
    }

    if dev_mode {
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
            args.push("ensurepip".to_owned());
            args.push("--upgrade".to_owned());
            args.push("--default-pip".to_owned());
        }
        let installer = args[0].clone();
        let mode = match installer.as_str() {
            "ensurepip" | "get-pip" => RunMode::InstallPip(installer),
            _ => panic!("--install-pip takes ensurepip or get-pip as first argument"),
        };
        (mode, args)
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
fn get_env_var_value(name: &str) -> Result<u8, std::env::VarError> {
    env::var(name).map(|value| u8::from_str(&value).unwrap_or(1))
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

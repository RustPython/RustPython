use clap::{App, AppSettings, Arg, ArgMatches};
use rustpython_vm::Settings;
use std::env;

pub enum RunMode {
    Script(String),
    Command(String),
    Module(String),
    InstallPip(InstallPipMode),
    Repl,
}

pub enum InstallPipMode {
    Ensurepip,
    GetPip,
}

pub fn opts_with_clap() -> (Settings, RunMode) {
    let app = App::new("RustPython");
    let matches = parse_arguments(app);
    settings_from(&matches)
}

fn parse_arguments<'a>(app: App<'a, '_>) -> ArgMatches<'a> {
    let app = app
        .setting(AppSettings::TrailingVarArg)
        .version(crate_version!())
        .author(crate_authors!())
        .about("Rust implementation of the Python language")
        .usage("rustpython [OPTIONS] [-c CMD | -m MODULE | FILE] [PYARGS]...")
        .arg(
            Arg::with_name("script")
                .required(false)
                .allow_hyphen_values(true)
                .multiple(true)
                .value_name("script, args")
                .min_values(1),
        )
        .arg(
            Arg::with_name("c")
                .short("c")
                .takes_value(true)
                .allow_hyphen_values(true)
                .multiple(true)
                .value_name("cmd, args")
                .min_values(1)
                .help("run the given string as a program"),
        )
        .arg(
            Arg::with_name("m")
                .short("m")
                .takes_value(true)
                .allow_hyphen_values(true)
                .multiple(true)
                .value_name("module, args")
                .min_values(1)
                .help("run library module as script"),
        )
        .arg(
            Arg::with_name("install_pip")
                .long("install-pip")
                .takes_value(true)
                .allow_hyphen_values(true)
                .multiple(true)
                .value_name("get-pip args")
                .min_values(0)
                .help(
                    "install the pip package manager for rustpython; \
                     requires rustpython be build with the ssl feature enabled.",
                ),
        )
        .arg(
            Arg::with_name("optimize")
                .short("O")
                .multiple(true)
                .help("Optimize. Set __debug__ to false. Remove debug statements."),
        )
        .arg(
            Arg::with_name("verbose")
                .short("v")
                .multiple(true)
                .help("Give the verbosity (can be applied multiple times)"),
        )
        .arg(
            Arg::with_name("debug")
                .short("d")
                .multiple(true)
                .help("Debug the parser."),
        )
        .arg(
            Arg::with_name("quiet")
                .short("q")
                .multiple(true)
                .help("Be quiet at startup."),
        )
        .arg(
            Arg::with_name("inspect")
                .short("i")
                .multiple(true)
                .help("Inspect interactively after running the script."),
        )
        .arg(
            Arg::with_name("no-user-site")
                .short("s")
                .multiple(true)
                .help("don't add user site directory to sys.path."),
        )
        .arg(
            Arg::with_name("no-site")
                .short("S")
                .multiple(true)
                .help("don't imply 'import site' on initialization"),
        )
        .arg(
            Arg::with_name("dont-write-bytecode")
                .short("B")
                .multiple(true)
                .help("don't write .pyc files on import"),
        )
        .arg(
            Arg::with_name("safe-path")
                .short("P")
                .multiple(true)
                .help("don’t prepend a potentially unsafe path to sys.path"),
        )
        .arg(
            Arg::with_name("ignore-environment")
                .short("E")
                .multiple(true)
                .help("Ignore environment variables PYTHON* such as PYTHONPATH"),
        )
        .arg(
            Arg::with_name("isolate")
                .short("I")
                .multiple(true)
                .help("isolate Python from the user's environment (implies -E and -s)"),
        )
        .arg(
            Arg::with_name("implementation-option")
                .short("X")
                .takes_value(true)
                .multiple(true)
                .number_of_values(1)
                .help("set implementation-specific option"),
        )
        .arg(
            Arg::with_name("warning-control")
                .short("W")
                .takes_value(true)
                .multiple(true)
                .number_of_values(1)
                .help("warning control; arg is action:message:category:module:lineno"),
        )
        .arg(
            Arg::with_name("check-hash-based-pycs")
                .long("check-hash-based-pycs")
                .takes_value(true)
                .number_of_values(1)
                .possible_values(&["always", "default", "never"])
                .help("control how Python invalidates hash-based .pyc files"),
        )
        .arg(
            Arg::with_name("bytes-warning")
                .short("b")
                .multiple(true)
                .help(
                    "issue warnings about using bytes where strings \
                     are usually expected (-bb: issue errors)",
                ),
        )
        .arg(Arg::with_name("unbuffered").short("u").multiple(true).help(
            "force the stdout and stderr streams to be unbuffered; \
             this option has no effect on stdin; also PYTHONUNBUFFERED=x",
        ));
    #[cfg(feature = "flame-it")]
    let app = app
        .arg(
            Arg::with_name("profile_output")
                .long("profile-output")
                .takes_value(true)
                .help("the file to output the profiling information to"),
        )
        .arg(
            Arg::with_name("profile_format")
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
    settings.isolated = matches.is_present("isolate");
    let ignore_environment = settings.isolated || matches.is_present("ignore-environment");
    settings.ignore_environment = ignore_environment;
    settings.interactive = !matches.is_present("c")
        && !matches.is_present("m")
        && (!matches.is_present("script") || matches.is_present("inspect"));
    settings.bytes_warning = matches.occurrences_of("bytes-warning");
    settings.import_site = !matches.is_present("no-site");

    if !ignore_environment {
        settings.path_list.extend(get_paths("RUSTPYTHONPATH"));
        settings.path_list.extend(get_paths("PYTHONPATH"));
    }

    // Now process command line flags:

    let count_flag = |arg, env| {
        let mut val = matches.occurrences_of(arg) as u8;
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
    let bool_flag = |arg, env| matches.is_present(arg) || bool_env_var(env);

    settings.user_site_directory =
        !(settings.isolated || bool_flag("no-user-site", "PYTHONNOUSERSITE"));
    settings.quiet = matches.is_present("quiet");
    settings.write_bytecode = !bool_flag("dont-write-bytecode", "PYTHONDONTWRITEBYTECODE");
    settings.safe_path = settings.isolated || bool_flag("safe-path", "PYTHONSAFEPATH");
    settings.inspect = bool_flag("inspect", "PYTHONINSPECT");
    settings.buffered_stdio = !bool_flag("unbuffered", "PYTHONUNBUFFERED");

    if !ignore_environment && env::var_os("PYTHONINTMAXSTRDIGITS").is_some() {
        settings.int_max_str_digits = match env::var("PYTHONINTMAXSTRDIGITS").unwrap().parse() {
            Ok(digits @ (0 | 640..)) => digits,
            _ => {
                error!("Fatal Python error: config_init_int_max_str_digits: PYTHONINTMAXSTRDIGITS: invalid limit; must be >= 640 or 0 for unlimited.\nPython runtime state: preinitialized");
                std::process::exit(1);
            }
        };
    }

    settings.check_hash_pycs_mode = matches
        .value_of("check-hash-based-pycs")
        .map(|val| val.parse().unwrap())
        .unwrap_or_default();

    let xopts = matches
        .values_of("implementation-option")
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
    if let Some(warnings) = matches.values_of("warning-control") {
        settings.warnoptions.extend(warnings.map(ToOwned::to_owned));
    }

    let (mode, argv) = if let Some(mut cmd) = matches.values_of("c") {
        let command = cmd.next().expect("clap ensure this exists");
        let argv = std::iter::once("-c".to_owned())
            .chain(cmd.map(ToOwned::to_owned))
            .collect();
        (RunMode::Command(command.to_owned()), argv)
    } else if let Some(mut cmd) = matches.values_of("m") {
        let module = cmd.next().expect("clap ensure this exists");
        let argv = std::iter::once("PLACEHOLDER".to_owned())
            .chain(cmd.map(ToOwned::to_owned))
            .collect();
        (RunMode::Module(module.to_owned()), argv)
    } else if let Some(get_pip_args) = matches.values_of("install_pip") {
        settings.isolated = true;
        let mut args: Vec<_> = get_pip_args.map(ToOwned::to_owned).collect();
        if args.is_empty() {
            args.extend(["ensurepip", "--upgrade", "--default-pip"].map(str::to_owned));
        }
        let mode = match &*args[0] {
            "ensurepip" => InstallPipMode::Ensurepip,
            "get-pip" => InstallPipMode::GetPip,
            _ => panic!("--install-pip takes ensurepip or get-pip as first argument"),
        };
        (RunMode::InstallPip(mode), args)
    } else if let Some(argv) = matches.values_of("script") {
        let argv: Vec<_> = argv.map(ToOwned::to_owned).collect();
        let script = argv[0].clone();
        (RunMode::Script(script), argv)
    } else {
        (RunMode::Repl, vec!["".to_owned()])
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

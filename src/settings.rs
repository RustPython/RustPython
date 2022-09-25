use clap::{App, AppSettings, Arg, ArgMatches};
use rustpython_vm::Settings;
use std::{env, str::FromStr};

pub enum RunMode {
    ScriptInteractive(Option<String>, bool),
    Command(String),
    Module(String),
    InstallPip(String),
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
                .help("install the pip package manager for rustpython; \
                        requires rustpython be build with the ssl feature enabled."
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
        .arg(Arg::with_name("debug").short("d").help("Debug the parser."))
        .arg(
            Arg::with_name("quiet")
                .short("q")
                .help("Be quiet at startup."),
        )
        .arg(
            Arg::with_name("inspect")
                .short("i")
                .help("Inspect interactively after running the script."),
        )
        .arg(
            Arg::with_name("no-user-site")
                .short("s")
                .help("don't add user site directory to sys.path."),
        )
        .arg(
            Arg::with_name("no-site")
                .short("S")
                .help("don't imply 'import site' on initialization"),
        )
        .arg(
            Arg::with_name("dont-write-bytecode")
                .short("B")
                .help("don't write .pyc files on import"),
        )
        .arg(
            Arg::with_name("ignore-environment")
                .short("E")
                .help("Ignore environment variables PYTHON* such as PYTHONPATH"),
        )
        .arg(
            Arg::with_name("isolate")
                .short("I")
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
                .default_value("default")
                .help("always|default|never\ncontrol how Python invalidates hash-based .pyc files"),
        )
        .arg(
            Arg::with_name("bytes-warning")
                .short("b")
                .multiple(true)
                .help("issue warnings about using bytes where strings are usually expected (-bb: issue errors)"),
        ).arg(
            Arg::with_name("unbuffered")
                .short("u")
                .help(
                    "force the stdout and stderr streams to be unbuffered; \
                        this option has no effect on stdin; also PYTHONUNBUFFERED=x",
                ),
        );
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
    settings.ignore_environment = matches.is_present("ignore-environment");
    settings.interactive = !matches.is_present("c")
        && !matches.is_present("m")
        && (!matches.is_present("script") || matches.is_present("inspect"));
    settings.bytes_warning = matches.occurrences_of("bytes-warning");
    settings.no_site = matches.is_present("no-site");

    let ignore_environment = settings.ignore_environment || settings.isolated;

    if !ignore_environment {
        settings.path_list.extend(get_paths("RUSTPYTHONPATH"));
        settings.path_list.extend(get_paths("PYTHONPATH"));
    }

    // Now process command line flags:
    if matches.is_present("debug") || (!ignore_environment && env::var_os("PYTHONDEBUG").is_some())
    {
        settings.debug = true;
    }

    if matches.is_present("inspect")
        || (!ignore_environment && env::var_os("PYTHONINSPECT").is_some())
    {
        settings.inspect = true;
    }

    if matches.is_present("optimize") {
        settings.optimize = matches.occurrences_of("optimize").try_into().unwrap();
    } else if !ignore_environment {
        if let Ok(value) = get_env_var_value("PYTHONOPTIMIZE") {
            settings.optimize = value;
        }
    }

    if matches.is_present("verbose") {
        settings.verbose = matches.occurrences_of("verbose").try_into().unwrap();
    } else if !ignore_environment {
        if let Ok(value) = get_env_var_value("PYTHONVERBOSE") {
            settings.verbose = value;
        }
    }

    if matches.is_present("no-user-site")
        || matches.is_present("isolate")
        || (!ignore_environment && env::var_os("PYTHONNOUSERSITE").is_some())
    {
        settings.no_user_site = true;
    }

    if matches.is_present("quiet") {
        settings.quiet = true;
    }

    if matches.is_present("dont-write-bytecode")
        || (!ignore_environment && env::var_os("PYTHONDONTWRITEBYTECODE").is_some())
    {
        settings.dont_write_bytecode = true;
    }

    settings.check_hash_based_pycs = matches
        .value_of("check-hash-based-pycs")
        .unwrap_or("default")
        .to_owned();

    let mut dev_mode = false;
    let mut warn_default_encoding = false;
    if let Some(xopts) = matches.values_of("implementation-option") {
        settings.xopts.extend(xopts.map(|s| {
            let mut parts = s.splitn(2, '=');
            let name = parts.next().unwrap().to_owned();
            if name == "dev" {
                dev_mode = true
            }
            if name == "warn_default_encoding" {
                warn_default_encoding = true
            }
            let value = parts.next().map(ToOwned::to_owned);
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
        settings.warnopts.push("default".to_owned())
    }
    if settings.bytes_warning > 0 {
        let warn = if settings.bytes_warning > 1 {
            "error::BytesWarning"
        } else {
            "default::BytesWarning"
        };
        settings.warnopts.push(warn.to_owned());
    }
    if let Some(warnings) = matches.values_of("warning-control") {
        settings.warnopts.extend(warnings.map(ToOwned::to_owned));
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
    } else if let Some(argv) = matches.values_of("script") {
        let argv: Vec<_> = argv.map(ToOwned::to_owned).collect();
        let script = argv[0].clone();
        (
            RunMode::ScriptInteractive(Some(script), matches.is_present("inspect")),
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
    env::var(name).map(|value| {
        if let Ok(value) = u8::from_str(&value) {
            value
        } else {
            1
        }
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
                        .unwrap_or_else(|_| panic!("{} isn't valid unicode", env_variable_name))
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

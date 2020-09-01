#[macro_use]
extern crate clap;
extern crate env_logger;
#[macro_use]
extern crate log;

use clap::{App, AppSettings, Arg, ArgMatches};
use rustpython_compiler::compile;
use rustpython_vm::{
    exceptions::print_exception,
    match_class,
    obj::{objint::PyInt, objtype},
    pyobject::{BorrowValue, ItemProtocol, PyResult},
    scope::Scope,
    util, InitParameter, PySettings, VirtualMachine,
};

use std::convert::TryInto;
use std::env;
use std::path::PathBuf;
use std::process;
use std::str::FromStr;

mod shell;

fn main() {
    #[cfg(feature = "flame-it")]
    let main_guard = flame::start_guard("RustPython main");
    env_logger::init();
    let app = App::new("RustPython");
    let matches = parse_arguments(app);
    let mut settings = create_settings(&matches);

    // We only include the standard library bytecode in WASI when initializing
    if cfg!(target_os = "wasi") {
        settings.initialization_parameter = InitParameter::InitializeInternal;
    }

    // don't translate newlines (\r\n <=> \n)
    #[cfg(windows)]
    {
        extern "C" {
            fn _setmode(fd: i32, flags: i32) -> i32;
        }
        unsafe {
            _setmode(0, libc::O_BINARY);
            _setmode(1, libc::O_BINARY);
            _setmode(2, libc::O_BINARY);
        }
    }

    let vm = VirtualMachine::new(settings);

    let res = run_rustpython(&vm, &matches);

    #[cfg(feature = "flame-it")]
    {
        main_guard.end();
        if let Err(e) = write_profile(&matches) {
            error!("Error writing profile information: {}", e);
        }
    }

    // See if any exception leaked out:
    if let Err(err) = res {
        if objtype::isinstance(&err, &vm.ctx.exceptions.system_exit) {
            let args = err.args();
            match args.borrow_value().len() {
                0 => return,
                1 => match_class!(match args.borrow_value()[0].clone() {
                    i @ PyInt => {
                        use num_traits::cast::ToPrimitive;
                        process::exit(i.borrow_value().to_i32().unwrap_or(0));
                    }
                    arg => {
                        if vm.is_none(&arg) {
                            return;
                        }
                        if let Ok(s) = vm.to_str(&arg) {
                            eprintln!("{}", s);
                        }
                    }
                }),
                _ => {
                    if let Ok(r) = vm.to_repr(args.as_object()) {
                        eprintln!("{}", r);
                    }
                }
            }
        } else {
            print_exception(&vm, err);
        }
        process::exit(1);
    }
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
                .help("set implementation-specific option"),
        )
        .arg(
            Arg::with_name("warning-control")
                .short("W")
                .takes_value(true)
                .help("warning control; arg is action:message:category:module:lineno"),
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
fn create_settings(matches: &ArgMatches) -> PySettings {
    let ignore_environment =
        matches.is_present("ignore-environment") || matches.is_present("isolate");
    let mut settings = PySettings::default();
    settings.ignore_environment = ignore_environment;

    // add the current directory to sys.path
    settings.path_list.push("".to_owned());

    // BUILDTIME_RUSTPYTHONPATH should be set when distributing
    if let Some(paths) = option_env!("BUILDTIME_RUSTPYTHONPATH") {
        settings.path_list.extend(
            std::env::split_paths(paths).map(|path| path.into_os_string().into_string().unwrap()),
        )
    } else {
        #[cfg(all(feature = "pylib", not(feature = "freeze-stdlib")))]
        settings.path_list.push(pylib::LIB_PATH.to_owned());
    }

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

    settings.no_site = matches.is_present("no-site");

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

    let argv = if let Some(script) = matches.values_of("script") {
        script.map(ToOwned::to_owned).collect()
    } else if let Some(module) = matches.values_of("m") {
        std::iter::once("PLACEHOLDER".to_owned())
            .chain(module.skip(1).map(ToOwned::to_owned))
            .collect()
    } else if let Some(cmd) = matches.values_of("c") {
        std::iter::once("-c".to_owned())
            .chain(cmd.skip(1).map(ToOwned::to_owned))
            .collect()
    } else {
        vec!["".to_owned()]
    };

    let hash_seed = match env::var("PYTHONHASHSEED") {
        Ok(s) if s == "random" => Some(None),
        Ok(s) => s.parse::<u32>().ok().map(Some),
        Err(_) => Some(None),
    };
    settings.hash_seed = hash_seed.unwrap_or_else(|| {
        error!("Fatal Python init error: PYTHONHASHSEED must be \"random\" or an integer in range [0; 4294967295]");
        process::exit(1)
    });

    settings.argv = argv;

    settings
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
            env::split_paths(&paths)
                .map(|path| {
                    path.into_os_string()
                        .into_string()
                        .unwrap_or_else(|_| panic!("{} isn't valid unicode", env_variable_name))
                })
                .collect::<Vec<_>>()
        })
}

#[cfg(feature = "flame-it")]
fn write_profile(matches: &ArgMatches) -> Result<(), Box<dyn std::error::Error>> {
    use std::{fs, io};

    enum ProfileFormat {
        Html,
        Text,
        Speedscope,
    }

    let profile_output = matches.value_of_os("profile_output");

    let profile_format = match matches.value_of("profile_format") {
        Some("html") => ProfileFormat::Html,
        Some("text") => ProfileFormat::Text,
        None if profile_output == Some("-".as_ref()) => ProfileFormat::Text,
        Some("speedscope") | None => ProfileFormat::Speedscope,
        Some(other) => {
            error!("Unknown profile format {}", other);
            process::exit(1);
        }
    };

    let profile_output = profile_output.unwrap_or_else(|| match profile_format {
        ProfileFormat::Html => "flame-graph.html".as_ref(),
        ProfileFormat::Text => "flame.txt".as_ref(),
        ProfileFormat::Speedscope => "flamescope.json".as_ref(),
    });

    let profile_output: Box<dyn io::Write> = if profile_output == "-" {
        Box::new(io::stdout())
    } else {
        Box::new(fs::File::create(profile_output)?)
    };

    let profile_output = io::BufWriter::new(profile_output);

    match profile_format {
        ProfileFormat::Html => flame::dump_html(profile_output)?,
        ProfileFormat::Text => flame::dump_text_to_writer(profile_output)?,
        ProfileFormat::Speedscope => flamescope::dump(profile_output)?,
    }

    Ok(())
}

fn run_rustpython(vm: &VirtualMachine, matches: &ArgMatches) -> PyResult<()> {
    let scope = vm.new_scope_with_builtins();
    let main_module = vm.new_module("__main__", scope.globals.clone());

    vm.get_attribute(vm.sys_module.clone(), "modules")?
        .set_item("__main__", main_module, vm)?;

    let site_result = vm.import("site", &[], 0);

    if site_result.is_err() {
        warn!(
            "Failed to import site, consider adding the Lib directory to your RUSTPYTHONPATH \
             environment variable",
        );
    }

    // Figure out if a -c option was given:
    if let Some(command) = matches.value_of("c") {
        run_command(&vm, scope, command.to_owned())?;
    } else if let Some(module) = matches.value_of("m") {
        run_module(&vm, module)?;
    } else if let Some(filename) = matches.value_of("script") {
        run_script(&vm, scope.clone(), filename)?;
        if matches.is_present("inspect") {
            shell::run_shell(&vm, scope)?;
        }
    } else {
        println!(
            "Welcome to the magnificent Rust Python {} interpreter \u{1f631} \u{1f596}",
            crate_version!()
        );
        shell::run_shell(&vm, scope)?;
    }

    Ok(())
}

fn _run_string(vm: &VirtualMachine, scope: Scope, source: &str, source_path: String) -> PyResult {
    let code_obj = vm
        .compile(source, compile::Mode::Exec, source_path.clone())
        .map_err(|err| vm.new_syntax_error(&err))?;
    // trace!("Code object: {:?}", code_obj.borrow());
    scope
        .globals
        .set_item("__file__", vm.ctx.new_str(source_path), vm)?;
    vm.run_code_obj(code_obj, scope)
}

fn run_command(vm: &VirtualMachine, scope: Scope, source: String) -> PyResult<()> {
    debug!("Running command {}", source);
    _run_string(vm, scope, &source, "<stdin>".to_owned())?;
    Ok(())
}

fn run_module(vm: &VirtualMachine, module: &str) -> PyResult<()> {
    debug!("Running module {}", module);
    let runpy = vm.import("runpy", &[], 0)?;
    let run_module_as_main = vm.get_attribute(runpy, "_run_module_as_main")?;
    vm.invoke(&run_module_as_main, vec![vm.ctx.new_str(module)])?;
    Ok(())
}

fn run_script(vm: &VirtualMachine, scope: Scope, script_file: &str) -> PyResult<()> {
    debug!("Running file {}", script_file);
    // Parse an ast from it:
    let file_path = PathBuf::from(script_file);
    let file_path = if file_path.is_file() {
        file_path
    } else if file_path.is_dir() {
        let main_file_path = file_path.join("__main__.py");
        if main_file_path.is_file() {
            main_file_path
        } else {
            error!(
                "can't find '__main__' module in '{}'",
                file_path.to_str().unwrap()
            );
            process::exit(1);
        }
    } else {
        error!(
            "can't open file '{}': No such file or directory",
            file_path.to_str().unwrap()
        );
        process::exit(1);
    };

    let dir = file_path.parent().unwrap().to_str().unwrap().to_owned();
    let sys_path = vm.get_attribute(vm.sys_module.clone(), "path").unwrap();
    vm.call_method(
        &sys_path,
        "insert",
        vec![vm.ctx.new_int(0), vm.ctx.new_str(dir)],
    )?;

    match util::read_file(&file_path) {
        Ok(source) => {
            _run_string(vm, scope, &source, file_path.to_str().unwrap().to_owned())?;
        }
        Err(err) => {
            error!(
                "Failed reading file '{}': {:?}",
                file_path.to_str().unwrap(),
                err.kind()
            );
            process::exit(1);
        }
    }
    Ok(())
}

#[test]
fn test_run_script() {
    let vm: VirtualMachine = Default::default();

    // test file run
    let r = run_script(
        &vm,
        vm.new_scope_with_builtins(),
        "tests/snippets/dir_main/__main__.py",
    );
    assert!(r.is_ok());

    // test module run
    let r = run_script(&vm, vm.new_scope_with_builtins(), "tests/snippets/dir_main");
    assert!(r.is_ok());
}

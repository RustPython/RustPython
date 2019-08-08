#[macro_use]
extern crate clap;
extern crate env_logger;
#[macro_use]
extern crate log;

use clap::{App, Arg, ArgMatches};
use rustpython_compiler::{compile, error::CompileError, error::CompileErrorType};
use rustpython_parser::error::ParseErrorType;
use rustpython_vm::{
    import,
    obj::objstr,
    print_exception,
    pyobject::{ItemProtocol, PyResult},
    scope::Scope,
    util, PySettings, VirtualMachine,
};
use std::convert::TryInto;

use std::env;
use std::path::PathBuf;
use std::process;
use std::str::FromStr;

fn main() {
    #[cfg(feature = "flame-it")]
    let main_guard = flame::start_guard("RustPython main");
    env_logger::init();
    let app = App::new("RustPython");
    let matches = parse_arguments(app);
    let settings = create_settings(&matches);
    let vm = VirtualMachine::new(settings);

    let res = run_rustpython(&vm, &matches);
    // See if any exception leaked out:
    handle_exception(&vm, res);

    #[cfg(feature = "flame-it")]
    {
        main_guard.end();
        if let Err(e) = write_profile(&matches) {
            error!("Error writing profile information: {}", e);
            process::exit(1);
        }
    }
}

fn parse_arguments<'a>(app: App<'a, '_>) -> ArgMatches<'a> {
    let app = app
        .version(crate_version!())
        .author(crate_authors!())
        .about("Rust implementation of the Python language")
        .usage("rustpython [OPTIONS] [-c CMD | -m MODULE | FILE | -] [PYARGS]...")
        .arg(
            Arg::with_name("script")
                .required(false)
                .multiple(true)
                .min_values(1)
                .allow_hyphen_values(true)
                .value_names(&["script", "args..."]),
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
            Arg::with_name("c")
                .short("c")
                .takes_value(true)
                .help("run the given string as a program"),
        )
        .arg(
            Arg::with_name("m")
                .short("m")
                .takes_value(true)
                .allow_hyphen_values(true)
                .multiple(true)
                // .value
                .value_names(&["module", "args..."])
                .help("run library module as script"),
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
    let ignore_environment = matches.is_present("ignore-environment");
    let mut settings: PySettings = Default::default();
    settings.ignore_environment = ignore_environment;

    if !ignore_environment {
        settings.path_list.append(&mut get_paths("RUSTPYTHONPATH"));
        settings.path_list.append(&mut get_paths("PYTHONPATH"));
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

    let mut argv = if let Some(script) = matches.values_of("script") {
        script.map(ToOwned::to_owned).collect()
    } else if let Some(mut module) = matches.values_of("m") {
        let argv0 = if let Ok(module_path) = std::fs::canonicalize(module.next().unwrap()) {
            module_path
                .into_os_string()
                .into_string()
                .expect("invalid utf8 in module path")
        } else {
            // if it's not a real file/don't have permissions it'll probably fail anyway
            String::new()
        };
        std::iter::once(argv0)
            .chain(module.map(ToOwned::to_owned))
            .collect()
    } else {
        vec![]
    };

    argv.extend(
        matches
            .values_of("pyargs")
            .unwrap_or_default()
            .map(ToOwned::to_owned),
    );

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
fn get_paths(env_variable_name: &str) -> Vec<String> {
    let paths = env::var_os(env_variable_name);
    match paths {
        Some(paths) => env::split_paths(&paths)
            .map(|path| {
                path.into_os_string()
                    .into_string()
                    .unwrap_or_else(|_| panic!("{} isn't valid unicode", env_variable_name))
            })
            .collect(),
        None => vec![],
    }
}

#[cfg(feature = "flame-it")]
fn write_profile(matches: &ArgMatches) -> Result<(), Box<dyn std::error::Error>> {
    use std::fs::File;

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

    let profile_output: Box<dyn std::io::Write> = if profile_output == "-" {
        Box::new(std::io::stdout())
    } else {
        Box::new(File::create(profile_output)?)
    };

    match profile_format {
        ProfileFormat::Html => flame::dump_html(profile_output)?,
        ProfileFormat::Text => flame::dump_text_to_writer(profile_output)?,
        ProfileFormat::Speedscope => flamescope::dump(profile_output)?,
    }

    Ok(())
}

fn run_rustpython(vm: &VirtualMachine, matches: &ArgMatches) -> PyResult<()> {
    import::init_importlib(&vm, true)?;

    if let Some(paths) = option_env!("BUILDTIME_RUSTPYTHONPATH") {
        let sys_path = vm.get_attribute(vm.sys_module.clone(), "path")?;
        for (i, path) in std::env::split_paths(paths).enumerate() {
            vm.call_method(
                &sys_path,
                "insert",
                vec![
                    vm.ctx.new_int(i),
                    vm.ctx.new_str(
                        path.into_os_string()
                            .into_string()
                            .expect("Invalid UTF8 in BUILDTIME_RUSTPYTHONPATH"),
                    ),
                ],
            )?;
        }
    }

    // Figure out if a -c option was given:
    if let Some(command) = matches.value_of("c") {
        run_command(&vm, command.to_string())?;
    } else if let Some(module) = matches.value_of("m") {
        run_module(&vm, module)?;
    } else {
        // Figure out if a script was passed:
        match matches.values_of("script") {
            None => run_shell(&vm)?,
            Some(mut filename) => run_script(&vm, filename.next().unwrap())?,
        }
    }

    Ok(())
}

fn _run_string(vm: &VirtualMachine, source: &str, source_path: String) -> PyResult {
    let code_obj = vm
        .compile(source, &compile::Mode::Exec, source_path.clone())
        .map_err(|err| vm.new_syntax_error(&err))?;
    // trace!("Code object: {:?}", code_obj.borrow());
    let attrs = vm.ctx.new_dict();
    attrs.set_item("__file__", vm.new_str(source_path), vm)?;
    vm.run_code_obj(code_obj, Scope::with_builtins(None, attrs, vm))
}

fn handle_exception<T>(vm: &VirtualMachine, result: PyResult<T>) {
    if let Err(err) = result {
        print_exception(vm, &err);
        process::exit(1);
    }
}

fn run_command(vm: &VirtualMachine, source: String) -> PyResult<()> {
    debug!("Running command {}", source);

    _run_string(vm, &source, "<stdin>".to_string())?;
    Ok(())
}

fn run_module(vm: &VirtualMachine, module: &str) -> PyResult<()> {
    debug!("Running module {}", module);
    vm.import(module, &vm.ctx.new_tuple(vec![]), 0)?;
    Ok(())
}

fn run_script(vm: &VirtualMachine, script_file: &str) -> PyResult<()> {
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

    let dir = file_path.parent().unwrap().to_str().unwrap().to_string();
    let sys_path = vm.get_attribute(vm.sys_module.clone(), "path").unwrap();
    vm.call_method(&sys_path, "insert", vec![vm.new_int(0), vm.new_str(dir)])?;

    match util::read_file(&file_path) {
        Ok(source) => {
            _run_string(vm, &source, file_path.to_str().unwrap().to_string())?;
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
    let r = run_script(&vm, "tests/snippets/dir_main/__main__.py");
    assert!(r.is_ok());

    // test module run
    let r = run_script(&vm, "tests/snippets/dir_main");
    assert!(r.is_ok());
}

fn shell_exec(vm: &VirtualMachine, source: &str, scope: Scope) -> Result<(), CompileError> {
    match vm.compile(source, &compile::Mode::Single, "<stdin>".to_string()) {
        Ok(code) => {
            match vm.run_code_obj(code, scope.clone()) {
                Ok(value) => {
                    // Save non-None values as "_"

                    use rustpython_vm::pyobject::{IdProtocol, IntoPyObject};

                    if !value.is(&vm.get_none()) {
                        let key = objstr::PyString::from("_").into_pyobject(vm);
                        scope.globals.set_item(key, value, vm).unwrap();
                    }
                }

                Err(err) => {
                    print_exception(vm, &err);
                }
            }

            Ok(())
        }
        // Don't inject syntax errors for line continuation
        Err(
            err @ CompileError {
                error: CompileErrorType::Parse(ParseErrorType::EOF),
                ..
            },
        ) => Err(err),
        Err(err) => {
            let exc = vm.new_syntax_error(&err);
            print_exception(vm, &exc);
            Err(err)
        }
    }
}

#[cfg(not(unix))]
fn get_history_path() -> PathBuf {
    PathBuf::from(".repl_history.txt")
}

#[cfg(unix)]
fn get_history_path() -> PathBuf {
    //work around for windows dependent builds. The xdg crate is unix specific
    //so access to the BaseDirectories struct breaks builds on python.
    extern crate xdg;

    let xdg_dirs = xdg::BaseDirectories::with_prefix("rustpython").unwrap();
    xdg_dirs.place_cache_file("repl_history.txt").unwrap()
}

fn get_prompt(vm: &VirtualMachine, prompt_name: &str) -> String {
    vm.get_attribute(vm.sys_module.clone(), prompt_name)
        .ok()
        .as_ref()
        .map(objstr::get_value)
        .unwrap_or_else(String::new)
}

#[cfg(not(target_os = "redox"))]
fn run_shell(vm: &VirtualMachine) -> PyResult<()> {
    use rustyline::{error::ReadlineError, Editor};

    println!(
        "Welcome to the magnificent Rust Python {} interpreter \u{1f631} \u{1f596}",
        crate_version!()
    );
    let vars = vm.new_scope_with_builtins();

    // Read a single line:
    let mut input = String::new();
    let mut repl = Editor::<()>::new();

    // Retrieve a `history_path_str` dependent on the OS
    let repl_history_path_str = &get_history_path();
    if repl.load_history(repl_history_path_str).is_err() {
        println!("No previous history.");
    }

    let mut continuing = false;

    loop {
        let prompt = if continuing {
            get_prompt(vm, "ps2")
        } else {
            get_prompt(vm, "ps1")
        };
        match repl.readline(&prompt) {
            Ok(line) => {
                debug!("You entered {:?}", line);
                input.push_str(&line);
                input.push('\n');
                repl.add_history_entry(line.trim_end());

                if continuing {
                    if line.is_empty() {
                        continuing = false;
                    } else {
                        continue;
                    }
                }

                match shell_exec(vm, &input, vars.clone()) {
                    Err(CompileError {
                        error: CompileErrorType::Parse(ParseErrorType::EOF),
                        ..
                    }) => {
                        continuing = true;
                        continue;
                    }
                    _ => {
                        input = String::new();
                    }
                }
            }
            Err(ReadlineError::Interrupted) => {
                // TODO: Raise a real KeyboardInterrupt exception
                println!("^C");
                continuing = false;
                continue;
            }
            Err(ReadlineError::Eof) => {
                break;
            }
            Err(err) => {
                println!("Error: {:?}", err);
                break;
            }
        };
    }
    repl.save_history(repl_history_path_str).unwrap();

    Ok(())
}

#[cfg(target_os = "redox")]
fn run_shell(vm: &VirtualMachine) -> PyResult<()> {
    use std::io::{self, BufRead, Write};

    println!(
        "Welcome to the magnificent Rust Python {} interpreter \u{1f631} \u{1f596}",
        crate_version!()
    );
    let vars = vm.new_scope_with_builtins();

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut stdout = stdout.lock();

    print!("{}", get_prompt(vm, "ps1"));
    stdout.flush().expect("flush failed");
    for line in stdin.lock().lines() {
        let mut line = line.expect("line failed");
        line.push('\n');
        let _ = shell_exec(vm, &line, vars.clone());
        print!("{}", get_prompt(vm, "ps1"));
        stdout.flush().expect("flush failed");
    }

    Ok(())
}

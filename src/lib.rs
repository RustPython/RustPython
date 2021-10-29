//! This is the `rustpython` binary. If you're looking to embed RustPython into your application,
//! you're likely looking for the [`rustpython-vm`](https://docs.rs/rustpython-vm) crate.
//!
//! You can install `rustpython` with `cargo install rustpython`, or if you'd like to inject your
//! own native modules you can make a binary crate that depends on the `rustpython` crate (and
//! probably `rustpython-vm`, too), and make a `main.rs` that looks like:
//!
//! ```no_run
//! use rustpython_vm::{pymodule, py_freeze};
//! fn main() {
//!     rustpython::run(|vm| {
//!         vm.add_native_module("mymod".to_owned(), Box::new(mymod::make_module));
//!         vm.add_frozen(py_freeze!(source = "def foo(): pass", module_name = "otherthing"));
//!     });
//! }
//!
//! #[pymodule]
//! mod mymod {
//!     use rustpython_vm::builtins::PyStrRef;
//TODO: use rustpython_vm::prelude::*;
//!
//!     #[pyfunction]
//!     fn do_thing(x: i32) -> i32 {
//!         x + 1
//!     }
//!
//!     #[pyfunction]
//!     fn other_thing(s: PyStrRef) -> (String, usize) {
//!         let new_string = format!("hello from rust, {}!", s);
//!         let prev_len = s.as_str().len();
//!         (new_string, prev_len)
//!     }
//! }
//! ```
//!
//! The binary will have all the standard arguments of a python interpreter (including a REPL!) but
//! it will have your modules loaded into the vm.
#![allow(clippy::needless_doctest_main, clippy::unnecessary_wraps)]

#[macro_use]
extern crate clap;
extern crate env_logger;
#[macro_use]
extern crate log;

use clap::{App, AppSettings, Arg, ArgMatches};
use rustpython_vm::{
    builtins::PyDictRef, builtins::PyInt, compile, match_class, scope::Scope, stdlib::sys,
    InitParameter, Interpreter, ItemProtocol, PyObjectRef, PyResult, PySettings, TryFromObject,
    TypeProtocol, VirtualMachine,
};

use std::env;
use std::path::Path;
use std::process;
use std::str::FromStr;

mod shell;

pub use rustpython_vm;

/// The main cli of the `rustpython` interpreter. This function will exit with `process::exit()`
/// based on the return code of the python code ran through the cli.
pub fn run<F>(init: F) -> !
where
    F: FnOnce(&mut VirtualMachine),
{
    #[cfg(feature = "flame-it")]
    let main_guard = flame::start_guard("RustPython main");
    env_logger::init();
    let app = App::new("RustPython");
    let matches = parse_arguments(app);
    let settings = create_settings(&matches);

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

    let interp = Interpreter::new_with_init(settings, |vm| {
        add_stdlib(vm);
        init(vm);
        InitParameter::External
    });

    let exitcode = interp.enter(move |vm| {
        let res = run_rustpython(vm, &matches);

        flush_std(vm);

        #[cfg(feature = "flame-it")]
        {
            main_guard.end();
            if let Err(e) = write_profile(&matches) {
                error!("Error writing profile information: {}", e);
            }
        }

        // See if any exception leaked out:
        let exitcode = match res {
            Ok(()) => 0,
            Err(err) if err.isinstance(&vm.ctx.exceptions.system_exit) => {
                let args = err.args();
                match args.as_slice() {
                    [] => 0,
                    [arg] => match_class!(match arg {
                        ref i @ PyInt => {
                            use num_traits::cast::ToPrimitive;
                            i.as_bigint().to_i32().unwrap_or(0)
                        }
                        arg => {
                            if vm.is_none(arg) {
                                0
                            } else {
                                if let Ok(s) = arg.str(vm) {
                                    eprintln!("{}", s);
                                }
                                1
                            }
                        }
                    }),
                    _ => {
                        if let Ok(r) = args.as_object().repr(vm) {
                            eprintln!("{}", r);
                        }
                        1
                    }
                }
            }
            Err(exc) => {
                vm.print_exception(exc);
                1
            }
        };

        let _ = vm.run_atexit_funcs();

        flush_std(vm);

        exitcode
    });

    process::exit(exitcode)
}

fn flush_std(vm: &VirtualMachine) {
    if let Ok(stdout) = sys::get_stdout(vm) {
        let _ = vm.call_method(&stdout, "flush", ());
    }
    if let Ok(stderr) = sys::get_stderr(vm) {
        let _ = vm.call_method(&stderr, "flush", ());
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

fn add_stdlib(vm: &mut VirtualMachine) {
    let _ = vm;
    #[cfg(feature = "stdlib")]
    {
        let stdlib = rustpython_stdlib::get_module_inits();
        for (name, init) in stdlib.into_iter() {
            vm.add_native_module(name, init);
        }
    }
}

/// Create settings by examining command line arguments and environment
/// variables.
fn create_settings(matches: &ArgMatches) -> PySettings {
    let mut settings = PySettings {
        isolated: matches.is_present("isolate"),
        ignore_environment: matches.is_present("ignore-environment"),
        interactive: !matches.is_present("c")
            && !matches.is_present("m")
            && (!matches.is_present("script") || matches.is_present("inspect")),
        bytes_warning: matches.occurrences_of("bytes-warning"),
        no_site: matches.is_present("no-site"),
        ..Default::default()
    };
    let ignore_environment = settings.ignore_environment || settings.isolated;

    // when rustpython-vm/pylib is enabled, PySettings::default().path_list has pylib::LIB_PATH
    let maybe_pylib = settings.path_list.pop();

    // add the current directory to sys.path
    settings.path_list.push("".to_owned());

    // BUILDTIME_RUSTPYTHONPATH should be set when distributing
    if let Some(paths) = option_env!("BUILDTIME_RUSTPYTHONPATH") {
        settings
            .path_list
            .extend(split_paths(paths).map(|path| path.into_os_string().into_string().unwrap()))
    } else {
        settings.path_list.extend(maybe_pylib);
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

    let mut dev_mode = false;
    if let Some(xopts) = matches.values_of("implementation-option") {
        settings.xopts.extend(xopts.map(|s| {
            let mut parts = s.splitn(2, '=');
            let name = parts.next().unwrap().to_owned();
            if name == "dev" {
                dev_mode = true
            }
            let value = parts.next().map(ToOwned::to_owned);
            (name, value)
        }));
    }
    settings.dev_mode = dev_mode;

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

    let argv = if let Some(script) = matches.values_of("script") {
        script.map(ToOwned::to_owned).collect()
    } else if let Some(module) = matches.values_of("m") {
        std::iter::once("PLACEHOLDER".to_owned())
            .chain(module.skip(1).map(ToOwned::to_owned))
            .collect()
    } else if let Some(get_pip_args) = matches.values_of("install_pip") {
        std::iter::once("get-pip.py".to_owned())
            .chain(get_pip_args.map(ToOwned::to_owned))
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
use env::split_paths;
#[cfg(target_os = "wasi")]
fn split_paths<T: AsRef<std::ffi::OsStr> + ?Sized>(
    s: &T,
) -> impl Iterator<Item = std::path::PathBuf> + '_ {
    use std::os::wasi::ffi::OsStrExt;
    let s = s.as_ref().as_bytes();
    s.split(|b| *b == b':')
        .map(|x| std::ffi::OsStr::from_bytes(x).to_owned().into())
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

fn setup_main_module(vm: &VirtualMachine) -> PyResult<Scope> {
    let scope = vm.new_scope_with_builtins();
    let main_module = vm.new_module("__main__", scope.globals.clone(), None);
    main_module
        .dict()
        .and_then(|d| {
            d.set_item(
                "__annotations__",
                vm.ctx.new_dict().as_object().to_owned(),
                vm,
            )
            .ok()
        })
        .expect("Failed to initialize __main__.__annotations__");

    vm.sys_module
        .clone()
        .get_attr("modules", vm)?
        .set_item("__main__", main_module, vm)?;

    Ok(scope)
}

#[cfg(feature = "ssl")]
fn install_pip(scope: Scope, vm: &VirtualMachine) -> PyResult {
    let get_getpip = rustpython_vm::py_compile!(
        source = r#"\
__import__("io").TextIOWrapper(
    __import__("urllib.request").request.urlopen("https://bootstrap.pypa.io/get-pip.py")
).read()
"#,
        mode = "eval"
    );
    eprintln!("downloading get-pip.py...");
    let getpip_code = vm.run_code_obj(vm.new_code_object(get_getpip), scope.clone())?;
    let getpip_code: rustpython_vm::builtins::PyStrRef = getpip_code
        .downcast()
        .expect("TextIOWrapper.read() should return str");
    eprintln!("running get-pip.py...");
    _run_string(vm, scope, getpip_code.as_str(), "get-pip.py".to_owned())
}

#[cfg(not(feature = "ssl"))]
fn install_pip(_: Scope, vm: &VirtualMachine) -> PyResult {
    Err(vm.new_exception_msg(
        vm.ctx.exceptions.system_error.clone(),
        "install-pip requires rustpython be build with the 'ssl' feature enabled.".to_owned(),
    ))
}

fn run_rustpython(vm: &VirtualMachine, matches: &ArgMatches) -> PyResult<()> {
    let scope = setup_main_module(vm)?;

    let site_result = vm.import("site", None, 0);

    if site_result.is_err() {
        warn!(
            "Failed to import site, consider adding the Lib directory to your RUSTPYTHONPATH \
             environment variable",
        );
    }

    // Figure out if a -c option was given:
    if let Some(command) = matches.value_of("c") {
        run_command(vm, scope, command.to_owned())?;
    } else if let Some(module) = matches.value_of("m") {
        run_module(vm, module)?;
    } else if matches.is_present("install_pip") {
        install_pip(scope, vm)?;
    } else if let Some(filename) = matches.value_of("script") {
        run_script(vm, scope.clone(), filename)?;
        if matches.is_present("inspect") {
            shell::run_shell(vm, scope)?;
        }
    } else {
        println!(
            "Welcome to the magnificent Rust Python {} interpreter \u{1f631} \u{1f596}",
            crate_version!()
        );
        shell::run_shell(vm, scope)?;
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
        .set_item("__file__", vm.new_pyobj(source_path), vm)?;
    vm.run_code_obj(code_obj, scope)
}

fn run_command(vm: &VirtualMachine, scope: Scope, source: String) -> PyResult<()> {
    debug!("Running command {}", source);
    _run_string(vm, scope, &source, "<stdin>".to_owned())?;
    Ok(())
}

fn run_module(vm: &VirtualMachine, module: &str) -> PyResult<()> {
    debug!("Running module {}", module);
    let runpy = vm.import("runpy", None, 0)?;
    let run_module_as_main = runpy.get_attr("_run_module_as_main", vm)?;
    vm.invoke(&run_module_as_main, (module,))?;
    Ok(())
}

fn get_importer(path: &str, vm: &VirtualMachine) -> PyResult<Option<PyObjectRef>> {
    let path_importer_cache = vm.sys_module.clone().get_attr("path_importer_cache", vm)?;
    let path_importer_cache = PyDictRef::try_from_object(vm, path_importer_cache)?;
    if let Some(importer) = path_importer_cache.get_item_option(path, vm)? {
        return Ok(Some(importer));
    }
    let path = vm.ctx.new_str(path);
    let path_hooks = vm.sys_module.clone().get_attr("path_hooks", vm)?;
    let mut importer = None;
    let path_hooks: Vec<PyObjectRef> = vm.extract_elements(&path_hooks)?;
    for path_hook in path_hooks {
        match vm.invoke(&path_hook, (path.clone(),)) {
            Ok(imp) => {
                importer = Some(imp);
                break;
            }
            Err(e) if e.isinstance(&vm.ctx.exceptions.import_error) => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(if let Some(imp) = importer {
        let imp = path_importer_cache.get_or_insert(vm, path.into(), || imp.clone())?;
        Some(imp)
    } else {
        None
    })
}

fn insert_sys_path(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<()> {
    let sys_path = vm.sys_module.clone().get_attr("path", vm).unwrap();
    vm.call_method(&sys_path, "insert", (0, obj))?;
    Ok(())
}

fn run_script(vm: &VirtualMachine, scope: Scope, script_file: &str) -> PyResult<()> {
    debug!("Running file {}", script_file);
    if get_importer(script_file, vm)?.is_some() {
        insert_sys_path(vm, vm.ctx.new_str(script_file).into())?;
        let runpy = vm.import("runpy", None, 0)?;
        let run_module_as_main = runpy.get_attr("_run_module_as_main", vm)?;
        vm.invoke(&run_module_as_main, (vm.ctx.new_str("__main__"), false))?;
        return Ok(());
    }
    let dir = Path::new(script_file).parent().unwrap().to_str().unwrap();
    insert_sys_path(vm, vm.ctx.new_str(dir).into())?;

    match std::fs::read_to_string(script_file) {
        Ok(source) => {
            _run_string(vm, scope, &source, script_file.to_owned())?;
        }
        Err(err) => {
            error!("Failed reading file '{}': {}", script_file, err);
            process::exit(1);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interpreter() -> Interpreter {
        Interpreter::new_with_init(PySettings::default(), |vm| {
            add_stdlib(vm);
            InitParameter::External
        })
    }

    #[test]
    fn test_run_script() {
        interpreter().enter(|vm| {
            vm.unwrap_pyresult((|| {
                let scope = setup_main_module(vm)?;
                // test file run
                run_script(vm, scope, "extra_tests/snippets/dir_main/__main__.py")?;

                let scope = setup_main_module(vm)?;
                // test module run
                run_script(vm, scope, "extra_tests/snippets/dir_main")?;

                Ok(())
            })());
        })
    }
}

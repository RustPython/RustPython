//! This is the `rustpython` binary. If you're looking to embed RustPython into your application,
//! you're likely looking for the [`rustpython_vm`] crate.
//!
//! You can install `rustpython` with `cargo install rustpython`. If you'd like to inject your
//! own native modules, you can make a binary crate that depends on the `rustpython` crate (and
//! probably [`rustpython_vm`], too), and make a `main.rs` that looks like:
//!
//! ```no_run
//! use rustpython::{InterpreterBuilder, InterpreterBuilderExt};
//! use rustpython_vm::{pymodule, py_freeze};
//!
//! fn main() -> std::process::ExitCode {
//!     let builder = InterpreterBuilder::new().init_stdlib();
//!     // Add a native module using builder.ctx
//!     let my_mod_def = my_mod::module_def(&builder.ctx);
//!     let builder = builder
//!         .add_native_module(my_mod_def)
//!         // Add a frozen module
//!         .add_frozen_modules(py_freeze!(source = "def foo(): pass", module_name = "other_thing"));
//!
//!     rustpython::run(builder)
//! }
//!
//! #[pymodule]
//! mod my_mod {
//!     use rustpython_vm::builtins::PyStrRef;
//!
//!     #[pyfunction]
//!     fn do_thing(x: i32) -> i32 {
//!         x + 1
//!     }
//!
//!     #[pyfunction]
//!     fn other_thing(s: PyStrRef) -> (String, usize) {
//!         let new_string = format!("hello from rust, {}!", s);
//!         let prev_len = s.byte_len();
//!         (new_string, prev_len)
//!     }
//! }
//! ```
//!
//! The binary will have all the standard arguments of a python interpreter (including a REPL!) but
//! it will have your modules loaded into the vm.
//!
//! See [`rustpython_derive`](../rustpython_derive/index.html) crate for documentation on macros used in the example above.

#![allow(clippy::needless_doctest_main)]

#[macro_use]
extern crate log;

#[cfg(feature = "flame-it")]
use vm::Settings;

mod interpreter;
mod settings;
mod shell;

#[cfg(feature = "rustpython-pylib")]
pub use rustpython_pylib as pylib;

use rustpython_vm::{
    AsObject, PyObjectRef, PyResult, VirtualMachine, builtins::PyBaseExceptionRef, scope::Scope,
};
use std::env;
use std::io::IsTerminal;
use std::process::ExitCode;

pub use interpreter::InterpreterBuilderExt;
pub use rustpython_vm::{self as vm, Interpreter, InterpreterBuilder};
pub use settings::{InstallPipMode, RunMode, parse_opts};
pub use shell::run_shell;

#[cfg(all(
    feature = "ssl",
    not(any(feature = "ssl-rustls", feature = "ssl-openssl")),
    not(target_arch = "wasm32"),
))]
compile_error!(
    "Feature \"ssl\" is now enabled by either \"ssl-rustls\" or \"ssl-openssl\". Do not manually pass \"ssl\" feature. To enable ssl-openssl, use --no-default-features to disable ssl-rustls*"
);

/// The main cli of the `rustpython` interpreter. This function will return `std::process::ExitCode`
/// based on the return code of the python code ran through the cli.
///
/// **Note**: This function provides no way to further initialize the VM after the builder is applied.
/// All VM initialization (adding native modules, init hooks, etc.) must be done through the
/// [`InterpreterBuilder`] parameter before calling this function.
#[must_use]
pub fn run(mut builder: InterpreterBuilder) -> ExitCode {
    env_logger::init();

    // NOTE: This is not a WASI convention. But it will be convenient since POSIX shell always defines it.
    #[cfg(target_os = "wasi")]
    {
        if let Ok(pwd) = env::var("PWD") {
            let _ = env::set_current_dir(pwd);
        };
    }

    let (settings, run_mode) = match parse_opts() {
        Ok(x) => x,
        Err(e) => {
            println!("{e}");
            return ExitCode::FAILURE;
        }
    };

    // don't translate newlines (\r\n <=> \n)
    #[cfg(windows)]
    {
        unsafe extern "C" {
            fn _setmode(fd: i32, flags: i32) -> i32;
        }
        unsafe {
            _setmode(0, libc::O_BINARY);
            _setmode(1, libc::O_BINARY);
            _setmode(2, libc::O_BINARY);
        }
    }

    builder = builder.settings(settings);

    #[cfg(feature = "pyo3")]
    {
        let def = rustpython_module_pyo3::module_def(&builder.ctx);
        builder = builder.add_native_module(def);
    }

    let interp = builder.interpreter();
    let exitcode = cfg_select! {
        feature = "capi" => rustpython_capi::run_main_interpreter(interp, move |vm| run_rustpython(vm, run_mode)),
        _ => interp.run_main(move |vm| run_rustpython(vm, run_mode)),
    };

    rustpython_vm::host_env::os::exit_code(exitcode)
}

fn get_pip(scope: Scope, vm: &VirtualMachine) -> PyResult<()> {
    let get_getpip = rustpython_vm::py_compile!(
        source = r#"\
__import__("io").TextIOWrapper(
    __import__("urllib.request").request.urlopen("https://bootstrap.pypa.io/get-pip.py")
).read()
"#,
        mode = "eval"
    );
    eprintln!("downloading get-pip.py...");
    let getpip_code = vm.run_code_obj(vm.new_code(get_getpip), vm.new_scope_with_builtins())?;
    let getpip_code: rustpython_vm::builtins::PyStrRef = getpip_code
        .downcast()
        .expect("TextIOWrapper.read() should return str");
    eprintln!("running get-pip.py...");
    vm.run_string(scope, getpip_code.expect_str(), "get-pip.py")?;
    Ok(())
}

fn install_pip(installer: InstallPipMode, scope: Scope, vm: &VirtualMachine) -> PyResult<()> {
    if !cfg!(feature = "ssl") {
        return Err(
            vm.new_system_error("install-pip requires rustpython be build with '--features=ssl'")
        );
    }

    match installer {
        InstallPipMode::Ensurepip => vm.run_module("ensurepip"),
        InstallPipMode::GetPip => get_pip(scope, vm),
    }
}

/// The working directory, when it can be named.
fn current_dir() -> Option<String> {
    env::current_dir().ok()?.into_os_string().into_string().ok()
}

/// `_Py_abspath`: `path` joined onto the working directory. Windows normalizes
/// the result through `GetFullPathNameW`; elsewhere it is left as joined.
fn abs_path(path: &str) -> String {
    if path.is_empty() || path == "." {
        return current_dir().unwrap_or_else(|| path.to_owned());
    }
    if cfg!(windows) {
        return std::path::absolute(path)
            .ok()
            .and_then(|p| p.into_os_string().into_string().ok())
            .unwrap_or_else(|| path.to_owned());
    }
    if std::path::Path::new(path).is_absolute() {
        return path.to_owned();
    }
    // Keep the relative path when the working directory is unavailable.
    current_dir().map_or_else(
        || path.to_owned(),
        |cwd| format!("{cwd}{}{path}", std::path::MAIN_SEPARATOR),
    )
}

/// `_PyPathConfig_ComputeSysPath0` for a script argument: the directory holding
/// the script, with symlinks resolved.
fn script_sys_path0(argv0: &str) -> String {
    // `realpath()`, which Windows has no counterpart for; there the path is
    // only made absolute.
    let resolved = if cfg!(windows) {
        Some(abs_path(argv0))
    } else {
        std::fs::canonicalize(argv0)
            .ok()
            .and_then(|p| p.into_os_string().into_string().ok())
    };
    let path0 = resolved.as_deref().unwrap_or(argv0);
    let Some(sep) = path0.rfind(std::path::is_separator) else {
        return String::new();
    };
    // Drop the trailing separator unless it is all that is left of a root.
    let end = if sep == 0 || (cfg!(windows) && path0[..sep].ends_with(':')) {
        sep + 1
    } else {
        sep
    };
    path0[..end].to_owned()
}

// pymain_run_file_obj in Modules/main.c
fn run_file(vm: &VirtualMachine, scope: Scope, argv0: &str) -> PyResult<u32> {
    // config_run_filename_abspath: __file__ and co_filename are absolute even
    // when the script was named relative to the working directory.
    let path = &abs_path(argv0);

    // Check if path is a package/directory with __main__.py
    if let Some(_importer) = get_importer(path, vm)? {
        vm.insert_sys_path(vm.new_pyobj(path.clone()))?;
        let runpy = vm.import("runpy", 0)?;
        let run_module_as_main = runpy.get_attr("_run_module_as_main", vm)?;
        run_module_as_main.call((vm::identifier!(vm, __main__).to_owned(), false), vm)?;
        return Ok(0);
    }

    // Add script directory to sys.path[0]
    if !vm.state.config.settings.safe_path {
        vm.insert_sys_path(vm.new_pyobj(script_sys_path0(argv0)))?;
    }

    cfg_select! {
        feature = "host_env" => match rustpython_vm::host_env::fs::metadata(path) {
            Ok(_) => vm.run_any_file(scope, path).map(|()| 0),
            Err(err) => cant_open_file(vm, path, &err),
        },
        _ => {
            // In sandbox mode, the binary reads the file and feeds source to the VM.
            // The VM itself has no filesystem access.
            let path = if path.is_empty() { "???" } else { path };
            match std::fs::read_to_string(path) {
                Ok(source) => vm.run_string(scope, &source, path).map(|_| 0),
                Err(err) => cant_open_file(vm, path, &err),
            }
        }
    }
}

fn cant_open_file(vm: &VirtualMachine, path: &str, err: &std::io::Error) -> PyResult<u32> {
    let program_name = &vm.state.config.paths.program_name;
    let filename_repr = vm.ctx.new_str(path).as_object().repr(vm)?;
    let errno = err.raw_os_error().unwrap_or(2);
    let strerror = vm::host_env::errno::strerror_string(errno).unwrap_or_default();
    writeln!(
        vm::stdlib::sys::PyStderr(vm),
        "{program_name}: can't open file {filename_repr}: [Errno {errno}] {strerror}"
    );
    Ok(2)
}

fn get_importer(path: &str, vm: &VirtualMachine) -> PyResult<Option<PyObjectRef>> {
    use rustpython_vm::builtins::PyDictRef;
    use rustpython_vm::convert::TryFromObject;

    let path_importer_cache = vm.sys_module.get_attr("path_importer_cache", vm)?;
    let path_importer_cache = PyDictRef::try_from_object(vm, path_importer_cache)?;
    if let Some(importer) = path_importer_cache.get_item_opt(path, vm)? {
        return Ok(Some(importer));
    }
    let path_obj = vm.ctx.new_str(path);
    let path_hooks = vm.sys_module.get_attr("path_hooks", vm)?;
    let mut importer = None;
    let path_hooks: Vec<PyObjectRef> = path_hooks.try_into_value(vm)?;
    for path_hook in path_hooks {
        match path_hook.call((path_obj.clone(),), vm) {
            Ok(imp) => {
                importer = Some(imp);
                break;
            }
            Err(e) if e.fast_isinstance(vm.ctx.exceptions.import_error) => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(if let Some(imp) = importer {
        let imp = path_importer_cache.get_or_insert(vm, path_obj.as_object(), || imp.clone())?;
        Some(imp)
    } else {
        None
    })
}

fn run_rustpython(vm: &VirtualMachine, run_mode: RunMode) -> u32 {
    #[cfg(feature = "flame-it")]
    let main_guard = flame::start_guard("RustPython main");

    let exitcode =
        pymain_run_python(vm, run_mode).unwrap_or_else(|exc| vm.handle_exit_exception(exc));

    #[cfg(feature = "flame-it")]
    {
        main_guard.end();
        if let Err(e) = write_profile(&vm.state.as_ref().config.settings) {
            error!("Error writing profile information: {}", e);
        }
    }

    exitcode
}

// pymain_run_python
fn pymain_run_python(vm: &VirtualMachine, run_mode: RunMode) -> PyResult<u32> {
    vm.check_stdlib_initialization()?;
    let scope = vm.new_scope_with_main()?;

    // The native warnings state already holds the default filters. Import the
    // Python module only when startup options require processing (-W, -b, dev).
    let warnoptions = vm.sys_module.dict().get_item_opt("warnoptions", vm)?;
    if warnoptions
        .as_ref()
        .and_then(|options| options.downcast_ref::<rustpython_vm::builtins::PyList>())
        .is_some_and(|options| options.__len__() > 0)
        && vm.import("warnings", 0).is_err()
    {
        warn!("Failed to import warnings module");
    }

    // Import site first, before setting sys.path[0]
    // This matches CPython's behavior where site.removeduppaths() runs
    // before sys.path[0] is set, preventing '' from being converted to cwd
    if vm.state.config.settings.import_site && vm.import("site", 0).is_err() {
        warn!(
            "Failed to import site, consider adding the Lib directory to your RUSTPYTHONPATH \
             environment variable",
        );
    }

    // CPython activates configured all-mode after site to avoid startup cycles.
    vm.apply_startup_lazy_imports();

    // _PyPathConfig_ComputeSysPath0 - set sys.path[0] after site import
    if !vm.state.config.settings.safe_path {
        let path0: Option<String> = match &run_mode {
            RunMode::Command(_) => Some(String::new()),
            RunMode::Module(_) => env::current_dir()
                .ok()
                .and_then(|p| p.to_str().map(|s| s.to_owned())),
            RunMode::Script(_) | RunMode::InstallPip(_) => None, // handled by run_script
            RunMode::Repl => Some(String::new()),
        };

        if let Some(path) = path0 {
            vm.insert_sys_path(vm.new_pyobj(path))?;
        }
    }

    // Enable faulthandler if -X faulthandler, PYTHONFAULTHANDLER or -X dev is set
    // _PyFaulthandler_Init()
    if vm.state.config.settings.faulthandler {
        let _ = vm.run_simple_string("import faulthandler; faulthandler.enable()");
    }

    let is_repl = matches!(run_mode, RunMode::Repl);
    // pymain_header
    if !vm.state.config.settings.quiet
        && (vm.state.config.settings.verbose > 0 || (is_repl && vm.stdin_is_interactive()))
    {
        eprintln!(
            "Welcome to the magnificent Rust Python {} interpreter \u{1f631} \u{1f596}",
            env!("CARGO_PKG_VERSION")
        );
        eprintln!(
            "RustPython {}.{}.{}",
            vm::version::MAJOR,
            vm::version::MINOR,
            vm::version::MICRO,
        );

        eprintln!("Type \"help\", \"copyright\", \"credits\" or \"license\" for more information.");
    }

    let set_running_main = vm.state.set_running_main();
    let exitcode =
        pymain_run_main(vm, scope, run_mode).unwrap_or_else(|exc| vm.handle_exit_exception(exc));
    if set_running_main {
        vm.state.set_not_running_main();
    }
    Ok(exitcode)
}

/// The part of pymain_run_python that runs while the main thread owns
/// `__main__`.
fn pymain_run_main(vm: &VirtualMachine, scope: Scope, run_mode: RunMode) -> PyResult<u32> {
    let is_repl = matches!(run_mode, RunMode::Repl);
    vm.check_signals()?;
    let result = match run_mode {
        RunMode::Command(command) => {
            debug!("Running command {command}");
            vm.run_string(scope, &command, "<string>").map(|_| 0)
        }
        RunMode::Module(module) => {
            debug!("Running module {module}");
            vm.run_module(&module).map(|()| 0)
        }
        RunMode::InstallPip(installer) => install_pip(installer, scope, vm).map(|()| 0),
        RunMode::Script(script_path) => {
            // pymain_run_file_obj
            debug!("Running script {script_path}");
            run_file(vm, scope, &script_path)
        }
        RunMode::Repl => Ok(pymain_run_stdin(vm, scope)),
    };
    let mut exitcode = result.unwrap_or_else(|exc| vm.handle_exit_exception(exc));
    vm.check_signals()?;

    pymain_repl(vm, !is_repl, &mut exitcode);
    Ok(exitcode)
}

/// pymain_err_print
///
/// Report `exc` and return the exit code if it ends the process.
fn pymain_err_print(vm: &VirtualMachine, exc: PyBaseExceptionRef) -> Option<u32> {
    if let Some(exitcode) = vm.handle_system_exit_and_keyboard_interrupt(&exc) {
        return Some(exitcode);
    }
    vm.print_uncaught_exception(&exc);
    None
}

// pymain_set_inspect
fn pymain_set_inspect(vm: &VirtualMachine, inspect: bool) {
    if let Err(exc) = vm.config_set_inspect(inspect) {
        eprintln!("Could not set the inspect flag");
        vm.print_uncaught_exception(&exc);
    }
}

// pymain_run_startup
fn pymain_run_startup(vm: &VirtualMachine, scope: Scope) -> Option<u32> {
    let exc = run_startup(vm, scope).err()?;
    pymain_err_print(vm, exc)
}

fn run_startup(vm: &VirtualMachine, scope: Scope) -> PyResult<()> {
    if vm.state.config.settings.ignore_environment {
        return Ok(());
    }
    let Some(startup) = env::var_os("PYTHONSTARTUP").filter(|value| !value.is_empty()) else {
        return Ok(());
    };
    let startup = vm.fsdecode(startup);
    vm.audit("cpython.run_startup", || (startup.clone(),))?;
    let could_not_open = |err: std::io::Error| {
        use vm::convert::ToPyException;
        writeln!(
            vm::stdlib::sys::PyStderr(vm),
            "Could not open PYTHONSTARTUP"
        );
        let exc = err.to_pyexception(vm);
        exc.as_object().set_attr("filename", startup.clone(), vm)?;
        Err(exc)
    };
    let path = startup.expect_str();
    cfg_select! {
        feature = "host_env" => match vm::host_env::fs::open(path) {
            Ok(_) => vm.run_any_file(scope, path),
            Err(err) => could_not_open(err),
        },
        _ => match std::fs::read_to_string(path) {
            Ok(source) => vm.run_string(scope, &source, path).map(drop),
            Err(err) => could_not_open(err),
        },
    }
}

// pymain_run_interactive_hook
fn pymain_run_interactive_hook(vm: &VirtualMachine) -> Option<u32> {
    let exc = run_interactive_hook(vm).err()?;
    writeln!(
        vm::stdlib::sys::PyStderr(vm),
        "Failed calling sys.__interactivehook__"
    );
    pymain_err_print(vm, exc)
}

fn run_interactive_hook(vm: &VirtualMachine) -> PyResult<()> {
    let Some(hook) = vm
        .sys_module
        .dict()
        .get_item_opt("__interactivehook__", vm)?
    else {
        return Ok(());
    };
    vm.audit("cpython.run_interactivehook", || (hook.clone(),))?;
    hook.call((), vm)?;
    Ok(())
}

// pymain_run_stdin
fn pymain_run_stdin(vm: &VirtualMachine, scope: Scope) -> u32 {
    if vm.stdin_is_interactive() {
        // do exit on SystemExit
        pymain_set_inspect(vm, false);

        if let Some(exitcode) = pymain_run_startup(vm, scope) {
            return exitcode;
        }
        if let Some(exitcode) = pymain_run_interactive_hook(vm) {
            return exitcode;
        }
    }
    pymain_run_repl(vm, false)
}

// pymain_repl
fn pymain_repl(vm: &VirtualMachine, run_code: bool, exitcode: &mut u32) {
    // Check this environment variable at the end, to give programs the
    // opportunity to set it from Python.
    if !vm.config_inspect()
        && !vm.state.config.settings.ignore_environment
        && env::var_os("PYTHONINSPECT").is_some_and(|value| !value.is_empty())
    {
        pymain_set_inspect(vm, true);
    }

    if !(vm.config_inspect() && vm.stdin_is_interactive() && run_code) {
        return;
    }

    pymain_set_inspect(vm, false);
    if let Some(code) = pymain_run_interactive_hook(vm) {
        *exitcode = code;
        return;
    }

    *exitcode = pymain_run_repl(vm, true);
}

// _pymain_run_repl
fn pymain_run_repl(vm: &VirtualMachine, startup: bool) -> u32 {
    run_repl(vm, startup).unwrap_or_else(|exc| vm.handle_exit_exception(exc))
}

fn run_repl(vm: &VirtualMachine, startup: bool) -> PyResult<u32> {
    vm.check_signals()?;
    vm.audit("cpython.run_stdin", || ())?;

    let tty = std::io::stdin().is_terminal();
    if tty
        && (vm.state.config.settings.ignore_environment
            || env::var_os("PYTHON_BASIC_REPL").is_none_or(|value| value.is_empty()))
    {
        match vm.import("_pyrepl", 0) {
            Ok(_) => return Ok(pymain_run_pyrepl(vm, startup)),
            Err(exc) if !exc.fast_isinstance(vm.ctx.exceptions.module_not_found_error) => {
                eprintln!("Could not import _pyrepl.main");
                return Err(exc);
            }
            Err(_) => {}
        }
    }

    // _PyRun_AnyFile(stdin, "<stdin>")
    vm.run_any_stdin()?;
    Ok(0)
}

// pymain_run_pyrepl
fn pymain_run_pyrepl(vm: &VirtualMachine, pythonstartup: bool) -> u32 {
    run_pyrepl(vm, pythonstartup).map_or_else(|exc| vm.handle_exit_exception(exc), |()| 0)
}

fn run_pyrepl(vm: &VirtualMachine, pythonstartup: bool) -> PyResult<()> {
    let pyrepl = vm.import("_pyrepl.main", 0).inspect_err(|_| {
        eprintln!("Could not import _pyrepl.main");
    })?;
    let console = pyrepl
        .get_attr("main", vm)
        .and_then(|main| main.get_attr("interactive_console", vm))
        .inspect_err(|_| {
            eprintln!("Could not access _pyrepl.main.interactive_console");
        })?;
    let kwargs: vm::function::KwArgs = [
        ("mainmodule".to_owned(), vm.add_main_module()?.into()),
        (
            "pythonstartup".to_owned(),
            vm.ctx.new_bool(pythonstartup).into(),
        ),
    ]
    .into_iter()
    .collect();
    console.call(vm::function::FuncArgs::new(vec![], kwargs), vm)?;
    Ok(())
}

#[cfg(feature = "flame-it")]
fn write_profile(settings: &Settings) -> Result<(), Box<dyn core::error::Error>> {
    use std::{fs, io};

    enum ProfileFormat {
        Html,
        Text,
        SpeedScope,
    }
    let profile_output = settings.profile_output.as_deref();
    let profile_format = match settings.profile_format.as_deref() {
        Some("html") => ProfileFormat::Html,
        Some("text") => ProfileFormat::Text,
        None if profile_output == Some("-".as_ref()) => ProfileFormat::Text,
        // spell-checker:ignore speedscope
        Some("speedscope") | None => ProfileFormat::SpeedScope,
        Some(other) => {
            error!("Unknown profile format {}", other);
            // TODO: Need to change to ExitCode or Termination
            std::process::exit(1);
        }
    };

    let profile_output = profile_output.unwrap_or_else(|| match profile_format {
        ProfileFormat::Html => "flame-graph.html".as_ref(),
        ProfileFormat::Text => "flame.txt".as_ref(),
        ProfileFormat::SpeedScope => "flamescope.json".as_ref(),
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
        ProfileFormat::SpeedScope => flamescope::dump(profile_output)?,
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustpython_vm::Interpreter;

    fn interpreter() -> Interpreter {
        InterpreterBuilder::new().init_stdlib().interpreter()
    }

    #[test]
    fn run_script() {
        interpreter().enter(|vm| {
            vm.unwrap_pyresult((|| {
                let scope = vm.new_scope_with_main()?;
                // test file run
                run_file(vm, scope, "extra_tests/snippets/dir_main/__main__.py")?;

                #[cfg(feature = "host_env")]
                {
                    let scope = vm.new_scope_with_main()?;
                    // test module run (directory with __main__.py)
                    run_file(vm, scope, "extra_tests/snippets/dir_main")?;
                }

                Ok(())
            })());
        })
    }
}

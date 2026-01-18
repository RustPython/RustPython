//! This is the `rustpython` binary. If you're looking to embed RustPython into your application,
//! you're likely looking for the [`rustpython_vm`] crate.
//!
//! You can install `rustpython` with `cargo install rustpython`, or if you'd like to inject your
//! own native modules you can make a binary crate that depends on the `rustpython` crate (and
//! probably [`rustpython_vm`], too), and make a `main.rs` that looks like:
//!
//! ```no_run
//! use rustpython_vm::{pymodule, py_freeze};
//! fn main() {
//!     rustpython::run(|vm| {
//!         vm.add_native_module("my_mod".to_owned(), Box::new(my_mod::make_module));
//!         vm.add_frozen(py_freeze!(source = "def foo(): pass", module_name = "other_thing"));
//!     });
//! }
//!
//! #[pymodule]
//! mod my_mod {
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

#![cfg_attr(all(target_os = "wasi", target_env = "p2"), feature(wasip2))]
#![allow(clippy::needless_doctest_main)]

#[macro_use]
extern crate log;

#[cfg(feature = "flame-it")]
use vm::Settings;

mod interpreter;
mod settings;
mod shell;

use rustpython_vm::{AsObject, PyObjectRef, PyResult, VirtualMachine, scope::Scope};
use std::env;
use std::io::IsTerminal;
use std::process::ExitCode;

pub use interpreter::InterpreterConfig;
pub use rustpython_vm as vm;
pub use settings::{InstallPipMode, RunMode, parse_opts};
pub use shell::run_shell;

#[cfg(all(
    feature = "ssl",
    not(any(feature = "ssl-rustls", feature = "ssl-openssl"))
))]
compile_error!(
    "Feature \"ssl\" is now enabled by either \"ssl-rustls\" or \"ssl-openssl\" to be enabled. Do not manually pass \"ssl\" feature. To enable ssl-openssl, use --no-default-features to disable ssl-rustls"
);

/// The main cli of the `rustpython` interpreter. This function will return `std::process::ExitCode`
/// based on the return code of the python code ran through the cli.
pub fn run(init: impl FnOnce(&mut VirtualMachine) + 'static) -> ExitCode {
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

    let mut config = InterpreterConfig::new().settings(settings);
    #[cfg(feature = "stdlib")]
    {
        config = config.init_stdlib();
    }
    config = config.init_hook(Box::new(init));

    let interp = config.interpreter();
    let exitcode = interp.run(move |vm| run_rustpython(vm, run_mode));

    rustpython_vm::common::os::exit_code(exitcode)
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
    let getpip_code = vm.run_code_obj(vm.ctx.new_code(get_getpip), vm.new_scope_with_builtins())?;
    let getpip_code: rustpython_vm::builtins::PyStrRef = getpip_code
        .downcast()
        .expect("TextIOWrapper.read() should return str");
    eprintln!("running get-pip.py...");
    vm.run_string(scope, getpip_code.as_str(), "get-pip.py".to_owned())?;
    Ok(())
}

fn install_pip(installer: InstallPipMode, scope: Scope, vm: &VirtualMachine) -> PyResult<()> {
    if !cfg!(feature = "ssl") {
        return Err(vm.new_exception_msg(
            vm.ctx.exceptions.system_error.to_owned(),
            "install-pip requires rustpython be build with '--features=ssl'".to_owned(),
        ));
    }

    match installer {
        InstallPipMode::Ensurepip => vm.run_module("ensurepip"),
        InstallPipMode::GetPip => get_pip(scope, vm),
    }
}

// pymain_run_file_obj in Modules/main.c
fn run_file(vm: &VirtualMachine, scope: Scope, path: &str) -> PyResult<()> {
    // Check if path is a package/directory with __main__.py
    if let Some(_importer) = get_importer(path, vm)? {
        vm.insert_sys_path(vm.new_pyobj(path))?;
        let runpy = vm.import("runpy", 0)?;
        let run_module_as_main = runpy.get_attr("_run_module_as_main", vm)?;
        run_module_as_main.call((vm::identifier!(vm, __main__).to_owned(), false), vm)?;
        return Ok(());
    }

    // Add script directory to sys.path[0]
    if !vm.state.config.settings.safe_path {
        let dir = std::path::Path::new(path)
            .parent()
            .and_then(|p| p.to_str())
            .unwrap_or("");
        vm.insert_sys_path(vm.new_pyobj(dir))?;
    }

    vm.run_any_file(scope, path)
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
        let imp = path_importer_cache.get_or_insert(vm, path_obj.into(), || imp.clone())?;
        Some(imp)
    } else {
        None
    })
}

// pymain_run_python
fn run_rustpython(vm: &VirtualMachine, run_mode: RunMode) -> PyResult<()> {
    #[cfg(feature = "flame-it")]
    let main_guard = flame::start_guard("RustPython main");

    let scope = vm.new_scope_with_main()?;

    // Import site first, before setting sys.path[0]
    // This matches CPython's behavior where site.removeduppaths() runs
    // before sys.path[0] is set, preventing '' from being converted to cwd
    let site_result = vm.import("site", 0);
    if site_result.is_err() {
        warn!(
            "Failed to import site, consider adding the Lib directory to your RUSTPYTHONPATH \
             environment variable",
        );
    }

    // Initialize warnings module to process sys.warnoptions
    // _PyWarnings_Init()
    if vm.import("warnings", 0).is_err() {
        warn!("Failed to import warnings module");
    }

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
    if !vm.state.config.settings.quiet
        && (vm.state.config.settings.verbose > 0 || (is_repl && std::io::stdin().is_terminal()))
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
    let res = match run_mode {
        RunMode::Command(command) => {
            debug!("Running command {command}");
            vm.run_string(scope.clone(), &command, "<string>".to_owned())
                .map(drop)
        }
        RunMode::Module(module) => {
            debug!("Running module {module}");
            vm.run_module(&module)
        }
        RunMode::InstallPip(installer) => install_pip(installer, scope.clone(), vm),
        RunMode::Script(script_path) => {
            // pymain_run_file_obj
            debug!("Running script {}", &script_path);
            run_file(vm, scope.clone(), &script_path)
        }
        RunMode::Repl => Ok(()),
    };
    let result = if is_repl || vm.state.config.settings.inspect {
        shell::run_shell(vm, scope)
    } else {
        res
    };

    #[cfg(feature = "flame-it")]
    {
        main_guard.end();
        if let Err(e) = write_profile(&vm.state.as_ref().config.settings) {
            error!("Error writing profile information: {}", e);
        }
    }

    result
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
        InterpreterConfig::new().init_stdlib().interpreter()
    }

    #[test]
    fn test_run_script() {
        interpreter().enter(|vm| {
            vm.unwrap_pyresult((|| {
                let scope = vm.new_scope_with_main()?;
                // test file run
                vm.run_any_file(scope, "extra_tests/snippets/dir_main/__main__.py")?;

                let scope = vm.new_scope_with_main()?;
                // test module run (directory with __main__.py)
                run_file(vm, scope, "extra_tests/snippets/dir_main")?;

                Ok(())
            })());
        })
    }
}

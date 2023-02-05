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
#![allow(clippy::needless_doctest_main)]

extern crate env_logger;
#[macro_use]
extern crate log;

#[cfg(feature = "flame-it")]
use vm::Settings;

mod interpreter;
mod settings;
mod shell;

use atty::Stream;
use clap::crate_version;
use rustpython_vm::{scope::Scope, PyResult, VirtualMachine};
use std::{env, process::ExitCode};

pub use interpreter::InterpreterConfig;
pub use rustpython_vm as vm;
pub use settings::{opts_with_clap, RunMode};

/// The main cli of the `rustpython` interpreter. This function will return with `std::process::ExitCode`
/// based on the return code of the python code ran through the cli.
pub fn run(init: impl FnOnce(&mut VirtualMachine) + 'static) -> ExitCode {
    env_logger::init();

    let (settings, run_mode) = opts_with_clap();

    // Be quiet if "quiet" arg is set OR stdin is not connected to a terminal
    let quiet_var = settings.quiet || !atty::is(Stream::Stdin);

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

    let mut config = InterpreterConfig::new().settings(settings);
    #[cfg(feature = "stdlib")]
    {
        config = config.init_stdlib();
    }
    config = config.init_hook(Box::new(init));

    let interp = config.interpreter();
    let exitcode = interp.run(move |vm| run_rustpython(vm, run_mode, quiet_var));

    ExitCode::from(exitcode)
}

fn setup_main_module(vm: &VirtualMachine) -> PyResult<Scope> {
    let scope = vm.new_scope_with_builtins();
    let main_module = vm.new_module("__main__", scope.globals.clone(), None);
    main_module
        .dict()
        .and_then(|d| {
            d.set_item("__annotations__", vm.ctx.new_dict().into(), vm)
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
    let getpip_code = vm.run_code_obj(vm.ctx.new_code(get_getpip), scope.clone())?;
    let getpip_code: rustpython_vm::builtins::PyStrRef = getpip_code
        .downcast()
        .expect("TextIOWrapper.read() should return str");
    eprintln!("running get-pip.py...");
    vm.run_code_string(scope, getpip_code.as_str(), "get-pip.py".to_owned())?;
    Ok(())
}

#[cfg(feature = "ssl")]
fn ensurepip(_: Scope, vm: &VirtualMachine) -> PyResult<()> {
    vm.run_module("ensurepip")
}

fn install_pip(_installer: &str, _scope: Scope, vm: &VirtualMachine) -> PyResult<()> {
    #[cfg(feature = "ssl")]
    {
        match _installer {
            "ensurepip" => ensurepip(_scope, vm),
            "get-pip" => get_pip(_scope, vm),
            _ => unreachable!(),
        }
    }

    #[cfg(not(feature = "ssl"))]
    Err(vm.new_exception_msg(
        vm.ctx.exceptions.system_error.to_owned(),
        "install-pip requires rustpython be build with '--features=ssl'".to_owned(),
    ))
}

fn run_rustpython(vm: &VirtualMachine, run_mode: RunMode, quiet: bool) -> PyResult<()> {
    #[cfg(feature = "flame-it")]
    let main_guard = flame::start_guard("RustPython main");

    let scope = setup_main_module(vm)?;

    let site_result = vm.import("site", None, 0);

    if site_result.is_err() {
        warn!(
            "Failed to import site, consider adding the Lib directory to your RUSTPYTHONPATH \
             environment variable",
        );
    }

    match run_mode {
        RunMode::Command(command) => {
            debug!("Running command {}", command);
            vm.run_code_string(scope, &command, "<stdin>".to_owned())?;
        }
        RunMode::Module(module) => {
            debug!("Running module {}", module);
            vm.run_module(&module)?;
        }
        RunMode::InstallPip(installer) => {
            install_pip(&installer, scope, vm)?;
        }
        RunMode::ScriptInteractive(script, interactive) => {
            if let Some(script) = script {
                debug!("Running script {}", &script);
                vm.run_script(scope.clone(), &script)?;
            } else if !quiet {
                println!(
                    "Welcome to the magnificent Rust Python {} interpreter \u{1f631} \u{1f596}",
                    crate_version!()
                );
            }
            if interactive {
                shell::run_shell(vm, scope)?;
            }
        }
    }
    #[cfg(feature = "flame-it")]
    {
        main_guard.end();
        if let Err(e) = write_profile(&vm.state.as_ref().settings) {
            error!("Error writing profile information: {}", e);
        }
    }
    Ok(())
}

#[cfg(feature = "flame-it")]
fn write_profile(settings: &Settings) -> Result<(), Box<dyn std::error::Error>> {
    use std::{fs, io};

    enum ProfileFormat {
        Html,
        Text,
        Speedscope,
    }
    let profile_output = settings.profile_output.as_deref();
    let profile_format = match settings.profile_format.as_deref() {
        Some("html") => ProfileFormat::Html,
        Some("text") => ProfileFormat::Text,
        None if profile_output == Some("-".as_ref()) => ProfileFormat::Text,
        Some("speedscope") | None => ProfileFormat::Speedscope,
        Some(other) => {
            error!("Unknown profile format {}", other);
            // TODO: Need to change to ExitCode or Termination
            std::process::exit(1);
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
                let scope = setup_main_module(vm)?;
                // test file run
                vm.run_script(scope, "extra_tests/snippets/dir_main/__main__.py")?;

                let scope = setup_main_module(vm)?;
                // test module run
                vm.run_script(scope, "extra_tests/snippets/dir_main")?;

                Ok(())
            })());
        })
    }
}

mod readline;
#[cfg(not(target_os = "wasi"))]
mod rustyline_helper;

use rustpython_compiler::{compile, error::CompileError, error::CompileErrorType};
use rustpython_parser::error::ParseErrorType;
use rustpython_vm::{
    obj::objtype,
    print_exception,
    pyobject::{ItemProtocol, PyObjectRef, PyResult},
    scope::Scope,
    VirtualMachine,
};

use readline::{Readline, ReadlineResult};

enum ShellExecResult {
    Ok,
    PyErr(PyObjectRef),
    Continue,
}

fn shell_exec(vm: &VirtualMachine, source: &str, scope: Scope) -> ShellExecResult {
    match vm.compile(source, compile::Mode::Single, "<stdin>".to_string()) {
        Ok(code) => {
            match vm.run_code_obj(code, scope.clone()) {
                Ok(value) => {
                    // Save non-None values as "_"
                    if !vm.is_none(&value) {
                        let key = "_";
                        scope.globals.set_item(key, value, vm).unwrap();
                    }
                    ShellExecResult::Ok
                }
                Err(err) => ShellExecResult::PyErr(err),
            }
        }
        Err(CompileError {
            error: CompileErrorType::Parse(ParseErrorType::EOF),
            ..
        }) => ShellExecResult::Continue,
        Err(err) => ShellExecResult::PyErr(vm.new_syntax_error(&err)),
    }
}

pub fn run_shell(vm: &VirtualMachine, scope: Scope) -> PyResult<()> {
    println!(
        "Welcome to the magnificent Rust Python {} interpreter \u{1f631} \u{1f596}",
        crate_version!()
    );

    let mut repl = Readline::new(vm, scope.clone());
    let mut full_input = String::new();

    // Retrieve a `history_path_str` dependent on the OS
    let repl_history_path = match dirs::config_dir() {
        Some(mut path) => {
            path.push("rustpython");
            path.push("repl_history.txt");
            path
        }
        None => ".repl_history.txt".into(),
    };

    if repl.load_history(&repl_history_path).is_err() {
        println!("No previous history.");
    }

    let mut continuing = false;

    loop {
        let prompt_name = if continuing { "ps2" } else { "ps1" };
        let prompt = vm
            .get_attribute(vm.sys_module.clone(), prompt_name)
            .and_then(|prompt| vm.to_str(&prompt));
        let prompt = match prompt {
            Ok(ref s) => s.as_str(),
            Err(_) => "",
        };
        let result = match repl.readline(prompt) {
            ReadlineResult::Line(line) => {
                debug!("You entered {:?}", line);

                repl.add_history_entry(line.trim_end()).unwrap();

                let stop_continuing = line.is_empty();

                if full_input.is_empty() {
                    full_input = line;
                } else {
                    full_input.push_str(&line);
                }
                full_input.push_str("\n");

                if continuing {
                    if stop_continuing {
                        continuing = false;
                    } else {
                        continue;
                    }
                }

                match shell_exec(vm, &full_input, scope.clone()) {
                    ShellExecResult::Ok => {
                        full_input.clear();
                        Ok(())
                    }
                    ShellExecResult::Continue => {
                        continuing = true;
                        Ok(())
                    }
                    ShellExecResult::PyErr(err) => {
                        full_input.clear();
                        Err(err)
                    }
                }
            }
            ReadlineResult::Interrupt => {
                continuing = false;
                full_input.clear();
                let keyboard_interrupt = vm
                    .new_empty_exception(vm.ctx.exceptions.keyboard_interrupt.clone())
                    .unwrap();
                Err(keyboard_interrupt)
            }
            ReadlineResult::EOF => {
                break;
            }
            ReadlineResult::EncodingError => {
                eprintln!("Invalid UTF-8 entered");
                Ok(())
            }
            ReadlineResult::Other(err) => {
                eprintln!("Readline error: {:?}", err);
                break;
            }
            ReadlineResult::IO(err) => {
                eprintln!("IO error: {:?}", err);
                break;
            }
        };

        if let Err(exc) = result {
            if objtype::isinstance(&exc, &vm.ctx.exceptions.system_exit) {
                repl.save_history(&repl_history_path).unwrap();
                return Err(exc);
            }
            print_exception(vm, &exc);
        }
    }
    repl.save_history(&repl_history_path).unwrap();

    Ok(())
}

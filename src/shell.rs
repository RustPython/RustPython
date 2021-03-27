mod helper;

use rustpython_parser::error::{LexicalErrorType, ParseErrorType};
use rustpython_vm::readline::{Readline, ReadlineResult};
use rustpython_vm::{
    compile::{self, CompileError, CompileErrorType},
    exceptions::{print_exception, PyBaseExceptionRef},
    pyobject::{BorrowValue, PyResult, TypeProtocol},
    scope::Scope,
    VirtualMachine,
};

enum ShellExecResult {
    Ok,
    PyErr(PyBaseExceptionRef),
    Continue,
}

fn shell_exec(vm: &VirtualMachine, source: &str, scope: Scope) -> ShellExecResult {
    match vm.compile(source, compile::Mode::Single, "<stdin>".to_owned()) {
        Ok(code) => match vm.run_code_obj(code, scope) {
            Ok(_val) => ShellExecResult::Ok,
            Err(err) => ShellExecResult::PyErr(err),
        },
        Err(CompileError {
            error: CompileErrorType::Parse(ParseErrorType::Lexical(LexicalErrorType::Eof)),
            ..
        })
        | Err(CompileError {
            error: CompileErrorType::Parse(ParseErrorType::Eof),
            ..
        }) => ShellExecResult::Continue,
        Err(err) => ShellExecResult::PyErr(vm.new_syntax_error(&err)),
    }
}

pub fn run_shell(vm: &VirtualMachine, scope: Scope) -> PyResult<()> {
    let mut repl = Readline::new(helper::ShellHelper::new(vm, scope.globals.clone()));
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
            Ok(ref s) => s.borrow_value(),
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
                full_input.push('\n');

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
                let keyboard_interrupt =
                    vm.new_exception_empty(vm.ctx.exceptions.keyboard_interrupt.clone());
                Err(keyboard_interrupt)
            }
            ReadlineResult::Eof => {
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
            ReadlineResult::Io(err) => {
                eprintln!("IO error: {:?}", err);
                break;
            }
        };

        if let Err(exc) = result {
            if exc.isinstance(&vm.ctx.exceptions.system_exit) {
                repl.save_history(&repl_history_path).unwrap();
                return Err(exc);
            }
            print_exception(vm, exc);
        }
    }
    repl.save_history(&repl_history_path).unwrap();

    Ok(())
}

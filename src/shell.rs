mod helper;

use rustpython_parser::error::{LexicalErrorType, ParseErrorType};
use rustpython_vm::{
    builtins::PyBaseExceptionRef,
    compiler::{self, CompileError, CompileErrorBody, CompileErrorType},
    readline::{Readline, ReadlineResult},
    scope::Scope,
    AsObject, PyResult, VirtualMachine,
};

use rustpython_parser::token::Tok;

enum ShellExecResult {
    Ok,
    PyErr(PyBaseExceptionRef),
    Continue,
}

fn shell_exec(
    vm: &VirtualMachine,
    source: &str,
    scope: Scope,
    empty_line_given: bool,
    continuing: bool,
) -> ShellExecResult {
    match vm.compile(source, compiler::Mode::Single, "<stdin>".to_owned()) {
        Ok(code) => {
            if empty_line_given || !continuing {
                // We want to execute the full code
                match vm.run_code_obj(code, scope) {
                    Ok(_val) => ShellExecResult::Ok,
                    Err(err) => ShellExecResult::PyErr(err),
                }
            } else {
                // We can just return an ok result
                ShellExecResult::Ok
            }
        }
        Err(CompileError {
            body:
                CompileErrorBody {
                    error: CompileErrorType::Parse(ParseErrorType::Lexical(LexicalErrorType::Eof)),
                    ..
                },
            ..
        })
        | Err(CompileError {
            body:
                CompileErrorBody {
                    error: CompileErrorType::Parse(ParseErrorType::Eof),
                    ..
                },
            ..
        }) => ShellExecResult::Continue,
        Err(err) => {
            let bad_error = match err.body.error {
                CompileErrorType::Parse(ref p) => {
                    if matches!(
                        p,
                        ParseErrorType::Lexical(LexicalErrorType::IndentationError)
                    ) {
                        continuing
                    } else {
                        !matches!(p, ParseErrorType::UnrecognizedToken(Tok::Dedent, _))
                    }
                }
                _ => false,
            };

            if empty_line_given || bad_error {
                ShellExecResult::PyErr(vm.new_syntax_error(&err))
            } else {
                ShellExecResult::Continue
            }
        }
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
            .sys_module
            .clone()
            .get_attr(prompt_name, vm)
            .and_then(|prompt| prompt.str(vm));
        let prompt = match prompt {
            Ok(ref s) => s.as_str(),
            Err(_) => "",
        };
        let result = match repl.readline(prompt) {
            ReadlineResult::Line(line) => {
                debug!("You entered {:?}", line);

                repl.add_history_entry(line.trim_end()).unwrap();

                let empty_line_given = line.is_empty();

                if full_input.is_empty() {
                    full_input = line;
                } else {
                    full_input.push_str(&line);
                }
                full_input.push('\n');

                match shell_exec(vm, &full_input, scope.clone(), empty_line_given, continuing) {
                    ShellExecResult::Ok => {
                        if continuing {
                            if empty_line_given {
                                // We should be exiting continue mode
                                continuing = false;
                                full_input.clear();
                                Ok(())
                            } else {
                                // We should stay in continue mode
                                continuing = true;
                                Ok(())
                            }
                        } else {
                            // We aren't in continue mode so proceed normally
                            continuing = false;
                            full_input.clear();
                            Ok(())
                        }
                    }
                    ShellExecResult::Continue => {
                        continuing = true;
                        Ok(())
                    }
                    ShellExecResult::PyErr(err) => {
                        continuing = false;
                        full_input.clear();
                        Err(err)
                    }
                }
            }
            ReadlineResult::Interrupt => {
                continuing = false;
                full_input.clear();
                let keyboard_interrupt =
                    vm.new_exception_empty(vm.ctx.exceptions.keyboard_interrupt.to_owned());
                Err(keyboard_interrupt)
            }
            ReadlineResult::Eof => {
                break;
            }
            ReadlineResult::Other(err) => {
                eprintln!("Readline error: {err:?}");
                break;
            }
            ReadlineResult::Io(err) => {
                eprintln!("IO error: {err:?}");
                break;
            }
        };

        if let Err(exc) = result {
            if exc.fast_isinstance(vm.ctx.exceptions.system_exit) {
                repl.save_history(&repl_history_path).unwrap();
                return Err(exc);
            }
            vm.print_exception(exc);
        }
    }
    repl.save_history(&repl_history_path).unwrap();

    Ok(())
}

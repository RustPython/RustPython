mod helper;

use rustpython_compiler::{
    CompileError, ParseError, parser::{FStringErrorType, LexicalErrorType, ParseErrorType}
};
use rustpython_vm::{
    AsObject, PyResult, VirtualMachine, builtins::PyBaseExceptionRef,
    compiler, readline::{Readline, ReadlineResult}, scope::Scope,
};

enum ShellExecResult {
    Ok,
    PyErr(PyBaseExceptionRef),
    ContinueBlock,
    ContinueLine,
}

fn shell_exec(
    vm: &VirtualMachine,
    source: &str,
    scope: Scope,
    empty_line_given: bool,
    continuing_block: bool,
) -> ShellExecResult {
    #[cfg(windows)]
    let source = &source.replace("\r\n", "\n");

    match vm.compile(source, compiler::Mode::Single, "<stdin>".to_owned()) {
        Ok(code) => {
            if empty_line_given || !continuing_block {
                vm.run_code_obj(code, scope)
                    .map(|_| ShellExecResult::Ok)
                    .unwrap_or_else(ShellExecResult::PyErr)
            } else {
                ShellExecResult::Ok
            }
        }

        Err(CompileError::Parse(ParseError { error, raw_location, .. })) => {
            use LexicalErrorType::*;
            use ParseErrorType::*;

            match &error {
                Lexical(Eof) |
                Lexical(FStringError(FStringErrorType::UnterminatedTripleQuotedString)) => {
                    ShellExecResult::ContinueLine
                }

                Lexical(UnclosedStringError) => {
                    let loc = raw_location.start().to_usize();
                    let mut iter = source.chars().skip(loc);
                    if iter.next().map_or(false, |q| iter.next() == Some(q) && iter.next() == Some(q)) {
                        return ShellExecResult::ContinueLine;
                    }
                    ShellExecResult::ContinueBlock
                }

                Lexical(IndentationError) => {
                    if continuing_block {
                        ShellExecResult::PyErr(vm.new_syntax_error(&CompileError::Parse(ParseError {
                            error,
                            raw_location,
                        }), Some(source)))
                    } else {
                        ShellExecResult::ContinueBlock
                    }
                }

                OtherError(msg) if msg.starts_with("Expected an indented block") => {
                    if continuing_block {
                        ShellExecResult::PyErr(vm.new_syntax_error(&CompileError::Parse(ParseError {
                            error,
                            raw_location,
                        }), Some(source)))
                    } else {
                        ShellExecResult::ContinueBlock
                    }
                }

                _ if empty_line_given => {
                    ShellExecResult::PyErr(vm.new_syntax_error(&CompileError::Parse(ParseError {
                        error,
                        raw_location,
                    }), Some(source)))
                }

                _ => ShellExecResult::ContinueBlock,
            }
        }

        Err(err) => {
            ShellExecResult::PyErr(vm.new_syntax_error(&err, Some(source)))
        }
    }
}

pub fn run_shell(vm: &VirtualMachine, scope: Scope) -> PyResult<()> {
    let mut repl = Readline::new(helper::ShellHelper::new(vm, scope.globals.clone()));
    let mut full_input = String::new();

    let repl_history_path = dirs::config_dir()
        .map(|mut path| { path.push("rustpython/repl_history.txt"); path })
        .unwrap_or_else(|| ".repl_history.txt".into());

    if repl.load_history(&repl_history_path).is_err() {
        println!("No previous history.");
    }

    let mut continuing_block = false;
    let mut continuing_line = false;

    loop {
        let prompt_name = if continuing_block || continuing_line { "ps2" } else { "ps1" };
        let prompt = vm.sys_module.get_attr(prompt_name, vm)
            .and_then(|p| p.str(vm)).map(|s| s.as_str()).unwrap_or("");

        continuing_line = false;
        match repl.readline(prompt) {
            ReadlineResult::Line(line) => {
                #[cfg(debug_assertions)]
                debug!("You entered {line:?}");

                repl.add_history_entry(line.trim_end()).ok();

                let empty_line = line.trim().is_empty();
                if full_input.is_empty() {
                    full_input = line;
                } else {
                    full_input.push_str(&line);
                }
                full_input.push('\n');

                match shell_exec(vm, &full_input, scope.clone(), empty_line, continuing_block) {
                    ShellExecResult::Ok => {
                        if !continuing_block || empty_line {
                            full_input.clear();
                            continuing_block = false;
                        }
                    }
                    ShellExecResult::ContinueLine => continuing_line = true,
                    ShellExecResult::ContinueBlock => continuing_block = true,
                    ShellExecResult::PyErr(err) => {
                        continuing_block = false;
                        full_input.clear();
                        vm.print_exception(err);
                    }
                }
            }

            ReadlineResult::Interrupt => {
                continuing_block = false;
                full_input.clear();
                let interrupt = vm.new_exception_empty(vm.ctx.exceptions.keyboard_interrupt.clone());
                vm.print_exception(interrupt);
            }

            ReadlineResult::Eof => break,

            ReadlineResult::Other(err) |
            ReadlineResult::Io(err) => {
                eprintln!("Readline error: {err:?}");
                break;
            }
        }
    }

    repl.save_history(&repl_history_path).unwrap();
    Ok(())
}

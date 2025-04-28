mod helper;

use rustpython_compiler::{
    CompileError, ParseError, parser::FStringErrorType, parser::LexicalErrorType,
    parser::ParseErrorType,
};
use rustpython_vm::{
    AsObject, PyResult, VirtualMachine,
    builtins::PyBaseExceptionRef,
    compiler::{self},
    readline::{Readline, ReadlineResult},
    scope::Scope,
};

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
    // compiling expects only UNIX style line endings, and will replace windows line endings
    // internally. Since we might need to analyze the source to determine if an error could be
    // resolved by future input, we need the location from the error to match the source code that
    // was actually compiled.
    #[cfg(windows)]
    let source = &source.replace("\r\n", "\n");
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
        Err(CompileError::Parse(ParseError {
            error: ParseErrorType::Lexical(LexicalErrorType::Eof),
            ..
        })) => ShellExecResult::Continue,
        Err(CompileError::Parse(ParseError {
            error:
                ParseErrorType::Lexical(LexicalErrorType::FStringError(
                    FStringErrorType::UnterminatedTripleQuotedString,
                )),
            ..
        })) => ShellExecResult::Continue,
        Err(err) => {
            // Check if the error is from an unclosed triple quoted string (which should always
            // continue)
            match err {
                CompileError::Parse(ParseError {
                    error: ParseErrorType::Lexical(LexicalErrorType::UnclosedStringError),
                    raw_location,
                    ..
                }) => {
                    let loc = raw_location.start().to_usize();
                    let mut iter = source.chars();
                    if let Some(quote) = iter.nth(loc) {
                        if iter.next() == Some(quote) && iter.next() == Some(quote) {
                            return ShellExecResult::Continue;
                        }
                    }
                }
                _ => (),
            };

            // bad_error == true if we are handling an error that should be thrown even if we are continuing
            // if its an indentation error, set to true if we are continuing and the error is on column 0,
            // since indentations errors on columns other than 0 should be ignored.
            // if its an unrecognized token for dedent, set to false

            let bad_error = match err {
                CompileError::Parse(ref p) => {
                    match &p.error {
                        ParseErrorType::Lexical(LexicalErrorType::IndentationError) => continuing, // && p.location.is_some()
                        ParseErrorType::OtherError(msg) => {
                            if msg.starts_with("Expected an indented block") {
                                continuing
                            } else {
                                true
                            }
                        }
                        _ => true, // !matches!(p, ParseErrorType::UnrecognizedToken(Tok::Dedent, _))
                    }
                }
                _ => true, // It is a bad error for everything else
            };

            // If we are handling an error on an empty line or an error worthy of throwing
            if empty_line_given || bad_error {
                ShellExecResult::PyErr(vm.new_syntax_error(&err, Some(source)))
            } else {
                ShellExecResult::Continue
            }
        }
    }
}

/// Enter a repl loop
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

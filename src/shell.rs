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
#[cfg(not(target_os = "wasi"))]
use rustyline_helper::ShellHelper;

use std::io;
use std::path::Path;

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

enum ReadlineResult {
    Line(String),
    EOF,
    Interrupt,
    IO(std::io::Error),
    EncodingError,
    Other(Box<dyn std::error::Error>),
}

#[allow(unused)]
struct BasicReadline;

#[allow(unused)]
impl BasicReadline {
    fn new(_vm: &VirtualMachine, _scope: Scope) -> Self {
        BasicReadline
    }

    fn load_history(&mut self, _path: &Path) -> io::Result<()> {
        Ok(())
    }

    fn save_history(&mut self, _path: &Path) -> io::Result<()> {
        Ok(())
    }

    fn add_history_entry(&mut self, _entry: &str) {}

    fn readline(&mut self, prompt: &str) -> ReadlineResult {
        use std::io::prelude::*;
        print!("{}", prompt);
        if let Err(e) = io::stdout().flush() {
            return ReadlineResult::IO(e);
        }

        match io::stdin().lock().lines().next() {
            Some(Ok(line)) => ReadlineResult::Line(line),
            None => ReadlineResult::EOF,
            Some(Err(e)) => match e.kind() {
                io::ErrorKind::Interrupted => ReadlineResult::Interrupt,
                io::ErrorKind::InvalidData => ReadlineResult::EncodingError,
                _ => ReadlineResult::IO(e),
            },
        }
    }
}

#[cfg(target_os = "wasi")]
type Readline = BasicReadline;

#[cfg(not(target_os = "wasi"))]
struct RustylineReadline<'vm> {
    repl: rustyline::Editor<ShellHelper<'vm>>,
}
#[cfg(not(target_os = "wasi"))]
impl<'vm> RustylineReadline<'vm> {
    fn new(vm: &'vm VirtualMachine, scope: Scope) -> Self {
        use rustyline::{CompletionType, Config, Editor};
        let mut repl = Editor::with_config(
            Config::builder()
                .completion_type(CompletionType::List)
                .build(),
        );
        repl.set_helper(Some(ShellHelper::new(vm, scope)));
        RustylineReadline { repl }
    }

    fn load_history(&mut self, path: &Path) -> rustyline::Result<()> {
        self.repl.load_history(path)
    }

    fn save_history(&mut self, path: &Path) -> rustyline::Result<()> {
        if !path.exists() {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
        }
        self.repl.save_history(path)
    }

    fn add_history_entry(&mut self, entry: &str) {
        self.repl.add_history_entry(entry);
    }

    fn readline(&mut self, prompt: &str) -> ReadlineResult {
        use rustyline::error::ReadlineError;
        match self.repl.readline(prompt) {
            Ok(line) => ReadlineResult::Line(line),
            Err(ReadlineError::Interrupted) => ReadlineResult::Interrupt,
            Err(ReadlineError::Eof) => ReadlineResult::EOF,
            Err(ReadlineError::Io(e)) => ReadlineResult::IO(e),
            #[cfg(unix)]
            Err(ReadlineError::Utf8Error) => ReadlineResult::EncodingError,
            #[cfg(windows)]
            Err(ReadlineError::Decode(_)) => ReadlineResult::EncodingError,
            Err(e) => ReadlineResult::Other(e.into()),
        }
    }
}

#[cfg(not(target_os = "wasi"))]
type Readline<'a> = RustylineReadline<'a>;

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

                repl.add_history_entry(line.trim_end());

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

use std::io;
use std::path::Path;

use rustpython_vm::{scope::Scope, VirtualMachine};

type OtherError = Box<dyn std::error::Error>;
type OtherResult<T> = Result<T, OtherError>;

pub enum ReadlineResult {
    Line(String),
    EOF,
    Interrupt,
    IO(std::io::Error),
    EncodingError,
    Other(OtherError),
}

#[allow(unused)]
mod basic_readline {
    use super::*;

    pub struct BasicReadline<'vm> {
        vm: &'vm VirtualMachine,
    }

    impl<'vm> BasicReadline<'vm> {
        pub fn new(vm: &'vm VirtualMachine, _scope: Scope) -> Self {
            BasicReadline { vm }
        }

        pub fn load_history(&mut self, _path: &Path) -> OtherResult<()> {
            Ok(())
        }

        pub fn save_history(&mut self, _path: &Path) -> OtherResult<()> {
            Ok(())
        }

        pub fn add_history_entry(&mut self, _entry: &str) -> OtherResult<()> {
            Ok(())
        }

        pub fn readline(&mut self, prompt: &str) -> ReadlineResult {
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
}

#[cfg(not(target_os = "wasi"))]
mod rustyline_readline {
    use super::{super::rustyline_helper::ShellHelper, *};

    pub struct RustylineReadline<'vm> {
        repl: rustyline::Editor<ShellHelper<'vm>>,
    }

    impl<'vm> RustylineReadline<'vm> {
        pub fn new(vm: &'vm VirtualMachine, scope: Scope) -> Self {
            use rustyline::{At, Cmd, CompletionType, Config, Editor, KeyPress, Movement, Word};
            let mut repl = Editor::with_config(
                Config::builder()
                    .completion_type(CompletionType::List)
                    .tab_stop(8)
                    .build(),
            );
            repl.bind_sequence(
                KeyPress::ControlLeft,
                Cmd::Move(Movement::BackwardWord(1, Word::Vi)),
            );
            repl.bind_sequence(
                KeyPress::ControlRight,
                Cmd::Move(Movement::ForwardWord(1, At::AfterEnd, Word::Vi)),
            );
            repl.set_helper(Some(ShellHelper::new(vm, scope)));
            RustylineReadline { repl }
        }

        pub fn load_history(&mut self, path: &Path) -> OtherResult<()> {
            self.repl.load_history(path)?;
            Ok(())
        }

        pub fn save_history(&mut self, path: &Path) -> OtherResult<()> {
            if !path.exists() {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
            }
            self.repl.save_history(path)?;
            Ok(())
        }

        pub fn add_history_entry(&mut self, entry: &str) -> OtherResult<()> {
            self.repl.add_history_entry(entry);
            Ok(())
        }

        pub fn readline(&mut self, prompt: &str) -> ReadlineResult {
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
}

#[cfg(target_os = "wasi")]
type ReadlineInner<'vm> = basic_readline::BasicReadline<'vm>;

#[cfg(not(target_os = "wasi"))]
type ReadlineInner<'vm> = rustyline_readline::RustylineReadline<'vm>;

pub struct Readline<'vm>(ReadlineInner<'vm>);

impl<'vm> Readline<'vm> {
    pub fn new(vm: &'vm VirtualMachine, scope: Scope) -> Self {
        Readline(ReadlineInner::new(vm, scope))
    }
    pub fn load_history(&mut self, path: &Path) -> OtherResult<()> {
        self.0.load_history(path)
    }
    pub fn save_history(&mut self, path: &Path) -> OtherResult<()> {
        self.0.save_history(path)
    }
    pub fn add_history_entry(&mut self, entry: &str) -> OtherResult<()> {
        self.0.add_history_entry(entry)
    }
    pub fn readline(&mut self, prompt: &str) -> ReadlineResult {
        self.0.readline(prompt)
    }
}

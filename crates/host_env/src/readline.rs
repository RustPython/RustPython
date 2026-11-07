//! Readline interface for REPLs
//!
//! This module provides a common interface for reading lines from the console, with support for history and completion.
//! It uses the [`rustyline`] crate on non-WASM platforms and a custom implementation on WASM platforms.

use std::{io, path::Path};

type OtherError = Box<dyn core::error::Error>;
type OtherResult<T> = Result<T, OtherError>;

pub enum ReadlineResult {
    Line(String),
    Eof,
    Interrupt,
    Io(io::Error),
    #[cfg(unix)]
    OsError(String),
    Other(OtherError),
}

fn stdio_readline(prompt: &str) -> ReadlineResult {
    use std::io::prelude::*;
    eprint!("{prompt}");
    if let Err(e) = io::stderr().flush() {
        return ReadlineResult::Io(e);
    }

    let line = io::stdin().lock().lines().next();
    match line {
        Some(Ok(line)) => ReadlineResult::Line(line),
        None => ReadlineResult::Eof,
        Some(Err(e)) if e.kind() == io::ErrorKind::Interrupted => ReadlineResult::Interrupt,
        Some(Err(e)) => ReadlineResult::Io(e),
    }
}

#[allow(unused)]
pub mod basic_readline {
    use super::*;

    pub trait Helper {}

    impl<T> Helper for T {}

    pub struct Readline<H: Helper> {
        helper: H,
    }

    impl<H: Helper> Readline<H> {
        pub const fn new(helper: H) -> Self {
            Self { helper }
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
            stdio_readline(prompt)
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub mod rustyline_readline {
    use super::*;

    pub trait Helper: rustyline::Helper {}
    impl<T: rustyline::Helper> Helper for T {}

    /// Readline: the REPL
    pub struct Readline<H: Helper> {
        repl: rustyline::Editor<H, rustyline::history::DefaultHistory>,
    }

    #[cfg(windows)]
    const EOF_CHAR: &str = "\u{001A}";

    impl<H: Helper> Readline<H> {
        pub fn new(helper: H) -> Self {
            use rustyline::*;
            let mut repl = Editor::with_config(
                Config::builder()
                    .completion_type(CompletionType::List)
                    .tab_stop(8)
                    .bracketed_paste(false) // multi-line paste
                    .build(),
            )
            .expect("failed to initialize line editor");
            repl.set_helper(Some(helper));

            // Bind CTRL + Z to insert EOF character on Windows
            #[cfg(windows)]
            {
                repl.bind_sequence(
                    KeyEvent::new('z', Modifiers::CTRL),
                    EventHandler::Simple(Cmd::Insert(1, EOF_CHAR.into())),
                );
            }

            Self { repl }
        }

        pub fn load_history(&mut self, path: &Path) -> OtherResult<()> {
            self.repl.load_history(path)?;
            Ok(())
        }

        pub fn save_history(&mut self, path: &Path) -> OtherResult<()> {
            if !path.exists()
                && let Some(parent) = path.parent()
            {
                crate::fs::create_dir_all(parent)?;
            }
            self.repl.save_history(path)?;
            Ok(())
        }

        pub fn add_history_entry(&mut self, entry: &str) -> OtherResult<()> {
            self.repl.add_history_entry(entry)?;
            Ok(())
        }

        pub fn readline(&mut self, prompt: &str) -> ReadlineResult {
            use std::io::IsTerminal;
            // Pipes have no termios. Write the prompt to stderr and read
            // stdin, matching PyOS_StdioReadline, so `python -i` under
            // spawn still emits `>>> `.
            if !io::stdin().is_terminal() {
                return stdio_readline(prompt);
            }
            use rustyline::error::ReadlineError;
            loop {
                break match self.repl.readline(prompt) {
                    Ok(line) => {
                        // Check for CTRL + Z on Windows
                        #[cfg(windows)]
                        {
                            use std::io::IsTerminal;

                            let trimmed = line.trim_end_matches(&['\r', '\n'][..]);
                            if trimmed == EOF_CHAR && io::stdin().is_terminal() {
                                return ReadlineResult::Eof;
                            }
                        }
                        ReadlineResult::Line(line)
                    }
                    Err(ReadlineError::Interrupted) => ReadlineResult::Interrupt,
                    Err(ReadlineError::Eof) => ReadlineResult::Eof,
                    Err(ReadlineError::Io(e)) => ReadlineResult::Io(e),
                    Err(ReadlineError::Signal(_)) => continue,
                    #[cfg(unix)]
                    Err(ReadlineError::Errno(num)) => ReadlineResult::OsError(num.to_string()),
                    Err(e) => ReadlineResult::Other(e.into()),
                };
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
use basic_readline as readline_inner;
#[cfg(not(target_arch = "wasm32"))]
use rustyline_readline as readline_inner;

pub use readline_inner::Helper;

pub struct Readline<H: Helper>(readline_inner::Readline<H>);

impl<H: Helper> Readline<H> {
    pub fn new(helper: H) -> Self {
        Self(readline_inner::Readline::new(helper))
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

/// A nested readline must fail rather than consume the outer call's input.
#[derive(Debug)]
pub struct ReentrantReadline;

#[derive(PartialEq, Eq)]
struct TerminalIdentity {
    streams: StreamIdentity,
}

#[cfg(unix)]
type StreamIdentity = (libc::dev_t, libc::ino_t, libc::dev_t);
#[cfg(windows)]
type StreamIdentity = [usize; 2];
#[cfg(not(any(unix, windows)))]
type StreamIdentity = ();

impl TerminalIdentity {
    fn current() -> io::Result<Self> {
        #[cfg(unix)]
        let streams = {
            let mut stat = core::mem::MaybeUninit::<libc::stat>::uninit();
            // Only an input change invalidates read-ahead. Unix rustyline uses
            // fd 1 directly, so an output-only change needs no new editor.
            if unsafe { libc::fstat(0, stat.as_mut_ptr()) } != 0 {
                return Err(io::Error::last_os_error());
            }
            let stat = unsafe { stat.assume_init() };
            (stat.st_dev, stat.st_ino, stat.st_rdev)
        };
        #[cfg(windows)]
        let streams = {
            use windows_sys::Win32::System::Console::{STD_INPUT_HANDLE, STD_OUTPUT_HANDLE};
            // Windows rustyline caches both handles, but has no read-ahead
            // buffer (ConsoleRawReader::unbuffer returns None).
            [
                crate::winapi::get_std_handle(STD_INPUT_HANDLE)?.unwrap_or_default() as usize,
                crate::winapi::get_std_handle(STD_OUTPUT_HANDLE)?.unwrap_or_default() as usize,
            ]
        };
        #[cfg(not(any(unix, windows)))]
        let streams = ();
        Ok(Self { streams })
    }
}

struct InputReader {
    identity: TerminalIdentity,
    reader: Readline<()>,
}

// stdin is process-wide, not thread-local. Retain read-ahead when the next
// input() runs on another thread, and serialize the terminal's raw-mode guards.
// A reentrant mutex plus a fallible borrow rejects same-thread recursion without
// deadlocking. The reader contains no Python references or interpreter helper.
static INPUT_READER: parking_lot::ReentrantMutex<core::cell::RefCell<Option<InputReader>>> =
    parking_lot::ReentrantMutex::new(core::cell::RefCell::new(None));

/// Read an interactive input line with the process's retained terminal reader.
/// The VM must detach while waiting for both the lock and terminal input.
pub fn input_readline(prompt: &str) -> Result<ReadlineResult, ReentrantReadline> {
    let guard = INPUT_READER.lock();
    let mut slot = guard.try_borrow_mut().map_err(|_| ReentrantReadline)?;
    let identity = match TerminalIdentity::current() {
        Ok(identity) => identity,
        Err(error) => return Ok(ReadlineResult::Io(error)),
    };
    if slot
        .as_ref()
        .is_none_or(|reader| reader.identity != identity)
    {
        *slot = Some(InputReader {
            identity,
            reader: Readline::new(()),
        });
    }
    Ok(slot.as_mut().unwrap().reader.readline(prompt))
}

/// Borrow the shared buffered stdin for one physical read. The caller retains
/// partial bytes across EINTR and checks signals after this lock is released.
pub fn stdio_readline_bytes(prompt: &str, line: &mut Vec<u8>) -> io::Result<usize> {
    use std::io::Write;
    let mut stderr = io::stderr().lock();
    stderr.write_all(prompt.as_bytes())?;
    stderr.flush()?;
    drop(stderr);
    read_line_bytes(&mut io::stdin().lock(), line)
}

fn read_line_bytes(reader: &mut impl io::BufRead, line: &mut Vec<u8>) -> io::Result<usize> {
    let mut read = 0;
    loop {
        // Unlike read_until, return Interrupted to the VM before retrying.
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return Ok(read);
        }
        let newline = available.iter().position(|&byte| byte == b'\n');
        let used = newline.map_or(available.len(), |index| index + 1);
        line.extend_from_slice(&available[..used]);
        reader.consume(used);
        read += used;
        if newline.is_some() {
            return Ok(read);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn nested_input_is_rejected_before_touching_the_terminal() {
        let guard = super::INPUT_READER.lock();
        let _active_reader = guard.borrow_mut();
        assert!(matches!(
            super::input_readline(""),
            Err(super::ReentrantReadline)
        ));
    }

    use super::read_line_bytes;
    use std::io::{self, BufRead, Read};

    struct ScriptedReader {
        chunks: alloc::collections::VecDeque<io::Result<Vec<u8>>>,
        consumed: usize,
    }

    impl ScriptedReader {
        fn new(chunks: impl IntoIterator<Item = io::Result<Vec<u8>>>) -> Self {
            Self {
                chunks: chunks.into_iter().collect(),
                consumed: 0,
            }
        }
    }

    impl Read for ScriptedReader {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            unreachable!("the transport uses BufRead directly")
        }
    }

    impl BufRead for ScriptedReader {
        fn fill_buf(&mut self) -> io::Result<&[u8]> {
            if self.chunks.front().is_some_and(Result::is_err) {
                return Err(self.chunks.pop_front().unwrap().unwrap_err());
            }
            Ok(self
                .chunks
                .front()
                .map_or(&[], |chunk| &chunk.as_ref().unwrap()[self.consumed..]))
        }

        fn consume(&mut self, amount: usize) {
            self.consumed += amount;
            if self.consumed == self.chunks.front().unwrap().as_ref().unwrap().len() {
                self.chunks.pop_front();
                self.consumed = 0;
            }
        }
    }

    #[test]
    fn physical_lines_preserve_buffered_remainder_and_count_only_new_bytes() {
        let mut reader = ScriptedReader::new([Ok(b"one\ntwo\ntail".to_vec())]);
        let mut line = b"prefix".to_vec();
        assert_eq!(read_line_bytes(&mut reader, &mut line).unwrap(), 4);
        assert_eq!(line, b"prefixone\n");
        line.clear();
        assert_eq!(read_line_bytes(&mut reader, &mut line).unwrap(), 4);
        assert_eq!(line, b"two\n");
        line.clear();
        assert_eq!(read_line_bytes(&mut reader, &mut line).unwrap(), 4);
        assert_eq!(line, b"tail");
        assert_eq!(read_line_bytes(&mut reader, &mut line).unwrap(), 0);
        assert_eq!(line, b"tail");
    }

    #[test]
    fn errors_return_immediately_and_leave_partial_bytes_for_the_caller() {
        for kind in [io::ErrorKind::Interrupted, io::ErrorKind::Other] {
            let mut reader = ScriptedReader::new([
                Ok(b"part".to_vec()),
                Err(io::Error::from(kind)),
                Ok(b"ial\nnext".to_vec()),
            ]);
            let mut line = Vec::new();
            assert_eq!(
                read_line_bytes(&mut reader, &mut line).unwrap_err().kind(),
                kind
            );
            assert_eq!(line, b"part");
            assert_eq!(read_line_bytes(&mut reader, &mut line).unwrap(), 4);
            assert_eq!(line, b"partial\n");
        }
    }

    #[test]
    fn eof_after_interruption_keeps_partial_bytes() {
        let mut reader = ScriptedReader::new([
            Ok(b"partial".to_vec()),
            Err(io::Error::from(io::ErrorKind::Interrupted)),
        ]);
        let mut line = Vec::new();
        assert_eq!(
            read_line_bytes(&mut reader, &mut line).unwrap_err().kind(),
            io::ErrorKind::Interrupted
        );
        assert_eq!(read_line_bytes(&mut reader, &mut line).unwrap(), 0);
        assert_eq!(line, b"partial");
    }
}

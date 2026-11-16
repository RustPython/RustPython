//! Embedding feature coverage, separate from the Python test_io behavior suite.
#![cfg(all(feature = "host_io", feature = "compiler", not(target_arch = "wasm32")))]

#[cfg(test)]
mod tests {
    use rustpython_vm::Interpreter;
    #[cfg(unix)]
    use std::{
        io::Write,
        os::fd::{AsRawFd, FromRawFd, OwnedFd},
    };

    #[cfg(unix)]
    fn pipe() -> (OwnedFd, std::fs::File) {
        let mut fds = [-1; 2];
        // SAFETY: pipe writes two descriptors to the supplied array.
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        // SAFETY: a successful pipe call transfers ownership of both descriptors.
        unsafe {
            (
                OwnedFd::from_raw_fd(fds[0]),
                std::fs::File::from_raw_fd(fds[1]),
            )
        }
    }

    #[test]
    #[cfg(unix)]
    fn fileio_available_without_other_host_modules() {
        let (reader, mut writer) = pipe();
        writer.write_all(b"host pipe").unwrap();
        drop(writer);
        let builder = Interpreter::builder(Default::default());
        #[cfg(native_host_io)]
        let builder = {
            let mut table = rustpython_wasi_host::Table::default();
            table
                .lend_stdio(
                    rustpython_wasi_host::Stdio::In,
                    std::fs::File::from(reader.try_clone().unwrap()),
                )
                .unwrap();
            builder.host_io(table)
        };
        builder.build().enter(|vm| {
            let scope = vm.new_scope_with_builtins();
            scope
                .globals
                .set_item(
                    "read_fd",
                    vm.ctx
                        .new_int(if cfg!(native_host_io) {
                            0
                        } else {
                            reader.as_raw_fd()
                        })
                        .into(),
                    vm,
                )
                .unwrap();
            scope
                .globals
                .set_item(
                    "full_host",
                    vm.ctx.new_bool(cfg!(feature = "host_env")).into(),
                    vm,
                )
                .unwrap();
            let source = r#"
import _io, sys
if not full_host:
    assert 'posix' not in sys.builtin_module_names
    assert '_ctypes' not in sys.builtin_module_names
with _io.FileIO(read_fd, 'rb', closefd=False) as raw:
    assert not raw.seekable()
    with _io.BufferedReader(raw) as buffered:
        data = bytearray(9)
        assert buffered.readinto(data) == 9
        assert data == b'host pipe'
        assert buffered.read() == b''
assert raw.closed
"#;
            if let Err(error) = vm.run_block_expr(scope, source) {
                let mut message = String::new();
                vm.write_exception(&mut message, &error).unwrap();
                panic!("{message}");
            }
        });
        // The embedding host still owns the descriptor after Python closes its wrapper.
        assert!(unsafe { libc::fcntl(reader.as_raw_fd(), libc::F_GETFD) } >= 0);
    }

    #[test]
    #[cfg(unix)]
    fn fileio_preserves_nonblocking_descriptor_result() {
        let (reader, _writer) = pipe();
        // SAFETY: reader owns a valid descriptor; F_SETFL takes integer flags.
        assert_eq!(
            unsafe { libc::fcntl(reader.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) },
            0
        );
        let builder = Interpreter::builder(Default::default());
        #[cfg(native_host_io)]
        let builder = {
            let mut table = rustpython_wasi_host::Table::default();
            table
                .lend_stdio(
                    rustpython_wasi_host::Stdio::In,
                    std::fs::File::from(reader.try_clone().unwrap()),
                )
                .unwrap();
            builder.host_io(table)
        };
        builder.build().enter(|vm| {
            let scope = vm.new_scope_with_builtins();
            scope
                .globals
                .set_item(
                    "read_fd",
                    vm.ctx
                        .new_int(if cfg!(native_host_io) {
                            0
                        } else {
                            reader.as_raw_fd()
                        })
                        .into(),
                    vm,
                )
                .unwrap();
            if let Err(error) = vm.run_block_expr(
                scope,
                r#"
import _io
with _io.FileIO(read_fd, 'rb', closefd=False) as raw:
    assert raw.read(1) is None
    assert raw.readinto(bytearray(1)) is None
"#,
            ) {
                let mut message = String::new();
                vm.write_exception(&mut message, &error).unwrap();
                panic!("{message}");
            }
        });
    }

    #[test]
    fn fileio_path_open_truncate_and_filename_error() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = rustpython_host_env::os::temp_dir().join(format!(
            "rustpython-host-io-{}-{unique}",
            rustpython_host_env::os::process_id()
        ));
        rustpython_host_env::fs::create_dir_all(&directory).unwrap();
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(directory.clone());
        let filename = directory.join("data.bin");
        let missing = directory.join("missing.bin");
        let builder = Interpreter::builder(Default::default());
        #[cfg(native_host_io)]
        let builder = builder.host_io(
            rustpython_wasi_host::Table::with_preopens([rustpython_wasi_host::Preopen {
                host: directory,
                guest: b"/hostio".to_vec(),
                write: true,
            }])
            .unwrap(),
        );
        builder.build().enter(|vm| {
            let scope = vm.new_scope_with_builtins();
            scope
                .globals
                .set_item(
                    "filename",
                    vm.ctx
                        .new_str(if cfg!(native_host_io) {
                            "/hostio/data.bin"
                        } else {
                            filename.to_str().unwrap()
                        })
                        .into(),
                    vm,
                )
                .unwrap();
            scope
                .globals
                .set_item(
                    "missing",
                    vm.ctx
                        .new_str(if cfg!(native_host_io) {
                            "/hostio/missing.bin"
                        } else {
                            missing.to_str().unwrap()
                        })
                        .into(),
                    vm,
                )
                .unwrap();
            if let Err(error) = vm.run_block_expr(
                scope,
                r#"
import _io
with _io.FileIO(filename, 'w+b') as file:
    assert file.write(b'abcdef') == 6
    assert file.tell() == 6
    assert file.truncate(3) == 3
    assert file.seek(0) == 0
    assert file.read() == b'abc'
try:
    _io.FileIO(missing, 'rb')
except FileNotFoundError as error:
    assert error.filename == missing
else:
    raise AssertionError('missing file unexpectedly opened')
"#,
            ) {
                let mut message = String::new();
                vm.write_exception(&mut message, &error).unwrap();
                panic!("{message}");
            }
        });
        assert_eq!(rustpython_host_env::fs::read(filename).unwrap(), b"abc");
    }
}

#[cfg(test)]
#[cfg(native_host_io)]
mod capabilities {
    use rustpython_vm::Interpreter;
    fn run(interp: &Interpreter, source: &str) {
        interp.enter(|vm| {
            let scope = vm.new_scope_with_builtins();
            if let Err(error) = vm.run_block_expr(scope, source) {
                let mut text = String::new();
                vm.write_exception(&mut text, &error).unwrap();
                panic!("{text}");
            }
        });
    }
    #[test]
    fn empty_table_denies_ambient_access() {
        run(
            &Interpreter::without_stdlib(Default::default()),
            r#"
import _io, sys
assert sys.stdin is None and sys.stdout is None and sys.stderr is None
assert not hasattr(sys, '_baserepl')
for fd in (0, 1, 2, 3, 100):
    for closefd in (True, False):
        try:
            _io.FileIO(fd, 'rb', closefd=closefd)
        except OSError as e:
            assert e.errno == 9
        else:
            raise AssertionError('ambient descriptor accepted')
try:
    _io.FileIO('/denied/input', 'rb')
except FileNotFoundError:
    pass
else:
    raise AssertionError('ambient path accepted')
try:
    _io.FileIO('ignored', 'rb', opener=lambda path, flags: 0)
except OSError as e:
    assert e.errno == 9
else:
    raise AssertionError('opener escaped table')
del sys.stderr
sys.__excepthook__(RuntimeError, RuntimeError('must not reach ambient stderr'), None)
"#,
        );
    }
    #[test]
    fn input_uses_guest_streams() {
        run(
            &Interpreter::without_stdlib(Default::default()),
            r#"
import sys
class Stream:
    encoding = 'utf-8'
    errors = 'strict'
    def __init__(self, fd): self.fd = fd
    def fileno(self): return self.fd
    def write(self, value): return len(value)
    def flush(self): pass
    def readline(self): return 'guest input\n'
sys.stdin = Stream(0)
sys.stdout = Stream(1)
sys.stderr = Stream(2)
assert input() == 'guest input'
"#,
        );
    }
    #[test]
    fn grants_reentry_and_interpreter_isolation() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = rustpython_host_env::os::temp_dir().join(format!("native-host-io-{unique}"));
        rustpython_host_env::fs::create_dir_all(&path).unwrap();
        rustpython_host_env::fs::write(path.join("input"), b"capability").unwrap();
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(path.clone());
        let table = rustpython_wasi_host::Table::with_preopens([rustpython_wasi_host::Preopen {
            host: path,
            guest: b"/allowed".to_vec(),
            write: true,
        }])
        .unwrap();
        let interp = Interpreter::builder(Default::default())
            .host_io(table)
            .build();
        run(
            &interp,
            r#"
import _io
f = _io.FileIO('/allowed/input', 'rb')
alias = _io.FileIO(f.fileno(), 'rb', closefd=False)
alias.close()
assert f.read(1) == b'c'
readonly = _io.FileIO(f.fileno(), 'wb', closefd=False)
try:
    readonly.write(b'x')
except PermissionError:
    pass
else:
    raise AssertionError('rights escalated')
readonly.close()
f.close()
obj = _io.FileIO.__new__(_io.FileIO)
keep = _io.FileIO('/allowed/input', 'rb')
seen = []
class Reentrant:
    def __index__(self):
        obj.__init__('/allowed/input', 'rb')
        seen.append(obj.fileno())
        return keep.fileno()
obj.__init__(Reentrant(), 'rb', closefd=False)
assert obj.read(1) == b'c'
try:
    _io.FileIO(seen[0], 'rb')
except OSError as e:
    assert e.errno == 9
else:
    raise AssertionError('old owned binding leaked')
obj.close()
assert keep.read(1) == b'a'
keep.close()
source = _io.FileIO('/allowed/input', 'rb')
same = _io.FileIO.__new__(_io.FileIO)
def same_opener(path, flags):
    same.__init__(source.fileno(), 'rb')
    return source.fileno()
same.__init__('ignored', 'rb', opener=same_opener)
assert same.read(1) == b'c'
same.close()
try: source.close()
except OSError as e: assert e.errno == 9
replacement = _io.FileIO('/allowed/input', 'rb')
displaced = _io.FileIO('/allowed/input', 'rb')
rebound = _io.FileIO.__new__(_io.FileIO)
def closed_opener(path, flags):
    rebound.__init__(displaced.fileno(), 'rb')
    displaced.close()
    return replacement.fileno()
rebound.__init__('ignored', 'rb', opener=closed_opener)
assert rebound.read(1) == b'c'
rebound.close()
try: replacement.close()
except OSError as e: assert e.errno == 9
"#,
        );
        let child = interp.create_subinterpreter();
        for separate in [&child, &Interpreter::without_stdlib(Default::default())] {
            run(
                separate,
                r#"
import _io
try: _io.FileIO('/allowed/input', 'rb')
except FileNotFoundError: pass
else: raise AssertionError('interpreter inherited grants')
"#,
            );
        }
    }
}

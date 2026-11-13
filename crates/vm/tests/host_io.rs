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
        Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let scope = vm.new_scope_with_builtins();
            scope
                .globals
                .set_item("read_fd", vm.ctx.new_int(reader.as_raw_fd()).into(), vm)
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
        Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let scope = vm.new_scope_with_builtins();
            scope
                .globals
                .set_item("read_fd", vm.ctx.new_int(reader.as_raw_fd()).into(), vm)
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
        Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let scope = vm.new_scope_with_builtins();
            scope
                .globals
                .set_item(
                    "filename",
                    vm.ctx.new_str(filename.to_str().unwrap()).into(),
                    vm,
                )
                .unwrap();
            scope
                .globals
                .set_item(
                    "missing",
                    vm.ctx.new_str(missing.to_str().unwrap()).into(),
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

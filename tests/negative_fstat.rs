#![allow(clippy::tests_outside_test_module)]

#[cfg(unix)]
#[test]
fn stat_rejects_negative_file_descriptors() {
    use rustpython::{InterpreterBuilder, InterpreterBuilderExt};

    InterpreterBuilder::new()
        .init_stdlib()
        .interpreter()
        .enter(|vm| {
            let source = r#"
import errno
import os

for stat in (os.stat, os.fstat):
    for fd in (-1, -5, -9, -100, -(2**31)):
        try:
            stat(fd)
        except OSError as exc:
            assert exc.errno == errno.EBADF, (stat, fd, exc)
        else:
            raise AssertionError((stat, fd, 'accepted a negative file descriptor'))
"#;
            if let Err(err) = vm.run_simple_string(source) {
                vm.print_exception(&err);
                panic!("negative descriptor regression failed");
            }
        });
}

#![cfg(any(target_os = "linux", target_os = "freebsd"))]

use std::{fs::File, os::fd::AsRawFd, process::Command};

#[test]
fn closerange_boundaries() {
    const CHILD_CASE: &str = "RUSTPYTHON_CLOSERANGE_CHILD_CASE";
    if let Ok(case) = std::env::var(CHILD_CASE) {
        let sentinel = File::open("/dev/null").unwrap();
        let fd = sentinel.as_raw_fd();
        let (low, high) = match case.as_str() {
            "zero" => (0, 0),
            "negative_zero" => (-1, 0),
            "equal" => (fd, fd),
            "reversed" => (fd + 1, fd),
            "negative" => (-2, -1),
            "single" => {
                let target = File::open("/dev/null").unwrap();
                let other = File::open("/dev/null").unwrap();
                let target_fd = target.as_raw_fd();
                rustpython_host_env::crt_fd::closerange(target_fd, target_fd + 1);
                let valid = sentinel.metadata().is_ok()
                    && other.metadata().is_ok()
                    && target.metadata().unwrap_err().raw_os_error() == Some(libc::EBADF);
                // Exit without dropping descriptors that closerange already closed.
                std::process::exit(if valid { 0 } else { 1 });
            }
            _ => panic!("unknown closerange case"),
        };
        rustpython_host_env::crt_fd::closerange(low, high);
        let valid = sentinel.metadata().is_ok();
        std::process::exit(if valid { 0 } else { 1 });
    }

    // A regression may close all descriptors, so never run these in the test runner.
    for case in [
        "zero",
        "negative_zero",
        "equal",
        "reversed",
        "negative",
        "single",
    ] {
        let status = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "closerange_boundaries", "--nocapture"])
            .env(CHILD_CASE, case)
            .status()
            .unwrap();
        assert!(status.success(), "closerange case {case}: {status}");
    }
}

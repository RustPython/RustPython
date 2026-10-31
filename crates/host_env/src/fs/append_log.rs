use parking_lot::Mutex;
use std::{
    fs::{File, OpenOptions},
    io::{self, Write},
    path::Path,
};

/// A replaceable append-only log destination, shared by its owner's consumers.
/// File replacement and complete record writes are serialized per destination.
#[derive(Debug, Default)]
pub struct AppendLog {
    file: Mutex<Option<File>>,
}

impl AppendLog {
    /// Close the old destination before opening its replacement. On failure the
    /// log stays disabled. The header is written only to empty regular files.
    pub fn set_path(&self, path: Option<&Path>, header: &[u8]) -> io::Result<()> {
        let mut slot = self.file.lock();
        *slot = None;
        let Some(path) = path else {
            return Ok(());
        };
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        let metadata = file.metadata()?;
        if metadata.is_file() && metadata.len() == 0 {
            file.write_all(header)?;
            file.flush()?;
        }
        *slot = Some(file);
        Ok(())
    }

    pub fn enabled(&self) -> bool {
        self.file.lock().is_some()
    }

    /// Append and flush a complete record, or do nothing if disabled.
    pub fn write(&self, record: &[u8]) -> io::Result<()> {
        if let Some(file) = self.file.lock().as_mut() {
            file.write_all(record)?;
            file.flush()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::{AtomicUsize, Ordering};
    use std::path::PathBuf;

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            loop {
                let path = std::env::temp_dir().join(format!(
                    "rustpython-append-log-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
                match std::fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(e) => panic!("create test directory: {e}"),
                }
            }
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn append_replace_and_disable() {
        let dir = TestDir::new();
        let first = dir.0.join("first");
        let second = dir.0.join("second");
        let log = AppendLog::default();
        assert!(!log.enabled());
        log.write(b"ignored\n").unwrap();
        log.set_path(Some(&first), b"header\n").unwrap();
        log.write(b"one\n").unwrap();
        log.set_path(Some(&first), b"header\n").unwrap();
        log.write(b"two\n").unwrap();
        log.set_path(Some(&second), b"header\n").unwrap();
        log.write(b"three\n").unwrap();
        log.set_path(None, b"header\n").unwrap();
        log.write(b"ignored\n").unwrap();
        assert!(!log.enabled());
        assert_eq!(std::fs::read(first).unwrap(), b"header\none\ntwo\n");
        assert_eq!(std::fs::read(second).unwrap(), b"header\nthree\n");
    }

    #[test]
    fn failed_replacement_disables_the_old_destination() {
        let dir = TestDir::new();
        let path = dir.0.join("log");
        let log = AppendLog::default();
        log.set_path(Some(&path), b"header\n").unwrap();
        assert!(
            log.set_path(Some(&dir.0.join("missing/log")), b"header\n")
                .is_err()
        );
        assert!(!log.enabled());
        log.write(b"ignored\n").unwrap();
        assert_eq!(std::fs::read(path).unwrap(), b"header\n");
    }

    #[test]
    fn concurrent_records_are_complete() {
        let dir = TestDir::new();
        let path = dir.0.join("log");
        let log = AppendLog::default();
        log.set_path(Some(&path), b"").unwrap();
        std::thread::scope(|scope| {
            for n in 0..4 {
                let log = &log;
                scope.spawn(move || {
                    for _ in 0..100 {
                        log.write(format!("{n}\n").as_bytes()).unwrap();
                    }
                });
            }
        });
        let contents = std::fs::read_to_string(path).unwrap();
        assert_eq!(contents.lines().count(), 400);
        for n in 0..4 {
            assert_eq!(
                contents
                    .lines()
                    .filter(|line| *line == n.to_string())
                    .count(),
                100
            );
        }
    }
}

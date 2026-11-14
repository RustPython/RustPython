//! Explicitly preopened, read-only directory capability.
//!
//! Only construction uses ambient authority. All subsequent paths are resolved
//! by cap-std relative to the owned directory, including symlink traversal.
//! This is not a guest ABI or a full filesystem implementation. No write,
//! ambient reopen, or parent-directory capability is exposed.
use cap_std::{ambient_authority, fs::Dir};
use std::{
    fs::File,
    io,
    path::{Component, Path},
};

pub struct ReadOnlyRoot {
    dir: Dir,
}

impl ReadOnlyRoot {
    /// Preopen precisely the host path selected by the embedding application.
    pub fn preopen(path: &Path) -> io::Result<Self> {
        Ok(Self {
            dir: Dir::open_ambient_dir(path, ambient_authority())?,
        })
    }

    /// The returned handle is opened read-only. Passing its ownership into a
    /// descriptor table does not grant write or truncate access.
    pub fn open_read(&self, path: &Path) -> io::Result<File> {
        validate_path(path)?;
        Ok(self.dir.open(path)?.into_std())
    }
}

fn validate_path(path: &Path) -> io::Result<()> {
    if path.components().any(|part| {
        matches!(
            part,
            Component::Prefix(_) | Component::RootDir | Component::ParentDir
        )
    }) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "path must remain inside the preopened root",
        ));
    }
    #[cfg(windows)]
    if path.to_str().is_none() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "non-UTF-8 Windows paths are not supported yet",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        path::PathBuf,
        sync::atomic::{AtomicUsize, Ordering},
    };

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            loop {
                let path = std::env::temp_dir().join(format!(
                    "rustpython-cap-fs-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
                match std::fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(e) => panic!("{e}"),
                }
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn read_nested_file_without_write_or_truncate_access() {
        let fixture = Fixture::new();
        std::fs::create_dir(fixture.0.join("nested")).unwrap();
        std::fs::write(fixture.0.join("nested/source.py"), b"answer = 42\n").unwrap();
        let root = ReadOnlyRoot::preopen(&fixture.0).unwrap();
        let mut file = root.open_read(Path::new("nested/source.py")).unwrap();
        let mut data = String::new();
        file.read_to_string(&mut data).unwrap();
        assert_eq!(data, "answer = 42\n");
        assert!(file.write_all(b"overwrite").is_err());
        assert!(file.set_len(0).is_err());
        assert!(root.open_read(Path::new("missing")).is_err());
    }

    #[test]
    fn ambient_and_parent_paths_are_denied() {
        let fixture = Fixture::new();
        let root = ReadOnlyRoot::preopen(&fixture.0).unwrap();
        for path in [
            Path::new(".."),
            fixture.0.as_path(),
            Path::new("child/../../outside"),
        ] {
            assert!(root.open_read(path).is_err());
        }
    }

    #[test]
    #[cfg(unix)]
    fn symlinks_remain_confined_and_unix_names_are_lossless() {
        use std::{
            ffi::OsStr,
            os::unix::{ffi::OsStrExt, fs::symlink},
        };
        let fixture = Fixture::new();
        let outside = Fixture::new();
        std::fs::write(outside.0.join("secret"), b"secret").unwrap();
        std::fs::write(fixture.0.join("inside"), b"inside").unwrap();
        symlink("inside", fixture.0.join("internal")).unwrap();
        symlink(outside.0.join("secret"), fixture.0.join("escape")).unwrap();
        symlink(&outside.0, fixture.0.join("escape-dir")).unwrap();
        let root = ReadOnlyRoot::preopen(&fixture.0).unwrap();
        let mut data = String::new();
        root.open_read(Path::new("internal"))
            .unwrap()
            .read_to_string(&mut data)
            .unwrap();
        assert_eq!(data, "inside");
        assert!(root.open_read(Path::new("escape")).is_err());
        assert!(root.open_read(Path::new("escape-dir/secret")).is_err());
        let raw = OsStr::from_bytes(b"raw-\xff");
        std::fs::write(fixture.0.join(raw), b"bytes").unwrap();
        let mut raw_contents = Vec::new();
        root.open_read(Path::new(raw))
            .unwrap()
            .read_to_end(&mut raw_contents)
            .unwrap();
        assert_eq!(raw_contents, b"bytes");
    }
}

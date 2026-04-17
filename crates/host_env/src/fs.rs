use std::{
    fs::{self, File, Metadata, ReadDir},
    io,
    path::Path,
};

pub fn open(path: impl AsRef<Path>) -> io::Result<File> {
    File::open(path)
}

pub fn read(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
    fs::read(path)
}

pub fn read_to_string(path: impl AsRef<Path>) -> io::Result<String> {
    fs::read_to_string(path)
}

pub fn read_dir(path: impl AsRef<Path>) -> io::Result<ReadDir> {
    fs::read_dir(path)
}

pub fn create_dir_all(path: impl AsRef<Path>) -> io::Result<()> {
    fs::create_dir_all(path)
}

pub fn remove_dir(path: impl AsRef<Path>) -> io::Result<()> {
    fs::remove_dir(path)
}

pub fn remove_file(path: impl AsRef<Path>) -> io::Result<()> {
    fs::remove_file(path)
}

pub fn metadata(path: impl AsRef<Path>) -> io::Result<Metadata> {
    fs::metadata(path)
}

pub fn symlink_metadata(path: impl AsRef<Path>) -> io::Result<Metadata> {
    fs::symlink_metadata(path)
}

pub fn open_write(path: impl AsRef<Path>) -> io::Result<File> {
    fs::OpenOptions::new().write(true).open(path)
}

pub fn canonicalize(path: impl AsRef<Path>) -> io::Result<std::path::PathBuf> {
    fs::canonicalize(path)
}

#[cfg(windows)]
pub fn open_write_with_custom_flags(path: impl AsRef<Path>, flags: u32) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;

    fs::OpenOptions::new()
        .write(true)
        .custom_flags(flags)
        .open(path)
}

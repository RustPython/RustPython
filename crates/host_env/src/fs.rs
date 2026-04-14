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

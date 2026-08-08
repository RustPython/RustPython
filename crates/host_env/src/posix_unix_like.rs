//! Common POSIX implementations across Unix-likes.

use std::{io, path::Path};

use rustix::{
    fd::AsFd,
    fs::{self, AtFlags},
};

pub use rustix::fs::RawMode;

use crate::{crt_fd, fileutils::StatStruct};

/// https://pubs.opengroup.org/onlinepubs/9799919799/functions/mkdir.html
pub fn make_dir(
    dir_fd: Option<crt_fd::Borrowed<'_>>,
    path: impl AsRef<Path>,
    mode: fs::RawMode,
) -> io::Result<()> {
    let dir_fd = dir_fd.as_ref().map_or(fs::CWD, AsFd::as_fd);
    fs::mkdirat(dir_fd, path.as_ref(), mode.into()).map_err(Into::into)
}

/// https://pubs.opengroup.org/onlinepubs/9799919799/functions/rename.html
pub fn rename(
    from: impl AsRef<Path>,
    from_fd: Option<crt_fd::Borrowed<'_>>,
    to: impl AsRef<Path>,
    to_fd: Option<crt_fd::Borrowed<'_>>,
) -> io::Result<()> {
    let from = from.as_ref();
    let from_fd = from_fd.as_ref().map_or(fs::CWD, AsFd::as_fd);
    let to = to.as_ref();
    let to_fd = to_fd.as_ref().map_or(fs::CWD, AsFd::as_fd);
    fs::renameat(from_fd, from, to_fd, to).map_err(Into::into)
}

/// https://docs.python.org/3/library/os.html#os.replace
///
/// Atomically replace `to` with `from`.
/// POSIX's rename already atomically replaces targets, so this function just forwards to [`rename`].
#[inline]
pub fn replace(
    from: impl AsRef<Path>,
    from_fd: Option<crt_fd::Borrowed<'_>>,
    to: impl AsRef<Path>,
    to_fd: Option<crt_fd::Borrowed<'_>>,
) -> io::Result<()> {
    rename(from, from_fd, to, to_fd)
}

pub fn stat_path(
    path: impl AsRef<Path>,
    dir_fd: Option<crt_fd::Borrowed<'_>>,
    follow_symlinks: bool,
) -> io::Result<Option<StatStruct>> {
    let flags = if follow_symlinks {
        AtFlags::empty()
    } else {
        AtFlags::SYMLINK_NOFOLLOW
    };
    let dir_fd = dir_fd.as_ref().map_or(fs::CWD, AsFd::as_fd);
    fs::statat(dir_fd, path.as_ref(), flags)
        .map(Option::Some)
        .map_err(Into::into)
}

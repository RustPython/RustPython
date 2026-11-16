// cspell:ignore ACCES BADF DEADLK ILSEQ MFILE NOSPC SPIPE TXTBSY FDFLAGS fdflags fdstat iovecs iovs nwritten badf
//! WASI preview1 subset used by the no-JS `host-io` guest.
//!
//! Import module: `wasi_snapshot_preview1`. Access is decided by preopens and
//! per-descriptor rights. Guest integers index this table; they are never
//! native descriptors. `path_open` paths must be valid UTF-8, as WASI
//! preview1 requires.

use cap_std::{
    ambient_authority,
    fs::{Dir, File, OpenOptions},
};
use std::{
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
};

pub const MODULE: &str = "wasi_snapshot_preview1";
pub const FUNCTIONS: &[&str] = &[
    "fd_read",
    "fd_write",
    "fd_seek",
    "fd_close",
    "fd_sync",
    "fd_filestat_set_size",
    "fd_fdstat_get",
    "fd_prestat_get",
    "fd_prestat_dir_name",
    "path_open",
];

pub const SUCCESS: u16 = 0;
pub const TOOBIG: u16 = 1;
pub const ACCES: u16 = 2;
pub const ADDRINUSE: u16 = 3;
pub const ADDRNOTAVAIL: u16 = 4;
pub const AGAIN: u16 = 6;
pub const BADF: u16 = 8;
pub const BUSY: u16 = 10;
pub const CONNABORTED: u16 = 13;
pub const CONNREFUSED: u16 = 14;
pub const CONNRESET: u16 = 15;
pub const DEADLK: u16 = 16;
pub const DQUOT: u16 = 19;
pub const EXIST: u16 = 20;
pub const FAULT: u16 = 21;
pub const FBIG: u16 = 22;
pub const ILSEQ: u16 = 25;
pub const INTR: u16 = 27;
pub const INVAL: u16 = 28;
pub const IO: u16 = 29;
pub const ISDIR: u16 = 31;
pub const MFILE: u16 = 33;
pub const MLINK: u16 = 34;
pub const NAMETOOLONG: u16 = 37;
pub const NOENT: u16 = 44;
pub const NOMEM: u16 = 48;
pub const NOSPC: u16 = 51;
pub const NOTSUP: u16 = 58;
pub const NOTDIR: u16 = 54;
pub const NOTEMPTY: u16 = 55;
pub const NOTCONN: u16 = 53;
pub const OVERFLOW: u16 = 61;
pub const PERM: u16 = 63;
pub const PIPE: u16 = 64;
pub const ROFS: u16 = 69;
pub const SPIPE: u16 = 70;
pub const STALE: u16 = 72;
pub const TIMEDOUT: u16 = 73;
pub const TXTBSY: u16 = 74;
pub const XDEV: u16 = 75;
pub const NOTCAPABLE: u16 = 76;

const FILETYPE_UNKNOWN: u8 = 0;
const FILETYPE_DIRECTORY: u8 = 3;
const FILETYPE_REGULAR_FILE: u8 = 4;

#[cfg(test)]
const LOOKUP_SYMLINK_FOLLOW: i32 = 1;

const OFLAGS_CREAT: u16 = 1;
const OFLAGS_DIRECTORY: u16 = 2;
const OFLAGS_EXCL: u16 = 4;
const OFLAGS_TRUNC: u16 = 8;

const FDFLAGS_APPEND: u16 = 1;

const RIGHTS_FD_DATASYNC: u64 = 1 << 0;
const RIGHTS_FD_READ: u64 = 1 << 1;
const RIGHTS_FD_SEEK: u64 = 1 << 2;
const RIGHTS_FD_SYNC: u64 = 1 << 4;
const RIGHTS_FD_TELL: u64 = 1 << 5;
const RIGHTS_FD_WRITE: u64 = 1 << 6;
const RIGHTS_FD_ALLOCATE: u64 = 1 << 8;
const RIGHTS_PATH_CREATE_DIRECTORY: u64 = 1 << 9;
const RIGHTS_PATH_CREATE_FILE: u64 = 1 << 10;
const RIGHTS_PATH_LINK_SOURCE: u64 = 1 << 11;
const RIGHTS_PATH_LINK_TARGET: u64 = 1 << 12;
const RIGHTS_PATH_OPEN: u64 = 1 << 13;
const RIGHTS_PATH_RENAME_SOURCE: u64 = 1 << 16;
const RIGHTS_PATH_RENAME_TARGET: u64 = 1 << 17;
const RIGHTS_PATH_FILESTAT_SET_SIZE: u64 = 1 << 19;
const RIGHTS_PATH_FILESTAT_SET_TIMES: u64 = 1 << 20;
#[cfg(test)]
const RIGHTS_FD_FILESTAT_GET: u64 = 1 << 21;
const RIGHTS_FD_FILESTAT_SET_SIZE: u64 = 1 << 22;
const RIGHTS_FD_FILESTAT_SET_TIMES: u64 = 1 << 23;
const RIGHTS_PATH_SYMLINK: u64 = 1 << 24;
const RIGHTS_PATH_REMOVE_DIRECTORY: u64 = 1 << 25;
const RIGHTS_PATH_UNLINK_FILE: u64 = 1 << 26;

const RIGHTS_ALL: u64 = (1 << 30) - 1;

const RIGHTS_WRITE_RELATED: u64 = RIGHTS_FD_WRITE
    | RIGHTS_FD_ALLOCATE
    | RIGHTS_FD_FILESTAT_SET_SIZE
    | RIGHTS_FD_FILESTAT_SET_TIMES
    | RIGHTS_PATH_CREATE_DIRECTORY
    | RIGHTS_PATH_CREATE_FILE
    | RIGHTS_PATH_LINK_SOURCE
    | RIGHTS_PATH_LINK_TARGET
    | RIGHTS_PATH_RENAME_SOURCE
    | RIGHTS_PATH_RENAME_TARGET
    | RIGHTS_PATH_FILESTAT_SET_SIZE
    | RIGHTS_PATH_FILESTAT_SET_TIMES
    | RIGHTS_PATH_SYMLINK
    | RIGHTS_PATH_REMOVE_DIRECTORY
    | RIGHTS_PATH_UNLINK_FILE;

/// Rights that must be granted when requested; extra bits are masked off.
const RIGHTS_CRITICAL: u64 = RIGHTS_FD_READ
    | RIGHTS_FD_WRITE
    | RIGHTS_FD_SEEK
    | RIGHTS_FD_TELL
    | RIGHTS_FD_SYNC
    | RIGHTS_FD_DATASYNC
    | RIGHTS_FD_ALLOCATE
    | RIGHTS_FD_FILESTAT_SET_SIZE
    | RIGHTS_PATH_OPEN
    | RIGHTS_PATH_CREATE_FILE
    | RIGHTS_PATH_CREATE_DIRECTORY
    | RIGHTS_PATH_FILESTAT_SET_SIZE;

const MAX_TRANSFER: usize = 64 * 1024;
const MAX_PATH: u32 = 65536;

const WHENCE_SET: i32 = 0;
const WHENCE_CUR: i32 = 1;
const WHENCE_END: i32 = 2;

pub struct Preopen {
    pub host: PathBuf,
    pub guest: Vec<u8>,
    pub write: bool,
}

enum Resource {
    File(File),
    Dir {
        dir: Dir,
        preopen_name: Option<Vec<u8>>,
    },
}

struct Entry {
    resource: Resource,
    rights_base: u64,
    rights_inheriting: u64,
    fdflags: u16,
    filetype: u8,
}

impl Entry {
    fn has(&self, right: u64) -> bool {
        self.rights_base & right != 0
    }

    fn require(&self, right: u64) -> Result<(), u16> {
        if self.has(right) {
            Ok(())
        } else {
            Err(NOTCAPABLE)
        }
    }

    fn is_dir(&self) -> bool {
        self.filetype == FILETYPE_DIRECTORY
    }

    fn is_readonly_dir(&self) -> bool {
        self.is_dir() && self.rights_inheriting & RIGHTS_FD_WRITE == 0
    }
}

pub struct Table {
    entries: Vec<Option<Entry>>,
}

impl Default for Table {
    fn default() -> Self {
        Self {
            entries: vec![None, None, None],
        }
    }
}

impl Table {
    pub fn with_preopens(preopens: impl IntoIterator<Item = Preopen>) -> io::Result<Self> {
        let mut table = Self::default();
        for preopen in preopens {
            let dir = Dir::open_ambient_dir(&preopen.host, ambient_authority())?;
            let rights = if preopen.write {
                RIGHTS_ALL
            } else {
                RIGHTS_ALL & !RIGHTS_WRITE_RELATED
            };
            table
                .push(Entry {
                    resource: Resource::Dir {
                        dir,
                        preopen_name: Some(preopen.guest),
                    },
                    rights_base: rights,
                    rights_inheriting: rights,
                    fdflags: 0,
                    filetype: FILETYPE_DIRECTORY,
                })
                .map_err(|_| io::Error::other("descriptor table full"))?;
        }
        Ok(table)
    }

    fn push(&mut self, entry: Entry) -> Result<i32, u16> {
        let index = self
            .entries
            .iter()
            .enumerate()
            .skip(3)
            .find_map(|(index, slot)| slot.is_none().then_some(index))
            .unwrap_or(self.entries.len().max(3));
        let fd = i32::try_from(index).map_err(|_| MFILE)?;
        if index >= self.entries.len() {
            self.entries.resize_with(index + 1, || None);
        }
        self.entries[index] = Some(entry);
        Ok(fd)
    }

    fn get(&self, fd: i32) -> Result<&Entry, u16> {
        usize::try_from(fd)
            .ok()
            .and_then(|fd| self.entries.get(fd))
            .and_then(Option::as_ref)
            .ok_or(BADF)
    }

    fn get_mut(&mut self, fd: i32) -> Result<&mut Entry, u16> {
        usize::try_from(fd)
            .ok()
            .and_then(|fd| self.entries.get_mut(fd))
            .and_then(Option::as_mut)
            .ok_or(BADF)
    }

    pub fn close(&mut self, fd: i32) -> Result<(), u16> {
        let Some(entry) = usize::try_from(fd)
            .ok()
            .and_then(|fd| self.entries.get_mut(fd))
            .and_then(Option::take)
        else {
            return Err(BADF);
        };
        drop(entry);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn path_open(
        &mut self,
        dirfd: i32,
        _dirflags: i32,
        path: &[u8],
        oflags: u16,
        fs_rights_base: u64,
        fs_rights_inheriting: u64,
        fdflags: u16,
    ) -> Result<i32, u16> {
        if path.len() > MAX_PATH as usize {
            return Err(NAMETOOLONG);
        }
        let path = guest_path(path)?;
        let (dir, dir_rights_base, dir_rights_inheriting, readonly) = {
            let dir_entry = self.get(dirfd)?;
            match &dir_entry.resource {
                Resource::Dir { dir, .. } => {
                    dir_entry.require(RIGHTS_PATH_OPEN)?;
                    (
                        dir.try_clone().map_err(wasi_from_io)?,
                        dir_entry.rights_base,
                        dir_entry.rights_inheriting,
                        dir_entry.is_readonly_dir(),
                    )
                }
                Resource::File(_) => return Err(NOTDIR),
            }
        };

        if readonly
            && (oflags & (OFLAGS_CREAT | OFLAGS_TRUNC | OFLAGS_EXCL) != 0
                || fdflags & FDFLAGS_APPEND != 0)
        {
            return Err(NOTCAPABLE);
        }

        let mut needed_dir = RIGHTS_PATH_OPEN;
        if oflags & OFLAGS_CREAT != 0 {
            needed_dir |= RIGHTS_PATH_CREATE_FILE;
        }
        if oflags & OFLAGS_TRUNC != 0 {
            needed_dir |= RIGHTS_PATH_FILESTAT_SET_SIZE;
        }
        if dir_rights_base & needed_dir != needed_dir {
            return Err(NOTCAPABLE);
        }
        if fs_rights_base & RIGHTS_CRITICAL & !dir_rights_inheriting != 0 {
            return Err(NOTCAPABLE);
        }

        let granted_base = fs_rights_base & dir_rights_inheriting;
        let granted_inheriting = fs_rights_inheriting & dir_rights_inheriting;

        if oflags & OFLAGS_DIRECTORY != 0 {
            let opened = dir.open_dir(path).map_err(wasi_from_io)?;
            return self.push(Entry {
                resource: Resource::Dir {
                    dir: opened,
                    preopen_name: None,
                },
                rights_base: granted_base,
                rights_inheriting: granted_inheriting,
                fdflags: fdflags & FDFLAGS_APPEND,
                filetype: FILETYPE_DIRECTORY,
            });
        }

        let want_read = granted_base & RIGHTS_FD_READ != 0;
        let want_write = granted_base & RIGHTS_FD_WRITE != 0;
        if !want_read && !want_write {
            return Err(INVAL);
        }

        let mut opts = OpenOptions::new();
        opts.read(want_read);
        if want_write {
            opts.write(true);
            if fdflags & FDFLAGS_APPEND != 0 {
                opts.append(true);
            }
            if oflags & OFLAGS_TRUNC != 0 {
                opts.truncate(true);
            }
            if oflags & OFLAGS_CREAT != 0 && oflags & OFLAGS_EXCL != 0 {
                opts.create_new(true);
            } else if oflags & OFLAGS_CREAT != 0 {
                opts.create(true);
            }
        } else {
            // Read-only at the OS as well as in the rights mask.
            opts.write(false);
        }

        let file = dir.open_with(path, &opts).map_err(wasi_from_io)?;
        let metadata = file.metadata().map_err(wasi_from_io)?;
        if metadata.is_dir() {
            let dir = Dir::from_std_file(file.into_std());
            return self.push(Entry {
                resource: Resource::Dir {
                    dir,
                    preopen_name: None,
                },
                rights_base: granted_base,
                rights_inheriting: granted_inheriting,
                fdflags: fdflags & FDFLAGS_APPEND,
                filetype: FILETYPE_DIRECTORY,
            });
        }
        let filetype = if metadata.is_file() {
            FILETYPE_REGULAR_FILE
        } else {
            FILETYPE_UNKNOWN
        };
        self.push(Entry {
            resource: Resource::File(file),
            rights_base: granted_base,
            rights_inheriting: granted_inheriting,
            fdflags: fdflags & FDFLAGS_APPEND,
            filetype,
        })
    }

    pub fn fd_read(&mut self, fd: i32, bufs: &mut [&mut [u8]]) -> Result<usize, u16> {
        let entry = self.get_mut(fd)?;
        entry.require(RIGHTS_FD_READ)?;
        if entry.is_dir() {
            return Err(ISDIR);
        }
        let Resource::File(file) = &mut entry.resource else {
            return Err(ISDIR);
        };
        transfer_read(file, bufs)
    }

    pub fn fd_write(&mut self, fd: i32, bufs: &[&[u8]]) -> Result<usize, u16> {
        let entry = self.get_mut(fd)?;
        entry.require(RIGHTS_FD_WRITE)?;
        if entry.is_dir() {
            return Err(ISDIR);
        }
        let Resource::File(file) = &mut entry.resource else {
            return Err(ISDIR);
        };
        transfer_write(file, bufs)
    }

    pub fn fd_seek(&mut self, fd: i32, offset: i64, whence: i32) -> Result<u64, u16> {
        if !matches!(whence, WHENCE_SET | WHENCE_CUR | WHENCE_END) {
            return Err(INVAL);
        }
        let entry = self.get_mut(fd)?;
        if whence == WHENCE_CUR && offset == 0 {
            if !entry.has(RIGHTS_FD_TELL) && !entry.has(RIGHTS_FD_SEEK) {
                return Err(NOTCAPABLE);
            }
        } else {
            entry.require(RIGHTS_FD_SEEK)?;
        }
        if entry.is_dir() {
            return Err(ISDIR);
        }
        let Resource::File(file) = &mut entry.resource else {
            return Err(ISDIR);
        };
        let pos = match whence {
            WHENCE_SET => {
                let offset = u64::try_from(offset).map_err(|_| INVAL)?;
                SeekFrom::Start(offset)
            }
            WHENCE_CUR => SeekFrom::Current(offset),
            WHENCE_END => SeekFrom::End(offset),
            _ => return Err(INVAL),
        };
        file.seek(pos).map_err(wasi_from_io)
    }

    pub fn fd_sync(&mut self, fd: i32) -> Result<(), u16> {
        let entry = self.get_mut(fd)?;
        entry.require(RIGHTS_FD_SYNC)?;
        match &mut entry.resource {
            Resource::File(file) => file.sync_all().map_err(wasi_from_io),
            Resource::Dir { dir, .. } => dir
                .try_clone()
                .and_then(|clone| clone.into_std_file().sync_all())
                .map_err(wasi_from_io),
        }
    }

    pub fn fd_filestat_set_size(&mut self, fd: i32, size: i64) -> Result<(), u16> {
        let size = u64::try_from(size).map_err(|_| INVAL)?;
        let entry = self.get_mut(fd)?;
        entry.require(RIGHTS_FD_FILESTAT_SET_SIZE)?;
        if entry.is_dir() {
            return Err(ISDIR);
        }
        let Resource::File(file) = &mut entry.resource else {
            return Err(ISDIR);
        };
        file.set_len(size).map_err(wasi_from_io)
    }

    pub fn fdstat(&self, fd: i32) -> Result<(u8, u16, u64, u64), u16> {
        let entry = self.get(fd)?;
        Ok((
            entry.filetype,
            entry.fdflags,
            entry.rights_base,
            entry.rights_inheriting,
        ))
    }

    pub fn prestat(&self, fd: i32) -> Result<&[u8], u16> {
        let entry = self.get(fd)?;
        match &entry.resource {
            Resource::Dir {
                preopen_name: Some(name),
                ..
            } => Ok(name.as_slice()),
            _ => Err(BADF),
        }
    }
}

fn transfer_read(file: &mut File, bufs: &mut [&mut [u8]]) -> Result<usize, u16> {
    let mut total = 0usize;
    let mut remaining = MAX_TRANSFER;
    for buf in bufs {
        if remaining == 0 {
            break;
        }
        let n = buf.len().min(remaining);
        if n == 0 {
            continue;
        }
        match file.read(&mut buf[..n]) {
            Ok(0) => break,
            Ok(k) => {
                total += k;
                remaining -= k;
                if k < n {
                    break;
                }
            }
            Err(err) if total > 0 => {
                if err.kind() == io::ErrorKind::Interrupted {
                    break;
                }
                return Err(wasi_from_io(err));
            }
            Err(err) => return Err(wasi_from_io(err)),
        }
    }
    Ok(total)
}

fn transfer_write(file: &mut File, bufs: &[&[u8]]) -> Result<usize, u16> {
    let mut total = 0usize;
    let mut remaining = MAX_TRANSFER;
    for buf in bufs {
        if remaining == 0 {
            break;
        }
        let n = buf.len().min(remaining);
        if n == 0 {
            continue;
        }
        match file.write(&buf[..n]) {
            Ok(0) => break,
            Ok(k) => {
                total += k;
                remaining -= k;
                if k < n {
                    break;
                }
            }
            Err(err) if total > 0 => {
                if err.kind() == io::ErrorKind::Interrupted {
                    break;
                }
                return Err(wasi_from_io(err));
            }
            Err(err) => return Err(wasi_from_io(err)),
        }
    }
    Ok(total)
}

fn guest_path(bytes: &[u8]) -> Result<&Path, u16> {
    if bytes.is_empty() {
        return Ok(Path::new("."));
    }
    if bytes.contains(&0) {
        return Err(INVAL);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| INVAL)?;
    let path = Path::new(text);
    if path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::Prefix(_) | Component::RootDir))
    {
        return Err(NOTCAPABLE);
    }
    Ok(path)
}

fn wasi_from_io(err: io::Error) -> u16 {
    match err.kind() {
        io::ErrorKind::NotFound => NOENT,
        io::ErrorKind::PermissionDenied => match err.raw_os_error() {
            Some(13) => ACCES,
            Some(1) => PERM,
            _ => NOTCAPABLE,
        },
        io::ErrorKind::AlreadyExists => EXIST,
        io::ErrorKind::WouldBlock => AGAIN,
        io::ErrorKind::Interrupted => INTR,
        io::ErrorKind::InvalidInput => INVAL,
        io::ErrorKind::InvalidData => ILSEQ,
        io::ErrorKind::UnexpectedEof => IO,
        io::ErrorKind::OutOfMemory => NOMEM,
        io::ErrorKind::Unsupported => NOTSUP,
        io::ErrorKind::TimedOut => TIMEDOUT,
        io::ErrorKind::BrokenPipe => PIPE,
        io::ErrorKind::AddrInUse => ADDRINUSE,
        io::ErrorKind::AddrNotAvailable => ADDRNOTAVAIL,
        io::ErrorKind::ConnectionAborted => CONNABORTED,
        io::ErrorKind::ConnectionRefused => CONNREFUSED,
        io::ErrorKind::ConnectionReset => CONNRESET,
        io::ErrorKind::NotConnected => NOTCONN,
        io::ErrorKind::IsADirectory => ISDIR,
        io::ErrorKind::NotADirectory => NOTDIR,
        io::ErrorKind::DirectoryNotEmpty => NOTEMPTY,
        io::ErrorKind::ReadOnlyFilesystem => ROFS,
        io::ErrorKind::StorageFull => NOSPC,
        io::ErrorKind::NotSeekable => SPIPE,
        io::ErrorKind::FileTooLarge => FBIG,
        io::ErrorKind::TooManyLinks => MLINK,
        io::ErrorKind::QuotaExceeded => DQUOT,
        io::ErrorKind::Deadlock => DEADLK,
        io::ErrorKind::ArgumentListTooLong => TOOBIG,
        io::ErrorKind::CrossesDevices => XDEV,
        io::ErrorKind::ExecutableFileBusy => TXTBSY,
        io::ErrorKind::StaleNetworkFileHandle => STALE,
        io::ErrorKind::ResourceBusy => BUSY,
        _ => IO,
    }
}

pub mod guest_memory;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guest_memory::check_range;
    use crate::guest_memory::*;
    use std::{
        path::PathBuf,
        sync::atomic::{AtomicUsize, Ordering},
    };

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            loop {
                let path = std::env::temp_dir().join(format!(
                    "rustpython-wasi-fs-{}-{}",
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
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn rights_read() -> u64 {
        RIGHTS_FD_READ
            | RIGHTS_FD_SEEK
            | RIGHTS_FD_TELL
            | RIGHTS_FD_SYNC
            | RIGHTS_FD_FILESTAT_GET
            | RIGHTS_FD_FILESTAT_SET_TIMES
    }

    fn rights_write() -> u64 {
        rights_read() | RIGHTS_FD_WRITE | RIGHTS_FD_ALLOCATE | RIGHTS_FD_FILESTAT_SET_SIZE
    }

    #[test]
    fn read_only_preopen_denies_write_flags_without_touching_the_file() {
        let fixture = Fixture::new();
        std::fs::write(fixture.0.join("inside"), b"unchanged").unwrap();
        let mut table = Table::with_preopens([Preopen {
            host: fixture.0.clone(),
            guest: b"/".to_vec(),
            write: false,
        }])
        .unwrap();
        for oflags in [
            OFLAGS_CREAT,
            OFLAGS_TRUNC,
            OFLAGS_EXCL,
            OFLAGS_CREAT | OFLAGS_TRUNC,
        ] {
            assert_eq!(
                table
                    .path_open(
                        3,
                        LOOKUP_SYMLINK_FOLLOW,
                        b"inside",
                        oflags,
                        rights_write(),
                        0,
                        0
                    )
                    .unwrap_err(),
                NOTCAPABLE
            );
        }
        assert_eq!(
            table
                .path_open(
                    3,
                    LOOKUP_SYMLINK_FOLLOW,
                    b"inside",
                    0,
                    rights_write(),
                    0,
                    FDFLAGS_APPEND,
                )
                .unwrap_err(),
            NOTCAPABLE
        );
        assert_eq!(
            table
                .path_open(3, LOOKUP_SYMLINK_FOLLOW, b"inside", 0, rights_write(), 0, 0)
                .unwrap_err(),
            NOTCAPABLE
        );
        let fd = table
            .path_open(3, LOOKUP_SYMLINK_FOLLOW, b"inside", 0, rights_read(), 0, 0)
            .unwrap();
        assert_eq!(table.fd_write(fd, &[b"x"]).unwrap_err(), NOTCAPABLE);
        assert_eq!(table.fd_filestat_set_size(fd, 0).unwrap_err(), NOTCAPABLE);
        assert_eq!(
            std::fs::read(fixture.0.join("inside")).unwrap(),
            b"unchanged"
        );
    }

    #[test]
    fn escape_and_absolute_paths_are_notcapable() {
        let fixture = Fixture::new();
        std::fs::create_dir(fixture.0.join("child")).unwrap();
        let mut table = Table::with_preopens([Preopen {
            host: fixture.0.clone(),
            guest: b"/".to_vec(),
            write: true,
        }])
        .unwrap();
        for path in [
            b"..".as_slice(),
            b"../outside",
            b"/etc/passwd",
            b"child/../../outside",
        ] {
            assert_eq!(
                table
                    .path_open(3, LOOKUP_SYMLINK_FOLLOW, path, 0, rights_read(), 0, 0)
                    .unwrap_err(),
                NOTCAPABLE,
                "{}",
                String::from_utf8_lossy(path)
            );
        }
    }

    #[test]
    fn close_once_then_badf() {
        let fixture = Fixture::new();
        std::fs::write(fixture.0.join("f"), b"ab").unwrap();
        let mut table = Table::with_preopens([Preopen {
            host: fixture.0.clone(),
            guest: b"/".to_vec(),
            write: true,
        }])
        .unwrap();
        let fd = table
            .path_open(3, LOOKUP_SYMLINK_FOLLOW, b"f", 0, rights_read(), 0, 0)
            .unwrap();
        table.close(fd).unwrap();
        assert_eq!(table.close(fd), Err(BADF));
        assert_eq!(table.fd_read(fd, &mut [&mut [0; 1]]), Err(BADF));
        assert!(matches!(table.get(0), Err(BADF)));
        assert!(matches!(table.get(1), Err(BADF)));
        assert!(matches!(table.get(2), Err(BADF)));
        assert!(matches!(table.get(-1), Err(BADF)));
    }

    #[test]
    fn path_open_requires_utf8() {
        let fixture = Fixture::new();
        std::fs::write(fixture.0.join("café"), b"coffee").unwrap();
        let mut table = Table::with_preopens([Preopen {
            host: fixture.0.clone(),
            guest: b"/".to_vec(),
            write: true,
        }])
        .unwrap();
        assert_eq!(
            table
                .path_open(
                    3,
                    LOOKUP_SYMLINK_FOLLOW,
                    b"raw-\xff",
                    0,
                    rights_read(),
                    0,
                    0,
                )
                .unwrap_err(),
            INVAL
        );
        let fd = table
            .path_open(
                3,
                LOOKUP_SYMLINK_FOLLOW,
                "café".as_bytes(),
                0,
                rights_read(),
                0,
                0,
            )
            .unwrap();
        let mut buf = [0u8; 16];
        let n = table.fd_read(fd, &mut [&mut buf]).unwrap();
        assert_eq!(&buf[..n], b"coffee");
    }

    #[test]
    #[cfg(unix)]
    fn confined_symlinks() {
        let fixture = Fixture::new();
        let outside = Fixture::new();
        std::fs::write(outside.0.join("secret"), b"secret").unwrap();
        std::fs::write(fixture.0.join("inside"), b"inside").unwrap();
        std::os::unix::fs::symlink("inside", fixture.0.join("internal")).unwrap();
        std::os::unix::fs::symlink(outside.0.join("secret"), fixture.0.join("escape")).unwrap();
        std::os::unix::fs::symlink(&outside.0, fixture.0.join("escape-dir")).unwrap();
        let mut table = Table::with_preopens([Preopen {
            host: fixture.0.clone(),
            guest: b"/".to_vec(),
            write: true,
        }])
        .unwrap();
        let fd = table
            .path_open(
                3,
                LOOKUP_SYMLINK_FOLLOW,
                b"internal",
                0,
                rights_read(),
                0,
                0,
            )
            .unwrap();
        let mut buf = [0u8; 8];
        let n = table.fd_read(fd, &mut [&mut buf]).unwrap();
        assert_eq!(&buf[..n], b"inside");
        assert_eq!(
            table
                .path_open(3, LOOKUP_SYMLINK_FOLLOW, b"escape", 0, rights_read(), 0, 0)
                .unwrap_err(),
            NOTCAPABLE
        );
        assert_eq!(
            table
                .path_open(
                    3,
                    LOOKUP_SYMLINK_FOLLOW,
                    b"escape-dir/secret",
                    0,
                    rights_read(),
                    0,
                    0,
                )
                .unwrap_err(),
            NOTCAPABLE
        );
    }

    #[test]
    fn guest_range_check() {
        assert!(check_range(65536, 65536, 0).is_ok());
        assert_eq!(check_range(65536, 65536, 1).unwrap_err(), FAULT);
        assert_eq!(check_range(65536, u32::MAX, u32::MAX).unwrap_err(), FAULT);

        struct MissingMemory;
        impl GuestMemory for MissingMemory {
            fn validate(&self) -> Result<(), u16> {
                Err(IO)
            }
            fn size(&self) -> u64 {
                unreachable!()
            }
            fn read(&self, _: u32, _: &mut [u8]) -> Result<(), u16> {
                unreachable!()
            }
            fn write(&self, _: u32, _: &[u8]) -> Result<(), u16> {
                unreachable!()
            }
        }
        let mut table = Table::default();
        let memory = MissingMemory;
        assert_eq!(fd_read(&mut table, &memory, -1, -1, -1, -1), i32::from(IO));
        assert_eq!(
            fd_prestat_dir_name(&mut table, &memory, -1, -1, -1),
            i32::from(INVAL)
        );
        assert_eq!(
            fd_prestat_dir_name(&mut table, &memory, -1, -1, 0),
            i32::from(IO)
        );
        assert_eq!(
            path_open(&mut table, &memory, -1, 0, -1, -1, 0, 0, 0, 0, -1),
            i32::from(INVAL)
        );
        assert_eq!(
            path_open(
                &mut table,
                &memory,
                -1,
                0,
                -1,
                MAX_PATH as i32 + 1,
                0,
                0,
                0,
                0,
                -1
            ),
            i32::from(NAMETOOLONG)
        );
        assert_eq!(
            path_open(&mut table, &memory, -1, 0, -1, 0, 0, 0, 0, 0, -1),
            i32::from(IO)
        );
    }

    #[test]
    fn permission_denied_without_os_code_is_notcapable() {
        assert_eq!(
            wasi_from_io(io::Error::new(io::ErrorKind::PermissionDenied, "escape")),
            NOTCAPABLE
        );
        assert_eq!(
            wasi_from_io(io::Error::new(io::ErrorKind::NotFound, "missing")),
            NOENT
        );
    }

    #[test]
    #[cfg(unix)]
    fn invalid_guest_range_returns_fault_without_consuming_pipe_data() {
        use std::io::Write;
        use std::os::fd::{FromRawFd, IntoRawFd};
        use std::os::unix::net::UnixStream;

        let (reader, mut writer) = UnixStream::pair().unwrap();
        reader.set_nonblocking(true).unwrap();
        let raw = reader.into_raw_fd();
        // SAFETY: `raw` is the unique owner of the stream descriptor.
        let file = File::from_std(unsafe { std::fs::File::from_raw_fd(raw) });
        let mut table = Table::default();
        let fd = table
            .push(Entry {
                resource: Resource::File(file),
                rights_base: RIGHTS_FD_READ | RIGHTS_FD_SEEK | RIGHTS_FD_TELL,
                rights_inheriting: 0,
                fdflags: 0,
                filetype: FILETYPE_UNKNOWN,
            })
            .unwrap();

        struct Memory(std::cell::RefCell<Vec<u8>>);
        impl GuestMemory for Memory {
            fn size(&self) -> u64 {
                self.0.borrow().len() as u64
            }
            fn read(&self, offset: u32, bytes: &mut [u8]) -> Result<(), u16> {
                let memory = self.0.borrow();
                let data = memory
                    .get(offset as usize..offset as usize + bytes.len())
                    .ok_or(FAULT)?;
                bytes.copy_from_slice(data);
                Ok(())
            }
            fn write(&self, offset: u32, bytes: &[u8]) -> Result<(), u16> {
                let mut memory = self.0.borrow_mut();
                let data = memory
                    .get_mut(offset as usize..offset as usize + bytes.len())
                    .ok_or(FAULT)?;
                data.copy_from_slice(bytes);
                Ok(())
            }
        }
        let memory = Memory(std::cell::RefCell::new(vec![0; 65536]));
        fn write_iovec(memory: &impl GuestMemory, ptr: u32, buf: u32, len: u32) {
            let mut bytes = [0u8; 8];
            bytes[..4].copy_from_slice(&buf.to_le_bytes());
            bytes[4..].copy_from_slice(&len.to_le_bytes());
            memory.write(ptr, &bytes).unwrap();
        }

        write_iovec(&memory, 16, 100, 3);
        assert_eq!(
            fd_read(&mut table, &memory, fd, 16, 1, 32),
            i32::from(AGAIN)
        );

        writer.write_all(b"abc").unwrap();
        write_iovec(&memory, 16, 65535, 3);
        assert_eq!(
            fd_read(&mut table, &memory, fd, 16, 1, 32),
            i32::from(FAULT)
        );

        write_iovec(&memory, 16, 100, 3);
        assert_eq!(
            fd_read(&mut table, &memory, fd, 16, 1, 32),
            i32::from(SUCCESS)
        );
        let mut nread = [0u8; 4];
        memory.read(32, &mut nread).unwrap();
        assert_eq!(u32::from_le_bytes(nread), 3);
        let mut got = [0u8; 3];
        memory.read(100, &mut got).unwrap();
        assert_eq!(&got, b"abc");

        drop(writer);
        assert_eq!(
            fd_read(&mut table, &memory, fd, 16, 1, 32),
            i32::from(SUCCESS)
        );
        memory.read(32, &mut nread).unwrap();
        assert_eq!(u32::from_le_bytes(nread), 0);
        assert_eq!(fd_close(&mut table, fd), i32::from(SUCCESS));
        assert_eq!(fd_close(&mut table, fd), i32::from(BADF));
        assert_eq!(fd_read(&mut table, &memory, fd, 16, 1, 32), i32::from(BADF));
    }
}

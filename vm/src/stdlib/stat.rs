use crate::{PyRef, VirtualMachine, builtins::PyModule};

#[pymodule]
mod stat {
    #[cfg(unix)]
    use libc;

    // Use libc::mode_t for Mode to match the system's definition
    #[cfg(unix)]
    type Mode = libc::mode_t;
    #[cfg(windows)]
    type Mode = u16; // Windows does not have mode_t, but stat constants are u16
    #[cfg(not(any(unix, windows)))]
    type Mode = u32; // Fallback for unknown targets

    #[cfg(unix)]
    #[pyattr]
    pub const S_IFDIR: Mode = libc::S_IFDIR;
    #[cfg(not(unix))]
    #[pyattr]
    pub const S_IFDIR: Mode = 0o040000;

    #[cfg(unix)]
    #[pyattr]
    pub const S_IFCHR: Mode = libc::S_IFCHR;
    #[cfg(not(unix))]
    #[pyattr]
    pub const S_IFCHR: Mode = 0o020000;

    #[cfg(unix)]
    #[pyattr]
    pub const S_IFBLK: Mode = libc::S_IFBLK;
    #[cfg(not(unix))]
    #[pyattr]
    pub const S_IFBLK: Mode = 0o060000;

    #[cfg(unix)]
    #[pyattr]
    pub const S_IFREG: Mode = libc::S_IFREG;
    #[cfg(not(unix))]
    #[pyattr]
    pub const S_IFREG: Mode = 0o100000;

    #[cfg(unix)]
    #[pyattr]
    pub const S_IFIFO: Mode = libc::S_IFIFO;
    #[cfg(not(unix))]
    #[pyattr]
    pub const S_IFIFO: Mode = 0o010000;

    #[cfg(unix)]
    #[pyattr]
    pub const S_IFLNK: Mode = libc::S_IFLNK;
    #[cfg(not(unix))]
    #[pyattr]
    pub const S_IFLNK: Mode = 0o120000;

    #[cfg(unix)]
    #[pyattr]
    pub const S_IFSOCK: Mode = libc::S_IFSOCK;
    #[cfg(not(unix))]
    #[pyattr]
    pub const S_IFSOCK: Mode = 0o140000;

    // TODO: RUSTPYTHON Support Solaris
    #[pyattr]
    pub const S_IFDOOR: Mode = 0;

    // TODO: RUSTPYTHON Support Solaris
    #[pyattr]
    pub const S_IFPORT: Mode = 0;

    // TODO: RUSTPYTHON Support BSD
    // https://man.freebsd.org/cgi/man.cgi?stat(2)

    #[cfg(target_os = "macos")]
    #[pyattr]
    pub const S_IFWHT: Mode = 0o160000;
    #[cfg(not(target_os = "macos"))]
    #[pyattr]
    pub const S_IFWHT: Mode = 0;

    // Permission bits
    #[cfg(unix)]
    #[pyattr]
    pub const S_ISUID: Mode = libc::S_ISUID;
    #[cfg(not(unix))]
    #[pyattr]
    pub const S_ISUID: Mode = 0o4000;

    #[cfg(unix)]
    #[pyattr]
    pub const S_ISGID: Mode = libc::S_ISGID;
    #[cfg(not(unix))]
    #[pyattr]
    pub const S_ISGID: Mode = 0o2000;

    #[cfg(unix)]
    #[pyattr]
    pub const S_ENFMT: Mode = libc::S_ISGID;
    #[cfg(not(unix))]
    #[pyattr]
    pub const S_ENFMT: Mode = 0o2000;

    #[cfg(unix)]
    #[pyattr]
    pub const S_ISVTX: Mode = libc::S_ISVTX;
    #[cfg(not(unix))]
    #[pyattr]
    pub const S_ISVTX: Mode = 0o1000;

    #[cfg(unix)]
    #[pyattr]
    pub const S_IRWXU: Mode = libc::S_IRWXU;
    #[cfg(not(unix))]
    #[pyattr]
    pub const S_IRWXU: Mode = 0o0700;

    #[cfg(unix)]
    #[pyattr]
    pub const S_IRUSR: Mode = libc::S_IRUSR;
    #[cfg(not(unix))]
    #[pyattr]
    pub const S_IRUSR: Mode = 0o0400;

    #[cfg(unix)]
    #[pyattr]
    pub const S_IREAD: Mode = libc::S_IRUSR;
    #[cfg(not(unix))]
    #[pyattr]
    pub const S_IREAD: Mode = 0o0400;

    #[cfg(unix)]
    #[pyattr]
    pub const S_IWUSR: Mode = libc::S_IWUSR;
    #[cfg(not(unix))]
    #[pyattr]
    pub const S_IWUSR: Mode = 0o0200;

    #[cfg(all(unix, not(target_os = "android"), not(target_os = "redox")))]
    #[pyattr]
    pub const S_IWRITE: Mode = libc::S_IWRITE;
    #[cfg(any(not(unix), target_os = "android", target_os = "redox"))]
    #[pyattr]
    pub const S_IWRITE: Mode = 0o0200;

    #[cfg(unix)]
    #[pyattr]
    pub const S_IXUSR: Mode = libc::S_IXUSR;
    #[cfg(not(unix))]
    #[pyattr]
    pub const S_IXUSR: Mode = 0o0100;

    #[cfg(all(unix, not(target_os = "android"), not(target_os = "redox")))]
    #[pyattr]
    pub const S_IEXEC: Mode = libc::S_IEXEC;
    #[cfg(any(not(unix), target_os = "android", target_os = "redox"))]
    #[pyattr]
    pub const S_IEXEC: Mode = 0o0100;

    #[cfg(unix)]
    #[pyattr]
    pub const S_IRWXG: Mode = libc::S_IRWXG;
    #[cfg(not(unix))]
    #[pyattr]
    pub const S_IRWXG: Mode = 0o0070;

    #[cfg(unix)]
    #[pyattr]
    pub const S_IRGRP: Mode = libc::S_IRGRP;
    #[cfg(not(unix))]
    #[pyattr]
    pub const S_IRGRP: Mode = 0o0040;

    #[cfg(unix)]
    #[pyattr]
    pub const S_IWGRP: Mode = libc::S_IWGRP;
    #[cfg(not(unix))]
    #[pyattr]
    pub const S_IWGRP: Mode = 0o0020;

    #[cfg(unix)]
    #[pyattr]
    pub const S_IXGRP: Mode = libc::S_IXGRP;
    #[cfg(not(unix))]
    #[pyattr]
    pub const S_IXGRP: Mode = 0o0010;

    #[cfg(unix)]
    #[pyattr]
    pub const S_IRWXO: Mode = libc::S_IRWXO;
    #[cfg(not(unix))]
    #[pyattr]
    pub const S_IRWXO: Mode = 0o0007;

    #[cfg(unix)]
    #[pyattr]
    pub const S_IROTH: Mode = libc::S_IROTH;
    #[cfg(not(unix))]
    #[pyattr]
    pub const S_IROTH: Mode = 0o0004;

    #[cfg(unix)]
    #[pyattr]
    pub const S_IWOTH: Mode = libc::S_IWOTH;
    #[cfg(not(unix))]
    #[pyattr]
    pub const S_IWOTH: Mode = 0o0002;

    #[cfg(unix)]
    #[pyattr]
    pub const S_IXOTH: Mode = libc::S_IXOTH;
    #[cfg(not(unix))]
    #[pyattr]
    pub const S_IXOTH: Mode = 0o0001;

    // Stat result indices
    #[pyattr]
    pub const ST_MODE: u32 = 0;

    #[pyattr]
    pub const ST_INO: u32 = 1;

    #[pyattr]
    pub const ST_DEV: u32 = 2;

    #[pyattr]
    pub const ST_NLINK: u32 = 3;

    #[pyattr]
    pub const ST_UID: u32 = 4;

    #[pyattr]
    pub const ST_GID: u32 = 5;

    #[pyattr]
    pub const ST_SIZE: u32 = 6;

    #[pyattr]
    pub const ST_ATIME: u32 = 7;

    #[pyattr]
    pub const ST_MTIME: u32 = 8;

    #[pyattr]
    pub const ST_CTIME: u32 = 9;

    const S_IFMT: Mode = 0o170000;

    const S_IMODE: Mode = 0o7777;

    #[pyfunction]
    #[allow(non_snake_case)]
    const fn S_ISDIR(mode: Mode) -> bool {
        (mode & S_IFMT) == S_IFDIR
    }

    #[pyfunction]
    #[allow(non_snake_case)]
    const fn S_ISCHR(mode: Mode) -> bool {
        (mode & S_IFMT) == S_IFCHR
    }

    #[pyfunction]
    #[allow(non_snake_case)]
    const fn S_ISREG(mode: Mode) -> bool {
        (mode & S_IFMT) == S_IFREG
    }

    #[pyfunction]
    #[allow(non_snake_case)]
    const fn S_ISBLK(mode: Mode) -> bool {
        (mode & S_IFMT) == S_IFBLK
    }

    #[pyfunction]
    #[allow(non_snake_case)]
    const fn S_ISFIFO(mode: Mode) -> bool {
        (mode & S_IFMT) == S_IFIFO
    }

    #[pyfunction]
    #[allow(non_snake_case)]
    const fn S_ISLNK(mode: Mode) -> bool {
        (mode & S_IFMT) == S_IFLNK
    }

    #[pyfunction]
    #[allow(non_snake_case)]
    const fn S_ISSOCK(mode: Mode) -> bool {
        (mode & S_IFMT) == S_IFSOCK
    }

    // TODO: RUSTPYTHON Support Solaris
    #[pyfunction]
    #[allow(non_snake_case)]
    const fn S_ISDOOR(_mode: Mode) -> bool {
        false
    }

    // TODO: RUSTPYTHON Support Solaris
    #[pyfunction]
    #[allow(non_snake_case)]
    const fn S_ISPORT(_mode: Mode) -> bool {
        false
    }

    // TODO: RUSTPYTHON Support BSD
    #[pyfunction]
    #[allow(non_snake_case)]
    const fn S_ISWHT(_mode: Mode) -> bool {
        false
    }

    #[pyfunction(name = "S_IMODE")]
    #[allow(non_snake_case)]
    const fn S_IMODE_method(mode: Mode) -> Mode {
        mode & S_IMODE
    }

    #[pyfunction(name = "S_IFMT")]
    #[allow(non_snake_case)]
    const fn S_IFMT_method(mode: Mode) -> Mode {
        // 0o170000 is from the S_IFMT definition in CPython include/fileutils.h
        mode & S_IFMT
    }

    #[pyfunction]
    const fn filetype(mode: Mode) -> char {
        if S_ISREG(mode) {
            '-'
        } else if S_ISDIR(mode) {
            'd'
        } else if S_ISLNK(mode) {
            'l'
        } else if S_ISBLK(mode) {
            'b'
        } else if S_ISCHR(mode) {
            'c'
        } else if S_ISFIFO(mode) {
            'p'
        } else if S_ISSOCK(mode) {
            's'
        } else if S_ISDOOR(mode) {
            'D' // TODO: RUSTPYTHON Support Solaris
        } else if S_ISPORT(mode) {
            'P' // TODO: RUSTPYTHON Support Solaris
        } else if S_ISWHT(mode) {
            'w' // TODO: RUSTPYTHON Support BSD
        } else {
            '?' // Unknown file type
        }
    }

    // Convert file mode to string representation
    #[pyfunction]
    fn filemode(mode: Mode) -> String {
        let mut result = String::with_capacity(10);

        // File type
        result.push(filetype(mode));

        // User permissions
        result.push(if mode & S_IRUSR != 0 { 'r' } else { '-' });
        result.push(if mode & S_IWUSR != 0 { 'w' } else { '-' });
        if mode & S_ISUID != 0 {
            result.push(if mode & S_IXUSR != 0 { 's' } else { 'S' });
        } else {
            result.push(if mode & S_IXUSR != 0 { 'x' } else { '-' });
        }

        // Group permissions
        result.push(if mode & S_IRGRP != 0 { 'r' } else { '-' });
        result.push(if mode & S_IWGRP != 0 { 'w' } else { '-' });
        if mode & S_ISGID != 0 {
            result.push(if mode & S_IXGRP != 0 { 's' } else { 'S' });
        } else {
            result.push(if mode & S_IXGRP != 0 { 'x' } else { '-' });
        }

        // Other permissions
        result.push(if mode & S_IROTH != 0 { 'r' } else { '-' });
        result.push(if mode & S_IWOTH != 0 { 'w' } else { '-' });
        if mode & S_ISVTX != 0 {
            result.push(if mode & S_IXOTH != 0 { 't' } else { 'T' });
        } else {
            result.push(if mode & S_IXOTH != 0 { 'x' } else { '-' });
        }

        result
    }

    // Windows file attributes (if on Windows)
    #[cfg(windows)]
    #[pyattr]
    pub const FILE_ATTRIBUTE_ARCHIVE: u32 =
        windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_ARCHIVE;

    #[cfg(windows)]
    #[pyattr]
    pub const FILE_ATTRIBUTE_COMPRESSED: u32 =
        windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_COMPRESSED;

    #[cfg(windows)]
    #[pyattr]
    pub const FILE_ATTRIBUTE_DEVICE: u32 =
        windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_DEVICE;

    #[cfg(windows)]
    #[pyattr]
    pub const FILE_ATTRIBUTE_DIRECTORY: u32 =
        windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_DIRECTORY;

    #[cfg(windows)]
    #[pyattr]
    pub const FILE_ATTRIBUTE_ENCRYPTED: u32 =
        windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_ENCRYPTED;

    #[cfg(windows)]
    #[pyattr]
    pub const FILE_ATTRIBUTE_HIDDEN: u32 =
        windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_HIDDEN;

    #[cfg(windows)]
    #[pyattr]
    pub const FILE_ATTRIBUTE_INTEGRITY_STREAM: u32 =
        windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_INTEGRITY_STREAM;

    #[cfg(windows)]
    #[pyattr]
    pub const FILE_ATTRIBUTE_NORMAL: u32 =
        windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL;

    #[cfg(windows)]
    #[pyattr]
    pub const FILE_ATTRIBUTE_NOT_CONTENT_INDEXED: u32 =
        windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NOT_CONTENT_INDEXED;

    #[cfg(windows)]
    #[pyattr]
    pub const FILE_ATTRIBUTE_NO_SCRUB_DATA: u32 =
        windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NO_SCRUB_DATA;

    #[cfg(windows)]
    #[pyattr]
    pub const FILE_ATTRIBUTE_OFFLINE: u32 =
        windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_OFFLINE;

    #[cfg(windows)]
    #[pyattr]
    pub const FILE_ATTRIBUTE_READONLY: u32 =
        windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_READONLY;

    #[cfg(windows)]
    #[pyattr]
    pub const FILE_ATTRIBUTE_REPARSE_POINT: u32 =
        windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

    #[cfg(windows)]
    #[pyattr]
    pub const FILE_ATTRIBUTE_SPARSE_FILE: u32 =
        windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_SPARSE_FILE;

    #[cfg(windows)]
    #[pyattr]
    pub const FILE_ATTRIBUTE_SYSTEM: u32 =
        windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_SYSTEM;

    #[cfg(windows)]
    #[pyattr]
    pub const FILE_ATTRIBUTE_TEMPORARY: u32 =
        windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_TEMPORARY;

    #[cfg(windows)]
    #[pyattr]
    pub const FILE_ATTRIBUTE_VIRTUAL: u32 =
        windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_VIRTUAL;

    // Unix file flags (if on Unix)
    #[cfg(target_os = "macos")]
    #[pyattr]
    pub const UF_NODUMP: u32 = libc::UF_NODUMP;
    #[cfg(not(target_os = "macos"))]
    #[pyattr]
    pub const UF_NODUMP: u32 = 0x00000001;

    #[cfg(target_os = "macos")]
    #[pyattr]
    pub const UF_IMMUTABLE: u32 = libc::UF_IMMUTABLE;
    #[cfg(not(target_os = "macos"))]
    #[pyattr]
    pub const UF_IMMUTABLE: u32 = 0x00000002;

    #[cfg(target_os = "macos")]
    #[pyattr]
    pub const UF_APPEND: u32 = libc::UF_APPEND;
    #[cfg(not(target_os = "macos"))]
    #[pyattr]
    pub const UF_APPEND: u32 = 0x00000004;

    #[cfg(target_os = "macos")]
    #[pyattr]
    pub const UF_OPAQUE: u32 = libc::UF_OPAQUE;
    #[cfg(not(target_os = "macos"))]
    #[pyattr]
    pub const UF_OPAQUE: u32 = 0x00000008;

    #[pyattr]
    pub const UF_NOUNLINK: u32 = 0x00000010;

    #[cfg(target_os = "macos")]
    #[pyattr]
    pub const UF_COMPRESSED: u32 = libc::UF_COMPRESSED;
    #[cfg(not(target_os = "macos"))]
    #[pyattr]
    pub const UF_COMPRESSED: u32 = 0x00000020;

    #[cfg(target_os = "macos")]
    #[pyattr]
    pub const UF_HIDDEN: u32 = libc::UF_HIDDEN;
    #[cfg(not(target_os = "macos"))]
    #[pyattr]
    pub const UF_HIDDEN: u32 = 0x00008000;

    #[cfg(target_os = "macos")]
    #[pyattr]
    pub const SF_ARCHIVED: u32 = libc::SF_ARCHIVED;
    #[cfg(not(target_os = "macos"))]
    #[pyattr]
    pub const SF_ARCHIVED: u32 = 0x00010000;

    #[cfg(target_os = "macos")]
    #[pyattr]
    pub const SF_IMMUTABLE: u32 = libc::SF_IMMUTABLE;
    #[cfg(not(target_os = "macos"))]
    #[pyattr]
    pub const SF_IMMUTABLE: u32 = 0x00020000;

    #[cfg(target_os = "macos")]
    #[pyattr]
    pub const SF_APPEND: u32 = libc::SF_APPEND;
    #[cfg(not(target_os = "macos"))]
    #[pyattr]
    pub const SF_APPEND: u32 = 0x00040000;

    #[pyattr]
    pub const SF_NOUNLINK: u32 = 0x00100000;

    #[pyattr]
    pub const SF_SNAPSHOT: u32 = 0x00200000;

    #[pyattr]
    pub const SF_FIRMLINK: u32 = 0x00800000;

    #[pyattr]
    pub const SF_DATALESS: u32 = 0x40000000;

    #[cfg(target_os = "macos")]
    #[pyattr]
    pub const SF_SUPPORTED: u32 = 0x009f0000;

    #[cfg(target_os = "macos")]
    #[pyattr]
    pub const SF_SETTABLE: u32 = 0x3fff0000;
    #[cfg(not(target_os = "macos"))]
    #[pyattr]
    pub const SF_SETTABLE: u32 = 0xffff0000;

    #[cfg(target_os = "macos")]
    #[pyattr]
    pub const SF_SYNTHETIC: u32 = 0xc0000000;
}

pub fn make_module(vm: &VirtualMachine) -> PyRef<PyModule> {
    stat::make_module(vm)
}

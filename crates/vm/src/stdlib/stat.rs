use crate::{PyRef, VirtualMachine, builtins::PyModule};

#[pymodule]
mod stat {
    // Use libc::mode_t for Mode to match the system's definition
    #[cfg(unix)]
    type Mode = libc::mode_t;
    #[cfg(windows)]
    type Mode = u16; // Windows does not have mode_t, but stat constants are u16
    #[cfg(not(any(unix, windows)))]
    type Mode = u32; // Fallback for unknown targets

    // libc_const macro for conditional compilation
    macro_rules! libc_const {
        (#[cfg($cfg:meta)] $name:ident, $fallback:expr) => {{
            #[cfg($cfg)]
            {
                libc::$name
            }
            #[cfg(not($cfg))]
            {
                $fallback
            }
        }};
    }

    #[pyattr]
    pub const S_IFDIR: Mode = libc_const!(
        #[cfg(unix)]
        S_IFDIR,
        0o040000
    );

    #[pyattr]
    pub const S_IFCHR: Mode = libc_const!(
        #[cfg(unix)]
        S_IFCHR,
        0o020000
    );

    #[pyattr]
    pub const S_IFBLK: Mode = libc_const!(
        #[cfg(unix)]
        S_IFBLK,
        0o060000
    );

    #[pyattr]
    pub const S_IFREG: Mode = libc_const!(
        #[cfg(unix)]
        S_IFREG,
        0o100000
    );

    #[pyattr]
    pub const S_IFIFO: Mode = libc_const!(
        #[cfg(unix)]
        S_IFIFO,
        0o010000
    );

    #[pyattr]
    pub const S_IFLNK: Mode = libc_const!(
        #[cfg(unix)]
        S_IFLNK,
        0o120000
    );

    #[pyattr]
    pub const S_IFSOCK: Mode = libc_const!(
        #[cfg(unix)]
        S_IFSOCK,
        0o140000
    );

    #[pyattr]
    pub const S_IFDOOR: Mode = 0; // TODO: RUSTPYTHON Support Solaris

    #[pyattr]
    pub const S_IFPORT: Mode = 0; // TODO: RUSTPYTHON Support Solaris

    // TODO: RUSTPYTHON Support BSD
    // https://man.freebsd.org/cgi/man.cgi?stat(2)

    #[pyattr]
    pub const S_IFWHT: Mode = if cfg!(target_os = "macos") {
        0o160000
    } else {
        0
    };

    // Permission bits

    #[pyattr]
    pub const S_ISUID: Mode = libc_const!(
        #[cfg(unix)]
        S_ISUID,
        0o4000
    );

    #[pyattr]
    pub const S_ISGID: Mode = libc_const!(
        #[cfg(unix)]
        S_ISGID,
        0o2000
    );

    #[pyattr]
    pub const S_ENFMT: Mode = libc_const!(
        #[cfg(unix)]
        S_ISGID,
        0o2000
    );

    #[pyattr]
    pub const S_ISVTX: Mode = libc_const!(
        #[cfg(unix)]
        S_ISVTX,
        0o1000
    );

    #[pyattr]
    pub const S_IRWXU: Mode = libc_const!(
        #[cfg(unix)]
        S_IRWXU,
        0o0700
    );

    #[pyattr]
    pub const S_IRUSR: Mode = libc_const!(
        #[cfg(unix)]
        S_IRUSR,
        0o0400
    );

    #[pyattr]
    pub const S_IREAD: Mode = libc_const!(
        #[cfg(unix)]
        S_IRUSR,
        0o0400
    );

    #[pyattr]
    pub const S_IWUSR: Mode = libc_const!(
        #[cfg(unix)]
        S_IWUSR,
        0o0200
    );

    #[pyattr]
    pub const S_IXUSR: Mode = libc_const!(
        #[cfg(unix)]
        S_IXUSR,
        0o0100
    );

    #[pyattr]
    pub const S_IRWXG: Mode = libc_const!(
        #[cfg(unix)]
        S_IRWXG,
        0o0070
    );

    #[pyattr]
    pub const S_IRGRP: Mode = libc_const!(
        #[cfg(unix)]
        S_IRGRP,
        0o0040
    );

    #[pyattr]
    pub const S_IWGRP: Mode = libc_const!(
        #[cfg(unix)]
        S_IWGRP,
        0o0020
    );

    #[pyattr]
    pub const S_IXGRP: Mode = libc_const!(
        #[cfg(unix)]
        S_IXGRP,
        0o0010
    );

    #[pyattr]
    pub const S_IRWXO: Mode = libc_const!(
        #[cfg(unix)]
        S_IRWXO,
        0o0007
    );

    #[pyattr]
    pub const S_IROTH: Mode = libc_const!(
        #[cfg(unix)]
        S_IROTH,
        0o0004
    );

    #[pyattr]
    pub const S_IWOTH: Mode = libc_const!(
        #[cfg(unix)]
        S_IWOTH,
        0o0002
    );

    #[pyattr]
    pub const S_IXOTH: Mode = libc_const!(
        #[cfg(unix)]
        S_IXOTH,
        0o0001
    );

    #[pyattr]
    pub const S_IWRITE: Mode = libc_const!(
        #[cfg(all(unix, not(target_os = "android"), not(target_os = "redox")))]
        S_IWRITE,
        0o0200
    );

    #[pyattr]
    pub const S_IEXEC: Mode = libc_const!(
        #[cfg(all(unix, not(target_os = "android"), not(target_os = "redox")))]
        S_IEXEC,
        0o0100
    );

    // Windows file attributes (if on Windows)

    #[cfg(windows)]
    #[pyattr]
    pub use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_ARCHIVE, FILE_ATTRIBUTE_COMPRESSED, FILE_ATTRIBUTE_DEVICE,
        FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_ENCRYPTED, FILE_ATTRIBUTE_HIDDEN,
        FILE_ATTRIBUTE_INTEGRITY_STREAM, FILE_ATTRIBUTE_NO_SCRUB_DATA, FILE_ATTRIBUTE_NORMAL,
        FILE_ATTRIBUTE_NOT_CONTENT_INDEXED, FILE_ATTRIBUTE_OFFLINE, FILE_ATTRIBUTE_READONLY,
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_SPARSE_FILE, FILE_ATTRIBUTE_SYSTEM,
        FILE_ATTRIBUTE_TEMPORARY, FILE_ATTRIBUTE_VIRTUAL,
    };

    // Unix file flags (if on Unix)

    #[pyattr]
    pub const UF_NODUMP: u32 = libc_const!(
        #[cfg(target_os = "macos")]
        UF_NODUMP,
        0x00000001
    );

    #[pyattr]
    pub const UF_IMMUTABLE: u32 = libc_const!(
        #[cfg(target_os = "macos")]
        UF_IMMUTABLE,
        0x00000002
    );

    #[pyattr]
    pub const UF_APPEND: u32 = libc_const!(
        #[cfg(target_os = "macos")]
        UF_APPEND,
        0x00000004
    );

    #[pyattr]
    pub const UF_OPAQUE: u32 = libc_const!(
        #[cfg(target_os = "macos")]
        UF_OPAQUE,
        0x00000008
    );

    #[pyattr]
    pub const UF_COMPRESSED: u32 = libc_const!(
        #[cfg(target_os = "macos")]
        UF_COMPRESSED,
        0x00000020
    );

    #[pyattr]
    pub const UF_HIDDEN: u32 = libc_const!(
        #[cfg(target_os = "macos")]
        UF_HIDDEN,
        0x00008000
    );

    #[pyattr]
    pub const SF_ARCHIVED: u32 = libc_const!(
        #[cfg(target_os = "macos")]
        SF_ARCHIVED,
        0x00010000
    );

    #[pyattr]
    pub const SF_IMMUTABLE: u32 = libc_const!(
        #[cfg(target_os = "macos")]
        SF_IMMUTABLE,
        0x00020000
    );

    #[pyattr]
    pub const SF_APPEND: u32 = libc_const!(
        #[cfg(target_os = "macos")]
        SF_APPEND,
        0x00040000
    );

    #[pyattr]
    pub const SF_SETTABLE: u32 = if cfg!(target_os = "macos") {
        0x3fff0000
    } else {
        0xffff0000
    };

    #[pyattr]
    pub const UF_NOUNLINK: u32 = 0x00000010;

    #[pyattr]
    pub const SF_NOUNLINK: u32 = 0x00100000;

    #[pyattr]
    pub const SF_SNAPSHOT: u32 = 0x00200000;

    #[pyattr]
    pub const SF_FIRMLINK: u32 = 0x00800000;

    #[pyattr]
    pub const SF_DATALESS: u32 = 0x40000000;

    // MacOS specific

    #[cfg(target_os = "macos")]
    #[pyattr]
    pub const SF_SUPPORTED: u32 = 0x009f0000;

    #[cfg(target_os = "macos")]
    #[pyattr]
    pub const SF_SYNTHETIC: u32 = 0xc0000000;

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
}

pub fn make_module(vm: &VirtualMachine) -> PyRef<PyModule> {
    stat::make_module(vm)
}

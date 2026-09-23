pub(crate) use _stat::module_def;

#[pymodule]
mod _stat {
    #![allow(unreachable_pub)]

    #[cfg(windows)]
    use rustpython_host_env::nt as host_nt;

    use malachite_bigint::Sign;
    use num_traits::ToPrimitive;

    use crate::{PyObjectRef, PyResult, VirtualMachine, convert::TryFromObject};

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
    pub const S_IFDOOR: Mode = rustpython_host_env::os::S_IFDOOR as Mode;

    #[pyattr]
    pub const S_IFPORT: Mode = rustpython_host_env::os::S_IFPORT as Mode;

    #[pyattr]
    pub const S_IFWHT: Mode = rustpython_host_env::os::S_IFWHT as Mode;

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
    pub use host_nt::{
        FILE_ATTRIBUTE_ARCHIVE, FILE_ATTRIBUTE_COMPRESSED, FILE_ATTRIBUTE_DEVICE,
        FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_ENCRYPTED, FILE_ATTRIBUTE_HIDDEN,
        FILE_ATTRIBUTE_INTEGRITY_STREAM, FILE_ATTRIBUTE_NO_SCRUB_DATA, FILE_ATTRIBUTE_NORMAL,
        FILE_ATTRIBUTE_NOT_CONTENT_INDEXED, FILE_ATTRIBUTE_OFFLINE, FILE_ATTRIBUTE_READONLY,
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_SPARSE_FILE, FILE_ATTRIBUTE_SYSTEM,
        FILE_ATTRIBUTE_TEMPORARY, FILE_ATTRIBUTE_VIRTUAL,
    };

    #[cfg(windows)]
    #[pyattr]
    pub use host_nt::{
        IO_REPARSE_TAG_APPEXECLINK, IO_REPARSE_TAG_MOUNT_POINT, IO_REPARSE_TAG_SYMLINK,
    };

    // Unix file flags (if on Unix)

    #[pyattr]
    pub use rustpython_host_env::os::{
        SF_APPEND, SF_ARCHIVED, SF_DATALESS, SF_FIRMLINK, SF_IMMUTABLE, SF_NOUNLINK, SF_SETTABLE,
        SF_SNAPSHOT, UF_APPEND, UF_COMPRESSED, UF_DATAVAULT, UF_HIDDEN, UF_IMMUTABLE, UF_NODUMP,
        UF_NOUNLINK, UF_OPAQUE, UF_SETTABLE, UF_TRACKED,
    };

    #[cfg(target_os = "macos")]
    #[pyattr]
    pub use rustpython_host_env::os::{SF_SUPPORTED, SF_SYNTHETIC};

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

    /// The mode an argument stands for, which is read as an unsigned long and
    /// then measured against the range a mode has. = mode_converter
    #[derive(Copy, Clone)]
    struct ModeArg(Mode);

    impl TryFromObject for ModeArg {
        fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
            let index = obj.try_index(vm)?;
            let value = index.as_bigint();
            // = PyLong_AsUnsignedLong
            if value.sign() == Sign::Minus {
                return Err(vm.new_overflow_error("can't convert negative value to unsigned int"));
            }
            let value = value.to_u64().ok_or_else(|| {
                vm.new_overflow_error("Python int too large to convert to C unsigned long")
            })?;
            Mode::try_from(value)
                .map(Self)
                .map_err(|_| vm.new_overflow_error("mode out of range"))
        }
    }

    const S_IFMT: Mode = 0o170000;

    const S_IMODE: Mode = 0o7777;

    #[pyfunction]
    #[allow(non_snake_case)]
    const fn S_ISDIR(mode: ModeArg) -> bool {
        (mode.0 & S_IFMT) == S_IFDIR
    }

    #[pyfunction]
    #[allow(non_snake_case)]
    const fn S_ISCHR(mode: ModeArg) -> bool {
        (mode.0 & S_IFMT) == S_IFCHR
    }

    #[pyfunction]
    #[allow(non_snake_case)]
    const fn S_ISREG(mode: ModeArg) -> bool {
        (mode.0 & S_IFMT) == S_IFREG
    }

    #[pyfunction]
    #[allow(non_snake_case)]
    const fn S_ISBLK(mode: ModeArg) -> bool {
        (mode.0 & S_IFMT) == S_IFBLK
    }

    #[pyfunction]
    #[allow(non_snake_case)]
    const fn S_ISFIFO(mode: ModeArg) -> bool {
        (mode.0 & S_IFMT) == S_IFIFO
    }

    #[pyfunction]
    #[allow(non_snake_case)]
    const fn S_ISLNK(mode: ModeArg) -> bool {
        (mode.0 & S_IFMT) == S_IFLNK
    }

    #[pyfunction]
    #[allow(non_snake_case)]
    const fn S_ISSOCK(mode: ModeArg) -> bool {
        (mode.0 & S_IFMT) == S_IFSOCK
    }

    // TODO: RUSTPYTHON Support Solaris
    #[pyfunction]
    #[allow(non_snake_case)]
    const fn S_ISDOOR(_mode: ModeArg) -> bool {
        false
    }

    // TODO: RUSTPYTHON Support Solaris
    #[pyfunction]
    #[allow(non_snake_case)]
    const fn S_ISPORT(_mode: ModeArg) -> bool {
        false
    }

    // TODO: RUSTPYTHON Support BSD
    #[pyfunction]
    #[allow(non_snake_case)]
    const fn S_ISWHT(_mode: ModeArg) -> bool {
        false
    }

    #[pyfunction(name = "S_IMODE")]
    #[allow(non_snake_case)]
    const fn S_IMODE_method(mode: ModeArg) -> Mode {
        mode.0 & S_IMODE
    }

    #[pyfunction(name = "S_IFMT")]
    #[allow(non_snake_case)]
    const fn S_IFMT_method(mode: ModeArg) -> Mode {
        // 0o170000 is from the S_IFMT definition in CPython include/fileutils.h
        mode.0 & S_IFMT
    }

    /// The one character a mode's file type is written as, which the module
    /// keeps to itself.
    const fn filetype(mode: ModeArg) -> char {
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
    fn filemode(mode: ModeArg) -> String {
        let mut result = String::with_capacity(10);

        // File type
        result.push(filetype(mode));

        // User permissions
        result.push(if mode.0 & S_IRUSR != 0 { 'r' } else { '-' });
        result.push(if mode.0 & S_IWUSR != 0 { 'w' } else { '-' });
        if mode.0 & S_ISUID != 0 {
            result.push(if mode.0 & S_IXUSR != 0 { 's' } else { 'S' });
        } else {
            result.push(if mode.0 & S_IXUSR != 0 { 'x' } else { '-' });
        }

        // Group permissions
        result.push(if mode.0 & S_IRGRP != 0 { 'r' } else { '-' });
        result.push(if mode.0 & S_IWGRP != 0 { 'w' } else { '-' });
        if mode.0 & S_ISGID != 0 {
            result.push(if mode.0 & S_IXGRP != 0 { 's' } else { 'S' });
        } else {
            result.push(if mode.0 & S_IXGRP != 0 { 'x' } else { '-' });
        }

        // Other permissions
        result.push(if mode.0 & S_IROTH != 0 { 'r' } else { '-' });
        result.push(if mode.0 & S_IWOTH != 0 { 'w' } else { '-' });
        if mode.0 & S_ISVTX != 0 {
            result.push(if mode.0 & S_IXOTH != 0 { 't' } else { 'T' });
        } else {
            result.push(if mode.0 & S_IXOTH != 0 { 'x' } else { '-' });
        }

        result
    }
}

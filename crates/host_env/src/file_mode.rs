bitflagset::bitflag! {
    #[derive(Copy, Clone, Debug, PartialEq, Eq)]
    #[repr(u8)]
    pub enum FileModeFlag {
        Created = 0,
        Readable = 1,
        Writable = 2,
        Appending = 3,
        Truncating = 4,
    }
}

bitflagset::bitflagset! {
    #[derive(Copy, Clone, PartialEq, Eq)]
    pub struct FileMode(u8): FileModeFlag
}

impl FileMode {
    pub const CREATED: Self = Self::from_element(FileModeFlag::Created);
    pub const READABLE: Self = Self::from_element(FileModeFlag::Readable);
    pub const WRITABLE: Self = Self::from_element(FileModeFlag::Writable);
    pub const APPENDING: Self = Self::from_element(FileModeFlag::Appending);
    pub const TRUNCATING: Self = Self::from_element(FileModeFlag::Truncating);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileModeError {
    Invalid,
    BadRwa,
}

impl FileModeError {
    pub fn error_msg(self, mode_str: &str) -> String {
        match self {
            Self::Invalid => format!("invalid mode: {mode_str}"),
            Self::BadRwa => {
                "Must have exactly one of create/read/write/append mode and at most one plus"
                    .to_owned()
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ParsedFileMode {
    pub mode: FileMode,
    pub flags: i32,
}

impl FileMode {
    pub const fn raw_mode(self) -> &'static str {
        if self.contains(&FileModeFlag::Created) {
            if self.contains(&FileModeFlag::Readable) {
                "xb+"
            } else {
                "xb"
            }
        } else if self.contains(&FileModeFlag::Appending) {
            if self.contains(&FileModeFlag::Readable) {
                "ab+"
            } else {
                "ab"
            }
        } else if self.contains(&FileModeFlag::Readable) {
            if self.contains(&FileModeFlag::Writable) {
                if self.contains(&FileModeFlag::Truncating) {
                    "wb+"
                } else {
                    "rb+"
                }
            } else {
                "rb"
            }
        } else {
            "wb"
        }
    }
}

pub fn parse_fileio_mode(
    mode_str: &str,
    mut mode: FileMode,
) -> Result<ParsedFileMode, FileModeError> {
    let mut flags = 0;
    let mut plus = false;
    let mut rwa = false;
    for c in mode_str.bytes() {
        match c {
            b'x' => {
                if rwa {
                    return Err(FileModeError::BadRwa);
                }
                rwa = true;
                mode |= FileMode::WRITABLE | FileMode::CREATED;
                flags |= crate::os::O_EXCL | crate::os::O_CREAT;
            }
            b'r' => {
                if rwa {
                    return Err(FileModeError::BadRwa);
                }
                rwa = true;
                mode |= FileMode::READABLE;
            }
            b'w' => {
                if rwa {
                    return Err(FileModeError::BadRwa);
                }
                rwa = true;
                mode |= FileMode::WRITABLE | FileMode::TRUNCATING;
                flags |= crate::os::O_CREAT | crate::os::O_TRUNC;
            }
            b'a' => {
                if rwa {
                    return Err(FileModeError::BadRwa);
                }
                rwa = true;
                mode |= FileMode::WRITABLE | FileMode::APPENDING;
                flags |= crate::os::O_APPEND | crate::os::O_CREAT;
            }
            b'+' => {
                if plus {
                    return Err(FileModeError::BadRwa);
                }
                plus = true;
                mode |= FileMode::READABLE | FileMode::WRITABLE;
            }
            b'b' => {}
            _ => return Err(FileModeError::Invalid),
        }
    }

    if !rwa {
        return Err(FileModeError::BadRwa);
    }

    if mode.is_superset(&(FileMode::READABLE | FileMode::WRITABLE)) {
        flags |= crate::os::O_RDWR;
    } else if mode.contains(&FileModeFlag::Readable) {
        flags |= crate::os::O_RDONLY;
    } else {
        flags |= crate::os::O_WRONLY;
    }

    #[cfg(windows)]
    {
        flags |= crate::os::O_BINARY | crate::os::O_NOINHERIT;
    }
    #[cfg(unix)]
    {
        flags |= crate::os::O_CLOEXEC;
    }

    Ok(ParsedFileMode { mode, flags })
}

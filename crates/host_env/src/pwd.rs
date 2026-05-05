use nix::unistd::{self, User};
use std::io;

#[derive(Debug, Clone)]
pub struct Passwd {
    pub name: String,
    pub passwd: String,
    pub uid: u32,
    pub gid: u32,
    pub gecos: String,
    pub dir: String,
    pub shell: String,
}

impl From<User> for Passwd {
    fn from(user: User) -> Self {
        let cstr_lossy = |s: alloc::ffi::CString| {
            s.into_string()
                .unwrap_or_else(|e| e.into_cstring().to_string_lossy().into_owned())
        };
        let pathbuf_lossy = |p: std::path::PathBuf| {
            p.into_os_string()
                .into_string()
                .unwrap_or_else(|s| s.to_string_lossy().into_owned())
        };
        Self {
            name: user.name,
            passwd: cstr_lossy(user.passwd),
            uid: user.uid.as_raw(),
            gid: user.gid.as_raw(),
            gecos: cstr_lossy(user.gecos),
            dir: pathbuf_lossy(user.dir),
            shell: pathbuf_lossy(user.shell),
        }
    }
}

pub fn getpwnam(name: &str) -> Option<Passwd> {
    User::from_name(name).ok().flatten().map(Into::into)
}

pub fn getpwuid(uid: libc::uid_t) -> io::Result<Option<Passwd>> {
    User::from_uid(unistd::Uid::from_raw(uid))
        .map(|user| user.map(Into::into))
        .map_err(io::Error::from)
}

#[cfg(not(target_os = "android"))]
pub fn getpwall() -> Vec<Passwd> {
    static GETPWALL: parking_lot::Mutex<()> = parking_lot::Mutex::new(());
    let _guard = GETPWALL.lock();
    let mut list = Vec::new();

    unsafe { libc::setpwent() };
    while let Some(ptr) = core::ptr::NonNull::new(unsafe { libc::getpwent() }) {
        list.push(User::from(unsafe { ptr.as_ref() }).into());
    }
    unsafe { libc::endpwent() };

    list
}

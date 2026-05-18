use std::io;

pub struct Group {
    pub name: String,
    pub passwd: String,
    pub gid: u32,
    pub mem: Vec<String>,
}

fn cstr_lossy(s: alloc::ffi::CString) -> String {
    s.into_string()
        .unwrap_or_else(|e| e.into_cstring().to_string_lossy().into_owned())
}

impl From<nix::unistd::Group> for Group {
    fn from(group: nix::unistd::Group) -> Self {
        Self {
            name: group.name,
            passwd: cstr_lossy(group.passwd),
            gid: group.gid.as_raw(),
            mem: group.mem,
        }
    }
}

pub fn getgrgid(gid: libc::gid_t) -> io::Result<Option<Group>> {
    nix::unistd::Group::from_gid(nix::unistd::Gid::from_raw(gid))
        .map(|group| group.map(Into::into))
        .map_err(io::Error::from)
}

pub fn getgrnam(name: &str) -> io::Result<Option<Group>> {
    nix::unistd::Group::from_name(name)
        .map(|group| group.map(Into::into))
        .map_err(io::Error::from)
}

pub fn getgrall() -> Vec<Group> {
    use core::ptr::NonNull;

    static GETGRALL: parking_lot::Mutex<()> = parking_lot::Mutex::new(());
    let _guard = GETGRALL.lock();
    let mut list = Vec::new();

    unsafe { libc::setgrent() };
    while let Some(ptr) = NonNull::new(unsafe { libc::getgrent() }) {
        let group = nix::unistd::Group::from(unsafe { ptr.as_ref() });
        list.push(group.into());
    }
    unsafe { libc::endgrent() };

    list
}

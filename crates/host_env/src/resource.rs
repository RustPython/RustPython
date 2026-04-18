use std::io;

#[derive(Debug, Clone, Copy)]
pub struct RUsage {
    pub ru_utime: libc::timeval,
    pub ru_stime: libc::timeval,
    pub ru_maxrss: libc::c_long,
    pub ru_ixrss: libc::c_long,
    pub ru_idrss: libc::c_long,
    pub ru_isrss: libc::c_long,
    pub ru_minflt: libc::c_long,
    pub ru_majflt: libc::c_long,
    pub ru_nswap: libc::c_long,
    pub ru_inblock: libc::c_long,
    pub ru_oublock: libc::c_long,
    pub ru_msgsnd: libc::c_long,
    pub ru_msgrcv: libc::c_long,
    pub ru_nsignals: libc::c_long,
    pub ru_nvcsw: libc::c_long,
    pub ru_nivcsw: libc::c_long,
}

impl From<libc::rusage> for RUsage {
    fn from(rusage: libc::rusage) -> Self {
        Self {
            ru_utime: rusage.ru_utime,
            ru_stime: rusage.ru_stime,
            ru_maxrss: rusage.ru_maxrss,
            ru_ixrss: rusage.ru_ixrss,
            ru_idrss: rusage.ru_idrss,
            ru_isrss: rusage.ru_isrss,
            ru_minflt: rusage.ru_minflt,
            ru_majflt: rusage.ru_majflt,
            ru_nswap: rusage.ru_nswap,
            ru_inblock: rusage.ru_inblock,
            ru_oublock: rusage.ru_oublock,
            ru_msgsnd: rusage.ru_msgsnd,
            ru_msgrcv: rusage.ru_msgrcv,
            ru_nsignals: rusage.ru_nsignals,
            ru_nvcsw: rusage.ru_nvcsw,
            ru_nivcsw: rusage.ru_nivcsw,
        }
    }
}

pub fn getrusage(who: i32) -> io::Result<RUsage> {
    unsafe {
        let mut rusage = core::mem::MaybeUninit::<libc::rusage>::uninit();
        if libc::getrusage(who, rusage.as_mut_ptr()) == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(rusage.assume_init().into())
        }
    }
}

pub fn getrlimit(resource: i32) -> io::Result<libc::rlimit> {
    unsafe {
        let mut rlimit = core::mem::MaybeUninit::<libc::rlimit>::uninit();
        if libc::getrlimit(resource as _, rlimit.as_mut_ptr()) == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(rlimit.assume_init())
        }
    }
}

pub fn setrlimit(resource: i32, limits: libc::rlimit) -> io::Result<()> {
    unsafe {
        if libc::setrlimit(resource as _, &limits) == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

#[cfg(not(any(target_os = "redox", target_os = "wasi")))]
pub fn disable_core_dumps() {
    let rl = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    unsafe {
        let _ = libc::setrlimit(libc::RLIMIT_CORE, &rl);
    }
}

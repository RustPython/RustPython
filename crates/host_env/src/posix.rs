use std::os::fd::BorrowedFd;

pub fn set_inheritable(fd: BorrowedFd<'_>, inheritable: bool) -> nix::Result<()> {
    use nix::fcntl;

    let flags = fcntl::FdFlag::from_bits_truncate(fcntl::fcntl(fd, fcntl::FcntlArg::F_GETFD)?);
    let mut new_flags = flags;
    new_flags.set(fcntl::FdFlag::FD_CLOEXEC, !inheritable);
    if flags != new_flags {
        fcntl::fcntl(fd, fcntl::FcntlArg::F_SETFD(new_flags))?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
#[must_use]
pub fn get_number_of_os_threads() -> isize {
    type MachPortT = libc::c_uint;
    type KernReturnT = libc::c_int;
    type MachMsgTypeNumberT = libc::c_uint;
    type ThreadActArrayT = *mut MachPortT;
    const KERN_SUCCESS: KernReturnT = 0;
    unsafe extern "C" {
        fn mach_task_self() -> MachPortT;
        fn task_for_pid(
            task: MachPortT,
            pid: libc::c_int,
            target_task: *mut MachPortT,
        ) -> KernReturnT;
        fn task_threads(
            target_task: MachPortT,
            act_list: *mut ThreadActArrayT,
            act_list_cnt: *mut MachMsgTypeNumberT,
        ) -> KernReturnT;
        fn vm_deallocate(
            target_task: MachPortT,
            address: libc::uintptr_t,
            size: libc::uintptr_t,
        ) -> KernReturnT;
    }

    let self_task = unsafe { mach_task_self() };
    let mut proc_task: MachPortT = 0;
    if unsafe { task_for_pid(self_task, libc::getpid(), &mut proc_task) } == KERN_SUCCESS {
        let mut threads: ThreadActArrayT = core::ptr::null_mut();
        let mut n_threads: MachMsgTypeNumberT = 0;
        if unsafe { task_threads(proc_task, &mut threads, &mut n_threads) } == KERN_SUCCESS {
            if !threads.is_null() {
                let _ = unsafe {
                    vm_deallocate(
                        self_task,
                        threads as libc::uintptr_t,
                        (n_threads as usize * core::mem::size_of::<MachPortT>()) as libc::uintptr_t,
                    )
                };
            }
            return n_threads as isize;
        }
    }
    0
}

#[cfg(target_os = "linux")]
#[must_use]
pub fn get_number_of_os_threads() -> isize {
    use std::io::Read as _;

    let mut file = match crate::fs::open("/proc/self/stat") {
        Ok(f) => f,
        Err(_) => return 0,
    };
    let mut buf = [0u8; 160];
    let n = match file.read(&mut buf) {
        Ok(n) => n,
        Err(_) => return 0,
    };
    let line = match core::str::from_utf8(&buf[..n]) {
        Ok(s) => s,
        Err(_) => return 0,
    };
    if let Some(field) = line.split_whitespace().nth(19) {
        return field.parse::<isize>().unwrap_or(0);
    }
    0
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
#[must_use]
pub fn get_number_of_os_threads() -> isize {
    0
}

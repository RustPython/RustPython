// spell-checker:disable

pub use msvcrt::*;

#[pymodule]
mod msvcrt {
    use crate::{
        PyRef, PyResult, VirtualMachine,
        builtins::{PyBytes, PyStrRef},
        common::{crt_fd, suppress_iph},
        convert::IntoPyException,
    };
    use itertools::Itertools;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::Diagnostics::Debug;

    #[pyattr]
    use windows_sys::Win32::System::Diagnostics::Debug::{
        SEM_FAILCRITICALERRORS, SEM_NOALIGNMENTFAULTEXCEPT, SEM_NOGPFAULTERRORBOX,
        SEM_NOOPENFILEERRORBOX,
    };

    pub fn setmode_binary(fd: crt_fd::Borrowed<'_>) {
        unsafe { suppress_iph!(_setmode(fd, libc::O_BINARY)) };
    }

    unsafe extern "C" {
        fn _getch() -> i32;
        fn _getwch() -> u32;
        fn _getche() -> i32;
        fn _getwche() -> u32;
        fn _putch(c: u32) -> i32;
        fn _putwch(c: u16) -> u32;
        fn _ungetch(c: i32) -> i32;
        fn _ungetwch(c: u32) -> u32;
        fn _locking(fd: i32, mode: i32, nbytes: i64) -> i32;
        fn _heapmin() -> i32;
        fn _kbhit() -> i32;
    }

    // Locking mode constants
    #[pyattr]
    const LK_UNLCK: i32 = 0; // Unlock
    #[pyattr]
    const LK_LOCK: i32 = 1; // Lock (blocking)
    #[pyattr]
    const LK_NBLCK: i32 = 2; // Non-blocking lock
    #[pyattr]
    const LK_RLCK: i32 = 3; // Lock for reading (same as LK_LOCK)
    #[pyattr]
    const LK_NBRLCK: i32 = 4; // Non-blocking lock for reading (same as LK_NBLCK)

    #[pyfunction]
    fn getch() -> Vec<u8> {
        let c = unsafe { _getch() };
        vec![c as u8]
    }
    #[pyfunction]
    fn getwch() -> String {
        let c = unsafe { _getwch() };
        char::from_u32(c).unwrap().to_string()
    }
    #[pyfunction]
    fn getche() -> Vec<u8> {
        let c = unsafe { _getche() };
        vec![c as u8]
    }
    #[pyfunction]
    fn getwche() -> String {
        let c = unsafe { _getwche() };
        char::from_u32(c).unwrap().to_string()
    }
    #[pyfunction]
    fn putch(b: PyRef<PyBytes>, vm: &VirtualMachine) -> PyResult<()> {
        let &c =
            b.as_bytes().iter().exactly_one().map_err(|_| {
                vm.new_type_error("putch() argument must be a byte string of length 1")
            })?;
        unsafe { suppress_iph!(_putch(c.into())) };
        Ok(())
    }
    #[pyfunction]
    fn putwch(s: PyStrRef, vm: &VirtualMachine) -> PyResult<()> {
        let c = s
            .as_str()
            .chars()
            .exactly_one()
            .map_err(|_| vm.new_type_error("putch() argument must be a string of length 1"))?;
        unsafe { suppress_iph!(_putwch(c as u16)) };
        Ok(())
    }

    #[pyfunction]
    fn ungetch(b: PyRef<PyBytes>, vm: &VirtualMachine) -> PyResult<()> {
        let &c = b.as_bytes().iter().exactly_one().map_err(|_| {
            vm.new_type_error("ungetch() argument must be a byte string of length 1")
        })?;
        let ret = unsafe { suppress_iph!(_ungetch(c as i32)) };
        if ret == -1 {
            // EOF returned means the buffer is full
            Err(vm.new_os_error(libc::ENOSPC))
        } else {
            Ok(())
        }
    }

    #[pyfunction]
    fn ungetwch(s: PyStrRef, vm: &VirtualMachine) -> PyResult<()> {
        let c =
            s.as_str().chars().exactly_one().map_err(|_| {
                vm.new_type_error("ungetwch() argument must be a string of length 1")
            })?;
        let ret = unsafe { suppress_iph!(_ungetwch(c as u32)) };
        if ret == 0xFFFF {
            // WEOF returned means the buffer is full
            Err(vm.new_os_error(libc::ENOSPC))
        } else {
            Ok(())
        }
    }

    #[pyfunction]
    fn kbhit() -> i32 {
        unsafe { _kbhit() }
    }

    #[pyfunction]
    fn locking(fd: i32, mode: i32, nbytes: i64, vm: &VirtualMachine) -> PyResult<()> {
        let ret = unsafe { suppress_iph!(_locking(fd, mode, nbytes)) };
        if ret == -1 {
            Err(vm.new_last_errno_error())
        } else {
            Ok(())
        }
    }

    #[pyfunction]
    fn heapmin(vm: &VirtualMachine) -> PyResult<()> {
        let ret = unsafe { suppress_iph!(_heapmin()) };
        if ret == -1 {
            Err(vm.new_last_errno_error())
        } else {
            Ok(())
        }
    }

    unsafe extern "C" {
        fn _setmode(fd: crt_fd::Borrowed<'_>, flags: i32) -> i32;
    }

    #[pyfunction]
    fn setmode(fd: crt_fd::Borrowed<'_>, flags: i32, vm: &VirtualMachine) -> PyResult<i32> {
        let flags = unsafe { suppress_iph!(_setmode(fd, flags)) };
        if flags == -1 {
            Err(vm.new_last_errno_error())
        } else {
            Ok(flags)
        }
    }

    #[pyfunction]
    fn open_osfhandle(handle: isize, flags: i32, vm: &VirtualMachine) -> PyResult<i32> {
        let ret = unsafe { suppress_iph!(libc::open_osfhandle(handle, flags)) };
        if ret == -1 {
            Err(vm.new_last_errno_error())
        } else {
            Ok(ret)
        }
    }

    #[pyfunction]
    fn get_osfhandle(fd: crt_fd::Borrowed<'_>, vm: &VirtualMachine) -> PyResult<isize> {
        crt_fd::as_handle(fd)
            .map(|h| h.as_raw_handle() as _)
            .map_err(|e| e.into_pyexception(vm))
    }

    #[allow(non_snake_case)]
    #[pyfunction]
    fn GetErrorMode() -> u32 {
        unsafe { suppress_iph!(Debug::GetErrorMode()) }
    }

    #[allow(non_snake_case)]
    #[pyfunction]
    fn SetErrorMode(mode: Debug::THREAD_ERROR_MODE, _: &VirtualMachine) -> u32 {
        unsafe { suppress_iph!(Debug::SetErrorMode(mode)) }
    }
}

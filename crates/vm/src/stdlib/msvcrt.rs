// spell-checker:disable

pub use msvcrt::*;

#[pymodule]
mod msvcrt {
    use crate::{
        PyRef, PyResult, VirtualMachine,
        builtins::{PyBytes, PyStrRef},
        common::{crt_fd, suppress_iph},
        convert::IntoPyException,
        stdlib::os::errno_err,
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
    }

    #[pyfunction]
    fn getch() -> Vec<u8> {
        let c = unsafe { _getch() };
        vec![c as u8]
    }
    #[pyfunction]
    fn getwch() -> String {
        let c = unsafe { _getwch() };
        std::char::from_u32(c).unwrap().to_string()
    }
    #[pyfunction]
    fn getche() -> Vec<u8> {
        let c = unsafe { _getche() };
        vec![c as u8]
    }
    #[pyfunction]
    fn getwche() -> String {
        let c = unsafe { _getwche() };
        std::char::from_u32(c).unwrap().to_string()
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

    unsafe extern "C" {
        fn _setmode(fd: crt_fd::Borrowed<'_>, flags: i32) -> i32;
    }

    #[pyfunction]
    fn setmode(fd: crt_fd::Borrowed<'_>, flags: i32, vm: &VirtualMachine) -> PyResult<i32> {
        let flags = unsafe { suppress_iph!(_setmode(fd, flags)) };
        if flags == -1 {
            Err(errno_err(vm))
        } else {
            Ok(flags)
        }
    }

    unsafe extern "C" {
        fn _open_osfhandle(osfhandle: isize, flags: i32) -> i32;
    }

    #[pyfunction]
    fn open_osfhandle(handle: isize, flags: i32, vm: &VirtualMachine) -> PyResult<i32> {
        let ret = unsafe { suppress_iph!(_open_osfhandle(handle, flags)) };
        if ret == -1 {
            Err(errno_err(vm))
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

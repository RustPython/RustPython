// spell-checker:disable

pub(crate) use msvcrt::*;

#[pymodule]
mod msvcrt {
    use crate::{
        PyRef, PyResult, VirtualMachine,
        builtins::{PyBytes, PyStrRef},
        convert::IntoPyException,
        host_env::crt_fd,
    };
    use itertools::Itertools;
    use rustpython_host_env::msvcrt as host_msvcrt;
    use std::os::windows::io::AsRawHandle;

    #[pyattr]
    use host_msvcrt::{
        SEM_FAILCRITICALERRORS, SEM_NOALIGNMENTFAULTEXCEPT, SEM_NOGPFAULTERRORBOX,
        SEM_NOOPENFILEERRORBOX,
    };

    pub(crate) fn setmode_binary(fd: crt_fd::Borrowed<'_>) {
        host_msvcrt::setmode_binary(fd);
    }

    // Locking mode constants
    #[pyattr]
    const LK_UNLCK: i32 = host_msvcrt::LK_UNLCK; // Unlock

    #[pyattr]
    const LK_LOCK: i32 = host_msvcrt::LK_LOCK; // Lock (blocking)

    #[pyattr]
    const LK_NBLCK: i32 = host_msvcrt::LK_NBLCK; // Non-blocking lock

    #[pyattr]
    const LK_RLCK: i32 = host_msvcrt::LK_RLCK; // Lock for reading (same as LK_LOCK)

    #[pyattr]
    const LK_NBRLCK: i32 = host_msvcrt::LK_NBRLCK; // Non-blocking lock for reading (same as LK_NBLCK)

    #[pyfunction]
    fn getch() -> Vec<u8> {
        host_msvcrt::getch()
    }

    #[pyfunction]
    fn getwch() -> String {
        host_msvcrt::getwch()
    }

    #[pyfunction]
    fn getche() -> Vec<u8> {
        host_msvcrt::getche()
    }

    #[pyfunction]
    fn getwche() -> String {
        host_msvcrt::getwche()
    }

    #[pyfunction]
    fn putch(b: PyRef<PyBytes>, vm: &VirtualMachine) -> PyResult<()> {
        let &c =
            b.as_bytes().iter().exactly_one().map_err(|_| {
                vm.new_type_error("putch() argument must be a byte string of length 1")
            })?;
        host_msvcrt::putch(c);
        Ok(())
    }

    #[pyfunction]
    fn putwch(s: PyStrRef, vm: &VirtualMachine) -> PyResult<()> {
        let c = s
            .expect_str()
            .chars()
            .exactly_one()
            .map_err(|_| vm.new_type_error("putch() argument must be a string of length 1"))?;
        host_msvcrt::putwch(c);
        Ok(())
    }

    #[pyfunction]
    fn ungetch(b: PyRef<PyBytes>, vm: &VirtualMachine) -> PyResult<()> {
        let &c = b.as_bytes().iter().exactly_one().map_err(|_| {
            vm.new_type_error("ungetch() argument must be a byte string of length 1")
        })?;
        host_msvcrt::ungetch(c).map_err(|e| e.into_pyexception(vm))
    }

    #[pyfunction]
    fn ungetwch(s: PyStrRef, vm: &VirtualMachine) -> PyResult<()> {
        let c =
            s.expect_str().chars().exactly_one().map_err(|_| {
                vm.new_type_error("ungetwch() argument must be a string of length 1")
            })?;
        host_msvcrt::ungetwch(c).map_err(|e| e.into_pyexception(vm))
    }

    #[pyfunction]
    fn kbhit() -> i32 {
        host_msvcrt::kbhit()
    }

    #[pyfunction]
    fn locking(fd: i32, mode: i32, nbytes: i64, vm: &VirtualMachine) -> PyResult<()> {
        host_msvcrt::locking(fd, mode, nbytes).map_err(|e| e.into_pyexception(vm))
    }

    #[pyfunction]
    fn heapmin(vm: &VirtualMachine) -> PyResult<()> {
        host_msvcrt::heapmin().map_err(|e| e.into_pyexception(vm))
    }

    #[pyfunction]
    fn setmode(fd: crt_fd::Borrowed<'_>, flags: i32, vm: &VirtualMachine) -> PyResult<i32> {
        host_msvcrt::setmode(fd, flags).map_err(|e| e.into_pyexception(vm))
    }

    #[pyfunction]
    fn open_osfhandle(handle: isize, flags: i32, vm: &VirtualMachine) -> PyResult<i32> {
        host_msvcrt::open_osfhandle(handle, flags).map_err(|e| e.into_pyexception(vm))
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
        host_msvcrt::get_error_mode()
    }

    #[allow(non_snake_case)]
    #[pyfunction]
    fn SetErrorMode(mode: host_msvcrt::ErrorMode, _: &VirtualMachine) -> u32 {
        host_msvcrt::set_error_mode(mode)
    }
}

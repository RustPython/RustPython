pub(crate) use _multiprocessing::make_module;

#[cfg(windows)]
#[pymodule]
mod _multiprocessing {
    use crate::vm::{PyResult, VirtualMachine, function::ArgBytesLike, stdlib::os};
    use windows_sys::Win32::Networking::WinSock::{self, SOCKET};

    #[pyfunction]
    fn closesocket(socket: usize, vm: &VirtualMachine) -> PyResult<()> {
        let res = unsafe { WinSock::closesocket(socket as SOCKET) };
        if res == 0 {
            Err(os::errno_err(vm))
        } else {
            Ok(())
        }
    }

    #[pyfunction]
    fn recv(socket: usize, size: usize, vm: &VirtualMachine) -> PyResult<libc::c_int> {
        let mut buf = vec![0; size];
        let n_read =
            unsafe { WinSock::recv(socket as SOCKET, buf.as_mut_ptr() as *mut _, size as i32, 0) };
        if n_read < 0 {
            Err(os::errno_err(vm))
        } else {
            Ok(n_read)
        }
    }

    #[pyfunction]
    fn send(socket: usize, buf: ArgBytesLike, vm: &VirtualMachine) -> PyResult<libc::c_int> {
        let ret = buf.with_ref(|b| unsafe {
            WinSock::send(socket as SOCKET, b.as_ptr() as *const _, b.len() as i32, 0)
        });
        if ret < 0 {
            Err(os::errno_err(vm))
        } else {
            Ok(ret)
        }
    }
}

#[cfg(not(windows))]
#[pymodule]
mod _multiprocessing {}

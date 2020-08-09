pub(crate) use _multiprocessing::make_module;

#[cfg(windows)]
#[pymodule]
mod _multiprocessing {
    use super::super::os;
    use crate::byteslike::PyBytesLike;
    use crate::pyobject::PyResult;
    use crate::VirtualMachine;
    use winapi::um::winsock2::{self, SOCKET};

    #[pyfunction]
    fn closesocket(socket: usize, vm: &VirtualMachine) -> PyResult<()> {
        let res = unsafe { winsock2::closesocket(socket as SOCKET) };
        if res == 0 {
            Err(os::errno_err(vm))
        } else {
            Ok(())
        }
    }

    #[pyfunction]
    fn recv(socket: usize, size: usize, vm: &VirtualMachine) -> PyResult<libc::c_int> {
        let mut buf = vec![0 as libc::c_char; size];
        let nread =
            unsafe { winsock2::recv(socket as SOCKET, buf.as_mut_ptr() as *mut _, size as i32, 0) };
        if nread < 0 {
            Err(os::errno_err(vm))
        } else {
            Ok(nread)
        }
    }

    #[pyfunction]
    fn send(socket: usize, buf: PyBytesLike, vm: &VirtualMachine) -> PyResult<libc::c_int> {
        let ret = buf.with_ref(|b| unsafe {
            winsock2::send(socket as SOCKET, b.as_ptr() as *const _, b.len() as i32, 0)
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

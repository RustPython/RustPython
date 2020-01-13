#[allow(unused_imports)]
use crate::obj::objbyteinner::PyBytesLike;
#[allow(unused_imports)]
use crate::pyobject::{PyObjectRef, PyResult};
use crate::VirtualMachine;

#[cfg(windows)]
use winapi::um::winsock2::{self, SOCKET};

#[cfg(windows)]
fn multiprocessing_closesocket(socket: usize, vm: &VirtualMachine) -> PyResult<()> {
    let res = unsafe { winsock2::closesocket(socket as SOCKET) };
    if res == 0 {
        Err(super::os::errno_err(vm))
    } else {
        Ok(())
    }
}

#[cfg(windows)]
fn multiprocessing_recv(socket: usize, size: usize, vm: &VirtualMachine) -> PyResult<libc::c_int> {
    let mut buf = vec![0 as libc::c_char; size];
    let nread =
        unsafe { winsock2::recv(socket as SOCKET, buf.as_mut_ptr() as *mut _, size as i32, 0) };
    if nread < 0 {
        Err(super::os::errno_err(vm))
    } else {
        Ok(nread)
    }
}

#[cfg(windows)]
fn multiprocessing_send(
    socket: usize,
    buf: PyBytesLike,
    vm: &VirtualMachine,
) -> PyResult<libc::c_int> {
    let ret = buf.with_ref(|b| unsafe {
        winsock2::send(socket as SOCKET, b.as_ptr() as *const _, b.len() as i32, 0)
    });
    if ret < 0 {
        Err(super::os::errno_err(vm))
    } else {
        Ok(ret)
    }
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let module = py_module!(vm, "_multiprocessing", {});
    extend_module_platform_specific(vm, &module);
    module
}

#[cfg(windows)]
fn extend_module_platform_specific(vm: &VirtualMachine, module: &PyObjectRef) {
    let ctx = &vm.ctx;
    extend_module!(vm, module, {
        "closesocket" => ctx.new_function(multiprocessing_closesocket),
        "recv" => ctx.new_function(multiprocessing_recv),
        "send" => ctx.new_function(multiprocessing_send),
    })
}

#[cfg(not(windows))]
fn extend_module_platform_specific(_vm: &VirtualMachine, _module: &PyObjectRef) {}

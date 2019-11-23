#![allow(non_snake_case)]

use std::io;
use winapi::shared::winerror;
use winapi::um::winnt::HANDLE;
use winapi::um::{handleapi, winbase};

use super::os::convert_io_error;
use crate::pyobject::{PyObjectRef, PyResult};
use crate::VirtualMachine;

fn winapi_CloseHandle(handle: usize, vm: &VirtualMachine) -> PyResult<()> {
    let res = unsafe { handleapi::CloseHandle(handle as HANDLE) };
    if res == 0 {
        Err(convert_io_error(vm, io::Error::last_os_error()))
    } else {
        Ok(())
    }
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;
    py_module!(vm, "_winapi", {
        "CloseHandle" => ctx.new_rustfunc(winapi_CloseHandle),
        "WAIT_OBJECT_0" => ctx.new_int(winbase::WAIT_OBJECT_0),
        "WAIT_ABANDONED" => ctx.new_int(winbase::WAIT_ABANDONED),
        "WAIT_ABANDONED_0" => ctx.new_int(winbase::WAIT_ABANDONED_0),
        "WAIT_TIMEOUT" => ctx.new_int(winerror::WAIT_TIMEOUT),
        "INFINITE" => ctx.new_int(winbase::INFINITE),
    })
}

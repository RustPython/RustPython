use super::os::errno_err;
use crate::builtins::bytes::PyBytesRef;
use crate::builtins::pystr::PyStrRef;
use crate::pyobject::{BorrowValue, PyObjectRef, PyResult};
use crate::VirtualMachine;

use itertools::Itertools;
use winapi::shared::minwindef::UINT;
use winapi::um::errhandlingapi::SetErrorMode;
use winapi::um::handleapi::INVALID_HANDLE_VALUE;
use winapi::um::winnt::HANDLE;

pub fn setmode_binary(fd: i32) {
    unsafe { suppress_iph!(_setmode(fd, libc::O_BINARY)) };
}

pub fn get_errno() -> i32 {
    let mut e = 0;
    unsafe { suppress_iph!(_get_errno(&mut e)) };
    e
}

extern "C" {
    fn _get_errno(pValue: *mut i32) -> i32;
}

extern "C" {
    fn _getch() -> i32;
    fn _getwch() -> u32;
    fn _getche() -> i32;
    fn _getwche() -> u32;
    fn _putch(c: u32) -> i32;
    fn _putwch(c: u16) -> u32;
}

fn msvcrt_getch() -> Vec<u8> {
    let c = unsafe { _getch() };
    vec![c as u8]
}
fn msvcrt_getwch() -> String {
    let c = unsafe { _getwch() };
    std::char::from_u32(c).unwrap().to_string()
}
fn msvcrt_getche() -> Vec<u8> {
    let c = unsafe { _getche() };
    vec![c as u8]
}
fn msvcrt_getwche() -> String {
    let c = unsafe { _getwche() };
    std::char::from_u32(c).unwrap().to_string()
}
fn msvcrt_putch(b: PyBytesRef, vm: &VirtualMachine) -> PyResult<()> {
    let &c = b.borrow_value().iter().exactly_one().map_err(|_| {
        vm.new_type_error("putch() argument must be a byte string of length 1".to_owned())
    })?;
    unsafe { suppress_iph!(_putch(c.into())) };
    Ok(())
}
fn msvcrt_putwch(s: PyStrRef, vm: &VirtualMachine) -> PyResult<()> {
    let c = s.borrow_value().chars().exactly_one().map_err(|_| {
        vm.new_type_error("putch() argument must be a string of length 1".to_owned())
    })?;
    unsafe { suppress_iph!(_putwch(c as u16)) };
    Ok(())
}

extern "C" {
    fn _setmode(fd: i32, flags: i32) -> i32;
}

fn msvcrt_setmode(fd: i32, flags: i32, vm: &VirtualMachine) -> PyResult<i32> {
    let flags = unsafe { suppress_iph!(_setmode(fd, flags)) };
    if flags == -1 {
        Err(errno_err(vm))
    } else {
        Ok(flags)
    }
}

extern "C" {
    fn _open_osfhandle(osfhandle: isize, flags: i32) -> i32;
    fn _get_osfhandle(fd: i32) -> libc::intptr_t;
}

fn msvcrt_open_osfhandle(handle: isize, flags: i32, vm: &VirtualMachine) -> PyResult<i32> {
    let ret = unsafe { suppress_iph!(_open_osfhandle(handle, flags)) };
    if ret == -1 {
        Err(errno_err(vm))
    } else {
        Ok(ret)
    }
}

fn msvcrt_get_osfhandle(fd: i32, vm: &VirtualMachine) -> PyResult<isize> {
    let ret = unsafe { suppress_iph!(_get_osfhandle(fd)) };
    if ret as HANDLE == INVALID_HANDLE_VALUE {
        Err(errno_err(vm))
    } else {
        Ok(ret)
    }
}

fn msvcrt_seterrormode(mode: UINT, _: &VirtualMachine) -> UINT {
    unsafe { suppress_iph!(SetErrorMode(mode)) }
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    use winapi::um::winbase::{
        SEM_FAILCRITICALERRORS, SEM_NOALIGNMENTFAULTEXCEPT, SEM_NOGPFAULTERRORBOX,
        SEM_NOOPENFILEERRORBOX,
    };

    let ctx = &vm.ctx;
    py_module!(vm, "msvcrt", {
        "getch" => named_function!(ctx, msvcrt, getch),
        "getwch" => named_function!(ctx, msvcrt, getwch),
        "getche" => named_function!(ctx, msvcrt, getche),
        "getwche" => named_function!(ctx, msvcrt, getwche),
        "putch" => named_function!(ctx, msvcrt, putch),
        "putwch" => named_function!(ctx, msvcrt, putwch),
        "setmode" => named_function!(ctx, msvcrt, setmode),
        "open_osfhandle" => named_function!(ctx, msvcrt, open_osfhandle),
        "get_osfhandle" => named_function!(ctx, msvcrt, get_osfhandle),
        "SetErrorMode" => named_function!(ctx, msvcrt, seterrormode),
        "SEM_FAILCRITICALERRORS" => ctx.new_int(SEM_FAILCRITICALERRORS),
        "SEM_NOALIGNMENTFAULTEXCEPT" => ctx.new_int(SEM_NOALIGNMENTFAULTEXCEPT),
        "SEM_NOGPFAULTERRORBOX" => ctx.new_int(SEM_NOGPFAULTERRORBOX),
        "SEM_NOOPENFILEERRORBOX" => ctx.new_int(SEM_NOOPENFILEERRORBOX),
    })
}

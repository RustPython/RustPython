use std::io;
use windows_sys::Win32::{
    Foundation::HANDLE,
    System::Console::{INPUT_RECORD, KEY_EVENT, WriteConsoleInputW},
};

use crate::windows::CheckWin32Bool;

fn console_handle(fd: i32) -> io::Result<HANDLE> {
    let raw_handle = unsafe { crate::suppress_iph!(libc::get_osfhandle(fd)) };
    if raw_handle == -1 {
        return Err(crate::os::errno_io_error());
    }
    Ok(raw_handle as HANDLE)
}

fn input_records(data: &[u16]) -> Vec<INPUT_RECORD> {
    let mut records: Vec<INPUT_RECORD> = Vec::with_capacity(data.len());
    for &wc in data {
        let mut rec: INPUT_RECORD = unsafe { core::mem::zeroed() };
        rec.EventType = KEY_EVENT as u16;
        rec.Event.KeyEvent.bKeyDown = 1;
        rec.Event.KeyEvent.wRepeatCount = 1;
        rec.Event.KeyEvent.uChar.UnicodeChar = wc;
        records.push(rec);
    }
    records
}

fn write_records(handle: HANDLE, records: &[INPUT_RECORD]) -> io::Result<()> {
    let size = records.len() as u32;
    let mut total: u32 = 0;
    while total < size {
        let mut wrote: u32 = 0;
        unsafe {
            WriteConsoleInputW(
                handle,
                records[total as usize..].as_ptr(),
                size - total,
                &mut wrote,
            )
        }
        .check_win32_bool()?;
        if wrote == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "WriteConsoleInputW made no progress",
            ));
        }
        total += wrote;
    }
    Ok(())
}

pub fn write_console_input(fd: i32, data: &[u16]) -> io::Result<()> {
    let handle = console_handle(fd)?;
    let records = input_records(data);
    write_records(handle, &records)
}

/// Resolve the descriptor after preparing the input records.
#[doc(hidden)]
pub fn write_console_input_with_fd(data: &[u16], get_fd: impl FnOnce() -> i32) -> io::Result<()> {
    let records = input_records(data);
    let handle = console_handle(get_fd())?;
    write_records(handle, &records)
}

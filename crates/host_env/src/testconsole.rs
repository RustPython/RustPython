use std::io;
use windows_sys::Win32::{
    Foundation::{HANDLE, INVALID_HANDLE_VALUE},
    System::Console::{INPUT_RECORD, KEY_EVENT, WriteConsoleInputW},
};

pub fn write_console_input(fd: i32, data: &[u16]) -> io::Result<()> {
    let handle = unsafe { libc::get_osfhandle(fd) } as HANDLE;
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }

    let size = data.len() as u32;
    let mut records: Vec<INPUT_RECORD> = Vec::with_capacity(data.len());
    for &wc in data {
        let mut rec: INPUT_RECORD = unsafe { core::mem::zeroed() };
        rec.EventType = KEY_EVENT as u16;
        rec.Event.KeyEvent.bKeyDown = 1;
        rec.Event.KeyEvent.wRepeatCount = 1;
        rec.Event.KeyEvent.uChar.UnicodeChar = wc;
        records.push(rec);
    }

    let mut total: u32 = 0;
    while total < size {
        let mut wrote: u32 = 0;
        let res = unsafe {
            WriteConsoleInputW(
                handle,
                records[total as usize..].as_ptr(),
                size - total,
                &mut wrote,
            )
        };
        if res == 0 {
            return Err(io::Error::last_os_error());
        }
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

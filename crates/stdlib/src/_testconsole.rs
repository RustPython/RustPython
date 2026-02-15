pub(crate) use _testconsole::module_def;

#[pymodule]
mod _testconsole {
    use crate::vm::{
        PyObjectRef, PyResult, VirtualMachine, convert::IntoPyException, function::ArgBytesLike,
    };
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;

    type Handle = windows_sys::Win32::Foundation::HANDLE;

    #[pyfunction]
    fn write_input(file: PyObjectRef, s: ArgBytesLike, vm: &VirtualMachine) -> PyResult<()> {
        use windows_sys::Win32::System::Console::{INPUT_RECORD, KEY_EVENT, WriteConsoleInputW};

        // Get the fd from the file object via fileno()
        let fd_obj = vm.call_method(&file, "fileno", ())?;
        let fd: i32 = fd_obj.try_into_value(vm)?;

        let handle = unsafe { libc::get_osfhandle(fd) } as Handle;
        if handle == INVALID_HANDLE_VALUE {
            return Err(std::io::Error::last_os_error().into_pyexception(vm));
        }

        let data = s.borrow_buf();
        let data = &*data;

        // Interpret as UTF-16-LE pairs
        if !data.len().is_multiple_of(2) {
            return Err(vm.new_value_error("buffer must contain UTF-16-LE data (even length)"));
        }
        let wchars: Vec<u16> = data
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();

        let size = wchars.len() as u32;

        // Create INPUT_RECORD array
        let mut records: Vec<INPUT_RECORD> = Vec::with_capacity(wchars.len());
        for &wc in &wchars {
            // SAFETY: zeroing and accessing the union field for KEY_EVENT
            let mut rec: INPUT_RECORD = unsafe { core::mem::zeroed() };
            rec.EventType = KEY_EVENT as u16;
            rec.Event.KeyEvent.bKeyDown = 1; // TRUE
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
                return Err(std::io::Error::last_os_error().into_pyexception(vm));
            }
            total += wrote;
        }

        Ok(())
    }

    #[pyfunction]
    fn read_output(_file: PyObjectRef) -> Option<()> {
        // Stub, same as CPython
        None
    }
}

pub(crate) use _testconsole::module_def;

#[pymodule]
mod _testconsole {
    use crate::vm::{
        PyObjectRef, PyResult, TryFromObject, VirtualMachine, convert::IntoPyException,
        function::ArgBytesLike,
    };
    use rustpython_host_env::testconsole as host_testconsole;

    #[derive(FromArgs)]
    struct WriteInputArgs {
        #[pyarg(any)]
        file: PyObjectRef,
        #[pyarg(any)]
        s: PyObjectRef,
    }

    #[pyfunction]
    fn write_input(args: WriteInputArgs, vm: &VirtualMachine) -> PyResult<()> {
        let s = ArgBytesLike::try_from_object(vm, args.s)?;
        // Get the fd from the file object via fileno()
        let fd_obj = vm.call_method(&args.file, "fileno", ())?;
        let fd: i32 = fd_obj.try_into_value(vm)?;

        // CPython interprets complete UTF-16-LE pairs and ignores a final byte.
        // Release the buffer borrow before the OS call and error conversion.
        let wchars: Vec<u16> = {
            let data = s.borrow_buf();
            let (chunks, _) = data.as_chunks::<2>();
            // CPython's Windows implementation casts the byte length to DWORD first.
            let count = (data.len() as u32 / 2) as usize;
            chunks[..count]
                .iter()
                .map(|chunk| u16::from_le_bytes(*chunk))
                .collect()
        };
        host_testconsole::write_console_input(fd, &wchars).map_err(|e| e.into_pyexception(vm))
    }

    #[derive(FromArgs)]
    struct ReadOutputArgs {
        #[pyarg(any)]
        file: PyObjectRef,
    }

    #[pyfunction]
    fn read_output(args: ReadOutputArgs) -> Option<()> {
        let _file = args.file;
        // Stub, same as CPython
        None
    }
}

pub(crate) use _testconsole::module_def;

#[pymodule]
mod _testconsole {
    use crate::vm::{
        AsObject, PyObjectRef, PyResult, TryFromObject, VirtualMachine, builtins::PyTypeRef,
        convert::IntoPyException, function::ArgBytesLike,
    };
    use rustpython_host_env::testconsole as host_testconsole;

    #[derive(FromArgs)]
    struct WriteInputArgs {
        #[pyarg(any)]
        file: PyObjectRef,
        #[pyarg(any)]
        s: ArgBytesLike,
    }

    #[pyfunction]
    fn write_input(args: WriteInputArgs, vm: &VirtualMachine) -> PyResult<()> {
        let s = args.s;
        let console_type = {
            let io = vm.import("_io", 0)?;
            PyTypeRef::try_from_object(vm, io.get_attr("_WindowsConsoleIO", vm)?)?
        };
        let is_console = args.file.fast_isinstance(&console_type);
        drop(console_type);
        if !is_console || crate::vm::stdlib::_io::raw_windows_console_fd(&args.file).is_none() {
            return Err(vm.new_type_error("expected raw console object"));
        }
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
        host_testconsole::write_console_input_with_fd(&wchars, || {
            crate::vm::stdlib::_io::raw_windows_console_fd(&args.file).unwrap_or(-1)
        })
        .map_err(|e| e.into_pyexception(vm))
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

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

        let data = s.borrow_buf();
        let data = &*data;

        // Interpret as UTF-16-LE pairs
        let (chunks, []) = data.as_chunks::<2>() else {
            return Err(vm.new_value_error("buffer must contain UTF-16-LE data (even length)"));
        };
        let wchars: Vec<u16> = chunks
            .iter()
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();
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

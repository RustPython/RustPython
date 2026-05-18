pub(crate) use _testconsole::module_def;

#[pymodule]
mod _testconsole {
    use crate::vm::{
        PyObjectRef, PyResult, VirtualMachine, convert::IntoPyException, function::ArgBytesLike,
    };
    use rustpython_host_env::testconsole as host_testconsole;

    #[pyfunction]
    fn write_input(file: PyObjectRef, s: ArgBytesLike, vm: &VirtualMachine) -> PyResult<()> {
        // Get the fd from the file object via fileno()
        let fd_obj = vm.call_method(&file, "fileno", ())?;
        let fd: i32 = fd_obj.try_into_value(vm)?;

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
        host_testconsole::write_console_input(fd, &wchars).map_err(|e| e.into_pyexception(vm))
    }

    #[pyfunction]
    fn read_output(_file: PyObjectRef) -> Option<()> {
        // Stub, same as CPython
        None
    }
}

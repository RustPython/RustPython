pub(crate) use decl::make_module;

#[pymodule(name = "marshal")]
mod decl {
    use crate::bytecode;
    use crate::obj::objbytes::{PyBytes, PyBytesRef};
    use crate::obj::objcode::{PyCode, PyCodeRef};
    use crate::obj::objmemory::try_buffer_from_object;
    use crate::pyobject::{IntoPyObject, PyObjectRef, PyResult, TryFromObject};
    use crate::vm::VirtualMachine;

    #[pyfunction]
    fn dumps(co: PyCodeRef) -> PyBytes {
        PyBytes::from(co.code.to_bytes())
    }

    #[pyfunction]
    fn dump(co: PyCodeRef, f: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        vm.call_method(&f, "write", vec![dumps(co).into_pyobject(vm)])?;
        Ok(())
    }

    #[pyfunction]
    fn loads(code_bytes: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyCode> {
        let buffer = try_buffer_from_object(code_bytes, vm)?;
        let bytes = buffer
            .as_contiguous()
            .ok_or_else(|| vm.new_value_error("buffer is not contiguous".to_owned()))?;
        let code = bytecode::CodeObject::from_bytes(&*bytes)
            .map_err(|_| vm.new_value_error("Couldn't deserialize python bytecode".to_owned()))?;
        Ok(PyCode { code })
    }

    #[pyfunction]
    fn load(f: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyCode> {
        let read_res = vm.call_method(&f, "read", vec![])?;
        // FIXME:
        let bytes = PyBytesRef::try_from_object(vm, read_res)?;
        loads(bytes.into_object(), vm)
    }
}

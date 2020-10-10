pub(crate) use decl::make_module;

#[pymodule(name = "marshal")]
mod decl {
    use crate::bytecode;
    use crate::obj::objbytes::{PyBytes, PyBytesRef};
    use crate::obj::objcode::{PyCode, PyCodeRef};
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
    fn loads(code_bytes: PyBytesRef, vm: &VirtualMachine) -> PyResult<PyCode> {
        let code = bytecode::CodeObject::from_bytes(&code_bytes)
            .map_err(|_| vm.new_value_error("Couldn't deserialize python bytecode".to_owned()))?;
        Ok(PyCode { code })
    }

    #[pyfunction]
    fn load(f: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyCode> {
        let read_res = vm.call_method(&f, "read", vec![])?;
        let bytes = PyBytesRef::try_from_object(vm, read_res)?;
        loads(bytes, vm)
    }
}

pub(crate) use decl::make_module;

#[pymodule(name = "marshal")]
mod decl {
    use crate::bytecode;
    use crate::obj::objbytes::{PyBytes, PyBytesRef};
    use crate::obj::objcode::{PyCode, PyCodeRef};
    use crate::pyobject::PyResult;
    use crate::vm::VirtualMachine;

    #[pyfunction]
    fn dumps(co: PyCodeRef) -> PyBytes {
        PyBytes::from(co.code.to_bytes())
    }

    #[pyfunction]
    fn loads(code_bytes: PyBytesRef, vm: &VirtualMachine) -> PyResult<PyCode> {
        let code = bytecode::CodeObject::from_bytes(&code_bytes)
            .map_err(|_| vm.new_value_error("Couldn't deserialize python bytecode".to_owned()))?;
        Ok(PyCode { code })
    }
}

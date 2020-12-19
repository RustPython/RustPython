pub(crate) use decl::make_module;

#[pymodule(name = "marshal")]
mod decl {
    use crate::builtins::bytes::PyBytes;
    use crate::builtins::code::{PyCode, PyCodeRef};
    use crate::bytecode;
    use crate::byteslike::PyBytesLike;
    use crate::common::borrow::BorrowValue;
    use crate::pyobject::{PyObjectRef, PyResult, TryFromObject};
    use crate::vm::VirtualMachine;

    #[pyfunction]
    fn dumps(co: PyCodeRef) -> PyBytes {
        PyBytes::from(co.code.map_clone_bag(&bytecode::BasicBag).to_bytes())
    }

    #[pyfunction]
    fn dump(co: PyCodeRef, f: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        vm.call_method(&f, "write", (dumps(co),))?;
        Ok(())
    }

    #[pyfunction]
    fn loads(code_bytes: PyBytesLike, vm: &VirtualMachine) -> PyResult<PyCode> {
        let code =
            bytecode::CodeObject::from_bytes(&*code_bytes.borrow_value()).map_err(|e| match e {
                bytecode::CodeDeserializeError::Eof => vm.new_exception_msg(
                    vm.ctx.exceptions.eof_error.clone(),
                    "end of file while deserializing bytecode".to_owned(),
                ),
                _ => vm.new_value_error("Couldn't deserialize python bytecode".to_owned()),
            })?;
        Ok(PyCode {
            code: vm.map_codeobj(code),
        })
    }

    #[pyfunction]
    fn load(f: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyCode> {
        let read_res = vm.call_method(&f, "read", ())?;
        let bytes = PyBytesLike::try_from_object(vm, read_res)?;
        loads(bytes, vm)
    }
}

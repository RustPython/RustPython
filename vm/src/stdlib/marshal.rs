pub(crate) use decl::make_module;

#[pymodule(name = "marshal")]
mod decl {
    use crate::{
        builtins::{PyBytes, PyCode},
        bytecode,
        function::ArgBytesLike,
        PyObjectRef, PyRef, PyResult, TryFromObject, VirtualMachine,
    };

    #[pyfunction]
    fn dumps(co: PyRef<PyCode>) -> PyBytes {
        PyBytes::from(co.code.map_clone_bag(&bytecode::BasicBag).to_bytes())
    }

    #[pyfunction]
    fn dump(co: PyRef<PyCode>, f: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        vm.call_method(&f, "write", (dumps(co),))?;
        Ok(())
    }

    #[pyfunction]
    fn loads(code_bytes: ArgBytesLike, vm: &VirtualMachine) -> PyResult<PyCode> {
        let buf = &*code_bytes.borrow_buf();
        let code = bytecode::CodeObject::from_bytes(buf).map_err(|e| match e {
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
        let bytes = ArgBytesLike::try_from_object(vm, read_res)?;
        loads(bytes, vm)
    }
}

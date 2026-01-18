use super::*;
use rustpython_codegen::compile::ruff_int_to_bigint;
use rustpython_compiler_core::SourceFile;

impl Node for ast::Identifier {
    fn ast_to_object(self, vm: &VirtualMachine, _source_file: &SourceFile) -> PyObjectRef {
        let id = self.as_str();
        vm.ctx.new_str(id).into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        _source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let py_str = PyStrRef::try_from_object(vm, object)?;
        Ok(Self::new(py_str.as_str(), TextRange::default()))
    }
}

impl Node for ast::Int {
    fn ast_to_object(self, vm: &VirtualMachine, _source_file: &SourceFile) -> PyObjectRef {
        vm.ctx.new_int(ruff_int_to_bigint(&self).unwrap()).into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        _source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        // FIXME: performance
        let value: PyIntRef = object.try_into_value(vm)?;
        let value = value.as_bigint().to_string();
        Ok(value.parse().unwrap())
    }
}

impl Node for bool {
    fn ast_to_object(self, vm: &VirtualMachine, _source_file: &SourceFile) -> PyObjectRef {
        vm.ctx.new_int(self as u8).into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        _source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        i32::try_from_object(vm, object).map(|i| i != 0)
    }
}

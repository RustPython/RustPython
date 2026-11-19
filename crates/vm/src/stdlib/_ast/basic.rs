use super::AstSourceContext;
use super::*;

impl Node for ast::Identifier {
    fn ast_to_object(self, vm: &VirtualMachine, _source_file: &AstSourceContext) -> PyObjectRef {
        let id = self.as_str();
        vm.ctx.intern_str(id).to_object()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        _source_file: &AstSourceContext,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        if !object.class().is(vm.ctx.types.str_type) {
            return Err(vm.new_type_error("AST identifier must be of type str"));
        }
        let py_str = PyUtf8StrRef::try_from_object(vm, object)
            .map_err(|_| vm.new_type_error("AST identifier must be of type str"))?;
        Ok(Self::new(py_str.as_str(), TextRange::default()))
    }
}

impl Node for bool {
    fn ast_to_object(self, vm: &VirtualMachine, _source_file: &AstSourceContext) -> PyObjectRef {
        vm.ctx.new_int(self as u8).into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        _source_file: &AstSourceContext,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        node_object_to_i32(vm, &object).map(|i| i != 0)
    }
}

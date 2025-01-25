use super::*;

impl Node for ruff::ElifElseClause {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        todo!()
    }

    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        todo!()
    }
}

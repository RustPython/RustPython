use super::*;

impl Node for ruff::ElifElseClause {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        todo!()
    }

    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        todo!()
    }
}

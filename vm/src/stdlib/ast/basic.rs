use super::*;

impl Node for ruff::Identifier {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let id = self.as_str();
        vm.ctx.new_str(id).into()
    }

    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        let py_str = PyStrRef::try_from_object(vm, object)?;
        Ok(ruff::Identifier::new(py_str.as_str(), TextRange::default()))
    }
}

impl Node for ruff::Int {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        if let Some(int) = self.as_i32() {
            vm.ctx.new_int(int)
        } else if let Some(int) = self.as_u32() {
            vm.ctx.new_int(int)
        } else if let Some(int) = self.as_i64() {
            vm.ctx.new_int(int)
        } else if let Some(int) = self.as_u64() {
            vm.ctx.new_int(int)
        } else {
            // FIXME: performance
            let int = self.to_string().parse().unwrap();
            vm.ctx.new_bigint(&int)
        }
        .into()
    }

    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        // FIXME: performance
        let value: PyIntRef = object.try_into_value(vm)?;
        let value = value.as_bigint().to_string();
        Ok(value.parse().unwrap())
    }
}

impl Node for bool {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_int(self as u8).into()
    }

    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        i32::try_from_object(vm, object).map(|i| i != 0)
    }
}

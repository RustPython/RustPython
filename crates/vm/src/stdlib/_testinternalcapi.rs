pub(crate) use _testinternalcapi::module_def;

#[pymodule]
mod _testinternalcapi {
    use crate::{PyObjectRef, VirtualMachine};

    #[pyfunction]
    fn has_inline_values(obj: PyObjectRef, _vm: &VirtualMachine) -> bool {
        obj.has_inline_values()
    }
}

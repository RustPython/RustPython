pub(crate) use sysconfig::make_module;

#[pymodule(name = "_sysconfig")]
pub(crate) mod sysconfig {
    use crate::{VirtualMachine, builtins::PyDictRef, convert::ToPyObject};

    #[pyfunction]
    fn config_vars(vm: &VirtualMachine) -> PyDictRef {
        let vars = vm.ctx.new_dict();
        vars.set_item("Py_GIL_DISABLED", true.to_pyobject(vm), vm)
            .unwrap();
        vars
    }
}

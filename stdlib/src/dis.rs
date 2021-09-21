pub(crate) use decl::make_module;

#[pymodule(name = "dis")]
mod decl {
    use crate::vm::{
        builtins::{PyCode, PyDictRef, PyStrRef},
        bytecode::CodeFlags,
        compile, ItemProtocol, PyObjectRef, PyRef, PyResult, TryFromObject, VirtualMachine,
    };

    #[pyfunction]
    fn dis(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let co = if let Ok(co) = vm.get_attribute(obj.clone(), "__code__") {
            // Method or function:
            co
        } else if let Ok(co_str) = PyStrRef::try_from_object(vm, obj.clone()) {
            // String:
            vm.compile(co_str.as_str(), compile::Mode::Exec, "<dis>".to_owned())
                .map_err(|err| vm.new_syntax_error(&err))?
                .into_object()
        } else {
            obj
        };
        disassemble(co, vm)
    }

    #[pyfunction]
    fn disassemble(co: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let code = PyRef::<PyCode>::try_from_object(vm, co)?;
        print!("{}", &code.code);
        Ok(())
    }

    #[pyattr(name = "COMPILER_FLAG_NAMES")]
    fn compiler_flag_names(vm: &VirtualMachine) -> PyDictRef {
        let dict = vm.ctx.new_dict();
        for (name, flag) in CodeFlags::NAME_MAPPING {
            dict.set_item(vm.ctx.new_int(flag.bits()), vm.ctx.new_utf8_str(name), vm)
                .unwrap();
        }
        dict
    }
}

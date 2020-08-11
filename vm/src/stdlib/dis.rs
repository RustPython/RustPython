pub(crate) use decl::make_module;

#[pymodule(name = "dis")]
mod decl {
    use crate::bytecode::CodeFlags;
    use crate::obj::objcode::PyCodeRef;
    use crate::obj::objdict::PyDictRef;
    use crate::obj::objstr::PyStringRef;
    use crate::pyobject::{BorrowValue, ItemProtocol, PyObjectRef, PyResult, TryFromObject};
    use crate::vm::VirtualMachine;
    use rustpython_compiler::compile;

    #[pyfunction]
    fn dis(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        // Method or function:
        if let Ok(co) = vm.get_attribute(obj.clone(), "__code__") {
            return disassemble(co, vm);
        }

        // String:
        if let Ok(co_str) = PyStringRef::try_from_object(vm, obj.clone()) {
            let code = vm
                .compile(
                    co_str.borrow_value(),
                    compile::Mode::Exec,
                    "<string>".to_owned(),
                )
                .map_err(|err| vm.new_syntax_error(&err))?
                .into_object();
            return disassemble(code, vm);
        }

        disassemble(obj, vm)
    }

    #[pyfunction]
    fn disassemble(co: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let code = &PyCodeRef::try_from_object(vm, co)?.code;
        print!("{}", code);
        Ok(vm.get_none())
    }

    #[pyattr(name = "COMPILER_FLAG_NAMES")]
    fn compiler_flag_names(vm: &VirtualMachine) -> PyDictRef {
        let dict = vm.ctx.new_dict();
        for (name, flag) in CodeFlags::NAME_MAPPING {
            dict.set_item(
                vm.ctx.new_int(flag.bits()),
                vm.ctx.new_str((*name).to_owned()),
                vm,
            )
            .unwrap();
        }
        dict
    }
}

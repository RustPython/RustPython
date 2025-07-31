pub(crate) use decl::make_module;

#[pymodule(name = "dis")]
mod decl {
    use rustpython_vm::{
        PyObjectRef, PyRef, PyResult, TryFromObject, VirtualMachine,
        builtins::{PyCode, PyDictRef, PyStrRef},
        bytecode::CodeFlags,
        function::OptionalArg,
    };

    #[derive(FromArgs)]
    struct DisArgs {
        #[pyarg(positional)]
        obj: PyObjectRef,
        #[pyarg(any, optional)]
        file: OptionalArg<PyObjectRef>,
    }

    #[pyfunction]
    fn dis(args: DisArgs, vm: &VirtualMachine) -> PyResult<()> {
        let DisArgs { obj, file } = args;
        let co = if let Ok(co) = obj.get_attr("__code__", vm) {
            // Method or function:
            PyRef::try_from_object(vm, co)?
        } else if let Ok(co_str) = PyStrRef::try_from_object(vm, obj.clone()) {
            #[cfg(not(feature = "compiler"))]
            {
                let _ = co_str;
                return Err(
                    vm.new_runtime_error("dis.dis() with str argument requires `compiler` feature")
                );
            }
            #[cfg(feature = "compiler")]
            {
                vm.compile(
                    co_str.as_str(),
                    crate::vm::compiler::Mode::Exec,
                    "<dis>".to_owned(),
                )
                .map_err(|err| vm.new_syntax_error(&err, Some(co_str.as_str())))?
            }
        } else {
            PyRef::try_from_object(vm, obj)?
        };
        disassemble_to_file(co, file.into_option(), vm)
    }

    #[pyfunction]
    fn disassemble(
        co: PyRef<PyCode>,
        file: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        disassemble_to_file(co, file.into_option(), vm)
    }

    fn disassemble_to_file(
        co: PyRef<PyCode>,
        file: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let output = format!("{}", &co.code);

        match file {
            Some(file_obj) => {
                // Write to the provided file object
                if let Ok(write_method) = file_obj.get_attr("write", vm) {
                    write_method.call((output,), vm)?;
                } else {
                    return Err(
                        vm.new_type_error("file argument must have a write method".to_owned())
                    );
                }
            }
            None => {
                // Write to stdout
                print!("{output}");
            }
        }
        Ok(())
    }

    #[pyattr(name = "COMPILER_FLAG_NAMES")]
    fn compiler_flag_names(vm: &VirtualMachine) -> PyDictRef {
        let dict = vm.ctx.new_dict();
        for (name, flag) in CodeFlags::NAME_MAPPING {
            dict.set_item(
                &*vm.new_pyobj(flag.bits()),
                vm.ctx.new_str(*name).into(),
                vm,
            )
            .unwrap();
        }
        dict
    }
}

pub(crate) use opcode::make_module;

#[pymodule]
mod opcode {
    use crate::vm::{
        AsObject, PyObjectRef, PyResult, VirtualMachine,
        builtins::{PyBool, PyInt, PyIntRef, PyNone},
        bytecode::Instruction,
        match_class,
    };
    use std::ops::Deref;

    struct Opcode(Instruction);

    impl Deref for Opcode {
        type Target = Instruction;

        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }

    impl Opcode {
        #[must_use]
        pub fn try_from_pyint(raw: PyIntRef, vm: &VirtualMachine) -> PyResult<Self> {
            let instruction = raw
                .try_to_primitive::<u8>(vm)
                .and_then(|v| {
                    Instruction::try_from(v).map_err(|_| {
                        vm.new_exception_empty(vm.ctx.exceptions.value_error.to_owned())
                    })
                })
                .map_err(|_| vm.new_value_error("invalid opcode or oparg"))?;

            Ok(Self(instruction))
        }
    }

    #[derive(FromArgs)]
    struct StackEffectArgs {
        #[pyarg(positional)]
        opcode: PyIntRef,
        #[pyarg(positional, optional)]
        oparg: Option<PyObjectRef>,
        #[pyarg(named, optional)]
        jump: Option<PyObjectRef>,
    }

    #[pyfunction]
    fn stack_effect(args: StackEffectArgs, vm: &VirtualMachine) -> PyResult<i32> {
        let oparg = args
            .oparg
            .map(|v| {
                if !v.fast_isinstance(vm.ctx.types.int_type) {
                    return Err(vm.new_type_error(format!(
                        "'{}' object cannot be interpreted as an integer",
                        v.class().name()
                    )));
                }
                v.downcast_ref::<PyInt>()?.try_to_primitive::<u32>(vm)
            })
            .unwrap_or(Ok(0))?;

        let jump = args
            .jump
            .map(|v| {
                match_class!(match v {
                    b @ PyBool => Ok(b.is(&vm.ctx.true_value)),
                    _n @ PyNone => Ok(false),
                    _ => {
                        return Err(
                            vm.new_value_error("stack_effect: jump must be False, True or None")
                        );
                    }
                })
            })
            .unwrap_or(Ok(-1))?;

        let opcode = Opcode::try_from_pyint(args.opcode, vm)?;

        Ok(opcode.stack_effect(oparg.into(), jump))
    }
}

pub(crate) use opcode::make_module;

#[pymodule]
mod opcode {
    use crate::vm::{
        AsObject, PyObjectRef, PyResult, VirtualMachine,
        builtins::{PyInt, PyIntRef},
        opcode::{Opcode, PseudoOpcode, RealOpcode},
    };
    #[pyattr]
    const ENABLE_SPECIALIZATION: u8 = 1;

    #[derive(FromArgs)]
    struct StackEffectArgs {
        #[pyarg(positional)]
        opcode: PyIntRef,
        #[pyarg(positional, optional)]
        oparg: Option<PyObjectRef>,
        #[pyarg(named, optional)]
        jump: Option<PyObjectRef>,
    }

    // https://github.com/python/cpython/blob/bcee1c322115c581da27600f2ae55e5439c027eb/Python/compile.c#L704-L767
    #[pyfunction]
    fn stack_effect(args: StackEffectArgs, vm: &VirtualMachine) -> PyResult<i32> {
        let invalid_opcode = || vm.new_value_error("invalid opcode or oparg");

        let oparg = args
            .oparg
            .map(|v| {
                if !v.fast_isinstance(vm.ctx.types.int_type) {
                    return Err(vm.new_type_error(format!(
                        "'{}' object cannot be interpreted as an integer",
                        v.class().name()
                    )));
                }
                v.downcast_ref::<PyInt>()
                    .ok_or_else(|| vm.new_type_error(""))?
                    .try_to_primitive::<i32>(vm)
            })
            .unwrap_or(Ok(0))?;

        let jump = args
            .jump
            .map(|v| {
                v.try_to_bool(vm).map_err(|_| {
                    vm.new_value_error("stack_effect: jump must be False, True or None")
                })
            })
            .unwrap_or(Ok(false))?;

        let raw_opcode = args.opcode.try_to_primitive::<u16>(vm)?;
        let opcode = Opcode::try_from(raw_opcode).map_err(|_| invalid_opcode())?;

        Ok(match opcode {
            Opcode::Real(r_op) => {
                // ExitInitCheck at CPython is under the pseudos match?
                // https://github.com/python/cpython/blob/bcee1c322115c581da27600f2ae55e5439c027eb/Python/compile.c#L736-L737
                if matches!(r_op, RealOpcode::ExitInitCheck) {
                    return Ok(-1);
                }

                if r_op.deopt().is_some() {
                    // Specialized instructions are not supported.
                    return Err(invalid_opcode());
                }

                let popped = r_op.num_popped(oparg);
                let pushed = r_op.num_pushed(oparg);

                if popped < 0 || pushed < 0 {
                    return Err(invalid_opcode());
                }
                pushed - popped
            }
            Opcode::Pseudo(p_op) => {
                match p_op {
                    PseudoOpcode::PopBlock | PseudoOpcode::Jump | PseudoOpcode::JumpNoInterrupt => {
                        0
                    }
                    // Exception handling pseudo-instructions
                    PseudoOpcode::SetupFinally => {
                        if jump {
                            1
                        } else {
                            0
                        }
                    }
                    PseudoOpcode::SetupCleanup => {
                        if jump {
                            2
                        } else {
                            0
                        }
                    }
                    PseudoOpcode::SetupWith => {
                        if jump {
                            1
                        } else {
                            0
                        }
                    }
                    PseudoOpcode::StoreFastMaybeNull => -1,
                    PseudoOpcode::LoadClosure => 1,
                    PseudoOpcode::LoadMethod => 1,
                    PseudoOpcode::LoadSuperMethod
                    | PseudoOpcode::LoadZeroSuperMethod
                    | PseudoOpcode::LoadZeroSuperAttr => -1,
                }
            }
        })
    }

    #[pyfunction]
    fn is_valid(opcode: i32) -> bool {
        Opcode::try_from(opcode).is_ok()
    }

    #[pyfunction]
    fn has_arg(opcode: i32) -> bool {
        RealOpcode::try_from(opcode).is_ok_and(|op| op.has_arg())
    }

    #[pyfunction]
    fn has_const(opcode: i32) -> bool {
        RealOpcode::try_from(opcode).is_ok_and(|op| op.has_const())
    }

    #[pyfunction]
    fn has_name(opcode: i32) -> bool {
        RealOpcode::try_from(opcode).is_ok_and(|op| op.has_name())
    }

    #[pyfunction]
    fn has_jump(opcode: i32) -> bool {
        RealOpcode::try_from(opcode).is_ok_and(|op| op.has_jump())
    }

    #[pyfunction]
    fn has_free(opcode: i32) -> bool {
        RealOpcode::try_from(opcode).is_ok_and(|op| op.has_free())
    }

    #[pyfunction]
    fn has_local(opcode: i32) -> bool {
        RealOpcode::try_from(opcode).is_ok_and(|op| op.has_local())
    }

    #[pyfunction]
    fn has_exc(opcode: i32) -> bool {
        RealOpcode::try_from(opcode).is_ok_and(|op| op.has_exc())
    }

    #[pyfunction]
    fn get_intrinsic1_descs(vm: &VirtualMachine) -> Vec<PyObjectRef> {
        [
            "INTRINSIC_1_INVALID",
            "INTRINSIC_PRINT",
            "INTRINSIC_IMPORT_STAR",
            "INTRINSIC_STOPITERATION_ERROR",
            "INTRINSIC_ASYNC_GEN_WRAP",
            "INTRINSIC_UNARY_POSITIVE",
            "INTRINSIC_LIST_TO_TUPLE",
            "INTRINSIC_TYPEVAR",
            "INTRINSIC_PARAMSPEC",
            "INTRINSIC_TYPEVARTUPLE",
            "INTRINSIC_SUBSCRIPT_GENERIC",
            "INTRINSIC_TYPEALIAS",
        ]
        .into_iter()
        .map(|x| vm.ctx.new_str(x).into())
        .collect()
    }

    #[pyfunction]
    fn get_intrinsic2_descs(vm: &VirtualMachine) -> Vec<PyObjectRef> {
        [
            "INTRINSIC_2_INVALID",
            "INTRINSIC_PREP_RERAISE_STAR",
            "INTRINSIC_TYPEVAR_WITH_BOUND",
            "INTRINSIC_TYPEVAR_WITH_CONSTRAINTS",
            "INTRINSIC_SET_FUNCTION_TYPE_PARAMS",
            "INTRINSIC_SET_TYPEPARAM_DEFAULT",
        ]
        .into_iter()
        .map(|x| vm.ctx.new_str(x).into())
        .collect()
    }

    #[pyfunction]
    fn get_nb_ops(vm: &VirtualMachine) -> Vec<PyObjectRef> {
        [
            ("NB_ADD", "+"),
            ("NB_AND", "&"),
            ("NB_FLOOR_DIVIDE", "//"),
            ("NB_LSHIFT", "<<"),
            ("NB_MATRIX_MULTIPLY", "@"),
            ("NB_MULTIPLY", "*"),
            ("NB_REMAINDER", "%"),
            ("NB_OR", "|"),
            ("NB_POWER", "**"),
            ("NB_RSHIFT", ">>"),
            ("NB_SUBTRACT", "-"),
            ("NB_TRUE_DIVIDE", "/"),
            ("NB_XOR", "^"),
            ("NB_INPLACE_ADD", "+="),
            ("NB_INPLACE_AND", "&="),
            ("NB_INPLACE_FLOOR_DIVIDE", "//="),
            ("NB_INPLACE_LSHIFT", "<<="),
            ("NB_INPLACE_MATRIX_MULTIPLY", "@="),
            ("NB_INPLACE_MULTIPLY", "*="),
            ("NB_INPLACE_REMAINDER", "%="),
            ("NB_INPLACE_OR", "|="),
            ("NB_INPLACE_POWER", "**="),
            ("NB_INPLACE_RSHIFT", ">>="),
            ("NB_INPLACE_SUBTRACT", "-="),
            ("NB_INPLACE_TRUE_DIVIDE", "/="),
            ("NB_INPLACE_XOR", "^="),
        ]
        .into_iter()
        .map(|(a, b)| {
            vm.ctx
                .new_tuple(vec![vm.ctx.new_str(a).into(), vm.ctx.new_str(b).into()])
                .into()
        })
        .collect()
    }

    #[pyfunction]
    fn get_executor(_code: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        // TODO
        Ok(vm.ctx.none())
    }

    #[pyfunction]
    fn get_specialization_stats(vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.none()
    }
}

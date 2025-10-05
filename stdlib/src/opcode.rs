pub(crate) use opcode::make_module;

#[pymodule]
mod opcode {
    use crate::vm::{
        AsObject, PyObjectRef, PyResult, VirtualMachine,
        builtins::{PyBool, PyInt, PyIntRef, PyNone},
        bytecode::Instruction,
        match_class,
        opcode::OpcodeId,
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

    // TODO: Make use of the auto generated `Instruction` enum (when we will have it)
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
                v.downcast_ref::<PyInt>()
                    .ok_or_else(|| vm.new_type_error(""))?
                    .try_to_primitive::<u32>(vm)
            })
            .unwrap_or(Ok(0))?;

        let jump = args
            .jump
            .map(|v| {
                match_class!(match v {
                    b @ PyBool => Ok(b.is(&vm.ctx.true_value)),
                    _n @ PyNone => Ok(false),
                    _ => {
                        Err(vm.new_value_error("stack_effect: jump must be False, True or None"))
                    }
                })
            })
            .unwrap_or(Ok(false))?;

        let instruction = args
            .opcode
            .try_to_primitive::<u8>(vm)
            .and_then(|v| {
                Instruction::try_from(v)
                    .map_err(|_| vm.new_exception_empty(vm.ctx.exceptions.value_error.to_owned()))
            })
            .map_err(|_| vm.new_value_error("invalid opcode or oparg"))?;

        Ok(instruction.stack_effect(oparg.into(), jump))
    }

    #[pyfunction]
    fn is_valid(opcode: i32) -> bool {
        OpcodeId::try_from(opcode).is_ok()
    }

    #[pyfunction]
    fn has_arg(opcode: i32) -> bool {
        OpcodeId::try_from(opcode).map_or(false, |oid| !oid.is_pseudo() && oid.has_arg())
    }

    #[pyfunction]
    fn has_const(opcode: i32) -> bool {
        OpcodeId::try_from(opcode).map_or(false, |oid| !oid.is_pseudo() && oid.has_const())
    }

    #[pyfunction]
    fn has_name(opcode: i32) -> bool {
        OpcodeId::try_from(opcode).map_or(false, |oid| !oid.is_pseudo() && oid.has_name())
    }

    #[pyfunction]
    fn has_jump(opcode: i32) -> bool {
        OpcodeId::try_from(opcode).map_or(false, |oid| !oid.is_pseudo() && oid.has_jump())
    }

    #[pyfunction]
    fn has_free(opcode: i32) -> bool {
        OpcodeId::try_from(opcode).map_or(false, |oid| !oid.is_pseudo() && oid.has_free())
    }

    #[pyfunction]
    fn has_local(opcode: i32) -> bool {
        OpcodeId::try_from(opcode).map_or(false, |oid| !oid.is_pseudo() && oid.has_local())
    }

    #[pyfunction]
    fn has_exc(opcode: i32) -> bool {
        OpcodeId::try_from(opcode).map_or(false, |oid| !oid.is_pseudo() && oid.has_exc())
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

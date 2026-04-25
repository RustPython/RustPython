pub(crate) use _opcode::module_def;

#[pymodule]
mod _opcode {
    use crate::vm::{
        AsObject, PyObjectRef, PyResult, VirtualMachine,
        builtins::{PyInt, PyIntRef},
        bytecode::{AnyInstruction, AnyOpcode, InstructionMetadata, Opcode, PseudoOpcode, oparg},
    };

    fn try_from_i32(raw: i32) -> Result<AnyOpcode, ()> {
        u16::try_from(raw)
            .map_err(|_| ())?
            .try_into()
            .map_err(|_| ())
    }

    // https://github.com/python/cpython/blob/v3.14.2/Include/opcode_ids.h#L252
    const HAVE_ARGUMENT: i32 = 43;

    // prepare specialization
    #[pyattr]
    const ENABLE_SPECIALIZATION: i8 = 1;

    #[pyattr]
    const ENABLE_SPECIALIZATION_FT: i8 = 1;

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
                v.downcast_ref::<PyInt>()
                    .ok_or_else(|| {
                        vm.new_type_error(format!(
                            "'{}' object cannot be interpreted as an integer",
                            v.class().name()
                        ))
                    })?
                    .try_to_primitive::<u32>(vm)
            })
            .unwrap_or(Ok(0))?;

        let jump: Option<bool> = match args.jump {
            Some(v) => {
                if vm.is_none(&v) {
                    None
                } else {
                    Some(v.try_to_bool(vm).map_err(|_| {
                        vm.new_value_error("stack_effect: jump must be False, True or None")
                    })?)
                }
            }
            None => None,
        };

        let instr = args
            .opcode
            .try_to_primitive::<u16>(vm)
            .and_then(|v| {
                AnyInstruction::try_from(v)
                    .map_err(|_| vm.new_exception_empty(vm.ctx.exceptions.value_error.to_owned()))
            })
            .map_err(|_| vm.new_value_error("invalid opcode or oparg"))?;

        // Raise ValueError if specialized.
        if instr.real().is_some_and(|op| op.deopt().is_some()) {
            return Err(vm.new_value_error("invalid opcode or oparg"));
        }

        let effect = match jump {
            Some(true) => instr.stack_effect_jump(oparg),
            Some(false) => instr.stack_effect(oparg),
            // jump=None: max of both paths (CPython convention)
            None => instr
                .stack_effect(oparg)
                .max(instr.stack_effect_jump(oparg)),
        };
        Ok(effect)
    }

    #[pyfunction]
    fn is_valid(opcode: i32) -> bool {
        try_from_i32(opcode).is_ok()
    }

    #[pyfunction]
    fn has_arg(opcode: i32) -> bool {
        try_from_i32(opcode).is_ok_and(|_| opcode > HAVE_ARGUMENT)
    }

    #[pyfunction]
    fn has_const(opcode: i32) -> bool {
        matches!(try_from_i32(opcode), Ok(AnyOpcode::Real(Opcode::LoadConst)))
    }

    #[pyfunction]
    fn has_name(opcode: i32) -> bool {
        matches!(
            try_from_i32(opcode),
            Ok(AnyOpcode::Real(
                Opcode::DeleteAttr
                    | Opcode::DeleteGlobal
                    | Opcode::DeleteName
                    | Opcode::ImportFrom
                    | Opcode::ImportName
                    | Opcode::LoadAttr
                    | Opcode::LoadGlobal
                    | Opcode::LoadName
                    | Opcode::StoreAttr
                    | Opcode::StoreGlobal
                    | Opcode::StoreName
            ))
        )
    }

    #[pyfunction]
    fn has_jump(opcode: i32) -> bool {
        matches!(
            try_from_i32(opcode),
            Ok(AnyOpcode::Real(
                Opcode::ForIter | Opcode::PopJumpIfFalse | Opcode::PopJumpIfTrue | Opcode::Send
            ) | AnyOpcode::Pseudo(PseudoOpcode::Jump))
        )
    }

    #[pyfunction]
    fn has_free(opcode: i32) -> bool {
        matches!(
            try_from_i32(opcode),
            Ok(AnyOpcode::Real(
                Opcode::DeleteDeref
                    | Opcode::LoadFromDictOrDeref
                    | Opcode::LoadDeref
                    | Opcode::StoreDeref
            ))
        )
    }

    #[pyfunction]
    fn has_local(opcode: i32) -> bool {
        matches!(
            try_from_i32(opcode),
            Ok(AnyOpcode::Real(
                Opcode::DeleteFast
                    | Opcode::LoadFast
                    | Opcode::LoadFastAndClear
                    | Opcode::StoreFast
                    | Opcode::StoreFastLoadFast
            ))
        )
    }

    #[pyfunction]
    fn has_exc(opcode: i32) -> bool {
        // No instructions have exception info in RustPython
        // (exception handling is done via exception table)
        let _ = opcode;
        false
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
        oparg::IntrinsicFunction2::iterator()
            .map(|x| vm.ctx.new_str(x.desc()).into())
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
            ("NB_SUBSCR", "[]"),
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
    fn get_special_method_names(vm: &VirtualMachine) -> Vec<PyObjectRef> {
        oparg::SpecialMethod::iterator()
            .map(|x| vm.ctx.new_str(x.to_string()).into())
            .collect()
    }

    #[pyfunction]
    fn get_executor(
        _code: PyObjectRef,
        _offset: i32,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        Ok(vm.ctx.none())
    }

    #[pyfunction]
    fn get_specialization_stats(vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.none()
    }
}

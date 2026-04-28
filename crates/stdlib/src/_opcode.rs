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
                    | Opcode::LoadFromDictOrGlobals
                    | Opcode::LoadGlobal
                    | Opcode::LoadName
                    | Opcode::LoadSuperAttr
                    | Opcode::StoreAttr
                    | Opcode::StoreGlobal
                    | Opcode::StoreName
                    | Opcode::InstrumentedLoadSuperAttr
            ))
        )
    }

    #[pyfunction]
    fn has_jump(opcode: i32) -> bool {
        matches!(
            try_from_i32(opcode),
            Ok(AnyOpcode::Real(
                Opcode::EndAsyncFor
                    | Opcode::ForIter
                    | Opcode::JumpBackward
                    | Opcode::JumpBackwardNoInterrupt
                    | Opcode::JumpForward
                    | Opcode::PopJumpIfFalse
                    | Opcode::PopJumpIfNone
                    | Opcode::PopJumpIfNotNone
                    | Opcode::PopJumpIfTrue
                    | Opcode::Send
                    | Opcode::InstrumentedForIter
                    | Opcode::InstrumentedEndAsyncFor
            ) | AnyOpcode::Pseudo(
                PseudoOpcode::Jump
                    | PseudoOpcode::JumpIfFalse
                    | PseudoOpcode::JumpIfTrue
                    | PseudoOpcode::JumpNoInterrupt
            ))
        )
    }

    #[pyfunction]
    fn has_free(opcode: i32) -> bool {
        matches!(
            try_from_i32(opcode),
            Ok(AnyOpcode::Real(
                Opcode::DeleteDeref
                    | Opcode::LoadFromDictOrDeref
                    | Opcode::MakeCell
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
                    | Opcode::LoadDeref
                    | Opcode::LoadFast
                    | Opcode::LoadFastAndClear
                    | Opcode::LoadFastBorrow
                    | Opcode::LoadFastBorrowLoadFastBorrow
                    | Opcode::LoadFastCheck
                    | Opcode::LoadFastLoadFast
                    | Opcode::StoreFast
                    | Opcode::StoreFastLoadFast
                    | Opcode::StoreFastStoreFast
            ) | AnyOpcode::Pseudo(PseudoOpcode::LoadClosure | PseudoOpcode::StoreFastMaybeNull))
        )
    }

    #[pyfunction]
    fn has_exc(opcode: i32) -> bool {
        // No instructions have exception info in RustPython
        // (exception handling is done via exception table)
        // This is for compatibility with CPython

        matches!(
            try_from_i32(opcode),
            Ok(AnyOpcode::Pseudo(
                PseudoOpcode::SetupCleanup | PseudoOpcode::SetupFinally | PseudoOpcode::SetupWith
            ))
        )
    }

    #[pyfunction]
    fn get_intrinsic1_descs(vm: &VirtualMachine) -> Vec<PyObjectRef> {
        oparg::IntrinsicFunction1::iter()
            .map(|x| vm.ctx.new_str(x.desc()).into())
            .collect()
    }

    #[pyfunction]
    fn get_intrinsic2_descs(vm: &VirtualMachine) -> Vec<PyObjectRef> {
        oparg::IntrinsicFunction2::iter()
            .map(|x| vm.ctx.new_str(x.desc()).into())
            .collect()
    }

    #[pyfunction]
    fn get_nb_ops(vm: &VirtualMachine) -> Vec<PyObjectRef> {
        oparg::BinaryOperator::iter()
            .map(|x| {
                vm.ctx
                    .new_tuple(vec![
                        vm.ctx.new_str(x.desc()).into(),
                        vm.ctx.new_str(x.to_string()).into(),
                    ])
                    .into()
            })
            .collect()
    }

    #[pyfunction]
    fn get_special_method_names(vm: &VirtualMachine) -> Vec<PyObjectRef> {
        oparg::SpecialMethod::iter()
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

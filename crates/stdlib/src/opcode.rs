pub(crate) use opcode::make_module;

#[pymodule]
mod opcode {
    use crate::vm::{
        AsObject, PyObjectRef, PyResult, VirtualMachine,
        builtins::{PyInt, PyIntRef},
        bytecode::Instruction,
    };
    use core::ops::Deref;

    struct Opcode(Instruction);

    impl Deref for Opcode {
        type Target = Instruction;

        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }

    impl Opcode {
        // https://github.com/python/cpython/blob/bcee1c322115c581da27600f2ae55e5439c027eb/Include/opcode_ids.h#L238
        const HAVE_ARGUMENT: i32 = 44;

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

        /// Check if opcode is valid (can be converted to an Instruction)
        #[must_use]
        pub fn is_valid(opcode: i32) -> bool {
            if !(0..=255).contains(&opcode) {
                return false;
            }
            Instruction::try_from(opcode as u8).is_ok()
        }

        /// Check if instruction has an argument
        #[must_use]
        pub fn has_arg(opcode: i32) -> bool {
            Self::is_valid(opcode) && opcode > Self::HAVE_ARGUMENT
        }

        /// Check if instruction uses co_consts
        #[must_use]
        pub fn has_const(opcode: i32) -> bool {
            Self::is_valid(opcode)
                && matches!(
                    Instruction::try_from(opcode as u8),
                    Ok(Instruction::LoadConst { .. } | Instruction::ReturnConst { .. })
                )
        }

        /// Check if instruction uses co_names
        #[must_use]
        pub fn has_name(opcode: i32) -> bool {
            Self::is_valid(opcode)
                && matches!(
                    Instruction::try_from(opcode as u8),
                    Ok(Instruction::DeleteAttr { .. }
                        | Instruction::DeleteGlobal(_)
                        | Instruction::DeleteName(_)
                        | Instruction::ImportFrom { .. }
                        | Instruction::ImportName { .. }
                        | Instruction::LoadAttr { .. }
                        | Instruction::LoadGlobal(_)
                        | Instruction::LoadAttrMethod { .. }
                        | Instruction::LoadName(_)
                        | Instruction::StoreAttr { .. }
                        | Instruction::StoreGlobal(_)
                        | Instruction::StoreName(_))
                )
        }

        /// Check if instruction is a jump
        #[must_use]
        pub fn has_jump(opcode: i32) -> bool {
            Self::is_valid(opcode)
                && matches!(
                    Instruction::try_from(opcode as u8),
                    Ok(Instruction::Break { .. }
                        | Instruction::Continue { .. }
                        | Instruction::ForIter { .. }
                        | Instruction::JumpIfFalseOrPop { .. }
                        | Instruction::JumpIfNotExcMatch(_)
                        | Instruction::JumpIfTrueOrPop { .. }
                        | Instruction::Jump { .. }
                        | Instruction::PopJumpIfFalse { .. }
                        | Instruction::PopJumpIfTrue { .. }
                        | Instruction::Send { .. })
                )
        }

        /// Check if instruction uses co_freevars/co_cellvars
        #[must_use]
        pub fn has_free(opcode: i32) -> bool {
            Self::is_valid(opcode)
                && matches!(
                    Instruction::try_from(opcode as u8),
                    Ok(Instruction::DeleteDeref(_)
                        | Instruction::LoadFromDictOrDeref(_)
                        | Instruction::LoadClosure(_)
                        | Instruction::LoadDeref(_)
                        | Instruction::StoreDeref(_))
                )
        }

        /// Check if instruction uses co_varnames (local variables)
        #[must_use]
        pub fn has_local(opcode: i32) -> bool {
            Self::is_valid(opcode)
                && matches!(
                    Instruction::try_from(opcode as u8),
                    Ok(Instruction::DeleteFast(_)
                        | Instruction::LoadFast(_)
                        | Instruction::LoadFastAndClear(_)
                        | Instruction::StoreFast(_)
                        | Instruction::StoreFastLoadFast { .. })
                )
        }

        /// Check if instruction has exception info
        #[must_use]
        pub fn has_exc(_opcode: i32) -> bool {
            // No instructions have exception info in RustPython
            // (exception handling is done via exception table)
            false
        }
    }

    #[pyattr]
    const ENABLE_SPECIALIZATION: i8 = 1;

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

        let jump = args
            .jump
            .map(|v| {
                v.try_to_bool(vm).map_err(|_| {
                    vm.new_value_error("stack_effect: jump must be False, True or None")
                })
            })
            .unwrap_or(Ok(false))?;

        let opcode = Opcode::try_from_pyint(args.opcode, vm)?;

        Ok(opcode.stack_effect(oparg.into(), jump))
    }

    #[pyfunction]
    fn is_valid(opcode: i32) -> bool {
        Opcode::is_valid(opcode)
    }

    #[pyfunction]
    fn has_arg(opcode: i32) -> bool {
        Opcode::has_arg(opcode)
    }

    #[pyfunction]
    fn has_const(opcode: i32) -> bool {
        Opcode::has_const(opcode)
    }

    #[pyfunction]
    fn has_name(opcode: i32) -> bool {
        Opcode::has_name(opcode)
    }

    #[pyfunction]
    fn has_jump(opcode: i32) -> bool {
        Opcode::has_jump(opcode)
    }

    #[pyfunction]
    fn has_free(opcode: i32) -> bool {
        Opcode::has_free(opcode)
    }

    #[pyfunction]
    fn has_local(opcode: i32) -> bool {
        Opcode::has_local(opcode)
    }

    #[pyfunction]
    fn has_exc(opcode: i32) -> bool {
        Opcode::has_exc(opcode)
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

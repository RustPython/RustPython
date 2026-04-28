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

#[cfg(test)]
mod tests {
    use crate::vm::{self, compiler::Mode};

    macro_rules! assert_dis_snapshot {
        ($value:expr) => {
            insta::with_settings!({snapshot_path => "./snapshots"}, {
                insta::assert_snapshot!(
                    insta::internals::AutoName,
                    dis($value.trim()),
                    stringify!($value).trim()
                )
            })
        };
    }

    /// Returns the [`dis.dis`](https://docs.python.org/3/library/dis.html#dis.dis) output.
    ///
    /// # Notes
    ///
    /// Memory addresses in the output are replaced with `0xdeadbeef` for consistency.
    fn dis(source: &str) -> String {
        let fname = String::from("<embedded>");

        let builder = vm::Interpreter::builder(Default::default());
        let stdlib_defs = crate::stdlib_module_defs(&builder.ctx);
        let interp = builder
            .add_native_modules(&stdlib_defs)
            .add_frozen_modules(rustpython_pylib::FROZEN_STDLIB)
            .build();

        interp.enter(|vm| {
            let scope = vm.new_scope_with_builtins();
            let code_obj = vm
                .compile(source, Mode::Exec, fname.clone())
                .map_err(|err| vm.new_syntax_error(&err, Some(source)))
                .unwrap();
            scope.globals.set_item("code", code_obj.into(), vm).unwrap();

            let py_source = r#"
import dis
import io
import re
import sys

old_stdout = sys.stdout
sys.stdout = buf = io.StringIO()
dis.dis(code)
sys.stdout = old_stdout

tmp_output = buf.getvalue()

# constant mem address
output = re.sub(r'(<code object \w+ at )0x[0-9a-fA-F]+', r'\g<1>0xdeadbeef', tmp_output)
"#;

            let py_code_obj = vm
                .compile(py_source, Mode::Exec, fname)
                .map_err(|err| vm.new_syntax_error(&err, Some(py_source)))
                .unwrap();

            vm.run_code_obj(py_code_obj, scope.clone()).unwrap();
            let py_output = scope.globals.get_item("output", vm).unwrap();
            py_output.str(vm).unwrap().to_string()
        })
    }

    #[test]
    fn test_if_ors() {
        assert_dis_snapshot!(
            "
if True or False or False:
    pass
"
        )
    }

    #[test]
    fn test_if_ands() {
        assert_dis_snapshot!(
            "
if True and False and False:
    pass
"
        )
    }

    #[test]
    fn test_if_mixed() {
        assert_dis_snapshot!(
            "
if (True and False) or (False and True):
    pass
"
        )
    }

    #[test]
    fn test_nested_bool_op() {
        assert_dis_snapshot!("x = Test() and False or False")
    }

    #[test]
    fn test_const_no_op() {
        assert_dis_snapshot!("x = not True")
    }

    #[test]
    fn test_constant_true_if_pass_keeps_line_anchor_nop() {
        assert_dis_snapshot!(
            "
if 1:
    pass
"
        )
    }

    #[test]
    fn test_nested_double_async_with() {
        assert_dis_snapshot!(
            "
async def test():
    for stop_exc in (StopIteration('spam'), StopAsyncIteration('ham')):
        with self.subTest(type=type(stop_exc)):
            try:
                async with egg():
                    raise stop_exc
            except Exception as ex:
                self.assertIs(ex, stop_exc)
            else:
                self.fail(f'{stop_exc} was suppressed')
"
        )
    }

    #[test]
    fn test_bare_function_annotations_check_attribute_and_subscript_expressions() {
        assert_dis_snapshot!(
            "
def f(one: int):
    int.new_attr: int
    [list][0].new_attr: [int, str]
    my_lst = [1]
    my_lst[one]: int
    return my_lst
"
        )
    }
}

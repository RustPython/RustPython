pub(crate) use _opcode::module_def;

#[pymodule]
mod _opcode {
    use crate::vm::{
        AsObject, PyObjectRef, PyResult, VirtualMachine,
        bytecode::{oparg, public_metadata::PublicOpcode},
    };

    fn try_from_i32(raw: i32) -> Option<PublicOpcode> {
        PublicOpcode::from_i32(raw)
    }

    // prepare specialization
    #[pyattr]
    const ENABLE_SPECIALIZATION: i8 = 1;

    #[pyattr]
    const ENABLE_SPECIALIZATION_FT: i8 = 1;

    #[derive(FromArgs)]
    struct StackEffectArgs {
        #[pyarg(positional)]
        opcode: PyObjectRef,
        #[pyarg(positional, optional)]
        oparg: Option<PyObjectRef>,
        #[pyarg(named, optional)]
        jump: Option<PyObjectRef>,
    }

    // Convert only after FromArgs has checked arity and keywords, so rejected
    // calls never invoke __index__. Keep these C API diagnostics local.
    fn opcode_int(obj: &PyObjectRef, vm: &VirtualMachine) -> PyResult<i32> {
        let value = obj.try_index(vm)?;
        i32::try_from(value.as_bigint())
            .map_err(|_| vm.new_overflow_error("Python int too large to convert to C int"))
    }

    #[allow(clippy::cast_possible_truncation, clippy::unnecessary_cast)]
    fn operand_int(obj: &PyObjectRef, vm: &VirtualMachine) -> PyResult<i32> {
        let value = obj.try_index(vm)?;
        let value = core::ffi::c_long::try_from(value.as_bigint())
            .map_err(|_| vm.new_overflow_error("Python int too large to convert to C long"))?;
        // Final _opcode.c narrows PyLong_AsLong's result to C int. This is the
        // explicit low-bit policy for supported two's-complement target ABIs;
        // it does not imply portable C signed-arithmetic overflow semantics.
        Ok(value as i32)
    }

    #[pyfunction]
    fn stack_effect(args: StackEffectArgs, vm: &VirtualMachine) -> PyResult<i32> {
        // Retain every original argument owner through conversion, validation,
        // effect selection and local error construction. Outer native transport
        // and result wrapping keep their existing ownership behavior.
        let opcode = opcode_int(&args.opcode, vm)?;
        let oparg = match args.oparg.as_ref() {
            Some(value) if !vm.is_none(value) => operand_int(value, vm)?,
            _ => 0,
        };
        let jump = match args.jump.as_ref() {
            None => None,
            Some(value) if vm.is_none(value) => None,
            Some(value) if value.is(&vm.ctx.true_value) => Some(true),
            Some(value) if value.is(&vm.ctx.false_value) => Some(false),
            Some(_) => {
                return Err(vm.new_value_error("stack_effect: jump must be False, True or None"));
            }
        };
        PublicOpcode::from_i32(opcode)
            // Preserve the signed C-int bit pattern at the generated API's
            // existing u32 boundary; it immediately reinterprets it as i32.
            .and_then(|opcode| opcode.stack_effect(oparg.cast_unsigned(), jump))
            .ok_or_else(|| vm.new_value_error("invalid opcode or oparg"))
    }

    #[derive(FromArgs)]
    struct OpcodeArg {
        #[pyarg(any)]
        opcode: PyObjectRef,
    }

    #[pyfunction]
    fn is_valid(OpcodeArg { opcode }: OpcodeArg, vm: &VirtualMachine) -> PyResult<bool> {
        Ok(try_from_i32(opcode_int(&opcode, vm)?).is_some())
    }

    #[pyfunction]
    fn has_arg(OpcodeArg { opcode }: OpcodeArg, vm: &VirtualMachine) -> PyResult<bool> {
        Ok(try_from_i32(opcode_int(&opcode, vm)?).is_some_and(|op| op.has_arg()))
    }

    #[pyfunction]
    fn has_const(OpcodeArg { opcode }: OpcodeArg, vm: &VirtualMachine) -> PyResult<bool> {
        Ok(try_from_i32(opcode_int(&opcode, vm)?).is_some_and(|op| op.has_const()))
    }

    #[pyfunction]
    fn has_name(OpcodeArg { opcode }: OpcodeArg, vm: &VirtualMachine) -> PyResult<bool> {
        Ok(try_from_i32(opcode_int(&opcode, vm)?).is_some_and(|op| op.has_name()))
    }

    #[pyfunction]
    fn has_jump(OpcodeArg { opcode }: OpcodeArg, vm: &VirtualMachine) -> PyResult<bool> {
        Ok(try_from_i32(opcode_int(&opcode, vm)?).is_some_and(|op| op.has_jump()))
    }

    #[pyfunction]
    fn has_free(OpcodeArg { opcode }: OpcodeArg, vm: &VirtualMachine) -> PyResult<bool> {
        Ok(try_from_i32(opcode_int(&opcode, vm)?).is_some_and(|op| op.has_free()))
    }

    #[pyfunction]
    fn has_local(OpcodeArg { opcode }: OpcodeArg, vm: &VirtualMachine) -> PyResult<bool> {
        Ok(try_from_i32(opcode_int(&opcode, vm)?).is_some_and(|op| op.has_local()))
    }

    #[pyfunction]
    fn has_exc(OpcodeArg { opcode }: OpcodeArg, vm: &VirtualMachine) -> PyResult<bool> {
        Ok(try_from_i32(opcode_int(&opcode, vm)?).is_some_and(|op| op.is_block_push()))
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

    #[derive(FromArgs)]
    #[allow(dead_code)]
    struct ExecutorArgs {
        #[pyarg(any)]
        code: PyObjectRef,
        #[pyarg(any)]
        offset: i32,
    }

    #[pyfunction]
    fn get_executor(_args: ExecutorArgs, vm: &VirtualMachine) -> PyObjectRef {
        // TODO
        vm.ctx.none()
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
            insta::assert_snapshot!(dis($value))
        };
    }

    /// Returns the [`dis.dis`](https://docs.python.org/3/library/dis.html#dis.dis) output.
    ///
    /// # Notes
    ///
    /// Memory addresses in the output are replaced with `0xdeadbeef` for consistency.
    fn dis(source: &str) -> String {
        const FNAME: &str = "<?>";

        let builder = vm::Interpreter::builder(Default::default());
        let stdlib_defs = crate::stdlib_module_defs(&builder.ctx);
        let interp = builder
            .add_native_modules(&stdlib_defs)
            .add_frozen_modules(rustpython_pylib::FROZEN_STDLIB)
            .build();

        interp.enter(|vm| {
            let scope = vm.new_scope_with_builtins();
            let code_obj = vm
                .compile(source.trim(), Mode::Exec, FNAME)
                .map_err(|err| err.into_pyexception(vm, Some(source)))
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
                .compile(py_source, Mode::Exec, FNAME)
                .map_err(|err| err.into_pyexception(vm, Some(py_source)))
                .unwrap();

            vm.run_code_obj(py_code_obj, scope.clone()).unwrap();
            let py_output = scope.globals.get_item("output", vm).unwrap();
            py_output.str(vm).unwrap().to_string()
        })
    }

    #[test]
    fn if_ors() {
        assert_dis_snapshot!(
            r#"
if True or False or False:
    pass
"#
        )
    }

    #[test]
    fn if_ands() {
        assert_dis_snapshot!(
            r#"
if True and False and False:
    pass
"#
        )
    }

    #[test]
    fn if_mixed() {
        assert_dis_snapshot!(
            r#"
if (True and False) or (False and True):
    pass
"#
        )
    }

    #[test]
    fn nested_bool_op() {
        assert_dis_snapshot!(
            r#"
x = Test() and False or False
"#
        )
    }

    #[test]
    fn const_no_op() {
        assert_dis_snapshot!(
            r#"
x = not True
"#
        )
    }

    #[test]
    fn constant_true_if_pass_keeps_line_anchor_nop() {
        assert_dis_snapshot!(
            r#"
if 1:
    pass
"#
        )
    }

    #[test]
    fn nested_double_async_with() {
        assert_dis_snapshot!(
            r#"
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
"#
        )
    }

    #[test]
    fn bare_function_annotations_check_attribute_and_subscript_expressions() {
        assert_dis_snapshot!(
            r#"
def f(one: int):
    int.new_attr: int
    [list][0].new_attr: [int, str]
    my_lst = [1]
    my_lst[one]: int
    return my_lst
"#
        )
    }
}

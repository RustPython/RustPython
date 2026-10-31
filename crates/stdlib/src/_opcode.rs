pub(crate) use _opcode::module_def;

#[pymodule]
mod _opcode {
    use crate::vm::{
        AsObject, PyObjectRef, PyResult, VirtualMachine,
        bytecode::{AnyOpcode, oparg},
    };

    fn try_from_i32(raw: i32) -> Result<AnyOpcode, ()> {
        u16::try_from(raw)
            .map_err(|_| ())?
            .try_into()
            .map_err(|_| ())
    }

    // prepare specialization
    #[pyattr]
    const ENABLE_SPECIALIZATION: i8 = 1;

    #[pyattr]
    const ENABLE_SPECIALIZATION_FT: i8 = 1;

    #[derive(FromArgs)]
    struct StackEffectArgs {
        #[pyarg(positional)]
        opcode: i32,
        #[pyarg(positional, optional)]
        oparg: Option<PyObjectRef>,
        #[pyarg(named, optional)]
        jump: Option<PyObjectRef>,
    }

    #[pyfunction]
    #[allow(clippy::unnecessary_cast)] // `c_long` is `i32` or `i64`
    fn stack_effect(args: StackEffectArgs, vm: &VirtualMachine) -> PyResult<i32> {
        let oparg = match args.oparg {
            // (int)PyLong_AsLong(oparg)
            Some(v) if !vm.is_none(&v) => v.try_into_value::<core::ffi::c_long>(vm)? as i32,
            _ => 0,
        };
        let jump = match args.jump {
            None => None,
            Some(v) if vm.is_none(&v) => None,
            Some(v) if v.is(&vm.ctx.true_value) => Some(true),
            Some(v) if v.is(&vm.ctx.false_value) => Some(false),
            Some(_) => {
                return Err(vm.new_value_error("stack_effect: jump must be False, True or None"));
            }
        };
        let invalid = || vm.new_value_error("invalid opcode or oparg");
        let instr = try_from_i32(args.opcode).map_err(|_| invalid())?;
        // Specialized instructions are not supported.
        if instr.real().is_some_and(|op| op.deopt().is_some()) {
            return Err(invalid());
        }
        let oparg = u32::try_from(oparg).map_err(|_| invalid())?;
        let effect = if jump == Some(false) {
            instr.stack_effect(oparg)
        } else {
            instr.stack_effect_jump(oparg)
        };
        Ok(effect)
    }

    #[derive(FromArgs)]
    struct OpcodeArg {
        #[pyarg(any)]
        opcode: i32,
    }

    #[pyfunction]
    fn is_valid(OpcodeArg { opcode }: OpcodeArg) -> bool {
        try_from_i32(opcode).is_ok()
    }

    #[pyfunction]
    fn has_arg(OpcodeArg { opcode }: OpcodeArg) -> bool {
        try_from_i32(opcode).is_ok_and(|op| op.has_arg())
    }

    #[pyfunction]
    fn has_const(OpcodeArg { opcode }: OpcodeArg) -> bool {
        try_from_i32(opcode).is_ok_and(|op| op.has_const())
    }

    #[pyfunction]
    fn has_name(OpcodeArg { opcode }: OpcodeArg) -> bool {
        try_from_i32(opcode).is_ok_and(|op| op.has_name())
    }

    #[pyfunction]
    fn has_jump(OpcodeArg { opcode }: OpcodeArg) -> bool {
        try_from_i32(opcode).is_ok_and(|op| op.has_jump())
    }

    #[pyfunction]
    fn has_free(OpcodeArg { opcode }: OpcodeArg) -> bool {
        try_from_i32(opcode).is_ok_and(|op| op.has_free())
    }

    #[pyfunction]
    fn has_local(OpcodeArg { opcode }: OpcodeArg) -> bool {
        try_from_i32(opcode).is_ok_and(|op| op.has_local())
    }

    #[pyfunction]
    fn has_exc(OpcodeArg { opcode }: OpcodeArg) -> bool {
        try_from_i32(opcode).is_ok_and(|op| op.is_block_push())
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
            insta::assert_snapshot!(dis($value, false))
        };
        (positions: $value:expr) => {
            insta::assert_snapshot!(dis($value, true))
        };
    }

    /// Returns the [`dis.dis`](https://docs.python.org/3/library/dis.html#dis.dis) output,
    /// with the source positions of the instructions if `show_positions` is true.
    ///
    /// # Notes
    ///
    /// Memory addresses in the output are replaced with `0xdeadbeef` for consistency.
    fn dis(source: &str, show_positions: bool) -> String {
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
            scope
                .globals
                .set_item("show_positions", vm.new_pyobj(show_positions), vm)
                .unwrap();

            let py_source = r#"
import dis
import io
import re
import sys

old_stdout = sys.stdout
sys.stdout = buf = io.StringIO()
dis.dis(code, show_positions=show_positions)
sys.stdout = old_stdout

tmp_output = buf.getvalue()

# constant mem address
output = re.sub(r'(<code object .*? at )0x[0-9a-fA-F]+', r'\g<1>0xdeadbeef', tmp_output)
"#;

            let py_code_obj = vm
                .compile(py_source, Mode::Exec, FNAME)
                .map_err(|err| err.into_pyexception(vm, Some(py_source)))
                .unwrap();

            vm.run_code_obj(py_code_obj, scope.clone())
                .unwrap_or_else(|err| {
                    let mut traceback = String::new();
                    let _ = vm.write_exception(&mut traceback, &err);
                    panic!("Python disassembly failed: {err:?}\n{traceback}");
                });
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

    #[test]
    fn empty_module_implicit_return_location() {
        // The implicit return is emitted with NO_LOCATION; propagate_line_numbers()
        // gives it the module RESUME location, whose line is 0.
        assert_dis_snapshot!(positions: "")
    }

    #[test]
    fn module_docstring_location() {
        // codegen_body() loads the docstring at its expression location and stores
        // __doc__ with NO_LOCATION.
        assert_dis_snapshot!(
            positions: r#"
"doc"
x = 1
"#
        )
    }

    #[test]
    fn redundant_nop_location() {
        // basicblock_remove_redundant_nops() copies the full NOP location into a
        // following no-location jump.
        assert_dis_snapshot!(
            positions: r#"
def f(x, y, z):
    while x:
        if y:
            pass
        elif z:
            if y < 0:
                return y
            if z:
                y = y + 1
        elif y:
            return 1
    return -1
"#
        )
    }

    #[test]
    fn annotated_multiline_function_firstlineno() {
        // codegen_function() computes firstlineno from the FunctionDef before
        // compiling annotations.
        assert_dis_snapshot!(
            positions: r#"

a = 1
def f(
    x: a,
): ...
"#
        )
    }

    #[test]
    fn annotation_scope_function_location() {
        // codegen_function_annotations() receives LOC(function) and uses it for the
        // annotation closure's BUILD_MAP/RETURN_VALUE and the MAKE_FUNCTION annotate
        // sequence.
        assert_dis_snapshot!(
            positions: r#"

def g():
    def f(x: not (int is int), /): ...
"#
        )
    }

    #[test]
    fn starred_arg_annotation_unpack_location() {
        // codegen_argannotation() visits `Ts` at the annotation location, then emits
        // UNPACK_SEQUENCE at LOC(function).
        assert_dis_snapshot!(
            positions: r#"
def f(*args: *Ts): pass
"#
        )
    }

    #[test]
    fn module_deferred_annotations_location() {
        // codegen_process_deferred_annotations() uses the first module statement
        // location from start_location().
        assert_dis_snapshot!(
            positions: r#"
import os
X: int
Y: str
"#
        )
    }

    #[test]
    fn not_compare_location() {
        // A Compare inside UnaryOp(Not) is parsed with the UnaryOp start location.
        assert_dis_snapshot!(
            positions: r#"
def f(self, other):
    return not self == other
"#
        )
    }

    #[test]
    fn not_chained_compare_location() {
        // A chained comparison keeps its inner range for the comparison scaffolding
        // and uses the UnaryOp range only for TO_BOOL and UNARY_NOT.
        assert_dis_snapshot!(
            positions: r#"
def f(c):
    return not (b" " <= c <= b"~")
"#
        )
    }

    #[test]
    fn type_param_scope_locations() {
        // codegen_type_params() emits type parameter ops at LOC(typeparam),
        // bound/default evaluators at LOC(e), and type alias plumbing at LOC(s).
        assert_dis_snapshot!(
            positions: r#"
type BoundGenericAlias[X: int] = set[X]
"#
        )
    }

    #[test]
    fn generic_typealias_wrapper_return_location() {
        // The generic parameters wrapper of a type alias returns at LOC(type alias).
        assert_dis_snapshot!(
            positions: r#"
type A[T] = T
"#
        )
    }

    #[test]
    fn generic_function_annotation_scope_location() {
        // codegen_function_annotations() gets LOC(function) inside the generic
        // parameters scope too.
        assert_dis_snapshot!(
            positions: r#"
def f[T](x: int): ...
"#
        )
    }

    #[test]
    fn generic_class_type_params_store_location() {
        // codegen_class() stores the .type_params cell at LOC(class).
        assert_dis_snapshot!(
            positions: r#"
def outer():
    class X[T]: ...
"#
        )
    }

    #[test]
    fn class_deferred_annotations_location() {
        // Deferred annotation setup and the class tail inherit the class body
        // location LOCATION(firstlineno, firstlineno, 0, 0).
        assert_dis_snapshot!(
            positions: r#"

class C:
    "doc"
    x: int
"#
        )
    }

    #[test]
    fn future_annotation_string_location() {
        // The stringized annotation is loaded at LOC(annotation) and stored at
        // LOC(AnnAssign).
        assert_dis_snapshot!(
            positions: r#"
from __future__ import annotations
class Bar:
    foo: Foo
"#
        )
    }

    #[test]
    fn lambda_dict_literal_location() {
        // codegen_dict() and codegen_subdict() use LOC(dict) for BUILD_MAP, MAP_ADD
        // and DICT_UPDATE.
        assert_dis_snapshot!(
            positions: r#"
f = lambda data: {'x': data}
g = lambda i: {**i}
"#
        )
    }

    #[test]
    fn match_or_alternative_locations() {
        // codegen_pattern_or() emits the COPY of each alternative at LOC(alt).
        assert_dis_snapshot!(
            positions: r#"
def f():
    x = False
    match 0:
        case 0 | 1 | 2 | 3:
            x = True
    return x
"#
        )
    }

    #[test]
    fn match_success_jump_location() {
        // codegen_match_inner() emits the success jump with NO_LOCATION.
        assert_dis_snapshot!(
            positions: r#"
def f(self):
    match 0:
        case 0:
            x = True
        case 0:
            x = False
    self.assertIs(x, True)
"#
        )
    }

    #[test]
    fn match_mapping_keys_location() {
        // codegen_pattern_mapping() returns to LOC(p) for the BUILD_TUPLE/MATCH_KEYS
        // scaffolding.
        assert_dis_snapshot!(
            positions: r#"
def f(self):
    x = {}
    y = None
    match x:
        case {0: 0}:
            y = 0
    self.assertIs(y, None)
"#
        )
    }

    #[test]
    fn match_class_scaffolding_location() {
        // codegen_pattern_class() returns to LOC(p) after visiting the class.
        assert_dis_snapshot!(
            positions: r#"
def f(x):
    match x:
        case bool(z):
            y = 0
    return y, z
"#
        )
    }

    #[test]
    fn taken_constant_boolop_location() {
        // The surviving load of a short-circuited BoolOp keeps the literal range.
        assert_dis_snapshot!(
            positions: r#"
def and_false(x):
    return False and x

def or_true(x):
    return True or x
"#
        )
    }

    #[test]
    fn assert_false_message_location() {
        // codegen_assert() emits LOAD_COMMON_CONSTANT and CALL at LOC(assert) and
        // RAISE_VARARGS at LOC(test).
        assert_dis_snapshot!(
            positions: r#"
def f():
    assert False, "x"
"#
        )
    }

    #[test]
    fn static_swap_implicit_return_location() {
        // Line numbers are resolved before apply_static_swaps() reorders the stores,
        // so the return epilogue keeps the location of the last store before the swap.
        assert_dis_snapshot!(
            positions: r#"
def f(a, b):
    a, b = a, b
    b, a = a, b
"#
        )
    }

    #[test]
    fn genexpr_closure_and_call_location() {
        // codegen_comprehension() uses LOC(e) for the closure, the outer CALL and the
        // implicit .0 load.
        assert_dis_snapshot!(
            positions: r#"
def f(parameters):
    return ((p, type(p)) for p in parameters)
"#
        )
    }

    #[test]
    fn implicit_call_genexpr_range() {
        // A sole unparenthesized generator expression argument spans the call
        // parentheses.
        assert_dis_snapshot!(
            positions: r#"
def implicit():
    return list(x for x in range(10))

def explicit():
    return list((x for x in range(10)))
"#
        )
    }

    #[test]
    fn implicit_call_genexpr_parenthesized_element_range() {
        // The call parentheses are part of the generator expression range even when
        // its element starts with a parenthesis.
        assert_dis_snapshot!(
            positions: r#"
def bytes_binop():
    return bytes((x ^ 0x5C) for x in range(256))

def dict_tuple(d):
    return dict((v, k) for (k, v) in d.items())

def plain_tuple_elt(xs):
    return list((x, y) for x, y in xs)

def explicit_gen(xs):
    return list(((x, y) for x, y in xs))
"#
        )
    }

    #[test]
    fn genexpr_filter_cleanup_location() {
        // jump_thread() copies the if_cleanup backedge location, elt_loc, to the
        // threaded cleanup path.
        assert_dis_snapshot!(
            positions: r#"
def simple(names):
    return (x for x in names if not _ishidden(x))

def boolop(fields):
    return (f for f in fields if f.init and not f.kw_only)
"#
        )
    }

    #[test]
    fn try_finally_exception_scaffolding_location() {
        // codegen_try_finally() emits the exception path scaffolding with NO_LOCATION.
        assert_dis_snapshot!(
            positions: r#"
def f(self, node):
    self.flag = True
    try:
        self.body(node)
    finally:
        self.flag = False
"#
        )
    }

    #[test]
    fn adjacent_no_location_entries() {
        // assemble_location_info() merges adjacent NO_LOCATION instructions into one
        // linetable entry.
        assert_dis_snapshot!(
            positions: r#"
def f(file):
    if sys.platform == "win32":
        try:
            import nt
            if not nt._supports_virtual_terminal():
                return False
        except (ImportError, AttributeError):
            return False
    try:
        return os.isatty(file.fileno())
    except OSError:
        return hasattr(file, "isatty") and file.isatty()
"#
        )
    }

    #[test]
    fn fstring_format_location() {
        // codegen_formatted_value() emits CONVERT_VALUE and FORMAT_* at
        // LOC(FormattedValue).
        assert_dis_snapshot!(
            positions: r#"
def simple(self):
    return f'{self.value}'

def spec(x):
    return f'{x!r:>3}'
"#
        )
    }

    #[test]
    fn joined_string_literals_location() {
        // Adjacent f-string literal fragments are one Constant spanning all of them.
        assert_dis_snapshot!(
            positions: r#"
def f(a):
    return (
        'x'
        f'y{a}z'
        'w'
    )
"#
        )
    }

    #[test]
    fn decorated_definition_locations() {
        // Decorators are evaluated first, the closure and the store use LOC(s), and
        // each decorator CALL uses the decorator location.
        assert_dis_snapshot!(
            positions: r#"
def dec(f): return f

class C:
    @dec
    def f(self):
        yield

@dec
class D:
    pass

class E:
    @dec
    def g(self, flags: int, /) -> memoryview:
        raise NotImplementedError
"#
        )
    }

    #[test]
    fn builtin_any_genexpr_call() {
        // any(genexpr) is inlined behind a builtin identity check that falls back to
        // the ordinary call.
        assert_dis_snapshot!(
            positions: r#"
def f(xs):
    return any(x for x in xs)
"#
        )
    }

    #[test]
    fn builtin_tuple_list_set_genexpr_calls() {
        // tuple(), list() and set() of a generator expression are inlined behind a
        // builtin identity check that falls back to the ordinary call.
        assert_dis_snapshot!(
            positions: r#"
def tuple_f(xs):
    return tuple(x for x in xs)

def list_f(xs):
    return list(x for x in xs)

def set_f(xs):
    return set(x for x in xs)
"#
        )
    }

    #[test]
    fn augassign_constant_slice_location() {
        // codegen_augassign() emits the COPY/COPY/BINARY_OP NB_SUBSCR at LOC(target).
        assert_dis_snapshot!(
            positions: r#"
def aug_const(x, y):
    x[1:2] += y
"#
        )
    }

    #[test]
    fn comprehension_list_iterable_location() {
        // codegen_comprehension_iter() emits GET_ITER at LOC(comp->iter).
        assert_dis_snapshot!(
            positions: r#"
async def f(i):
    return i

async def run_list():
    return [await c for c in [f(1), f(41)]]
"#
        )
    }

    #[test]
    fn inlined_comprehension_backedge_location() {
        // codegen_sync_comprehension_generator() emits the backedges at elt_loc.
        assert_dis_snapshot!(
            positions: r#"
async def f(i):
    return i

async def run_list():
    return [s for c in [f(''), f('abc')] for s in await c]
"#
        )
    }

    #[test]
    fn inlined_async_comprehension_end_async_for_location() {
        // codegen_async_comprehension_generator() emits END_ASYNC_FOR at the
        // comprehension location.
        assert_dis_snapshot!(
            positions: r#"
async def f(it):
    for i in it:
        yield i

async def run_list():
    return [i + 1 async for i in f([10, 20])]
"#
        )
    }

    #[test]
    fn async_for_anext_location() {
        // codegen_async_for() emits the GET_ANEXT/yield-from scaffolding at LOC(s).
        assert_dis_snapshot!(
            positions: r#"
async def f(source, buffer):
    async for i1, i2 in source():
        buffer.append(i1 + i2)
"#
        )
    }

    #[test]
    fn namedexpr_copy_location() {
        // The COPY of a named expression is at its location, between the value and
        // the target.
        assert_dis_snapshot!(
            positions: r#"
def outer():
    a = 10
    def spam():
        nonlocal a
        (a := 20)
"#
        )
    }

    #[test]
    fn with_cleanup_excludes_not_taken() {
        // The NOT_TAKEN after the with body is outside the exception table entries.
        assert_dis_snapshot!(
            positions: r#"
def f(c):
    with c:
        x = 1
    y = 2
"#
        )
    }

    #[test]
    fn async_for_end_async_for() {
        // END_ASYNC_FOR jumps back to the END_SEND of its awaitable loop.
        assert_dis_snapshot!(
            positions: r#"
async def f(xs):
    async for x in xs:
        pass
"#
        )
    }

    #[test]
    fn genexpr_prefix() {
        // The outermost iterator is acquired before RETURN_GENERATOR, outside the
        // StopIteration handler.
        assert_dis_snapshot!(
            positions: r#"
def f(xs):
    return (x for x in xs)

async def g(xs):
    return (x async for x in xs)
"#
        )
    }

    #[test]
    fn yield_from_and_await() {
        // GET_ITER and GET_AWAITABLE leave the iterator pair that SEND uses.
        assert_dis_snapshot!(
            positions: r#"
def f(xs):
    yield from xs

async def g(xs):
    return await xs
"#
        )
    }

    #[test]
    fn comprehension_collection_depth() {
        // The collection depth counts the slots of every nested iterator.
        assert_dis_snapshot!(
            positions: r#"
def f(xs, ys):
    return [x for x in xs for y in ys]

async def g(xs, ys):
    return [x async for x in xs for y in ys]
"#
        )
    }

    #[test]
    fn import_flags() {
        // The IMPORT_NAME oparg flags depend on the scope and the enclosing handlers.
        assert_dis_snapshot!(
            positions: r#"
import sample
from sample import member
from sample import *

def f():
    import sample

class C:
    from sample import member

try:
    import sample
except Exception:
    pass

try:
    operation()
except Exception:
    import sample

try:
    operation()
finally:
    import sample

with manager:
    import sample
"#
        )
    }

    #[test]
    fn loop_return_cleanup() {
        // A return pops two slots for a for loop and one for an async for loop.
        assert_dis_snapshot!(
            positions: r#"
def f(xs):
    for x in xs:
        return x

async def g(xs):
    async for x in xs:
        return x
"#
        )
    }

    #[test]
    fn list_set_async_genexpr_calls() {
        // list() and set() of an async generator expression stay ordinary calls.
        assert_dis_snapshot!(
            positions: r#"
async def f(xs):
    return list(x async for x in xs)

async def g(xs):
    return set(x async for x in xs)
"#
        )
    }
}

//! Builtin function definitions.
//!
//! Implements the list of [builtin Python functions](https://docs.python.org/3/library/builtins.html).
use crate::{Py, VirtualMachine, builtins::PyModule, class::PyClassImpl};
pub(crate) use builtins::{DOC, module_def};
pub use builtins::{ascii, print, reversed};

#[pymodule]
mod builtins {
    use crate::{
        AsObject, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject, VirtualMachine,
        builtins::{
            PyByteArray, PyBytes, PyDictRef, PyStr, PyStrRef, PyTuple, PyTupleRef, PyType,
            PyUtf8StrRef,
            enumerate::PyReverseSequenceIterator,
            function::{PyCell, PyCellRef, PyFunction},
            int::PyIntRef,
            iter::PyCallableIterator,
            list::{PyList, SortOptions},
        },
        bytecode,
        common::hash::PyHash,
        function::{
            ArgBytesLike, ArgCallable, ArgIndex, ArgIntoBool, ArgIterable, ArgMapping,
            ArgPrimitiveIndex, ArgStrOrBytesLike, Either, FsPath, FuncArgs, KwArgs, OptionalArg,
            OptionalOption, PosArgs,
        },
        protocol::{PyIter, PyIterReturn},
        py_io,
        readline::{Readline, ReadlineResult},
        stdlib::sys,
        types::PyComparisonOp,
        vm::compile_mode::{
            CompilerFlags, PY_EVAL_INPUT, PY_FILE_INPUT, PY_FUNC_TYPE_INPUT, PY_SINGLE_INPUT,
            compile_future_feature_mask, compile_future_features_from_flags,
        },
    };
    use itertools::Itertools;
    use num_traits::{Signed, ToPrimitive};
    use rustpython_common::wtf8::CodePoint;

    #[cfg(not(feature = "rustpython-compiler"))]
    const CODEGEN_NOT_SUPPORTED: &str =
        "can't compile() to bytecode when the `codegen` feature of rustpython is disabled";

    #[pyfunction]
    fn abs(x: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._abs(&x)
    }

    #[pyfunction]
    fn all(iterable: ArgIterable<ArgIntoBool>, vm: &VirtualMachine) -> PyResult<bool> {
        for item in iterable.iter(vm)? {
            if !item?.into_bool() {
                return Ok(false);
            }
        }
        Ok(true)
    }

    #[pyfunction]
    fn any(iterable: ArgIterable<ArgIntoBool>, vm: &VirtualMachine) -> PyResult<bool> {
        for item in iterable.iter(vm)? {
            if item?.into_bool() {
                return Ok(true);
            }
        }
        Ok(false)
    }

    #[pyfunction]
    pub fn ascii(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        obj.ascii(vm)
    }

    #[pyfunction]
    fn bin(x: PyIntRef) -> String {
        let x = x.as_bigint();
        if x.is_negative() {
            format!("-0b{:b}", x.abs())
        } else {
            format!("0b{x:b}")
        }
    }

    #[pyfunction]
    fn callable(obj: PyObjectRef) -> bool {
        obj.is_callable()
    }

    #[pyfunction]
    fn chr(i: PyIntRef, vm: &VirtualMachine) -> PyResult<CodePoint> {
        let value = i
            .as_bigint()
            .to_u32()
            .and_then(CodePoint::from_u32)
            .ok_or_else(|| vm.new_value_error("chr() arg not in range(0x110000)"))?;
        Ok(value)
    }

    #[derive(FromArgs)]
    #[allow(dead_code)]
    struct CompileArgs {
        source: PyObjectRef,
        // Resolved to FsPath at the start of compile() so that bytearray /
        // memoryview / other buffer-protocol objects raise TypeError, matching
        // CPython's PyUnicode_FSDecoder (str / bytes / __fspath__ only).
        filename: PyObjectRef,
        mode: PyUtf8StrRef,
        // CPython parity: flags / optimize accept any object with __index__,
        // not just exact int. Matches the argument conversion used by
        // builtin_compile_impl.
        #[pyarg(any, optional)]
        flags: OptionalArg<ArgPrimitiveIndex<i32>>,
        // CPython parity: dont_inherit goes through PyObject_IsTrue, so
        // arbitrary objects with `__bool__` are accepted (and any exception
        // raised inside `__bool__` propagates) — not the strict bool type.
        #[pyarg(any, optional)]
        dont_inherit: OptionalArg<ArgIntoBool>,
        #[pyarg(any, optional)]
        optimize: OptionalArg<ArgPrimitiveIndex<i32>>,
        #[pyarg(named, optional)]
        _feature_version: OptionalArg<i32>,
    }

    fn merge_compile_future_features(
        flags: i32,
        dont_inherit: bool,
        vm: &VirtualMachine,
    ) -> bytecode::CodeFlags {
        let mut future_features = compile_future_features_from_flags(flags);
        if !dont_inherit && let Some(frame) = vm.current_frame() {
            future_features |= bytecode::CodeFlags::from_bits_truncate(
                frame.code.flags.bits() & compile_future_feature_mask().bits(),
            );
        }
        future_features
    }

    fn audit_compile_source(vm: &VirtualMachine, source: &[u8], filename: &str) -> PyResult<()> {
        vm.sys_module.get_attr("audit", vm)?.call(
            (
                vm.ctx.new_str("compile"),
                vm.ctx.new_bytes(source.to_vec()),
                vm.ctx.new_str(filename),
            ),
            vm,
        )?;
        Ok(())
    }

    fn trim_eval_source_bytes(mut source: &[u8]) -> &[u8] {
        while let Some((&first, rest)) = source.split_first()
            && matches!(first, b' ' | b'\t')
        {
            source = rest;
        }
        source
    }

    fn decode_eval_exec_source_bytes(
        vm: &VirtualMachine,
        source: &[u8],
        filename: &str,
    ) -> PyResult<String> {
        #[cfg(feature = "parser")]
        {
            vm.decode_source_bytes(source, filename, false)
        }
        #[cfg(not(feature = "parser"))]
        {
            _ = filename;
            core::str::from_utf8(source)
                .map(str::to_owned)
                .map_err(|err| {
                    let msg = format!(
                        "(unicode error) 'utf-8' codec can't decode byte 0x{:x?} in position {}: invalid start byte",
                        source[err.valid_up_to()],
                        err.valid_up_to()
                    );
                    vm.new_exception_msg(vm.ctx.exceptions.syntax_error.to_owned(), msg.into())
                })
        }
    }

    #[cfg(any(feature = "parser", feature = "compiler"))]
    #[pyfunction]
    fn compile(args: CompileArgs, vm: &VirtualMachine) -> PyResult {
        #[cfg(not(feature = "ast"))]
        {
            _ = args; // to disable unused warning
            return Err(vm.new_type_error("AST Not Supported"));
        }
        #[cfg(feature = "ast")]
        {
            // CPython parity: PyUnicode_FSDecoder accepts only str / bytes /
            // __fspath__-bearing objects. Reject buffer-protocol types like
            // bytearray and memoryview that would otherwise pass through
            // `FsPath::TryFromObject`'s permissive fallback.
            let filename = FsPath::try_from_path_like(args.filename, true, vm)?;

            use crate::{class::PyClassImpl, stdlib::_ast};

            let feature_version = args._feature_version.into_option().unwrap_or(-1);

            let mode_str = args.mode.as_str();
            let flags: i32 = args.flags.map_or(0, |v| v.value);
            let cf = CompilerFlags::from_bits_retain(flags);

            if (flags & !CompilerFlags::ALLOWED_FLAGS.bits()) != 0 {
                return Err(vm.new_value_error("compile(): unrecognised flags"));
            }

            let optimize: i32 = args.optimize.map_or(-1, |v| v.value);
            let optimize: u8 = match optimize {
                -1 => vm.state.config.settings.optimize.min(2),
                0..=2 => optimize as u8,
                _ => return Err(vm.new_value_error("compile(): invalid optimize value")),
            };
            let dont_inherit = args.dont_inherit.map_or(false, ArgIntoBool::into_bool);
            let is_ast_only = cf.contains(CompilerFlags::ONLY_AST);
            let future_features = merge_compile_future_features(flags, dont_inherit, vm);

            let start = if mode_str == "exec" {
                PY_FILE_INPUT
            } else if mode_str == "eval" {
                PY_EVAL_INPUT
            } else if mode_str == "single" {
                PY_SINGLE_INPUT
            } else if mode_str == "func_type" {
                if !is_ast_only {
                    return Err(vm.new_value_error(
                        "compile() mode 'func_type' requires flag PyCF_ONLY_AST",
                    ));
                }
                PY_FUNC_TYPE_INPUT
            } else {
                let msg = if is_ast_only {
                    "compile() mode must be 'exec', 'eval', 'single' or 'func_type'"
                } else {
                    "compile() mode must be 'exec', 'eval' or 'single'"
                };
                return Err(vm.new_value_error(msg));
            };

            let ast_type = _ast::NodeAst::make_static_type().as_object().to_owned();
            if args.source.is_instance(&ast_type, vm)? {
                let explicit_future_annotations =
                    future_features.contains(bytecode::CodeFlags::FUTURE_ANNOTATIONS);
                vm.sys_module.get_attr("audit", vm)?.call(
                    (
                        vm.ctx.new_str("compile"),
                        args.source.clone(),
                        vm.ctx.none(),
                    ),
                    vm,
                )?;

                // compile(ast_node, ..., PyCF_ONLY_AST) returns the AST after validation
                if is_ast_only {
                    let (expected_type, expected_name) = _ast::mode_type_and_name(mode_str)
                        .ok_or_else(|| {
                            vm.new_value_error(
                                "compile() mode must be 'exec', 'eval', 'single' or 'func_type'",
                            )
                        })?;
                    if !args.source.is_instance(expected_type.as_object(), vm)? {
                        return Err(vm.new_type_error(format!(
                            "expected {} node, got {}",
                            expected_name,
                            args.source.class().name()
                        )));
                    }
                    #[cfg(not(feature = "rustpython-codegen"))]
                    {
                        _ast::validate_ast_object(vm, args.source.clone())?;
                        return Ok(args.source);
                    }
                    #[cfg(feature = "rustpython-codegen")]
                    {
                        return _ast::preprocess_ast_object(
                            vm,
                            args.source,
                            &filename.to_string_lossy(),
                            optimize,
                            cf.contains(CompilerFlags::OPTIMIZED_AST),
                            explicit_future_annotations,
                        );
                    }
                }

                #[cfg(not(feature = "rustpython-codegen"))]
                {
                    return Err(vm.new_type_error(CODEGEN_NOT_SUPPORTED));
                }
                #[cfg(feature = "rustpython-codegen")]
                {
                    let (expected_type, expected_name) = _ast::mode_type_and_name(mode_str)
                        .ok_or_else(|| {
                            vm.new_value_error("compile() mode must be 'exec', 'eval' or 'single'")
                        })?;
                    if !args.source.is_instance(expected_type.as_object(), vm)? {
                        return Err(vm.new_type_error(format!(
                            "expected {} node, got {}",
                            expected_name,
                            args.source.class().name()
                        )));
                    }
                    let mode = mode_str
                        .parse::<crate::compiler::Mode>()
                        .map_err(|err| vm.new_value_error(err.to_string()))?;
                    let mut opts = vm.compile_opts();
                    opts.optimize = optimize;
                    opts.allow_top_level_await = cf.contains(CompilerFlags::ALLOW_TOP_LEVEL_AWAIT);
                    opts.future_features = future_features;
                    return _ast::compile(vm, args.source, &filename.to_string_lossy(), mode, opts);
                }
            }

            #[cfg(not(feature = "parser"))]
            {
                Err(vm.new_type_error(
                    "can't compile() source code when the `parser` feature of rustpython is disabled",
                ))
            }
            #[cfg(feature = "parser")]
            {
                let source = ArgStrOrBytesLike::try_from_object(vm, args.source)?;

                let mut compile_flags = flags | future_features.bits() as i32;
                #[cfg(feature = "rustpython-compiler")]
                let compile_source = |source: &[u8], compile_flags: i32| {
                    vm.compile_string_object_with_flags(
                        source,
                        &filename.to_string_lossy(),
                        start,
                        compile_flags,
                        feature_version,
                        optimize as i32,
                    )
                };
                match &source {
                    ArgStrOrBytesLike::Str(source) => {
                        if source.as_bytes().contains(&0) {
                            return Err(vm.new_exception_msg(
                                vm.ctx.exceptions.syntax_error.to_owned(),
                                "source code string cannot contain null bytes".into(),
                            ));
                        }
                        audit_compile_source(
                            vm,
                            source.as_bytes(),
                            filename.to_string_lossy().as_ref(),
                        )?;
                        compile_flags |= CompilerFlags::IGNORE_COOKIE.bits();
                        #[cfg(feature = "rustpython-compiler")]
                        {
                            compile_source(source.as_bytes(), compile_flags)
                        }
                        #[cfg(not(feature = "rustpython-compiler"))]
                        {
                            Err(vm.new_value_error(CODEGEN_NOT_SUPPORTED))
                        }
                    }
                    ArgStrOrBytesLike::Buf(source) => {
                        let source_bytes = source.borrow_buf();
                        let source_bytes: &[u8] = &source_bytes;
                        if source_bytes.contains(&0) {
                            return Err(vm.new_exception_msg(
                                vm.ctx.exceptions.syntax_error.to_owned(),
                                "source code string cannot contain null bytes".into(),
                            ));
                        }
                        audit_compile_source(
                            vm,
                            source_bytes,
                            filename.to_string_lossy().as_ref(),
                        )?;
                        #[cfg(feature = "rustpython-compiler")]
                        {
                            compile_source(source_bytes, compile_flags)
                        }
                        #[cfg(not(feature = "rustpython-compiler"))]
                        {
                            Err(vm.new_value_error(CODEGEN_NOT_SUPPORTED))
                        }
                    }
                }
            }
        }
    }

    #[pyfunction]
    fn delattr(obj: PyObjectRef, attr: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let attr = attr.try_to_ref::<PyStr>(vm).map_err(|_e| {
            vm.new_type_error(format!(
                "attribute name must be string, not '{}'",
                attr.class().name()
            ))
        })?;
        obj.del_attr(attr, vm)
    }

    #[pyfunction]
    fn dir(obj: OptionalArg<PyObjectRef>, vm: &VirtualMachine) -> PyResult<PyList> {
        vm.dir(obj.into_option())
    }

    #[pyfunction]
    fn divmod(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._divmod(&a, &b)
    }

    #[derive(FromArgs)]
    struct ScopeArgs {
        #[pyarg(any, default)]
        globals: Option<PyObjectRef>,
        #[pyarg(any, default)]
        locals: Option<ArgMapping>,
    }

    impl ScopeArgs {
        fn make_scope(
            self,
            vm: &VirtualMachine,
            func_name: &'static str,
        ) -> PyResult<crate::scope::Scope> {
            fn validate_globals_dict(
                globals: &PyObject,
                vm: &VirtualMachine,
                func_name: &'static str,
            ) -> PyResult<()> {
                if !globals.fast_isinstance(vm.ctx.types.dict_type) {
                    return Err(match func_name {
                        "eval" => {
                            let is_mapping = globals.mapping_unchecked().check();
                            vm.new_type_error(if is_mapping {
                                "globals must be a real dict; try eval(expr, {}, mapping)"
                            } else {
                                "globals must be a dict"
                            })
                        }
                        "exec" => vm.new_type_error(format!(
                            "exec() globals must be a dict, not {}",
                            globals.class().name()
                        )),
                        _ => vm.new_type_error("globals must be a dict"),
                    });
                }
                Ok(())
            }

            let (globals, locals) = match self.globals {
                Some(globals) => {
                    validate_globals_dict(&globals, vm, func_name)?;

                    let globals = PyDictRef::try_from_object(vm, globals)?;
                    if !globals.contains_key(identifier!(vm, __builtins__), vm) {
                        let builtins_dict = vm.builtins.dict().into();
                        globals.set_item(identifier!(vm, __builtins__), builtins_dict, vm)?;
                    }
                    (
                        globals.clone(),
                        self.locals
                            .unwrap_or_else(|| ArgMapping::from_dict_exact(globals.clone())),
                    )
                }
                None => (
                    vm.current_globals(),
                    if let Some(locals) = self.locals {
                        locals
                    } else {
                        vm.current_locals()?
                    },
                ),
            };

            let scope = crate::scope::Scope::with_builtins(Some(locals), globals, vm);
            Ok(scope)
        }
    }

    #[derive(FromArgs)]
    struct ExecArgs {
        #[pyarg(positional)]
        source: Either<ArgStrOrBytesLike, PyRef<crate::builtins::PyCode>>,
        #[pyarg(any, default)]
        globals: Option<PyObjectRef>,
        #[pyarg(any, default)]
        locals: Option<ArgMapping>,
        #[pyarg(named, optional)]
        closure: OptionalOption<PyObjectRef>,
    }

    fn exec_closure(
        code_obj: &PyRef<crate::builtins::PyCode>,
        closure: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyRef<PyTuple<PyCellRef>>>> {
        let num_free = code_obj.freevars.len();
        let Some(closure) = closure else {
            if num_free == 0 {
                return Ok(None);
            }
            return Err(vm.new_type_error(format!(
                "code object requires a closure of exactly length {num_free}"
            )));
        };

        if num_free == 0 {
            return Err(vm.new_type_error("cannot use a closure with this code object"));
        }

        let closure_tuple = closure
            .downcast_exact::<PyTuple>(vm)
            .map_err(|_| {
                vm.new_type_error(format!(
                    "code object requires a closure of exactly length {num_free}"
                ))
            })?
            .into_pyref();
        if closure_tuple.len() != num_free {
            return Err(vm.new_type_error(format!(
                "code object requires a closure of exactly length {num_free}"
            )));
        }

        closure_tuple
            .try_into_typed::<PyCell>(vm)
            .map(Some)
            .map_err(|_| {
                vm.new_type_error(format!(
                    "code object requires a closure of exactly length {num_free}"
                ))
            })
    }

    #[pyfunction]
    fn eval(
        source: Either<ArgStrOrBytesLike, PyRef<crate::builtins::PyCode>>,
        scope: ScopeArgs,
        vm: &VirtualMachine,
    ) -> PyResult {
        let scope = scope.make_scope(vm, "eval")?;

        // source as string
        let code = match source {
            Either::A(either) => {
                let source = match &either {
                    ArgStrOrBytesLike::Str(source) => {
                        if source.as_bytes().contains(&0) {
                            return Err(vm.new_exception_msg(
                                vm.ctx.exceptions.syntax_error.to_owned(),
                                "source code string cannot contain null bytes".into(),
                            ));
                        }
                        let source = source.expect_str().trim_start_matches([' ', '\t']);
                        audit_compile_source(vm, source.as_bytes(), "<string>")?;
                        source.to_owned()
                    }
                    ArgStrOrBytesLike::Buf(source) => {
                        let source: &[u8] = &source.borrow_buf();
                        if source.contains(&0) {
                            return Err(vm.new_exception_msg(
                                vm.ctx.exceptions.syntax_error.to_owned(),
                                "source code string cannot contain null bytes".into(),
                            ));
                        }
                        let source = trim_eval_source_bytes(source);
                        audit_compile_source(vm, source, "<string>")?;
                        decode_eval_exec_source_bytes(vm, source, "eval")?
                    }
                };
                Ok(Either::A(vm.ctx.new_utf8_str(source)))
            }
            Either::B(code) => Ok(Either::B(code)),
        }?;
        run_code(vm, code, scope, crate::compiler::Mode::Eval, "eval", None)
    }

    #[pyfunction]
    fn exec(args: ExecArgs, vm: &VirtualMachine) -> PyResult {
        let ExecArgs {
            source,
            globals,
            locals,
            closure,
        } = args;
        let scope = ScopeArgs { globals, locals }.make_scope(vm, "exec")?;
        let closure = closure.flatten();
        let (source, closure) = match source {
            Either::A(either) => {
                if closure.is_some() {
                    return Err(
                        vm.new_type_error("closure can only be used when source is a code object")
                    );
                }
                let source = match &either {
                    ArgStrOrBytesLike::Str(source) => {
                        if source.as_bytes().contains(&0) {
                            return Err(vm.new_exception_msg(
                                vm.ctx.exceptions.syntax_error.to_owned(),
                                "source code string cannot contain null bytes".into(),
                            ));
                        }
                        audit_compile_source(vm, source.as_bytes(), "<string>")?;
                        source.expect_str().to_owned()
                    }
                    ArgStrOrBytesLike::Buf(source) => {
                        let source: &[u8] = &source.borrow_buf();
                        if source.contains(&0) {
                            return Err(vm.new_exception_msg(
                                vm.ctx.exceptions.syntax_error.to_owned(),
                                "source code string cannot contain null bytes".into(),
                            ));
                        }
                        audit_compile_source(vm, source, "<string>")?;
                        decode_eval_exec_source_bytes(vm, source, "exec")?
                    }
                };
                (Either::A(vm.ctx.new_utf8_str(source)), None)
            }
            Either::B(code) => {
                let closure = exec_closure(&code, closure, vm)?;
                (Either::B(code), closure)
            }
        };
        run_code(
            vm,
            source,
            scope,
            crate::compiler::Mode::Exec,
            "exec",
            closure,
        )
    }

    fn run_code(
        vm: &VirtualMachine,
        source: Either<PyUtf8StrRef, PyRef<crate::builtins::PyCode>>,
        scope: crate::scope::Scope,
        #[allow(unused_variables)] mode: crate::compiler::Mode,
        func: &str,
        closure: Option<PyRef<PyTuple<PyCellRef>>>,
    ) -> PyResult {
        // Determine code object:
        let code_obj = match source {
            #[cfg(feature = "rustpython-compiler")]
            Either::A(string) => {
                let source = string.as_str();
                let mut opts = vm.compile_opts();
                if let Some(frame) = vm.current_frame() {
                    opts.future_features = bytecode::CodeFlags::from_bits_truncate(
                        frame.code.flags.bits() & compile_future_feature_mask().bits(),
                    );
                }
                vm.compile_with_opts(source, mode, "<string>", opts)
                    .map_err(|err| err.into_pyexception(vm, Some(source)))?
            }
            #[cfg(not(feature = "rustpython-compiler"))]
            Either::A(_) => return Err(vm.new_type_error(CODEGEN_NOT_SUPPORTED)),
            Either::B(code_obj) => code_obj,
        };

        vm.sys_module
            .get_attr("audit", vm)?
            .call((vm.ctx.new_str("exec"), code_obj.clone()), vm)?;

        if closure.is_none() && !code_obj.freevars.is_empty() {
            return Err(vm.new_type_error(format!(
                "code object passed to {func}() may not contain free variables"
            )));
        }

        // Run the code:
        vm.run_code_obj_with_closure(code_obj, scope, closure)
    }

    #[pyfunction]
    fn format(
        value: PyObjectRef,
        format_spec: OptionalArg<PyStrRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyStrRef> {
        vm.format(&value, format_spec.unwrap_or(vm.ctx.new_str("")))
    }

    #[pyfunction]
    fn getattr(
        obj: PyObjectRef,
        attr: PyObjectRef,
        default: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let attr = attr.try_to_ref::<PyStr>(vm).map_err(|_e| {
            vm.new_type_error(format!(
                "attribute name must be string, not '{}'",
                attr.class().name()
            ))
        })?;

        if let OptionalArg::Present(default) = default {
            Ok(vm.get_attribute_opt(obj, attr)?.unwrap_or(default))
        } else {
            obj.get_attr(attr, vm)
        }
    }

    #[pyfunction]
    fn globals(vm: &VirtualMachine) -> PyDictRef {
        vm.current_globals()
    }

    #[pyfunction]
    fn hasattr(obj: PyObjectRef, attr: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        let attr = attr.try_to_ref::<PyStr>(vm).map_err(|_e| {
            vm.new_type_error(format!(
                "attribute name must be string, not '{}'",
                attr.class().name()
            ))
        })?;
        Ok(vm.get_attribute_opt(obj, attr)?.is_some())
    }

    #[pyfunction]
    fn hash(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyHash> {
        obj.hash(vm)
    }

    #[pyfunction]
    fn breakpoint(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        match vm
            .sys_module
            .get_attr(vm.ctx.intern_str("breakpointhook"), vm)
        {
            Ok(hook) => hook.as_ref().call(args, vm),
            Err(_) => Err(vm.new_runtime_error("lost sys.breakpointhook")),
        }
    }

    #[pyfunction]
    fn hex(number: ArgIndex) -> String {
        let number = number.into_int_ref();
        let n = number.as_bigint();
        format!("{n:#x}")
    }

    #[pyfunction]
    fn id(obj: PyObjectRef) -> usize {
        obj.get_id()
    }

    #[pyfunction]
    fn input(prompt: OptionalArg<PyStrRef>, vm: &VirtualMachine) -> PyResult {
        use std::io::IsTerminal;

        let stdin = sys::get_stdin(vm)?;
        let stdout = sys::get_stdout(vm)?;
        let stderr = sys::get_stderr(vm)?;

        let _ = vm.call_method(&stderr, "flush", ());

        let fd_matches = |obj, expected| {
            vm.call_method(obj, "fileno", ())
                .and_then(|o| i64::try_from_object(vm, o))
                .is_ok_and(|fd| fd == expected)
        };

        // Check if we should use rustyline (interactive terminal, not PTY child)
        let use_rustyline = fd_matches(&stdin, 0)
            && fd_matches(&stdout, 1)
            && std::io::stdin().is_terminal()
            && !is_pty_child();

        // Disable rustyline if prompt contains surrogates (not valid UTF-8 for terminal)
        let prompt_str = match &prompt {
            OptionalArg::Present(s) => s.to_str(),
            OptionalArg::Missing => Some(""),
        };
        let use_rustyline = use_rustyline && prompt_str.is_some();

        if use_rustyline {
            let prompt = prompt_str.unwrap();
            let mut readline = Readline::new(());
            match readline.readline(prompt) {
                ReadlineResult::Line(s) => Ok(vm.ctx.new_str(s).into()),
                ReadlineResult::Eof => {
                    Err(vm.new_exception_empty(vm.ctx.exceptions.eof_error.to_owned()))
                }
                ReadlineResult::Interrupt => {
                    Err(vm.new_exception_empty(vm.ctx.exceptions.keyboard_interrupt.to_owned()))
                }
                ReadlineResult::Io(e) => Err(vm.new_os_error(e.to_string())),
                #[cfg(unix)]
                ReadlineResult::OsError(num) => Err(vm.new_os_error(num)),
                ReadlineResult::Other(e) => Err(vm.new_runtime_error(e.to_string())),
            }
        } else {
            if let OptionalArg::Present(prompt) = prompt {
                vm.call_method(&stdout, "write", (prompt,))?;
            }
            let _ = vm.call_method(&stdout, "flush", ());
            py_io::file_readline(&stdin, None, vm)
        }
    }

    /// Check if we're running in a PTY child process (e.g., after pty.fork()).
    /// pty.fork() calls setsid(), making the child a session leader.
    /// In this case, rustyline may hang because it uses raw mode.
    #[cfg(unix)]
    fn is_pty_child() -> bool {
        crate::host_env::posix::is_session_leader()
    }

    #[cfg(not(unix))]
    fn is_pty_child() -> bool {
        false
    }

    #[pyfunction]
    fn isinstance(obj: PyObjectRef, typ: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        obj.is_instance(&typ, vm)
    }

    #[pyfunction]
    fn issubclass(subclass: PyObjectRef, typ: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        subclass.is_subclass(&typ, vm)
    }

    #[pyfunction]
    fn iter(
        iter_target: PyObjectRef,
        sentinel: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyIter> {
        if let OptionalArg::Present(sentinel) = sentinel {
            let callable = ArgCallable::try_from_object(vm, iter_target)?;
            let iterator = PyCallableIterator::new(callable, sentinel)
                .into_ref(&vm.ctx)
                .into();
            Ok(PyIter::new(iterator))
        } else {
            iter_target.get_iter(vm)
        }
    }

    #[pyfunction]
    fn aiter(iter_target: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        iter_target.get_aiter(vm)
    }

    #[pyfunction]
    fn anext(
        aiter: PyObjectRef,
        default_value: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        use crate::builtins::asyncgenerator::PyAnextAwaitable;

        // Check if object is an async iterator (has __anext__ method)
        if !aiter.class().has_attr(identifier!(vm, __anext__)) {
            return Err(vm.new_type_error(format!(
                "'{}' object is not an async iterator",
                aiter.class().name()
            )));
        }

        let awaitable = vm.call_method(&aiter, "__anext__", ())?;

        if let OptionalArg::Present(default) = default_value {
            Ok(PyAnextAwaitable::new(awaitable, default)
                .into_ref(&vm.ctx)
                .into())
        } else {
            Ok(awaitable)
        }
    }

    #[pyfunction]
    fn len(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
        obj.length(vm)
    }

    #[pyfunction]
    fn locals(vm: &VirtualMachine) -> PyResult<ArgMapping> {
        vm.current_locals()
    }

    fn min_or_max(
        mut args: FuncArgs,
        vm: &VirtualMachine,
        func_name: &str,
        op: PyComparisonOp,
    ) -> PyResult {
        let default = args.take_keyword("default");
        let key_func = args.take_keyword("key");

        if let Some(err) = args.check_kwargs_empty(vm) {
            return Err(err);
        }

        let candidates = match args.args.len().cmp(&1) {
            core::cmp::Ordering::Greater => {
                if default.is_some() {
                    return Err(vm.new_type_error(format!(
                        "Cannot specify a default for {func_name}() with multiple positional arguments"
                    )));
                }
                args.args
            }
            core::cmp::Ordering::Equal => args.args[0].try_to_value(vm)?,
            core::cmp::Ordering::Less => {
                // zero arguments means type error:
                return Err(
                    vm.new_type_error(format!("{func_name} expected at least 1 argument, got 0"))
                );
            }
        };

        let mut candidates_iter = candidates.into_iter();
        let mut x = match candidates_iter.next() {
            Some(x) => x,
            None => {
                return default.ok_or_else(|| {
                    vm.new_value_error(format!("{func_name}() iterable argument is empty"))
                });
            }
        };

        let key_func = key_func.filter(|f| !vm.is_none(f));
        if let Some(ref key_func) = key_func {
            let mut x_key = key_func.call((x.clone(),), vm)?;
            for y in candidates_iter {
                let y_key = key_func.call((y.clone(),), vm)?;
                if y_key.rich_compare_bool(&x_key, op, vm)? {
                    x = y;
                    x_key = y_key;
                }
            }
        } else {
            for y in candidates_iter {
                if y.rich_compare_bool(&x, op, vm)? {
                    x = y;
                }
            }
        }

        Ok(x)
    }

    #[pyfunction]
    fn max(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        min_or_max(args, vm, "max", PyComparisonOp::Gt)
    }

    #[pyfunction]
    fn min(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        min_or_max(args, vm, "min", PyComparisonOp::Lt)
    }

    #[pyfunction]
    fn next(
        iterator: PyObjectRef,
        default_value: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyIterReturn> {
        if !PyIter::check(&iterator) {
            return Err(vm.new_type_error(format!(
                "{} object is not an iterator",
                iterator.class().name()
            )));
        }
        PyIter::new(iterator)
            .next(vm)
            .map(|iter_ret| match iter_ret {
                PyIterReturn::Return(obj) => PyIterReturn::Return(obj),
                PyIterReturn::StopIteration(v) => {
                    default_value.map_or(PyIterReturn::StopIteration(v), PyIterReturn::Return)
                }
            })
    }

    #[pyfunction]
    fn oct(number: ArgIndex, vm: &VirtualMachine) -> PyObjectRef {
        let number = number.into_int_ref();
        let n = number.as_bigint();
        let s = if n.is_negative() {
            format!("-0o{:o}", n.abs())
        } else {
            format!("0o{n:o}")
        };

        vm.ctx.new_str(s).into()
    }

    #[pyfunction]
    fn ord(string: Either<ArgBytesLike, PyStrRef>, vm: &VirtualMachine) -> PyResult<u32> {
        match string {
            Either::A(bytes) => bytes.with_ref(|bytes| {
                let bytes_len = bytes.len();
                if bytes_len != 1 {
                    return Err(vm.new_type_error(format!(
                        "ord() expected a character, but string of length {bytes_len} found"
                    )));
                }
                Ok(u32::from(bytes[0]))
            }),
            Either::B(string) => match string.as_wtf8().code_points().exactly_one() {
                Ok(character) => Ok(character.to_u32()),
                Err(_) => {
                    let string_len = string.char_len();
                    Err(vm.new_type_error(format!(
                        "ord() expected a character, but string of length {string_len} found"
                    )))
                }
            },
        }
    }

    #[derive(FromArgs)]
    struct PowArgs {
        base: PyObjectRef,
        exp: PyObjectRef,
        #[pyarg(any, optional, name = "mod")]
        modulus: Option<PyObjectRef>,
    }

    #[pyfunction]
    fn pow(args: PowArgs, vm: &VirtualMachine) -> PyResult {
        let PowArgs {
            base: x,
            exp: y,
            modulus,
        } = args;
        let modulus = modulus
            .as_deref()
            .unwrap_or_else(|| vm.ctx.none.as_object());
        vm._pow(&x, &y, modulus)
    }

    #[pyfunction]
    pub(super) fn exit(exit_code_arg: OptionalArg<PyObjectRef>, vm: &VirtualMachine) -> PyResult {
        let code = exit_code_arg.unwrap_or_else(|| vm.ctx.new_int(0).into());
        Err(vm.new_exception(vm.ctx.exceptions.system_exit.to_owned(), vec![code]))
    }

    #[derive(Debug, Default, FromArgs)]
    pub struct PrintOptions {
        #[pyarg(named, default)]
        sep: Option<PyStrRef>,
        #[pyarg(named, default)]
        end: Option<PyStrRef>,
        #[pyarg(named, default = ArgIntoBool::FALSE)]
        flush: ArgIntoBool,
        #[pyarg(named, default)]
        file: Option<PyObjectRef>,
    }

    #[pyfunction]
    pub fn print(objects: PosArgs, options: PrintOptions, vm: &VirtualMachine) -> PyResult<()> {
        let file = match options.file {
            Some(f) => f,
            None => sys::get_stdout(vm)?,
        };
        let write = |obj: PyStrRef| vm.call_method(&file, "write", (obj,));

        let sep = options.sep.unwrap_or_else(|| vm.ctx.new_str(" "));

        let mut first = true;
        for object in objects {
            if first {
                first = false;
            } else {
                write(sep.clone())?;
            }

            write(object.str(vm)?)?;
        }

        let end = options.end.unwrap_or_else(|| vm.ctx.new_str("\n"));
        write(end)?;

        if options.flush.into() {
            vm.call_method(&file, "flush", ())?;
        }

        Ok(())
    }

    #[pyfunction]
    fn repr(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        obj.repr(vm)
    }

    #[pyfunction]
    pub fn reversed(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if let Some(reversed_method) = vm.get_method(obj.clone(), identifier!(vm, __reversed__)) {
            reversed_method?.call((), vm)
        } else {
            vm.get_method_or_type_error(obj.clone(), identifier!(vm, __getitem__), || {
                "argument to reversed() must be a sequence".to_owned()
            })?;
            let len = obj.length(vm)?;
            let obj_iterator = PyReverseSequenceIterator::new(obj, len);
            Ok(obj_iterator.into_pyobject(vm))
        }
    }

    #[derive(FromArgs)]
    pub(super) struct RoundArgs {
        number: PyObjectRef,
        #[pyarg(any, optional)]
        ndigits: OptionalOption<PyObjectRef>,
    }

    #[pyfunction]
    fn round(RoundArgs { number, ndigits }: RoundArgs, vm: &VirtualMachine) -> PyResult {
        let meth = vm
            .get_special_method(&number, identifier!(vm, __round__))?
            .ok_or_else(|| {
                vm.new_type_error(format!(
                    "type {} doesn't define __round__",
                    number.class().name()
                ))
            })?;
        match ndigits.flatten() {
            Some(obj) => {
                let ndigits = obj.try_index(vm)?;
                meth.invoke((ndigits,), vm)
            }
            None => {
                // without a parameter, the result type is coerced to int
                meth.invoke((), vm)
            }
        }
    }

    #[pyfunction]
    fn setattr(
        obj: PyObjectRef,
        attr: PyObjectRef,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let attr = attr.try_to_ref::<PyStr>(vm).map_err(|_e| {
            vm.new_type_error(format!(
                "attribute name must be string, not '{}'",
                attr.class().name()
            ))
        })?;
        obj.set_attr(attr, value, vm)?;
        Ok(())
    }

    // builtin_slice

    #[pyfunction]
    fn sorted(iterable: PyObjectRef, opts: SortOptions, vm: &VirtualMachine) -> PyResult<PyList> {
        let items: Vec<_> = iterable.try_to_value(vm)?;
        let lst = PyList::from(items);
        lst.sort(opts, vm)?;
        Ok(lst)
    }

    #[derive(FromArgs)]
    pub(super) struct SumArgs {
        #[pyarg(positional)]
        iterable: ArgIterable,
        #[pyarg(any, optional)]
        start: OptionalArg<PyObjectRef>,
    }

    #[expect(
        clippy::redundant_else,
        reason = "match_class! macro expansion arms has a `return` inside"
    )]
    #[pyfunction]
    fn sum(SumArgs { iterable, start }: SumArgs, vm: &VirtualMachine) -> PyResult {
        // Start with zero and add at will:
        let mut sum = start
            .into_option()
            .unwrap_or_else(|| vm.ctx.new_int(0).into());

        match_class!(match sum {
            PyStr =>
                return Err(vm.new_type_error("sum() can't sum strings [use ''.join(seq) instead]")),
            PyBytes =>
                return Err(vm.new_type_error("sum() can't sum bytes [use b''.join(seq) instead]")),
            PyByteArray =>
                return Err(
                    vm.new_type_error("sum() can't sum bytearray [use b''.join(seq) instead]")
                ),
            _ => (),
        });

        for item in iterable.iter(vm)? {
            sum = vm._add(&sum, &*item?)?;
        }
        Ok(sum)
    }

    #[derive(FromArgs)]
    struct ImportArgs {
        #[pyarg(any)]
        name: PyStrRef,
        #[pyarg(any, default)]
        globals: Option<PyObjectRef>,
        #[allow(dead_code)]
        #[pyarg(any, default)]
        locals: Option<PyObjectRef>,
        #[pyarg(any, default)]
        fromlist: Option<PyObjectRef>,
        #[pyarg(any, default)]
        level: i32,
    }

    #[pyfunction]
    fn __import__(args: ImportArgs, vm: &VirtualMachine) -> PyResult {
        crate::import::import_module_level(&args.name, args.globals, args.fromlist, args.level, vm)
    }

    #[pyfunction]
    fn vars(obj: OptionalArg, vm: &VirtualMachine) -> PyResult {
        if let OptionalArg::Present(obj) = obj {
            obj.get_attr(identifier!(vm, __dict__), vm)
                .map_err(|_| vm.new_type_error("vars() argument must have __dict__ attribute"))
        } else {
            Ok(vm.current_locals()?.into())
        }
    }

    #[pyfunction]
    pub(super) fn __build_class__(
        function: PyRef<PyFunction>,
        name: PyStrRef,
        bases: PosArgs,
        mut kwargs: KwArgs,
        vm: &VirtualMachine,
    ) -> PyResult {
        let name_obj: PyObjectRef = name.clone().into();

        // Update bases.
        let mut new_bases: Option<Vec<PyObjectRef>> = None;
        let bases = PyTuple::new_ref(bases.into_vec(), &vm.ctx);
        for (i, base) in bases.iter().enumerate() {
            if base.fast_isinstance(vm.ctx.types.type_type) {
                if let Some(bases) = &mut new_bases {
                    bases.push(base.clone());
                }
                continue;
            }
            let mro_entries =
                vm.get_attribute_opt(base.clone(), identifier!(vm, __mro_entries__))?;
            let entries = match mro_entries {
                Some(meth) => meth.call((bases.clone(),), vm)?,
                None => {
                    if let Some(bases) = &mut new_bases {
                        bases.push(base.clone());
                    }
                    continue;
                }
            };
            let entries: PyTupleRef = entries
                .downcast()
                .map_err(|_| vm.new_type_error("__mro_entries__ must return a tuple"))?;
            let new_bases = new_bases.get_or_insert_with(|| bases[..i].to_vec());
            new_bases.extend_from_slice(&entries);
        }

        let new_bases = new_bases.map(|v| PyTuple::new_ref(v, &vm.ctx));
        let (orig_bases, bases) = match new_bases {
            Some(new) => (Some(bases), new),
            None => (None, bases),
        };

        // Use downcast_exact to keep ref to old object on error.
        let metaclass = kwargs.pop_kwarg("metaclass").map_or_else(
            || {
                // if there are no bases, use type; else get the type of the first base
                Ok(if bases.is_empty() {
                    vm.ctx.types.type_type.to_owned()
                } else {
                    bases.first().unwrap().class().to_owned()
                })
            },
            |metaclass| {
                metaclass
                    .downcast_exact::<PyType>(vm)
                    .map(|m| m.into_pyref())
            },
        );

        let (metaclass, meta_name) = match metaclass {
            Ok(mut metaclass) => {
                for base in bases.iter() {
                    let base_class = base.class();
                    // if winner is subtype of tmptype, continue (winner is more derived)
                    if metaclass.fast_issubclass(base_class) {
                        continue;
                    }
                    // if tmptype is subtype of winner, update (tmptype is more derived)
                    if base_class.fast_issubclass(&metaclass) {
                        metaclass = base_class.to_owned();
                        continue;
                    }
                    // Metaclass conflict
                    return Err(vm.new_type_error(
                        "metaclass conflict: the metaclass of a derived class must be a (non-strict) \
                        subclass of the metaclasses of all its bases",
                    ));
                }
                let meta_name = metaclass.slot_name();
                (metaclass.to_owned().into(), meta_name.to_owned())
            }
            Err(obj) => (obj, "<metaclass>".to_owned()),
        };

        let bases: PyObjectRef = bases.into();

        // Prepare uses full __getattribute__ resolution chain.
        let namespace = vm
            .get_attribute_opt(metaclass.clone(), identifier!(vm, __prepare__))?
            .map_or(Ok(vm.ctx.new_dict().into()), |prepare| {
                let args = FuncArgs::new(vec![name_obj.clone(), bases.clone()], kwargs.clone());
                prepare.call(args, vm)
            })?;

        // Accept any PyMapping as namespace.
        let namespace = ArgMapping::try_from_object(vm, namespace.clone()).map_err(|_| {
            vm.new_type_error(format!(
                "{}.__prepare__() must return a mapping, not {}",
                meta_name,
                namespace.class()
            ))
        })?;

        // For PEP 695 classes, set .type_params in namespace before calling the function
        if let Ok(type_params) = function
            .as_object()
            .get_attr(identifier!(vm, __type_params__), vm)
            && let Some(type_params_tuple) = type_params.downcast_ref::<PyTuple>()
            && !type_params_tuple.is_empty()
        {
            // Set .type_params in namespace so the compiler-generated code can use it
            namespace
                .as_object()
                .set_item(vm.ctx.intern_str(".type_params"), type_params, vm)?;
        }

        let classcell = function.invoke_with_locals(().into(), Some(namespace.clone()), vm)?;
        let classcell = <Option<PyCellRef>>::try_from_object(vm, classcell)?;

        if let Some(orig_bases) = orig_bases {
            namespace.as_object().set_item(
                identifier!(vm, __orig_bases__),
                orig_bases.into(),
                vm,
            )?;
        }

        // Remove .type_params from namespace before creating the class
        namespace
            .as_object()
            .del_item(vm.ctx.intern_str(".type_params"), vm)
            .ok();

        let args = FuncArgs::new(vec![name_obj, bases, namespace.into()], kwargs);
        let class = metaclass.call(args, vm)?;

        // For PEP 695 classes, set __type_params__ on the class from the function
        if let Ok(type_params) = function
            .as_object()
            .get_attr(identifier!(vm, __type_params__), vm)
            && let Some(type_params_tuple) = type_params.downcast_ref::<PyTuple>()
            && !type_params_tuple.is_empty()
        {
            class.set_attr(identifier!(vm, __type_params__), type_params.clone(), vm)?;
            // Also set __parameters__ for compatibility with typing module
            class.set_attr(identifier!(vm, __parameters__), type_params, vm)?;
        }

        // only check cell if cls is a type and cell is a cell object
        if let Some(ref classcell) = classcell
            && class.fast_isinstance(vm.ctx.types.type_type)
        {
            let cell_value = classcell.get().ok_or_else(|| {
                vm.new_runtime_error(format!(
                    "__class__ not set defining {name:?} as {class:?}. Was __classcell__ propagated to type.__new__?"
                ))
            })?;

            if !cell_value.is(&class) {
                return Err(vm.new_type_error(format!(
                    "__class__ set to {cell_value:?} defining {name:?} as {class:?}"
                )));
            }
        }

        Ok(class)
    }
}

pub fn init_module(vm: &VirtualMachine, module: &Py<PyModule>) {
    let ctx = &vm.ctx;

    let _ = crate::protocol::VecBuffer::make_static_type();

    module.__init_methods(vm).unwrap();
    builtins::module_exec(vm, module).unwrap();

    let debug_mode: bool = vm.state.config.settings.optimize == 0;
    // Create dynamic ExceptionGroup with multiple inheritance (BaseExceptionGroup + Exception)
    let exception_group = crate::exception_group::exception_group();

    extend_module!(vm, module, {
        "__debug__" => ctx.new_bool(debug_mode),

        "bool" => ctx.types.bool_type.to_owned(),
        "bytearray" => ctx.types.bytearray_type.to_owned(),
        "bytes" => ctx.types.bytes_type.to_owned(),
        "classmethod" => ctx.types.classmethod_type.to_owned(),
        "complex" => ctx.types.complex_type.to_owned(),
        "dict" => ctx.types.dict_type.to_owned(),
        "enumerate" => ctx.types.enumerate_type.to_owned(),
        "float" => ctx.types.float_type.to_owned(),
        "frozenset" => ctx.types.frozenset_type.to_owned(),
        "filter" => ctx.types.filter_type.to_owned(),
        "int" => ctx.types.int_type.to_owned(),
        "list" => ctx.types.list_type.to_owned(),
        "map" => ctx.types.map_type.to_owned(),
        "memoryview" => ctx.types.memoryview_type.to_owned(),
        "object" => ctx.types.object_type.to_owned(),
        "property" => ctx.types.property_type.to_owned(),
        "range" => ctx.types.range_type.to_owned(),
        "set" => ctx.types.set_type.to_owned(),
        "slice" => ctx.types.slice_type.to_owned(),
        "staticmethod" => ctx.types.staticmethod_type.to_owned(),
        "str" => ctx.types.str_type.to_owned(),
        "super" => ctx.types.super_type.to_owned(),
        "tuple" => ctx.types.tuple_type.to_owned(),
        "type" => ctx.types.type_type.to_owned(),
        "zip" => ctx.types.zip_type.to_owned(),

        // Constants
        "None" => ctx.none(),
        "True" => ctx.new_bool(true),
        "False" => ctx.new_bool(false),
        "NotImplemented" => ctx.not_implemented(),
        "Ellipsis" => vm.ctx.ellipsis.clone(),

        // ordered by exception_hierarchy.txt
        // Exceptions:
        "BaseException" => ctx.exceptions.base_exception_type.to_owned(),
        "BaseExceptionGroup" => ctx.exceptions.base_exception_group.to_owned(),
        "ExceptionGroup" => exception_group.to_owned(),
        "SystemExit" => ctx.exceptions.system_exit.to_owned(),
        "KeyboardInterrupt" => ctx.exceptions.keyboard_interrupt.to_owned(),
        "GeneratorExit" => ctx.exceptions.generator_exit.to_owned(),
        "Exception" => ctx.exceptions.exception_type.to_owned(),
        "StopIteration" => ctx.exceptions.stop_iteration.to_owned(),
        "StopAsyncIteration" => ctx.exceptions.stop_async_iteration.to_owned(),
        "ArithmeticError" => ctx.exceptions.arithmetic_error.to_owned(),
        "FloatingPointError" => ctx.exceptions.floating_point_error.to_owned(),
        "OverflowError" => ctx.exceptions.overflow_error.to_owned(),
        "ZeroDivisionError" => ctx.exceptions.zero_division_error.to_owned(),
        "AssertionError" => ctx.exceptions.assertion_error.to_owned(),
        "AttributeError" => ctx.exceptions.attribute_error.to_owned(),
        "BufferError" => ctx.exceptions.buffer_error.to_owned(),
        "EOFError" => ctx.exceptions.eof_error.to_owned(),
        "ImportError" => ctx.exceptions.import_error.to_owned(),
        "ModuleNotFoundError" => ctx.exceptions.module_not_found_error.to_owned(),
        "LookupError" => ctx.exceptions.lookup_error.to_owned(),
        "IndexError" => ctx.exceptions.index_error.to_owned(),
        "KeyError" => ctx.exceptions.key_error.to_owned(),
        "MemoryError" => ctx.exceptions.memory_error.to_owned(),
        "NameError" => ctx.exceptions.name_error.to_owned(),
        "UnboundLocalError" => ctx.exceptions.unbound_local_error.to_owned(),
        "OSError" => ctx.exceptions.os_error.to_owned(),
        // OSError alias
        "IOError" => ctx.exceptions.os_error.to_owned(),
        "EnvironmentError" => ctx.exceptions.os_error.to_owned(),
        "BlockingIOError" => ctx.exceptions.blocking_io_error.to_owned(),
        "ChildProcessError" => ctx.exceptions.child_process_error.to_owned(),
        "ConnectionError" => ctx.exceptions.connection_error.to_owned(),
        "BrokenPipeError" => ctx.exceptions.broken_pipe_error.to_owned(),
        "ConnectionAbortedError" => ctx.exceptions.connection_aborted_error.to_owned(),
        "ConnectionRefusedError" => ctx.exceptions.connection_refused_error.to_owned(),
        "ConnectionResetError" => ctx.exceptions.connection_reset_error.to_owned(),
        "FileExistsError" => ctx.exceptions.file_exists_error.to_owned(),
        "FileNotFoundError" => ctx.exceptions.file_not_found_error.to_owned(),
        "InterruptedError" => ctx.exceptions.interrupted_error.to_owned(),
        "IsADirectoryError" => ctx.exceptions.is_a_directory_error.to_owned(),
        "NotADirectoryError" => ctx.exceptions.not_a_directory_error.to_owned(),
        "PermissionError" => ctx.exceptions.permission_error.to_owned(),
        "ProcessLookupError" => ctx.exceptions.process_lookup_error.to_owned(),
        "TimeoutError" => ctx.exceptions.timeout_error.to_owned(),
        "ReferenceError" => ctx.exceptions.reference_error.to_owned(),
        "RuntimeError" => ctx.exceptions.runtime_error.to_owned(),
        "PythonFinalizationError" => ctx.exceptions.python_finalization_error.to_owned(),
        "NotImplementedError" => ctx.exceptions.not_implemented_error.to_owned(),
        "RecursionError" => ctx.exceptions.recursion_error.to_owned(),
        "SyntaxError" =>  ctx.exceptions.syntax_error.to_owned(),
        "_IncompleteInputError" =>  ctx.exceptions.incomplete_input_error.to_owned(),
        "IndentationError" =>  ctx.exceptions.indentation_error.to_owned(),
        "TabError" =>  ctx.exceptions.tab_error.to_owned(),
        "SystemError" => ctx.exceptions.system_error.to_owned(),
        "TypeError" => ctx.exceptions.type_error.to_owned(),
        "ValueError" => ctx.exceptions.value_error.to_owned(),
        "UnicodeError" => ctx.exceptions.unicode_error.to_owned(),
        "UnicodeDecodeError" => ctx.exceptions.unicode_decode_error.to_owned(),
        "UnicodeEncodeError" => ctx.exceptions.unicode_encode_error.to_owned(),
        "UnicodeTranslateError" => ctx.exceptions.unicode_translate_error.to_owned(),

        // Warnings
        "Warning" => ctx.exceptions.warning.to_owned(),
        "DeprecationWarning" => ctx.exceptions.deprecation_warning.to_owned(),
        "PendingDeprecationWarning" => ctx.exceptions.pending_deprecation_warning.to_owned(),
        "RuntimeWarning" => ctx.exceptions.runtime_warning.to_owned(),
        "SyntaxWarning" => ctx.exceptions.syntax_warning.to_owned(),
        "UserWarning" => ctx.exceptions.user_warning.to_owned(),
        "FutureWarning" => ctx.exceptions.future_warning.to_owned(),
        "ImportWarning" => ctx.exceptions.import_warning.to_owned(),
        "UnicodeWarning" => ctx.exceptions.unicode_warning.to_owned(),
        "BytesWarning" => ctx.exceptions.bytes_warning.to_owned(),
        "ResourceWarning" => ctx.exceptions.resource_warning.to_owned(),
        "EncodingWarning" => ctx.exceptions.encoding_warning.to_owned(),
    });

    #[cfg(feature = "jit")]
    extend_module!(vm, module, {
        "JitError" => ctx.exceptions.jit_error.to_owned(),
    });

    #[cfg(windows)]
    extend_module!(vm, module, {
        // OSError alias for Windows
        "WindowsError" => ctx.exceptions.os_error.to_owned(),
    });
}

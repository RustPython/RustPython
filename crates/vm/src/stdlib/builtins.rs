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
            enumerate::PyReverseSequenceIterator,
            function::{PyCellRef, PyFunction},
            int::PyIntRef,
            iter::PyCallableIterator,
            list::{PyList, SortOptions},
        },
        common::{hash::PyHash, str::to_ascii},
        function::{
            ArgBytesLike, ArgCallable, ArgIndex, ArgIntoBool, ArgIterable, ArgMapping,
            ArgStrOrBytesLike, Either, FsPath, FuncArgs, KwArgs, OptionalArg, OptionalOption,
            PosArgs,
        },
        protocol::{PyIter, PyIterReturn},
        py_io,
        readline::{Readline, ReadlineResult},
        stdlib::sys,
        types::PyComparisonOp,
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
    pub fn ascii(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<ascii::AsciiString> {
        let repr = obj.repr(vm)?;
        let ascii = to_ascii(repr.as_str());
        Ok(ascii)
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
            .try_to_primitive::<isize>(vm)?
            .to_u32()
            .and_then(CodePoint::from_u32)
            .ok_or_else(|| vm.new_value_error("chr() arg not in range(0x110000)"))?;
        Ok(value)
    }

    #[derive(FromArgs)]
    #[allow(dead_code)]
    struct CompileArgs {
        source: PyObjectRef,
        filename: FsPath,
        mode: PyStrRef,
        #[pyarg(any, optional)]
        flags: OptionalArg<PyIntRef>,
        #[pyarg(any, optional)]
        dont_inherit: OptionalArg<bool>,
        #[pyarg(any, optional)]
        optimize: OptionalArg<PyIntRef>,
        #[pyarg(any, optional)]
        _feature_version: OptionalArg<i32>,
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
            use crate::{class::PyClassImpl, stdlib::ast};

            let feature_version = feature_version_from_arg(args._feature_version, vm)?;

            let mode_str = args.mode.as_str();

            let optimize: i32 = args.optimize.map_or(Ok(-1), |v| v.try_to_primitive(vm))?;
            let optimize: u8 = if optimize == -1 {
                vm.state.config.settings.optimize
            } else {
                optimize
                    .try_into()
                    .map_err(|_| vm.new_value_error("compile() optimize value invalid"))?
            };

            if args
                .source
                .fast_isinstance(&ast::NodeAst::make_class(&vm.ctx))
            {
                use num_traits::Zero;
                let flags: i32 = args.flags.map_or(Ok(0), |v| v.try_to_primitive(vm))?;
                let is_ast_only = !(flags & ast::PY_CF_ONLY_AST).is_zero();

                // func_type mode requires PyCF_ONLY_AST
                if mode_str == "func_type" && !is_ast_only {
                    return Err(vm.new_value_error(
                        "compile() mode 'func_type' requires flag PyCF_ONLY_AST".to_owned(),
                    ));
                }

                // compile(ast_node, ..., PyCF_ONLY_AST) returns the AST after validation
                if is_ast_only {
                    let (expected_type, expected_name) = ast::mode_type_and_name(&vm.ctx, mode_str)
                        .ok_or_else(|| {
                            vm.new_value_error(
                                "compile() mode must be 'exec', 'eval', 'single' or 'func_type'"
                                    .to_owned(),
                            )
                        })?;
                    if !args.source.fast_isinstance(&expected_type) {
                        return Err(vm.new_type_error(format!(
                            "expected {} node, got {}",
                            expected_name,
                            args.source.class().name()
                        )));
                    }
                    ast::validate_ast_object(vm, args.source.clone())?;
                    return Ok(args.source);
                }

                #[cfg(not(feature = "rustpython-codegen"))]
                {
                    return Err(vm.new_type_error(CODEGEN_NOT_SUPPORTED.to_owned()));
                }
                #[cfg(feature = "rustpython-codegen")]
                {
                    let mode = mode_str
                        .parse::<crate::compiler::Mode>()
                        .map_err(|err| vm.new_value_error(err.to_string()))?;
                    return ast::compile(
                        vm,
                        args.source,
                        &args.filename.to_string_lossy(),
                        mode,
                        Some(optimize),
                    );
                }
            }

            #[cfg(not(feature = "parser"))]
            {
                const PARSER_NOT_SUPPORTED: &str = "can't compile() source code when the `parser` feature of rustpython is disabled";
                Err(vm.new_type_error(PARSER_NOT_SUPPORTED.to_owned()))
            }
            #[cfg(feature = "parser")]
            {
                use crate::convert::ToPyException;
                use num_traits::Zero;
                use ruff_python_parser as parser;

                let source = ArgStrOrBytesLike::try_from_object(vm, args.source)?;
                let source = source.borrow_bytes();

                // TODO: compiler::compile should probably get bytes
                let source = core::str::from_utf8(&source)
                    .map_err(|e| vm.new_unicode_decode_error(e.to_string()))?;

                let flags = args.flags.map_or(Ok(0), |v| v.try_to_primitive(vm))?;

                if !(flags & !ast::PY_COMPILE_FLAGS_MASK).is_zero() {
                    return Err(vm.new_value_error("compile() unrecognized flags"));
                }

                let allow_incomplete = !(flags & ast::PY_CF_ALLOW_INCOMPLETE_INPUT).is_zero();
                let type_comments = !(flags & ast::PY_CF_TYPE_COMMENTS).is_zero();

                let optimize_level = optimize;

                if (flags & ast::PY_CF_ONLY_AST).is_zero() {
                    #[cfg(not(feature = "compiler"))]
                    {
                        Err(vm.new_value_error(CODEGEN_NOT_SUPPORTED.to_owned()))
                    }
                    #[cfg(feature = "compiler")]
                    {
                        if let Some(feature_version) = feature_version {
                            let mode = mode_str
                                .parse::<parser::Mode>()
                                .map_err(|err| vm.new_value_error(err.to_string()))?;
                            let _ = ast::parse(
                                vm,
                                source,
                                mode,
                                optimize_level,
                                Some(feature_version),
                                type_comments,
                            )
                            .map_err(|e| (e, Some(source), allow_incomplete).to_pyexception(vm))?;
                        }

                        let mode = mode_str
                            .parse::<crate::compiler::Mode>()
                            .map_err(|err| vm.new_value_error(err.to_string()))?;

                        let mut opts = vm.compile_opts();
                        opts.optimize = optimize;

                        let code = vm
                            .compile_with_opts(
                                source,
                                mode,
                                args.filename.to_string_lossy().into_owned(),
                                opts,
                            )
                            .map_err(|err| {
                                (err, Some(source), allow_incomplete).to_pyexception(vm)
                            })?;
                        Ok(code.into())
                    }
                } else {
                    if mode_str == "func_type" {
                        return ast::parse_func_type(vm, source, optimize_level, feature_version)
                            .map_err(|e| (e, Some(source), allow_incomplete).to_pyexception(vm));
                    }

                    let mode = mode_str
                        .parse::<parser::Mode>()
                        .map_err(|err| vm.new_value_error(err.to_string()))?;
                    let parsed = ast::parse(
                        vm,
                        source,
                        mode,
                        optimize_level,
                        feature_version,
                        type_comments,
                    )
                    .map_err(|e| (e, Some(source), allow_incomplete).to_pyexception(vm))?;

                    if mode_str == "single" {
                        return ast::wrap_interactive(vm, parsed);
                    }

                    Ok(parsed)
                }
            }
        }
    }

    #[cfg(feature = "ast")]
    fn feature_version_from_arg(
        feature_version: OptionalArg<i32>,
        vm: &VirtualMachine,
    ) -> PyResult<Option<ruff_python_ast::PythonVersion>> {
        let minor = match feature_version.into_option() {
            Some(minor) => minor,
            None => return Ok(None),
        };

        if minor < 0 {
            return Ok(None);
        }

        let minor = u8::try_from(minor)
            .map_err(|_| vm.new_value_error("compile() _feature_version out of range"))?;
        Ok(Some(ruff_python_ast::PythonVersion { major: 3, minor }))
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
                                    .to_owned()
                            } else {
                                "globals must be a dict".to_owned()
                            })
                        }
                        "exec" => vm.new_type_error(format!(
                            "exec() globals must be a dict, not {}",
                            globals.class().name()
                        )),
                        _ => vm.new_type_error("globals must be a dict".to_owned()),
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
                    vm.current_globals().clone(),
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
                let source: &[u8] = &either.borrow_bytes();
                if source.contains(&0) {
                    return Err(vm.new_exception_msg(
                        vm.ctx.exceptions.syntax_error.to_owned(),
                        "source code string cannot contain null bytes".to_owned(),
                    ));
                }

                let source = core::str::from_utf8(source).map_err(|err| {
                    let msg = format!(
                        "(unicode error) 'utf-8' codec can't decode byte 0x{:x?} in position {}: invalid start byte",
                        source[err.valid_up_to()],
                        err.valid_up_to()
                    );

                    vm.new_exception_msg(vm.ctx.exceptions.syntax_error.to_owned(), msg)
                })?;
                Ok(Either::A(vm.ctx.new_str(source.trim_start())))
            }
            Either::B(code) => Ok(Either::B(code)),
        }?;
        run_code(vm, code, scope, crate::compiler::Mode::Eval, "eval")
    }

    #[pyfunction]
    fn exec(
        source: Either<PyStrRef, PyRef<crate::builtins::PyCode>>,
        scope: ScopeArgs,
        vm: &VirtualMachine,
    ) -> PyResult {
        let scope = scope.make_scope(vm, "exec")?;
        run_code(vm, source, scope, crate::compiler::Mode::Exec, "exec")
    }

    fn run_code(
        vm: &VirtualMachine,
        source: Either<PyStrRef, PyRef<crate::builtins::PyCode>>,
        scope: crate::scope::Scope,
        #[allow(unused_variables)] mode: crate::compiler::Mode,
        func: &str,
    ) -> PyResult {
        // Determine code object:
        let code_obj = match source {
            #[cfg(feature = "rustpython-compiler")]
            Either::A(string) => vm
                .compile(string.as_str(), mode, "<string>".to_owned())
                .map_err(|err| vm.new_syntax_error(&err, Some(string.as_str())))?,
            #[cfg(not(feature = "rustpython-compiler"))]
            Either::A(_) => return Err(vm.new_type_error(CODEGEN_NOT_SUPPORTED.to_owned())),
            Either::B(code_obj) => code_obj,
        };

        if !code_obj.freevars.is_empty() {
            return Err(vm.new_type_error(format!(
                "code object passed to {func}() may not contain free variables"
            )));
        }

        // Run the code:
        vm.run_code_obj(code_obj, scope)
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
        vm.current_globals().clone()
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

        if use_rustyline {
            let prompt = prompt.as_ref().map_or("", |s| s.as_str());
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
                ReadlineResult::OsError(num) => Err(vm.new_os_error(num.to_string())),
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
        use nix::unistd::{getpid, getsid};
        // If this process is a session leader, we're likely in a PTY child
        getsid(None) == Ok(getpid())
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
    fn oct(number: ArgIndex, vm: &VirtualMachine) -> PyResult {
        let number = number.into_int_ref();
        let n = number.as_bigint();
        let s = if n.is_negative() {
            format!("-0o{:o}", n.abs())
        } else {
            format!("0o{n:o}")
        };

        Ok(vm.ctx.new_str(s).into())
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
        let modulus = modulus.as_ref().map_or(vm.ctx.none.as_object(), |m| m);
        vm._pow(&x, &y, modulus)
    }

    #[pyfunction]
    pub fn exit(exit_code_arg: OptionalArg<PyObjectRef>, vm: &VirtualMachine) -> PyResult {
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

        let sep = options
            .sep
            .unwrap_or_else(|| PyStr::from(" ").into_ref(&vm.ctx));

        let mut first = true;
        for object in objects {
            if first {
                first = false;
            } else {
                write(sep.clone())?;
            }

            write(object.str(vm)?)?;
        }

        let end = options
            .end
            .unwrap_or_else(|| PyStr::from("\n").into_ref(&vm.ctx));
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
    pub struct RoundArgs {
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
    pub struct SumArgs {
        #[pyarg(positional)]
        iterable: ArgIterable,
        #[pyarg(any, optional)]
        start: OptionalArg<PyObjectRef>,
    }

    #[pyfunction]
    fn sum(SumArgs { iterable, start }: SumArgs, vm: &VirtualMachine) -> PyResult {
        // Start with zero and add at will:
        let mut sum = start
            .into_option()
            .unwrap_or_else(|| vm.ctx.new_int(0).into());

        match_class!(match sum {
            PyStr =>
                return Err(vm.new_type_error(
                    "sum() can't sum strings [use ''.join(seq) instead]".to_owned()
                )),
            PyBytes =>
                return Err(vm.new_type_error(
                    "sum() can't sum bytes [use b''.join(seq) instead]".to_owned()
                )),
            PyByteArray =>
                return Err(vm.new_type_error(
                    "sum() can't sum bytearray [use b''.join(seq) instead]".to_owned()
                )),
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
    pub fn __build_class__(
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
        let metaclass = kwargs
            .pop_kwarg("metaclass")
            .map(|metaclass| {
                metaclass
                    .downcast_exact::<PyType>(vm)
                    .map(|m| m.into_pyref())
            })
            .unwrap_or_else(|| {
                // if there are no bases, use type; else get the type of the first base
                Ok(if bases.is_empty() {
                    vm.ctx.types.type_type.to_owned()
                } else {
                    bases.first().unwrap().class().to_owned()
                })
            });

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
                    "__class__ not set defining {:?} as {:?}. Was __classcell__ propagated to type.__new__?",
                    name, class
                ))
            })?;

            if !cell_value.is(&class) {
                return Err(vm.new_type_error(format!(
                    "__class__ set to {:?} defining {:?} as {:?}",
                    cell_value, name, class
                )));
            }
        }

        Ok(class)
    }
}

pub fn init_module(vm: &VirtualMachine, module: &Py<PyModule>) {
    let ctx = &vm.ctx;

    crate::protocol::VecBuffer::make_class(&vm.ctx);

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

//! Builtin function definitions.
//!
//! Implements the list of [builtin Python functions](https://docs.python.org/3/library/builtins.html).
use crate::{PyClassImpl, PyObjectRef, VirtualMachine};

/// Built-in functions, exceptions, and other objects.
///
/// Noteworthy: None is the `nil' object; Ellipsis represents `...' in slices.
#[pymodule]
mod builtins {
    #[cfg(feature = "rustpython-compiler")]
    use crate::compile;
    use crate::{
        builtins::{
            enumerate::PyReverseSequenceIterator,
            function::{PyCellRef, PyFunctionRef},
            int::PyIntRef,
            iter::PyCallableIterator,
            list::{PyList, SortOptions},
            PyByteArray, PyBytes, PyBytesRef, PyCode, PyDictRef, PyStr, PyStrRef, PyTuple,
            PyTupleRef, PyType,
        },
        common::{hash::PyHash, str::to_ascii},
        function::{
            ArgBytesLike, ArgCallable, ArgIntoBool, ArgIterable, FuncArgs, KwArgs, OptionalArg,
            OptionalOption, PosArgs,
        },
        protocol::{PyIter, PyIterReturn, PyMapping},
        py_io,
        readline::{Readline, ReadlineResult},
        scope::Scope,
        stdlib::sys,
        types::PyComparisonOp,
        utils::Either,
        IdProtocol, ItemProtocol, PyArithmeticValue, PyClassImpl, PyObject, PyObjectRef,
        PyObjectWrap, PyRef, PyResult, PyValue, TryFromObject, TypeProtocol, VirtualMachine,
    };
    use num_traits::{Signed, ToPrimitive, Zero};

    #[pyfunction]
    fn abs(x: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._abs(&x)
    }

    #[pyfunction]
    fn all(iterable: ArgIterable<ArgIntoBool>, vm: &VirtualMachine) -> PyResult<bool> {
        for item in iterable.iter(vm)? {
            if !item?.to_bool() {
                return Ok(false);
            }
        }
        Ok(true)
    }

    #[pyfunction]
    fn any(iterable: ArgIterable<ArgIntoBool>, vm: &VirtualMachine) -> PyResult<bool> {
        for item in iterable.iter(vm)? {
            if item?.to_bool() {
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
            format!("0b{:b}", x)
        }
    }

    // builtin_breakpoint

    #[pyfunction]
    fn callable(obj: PyObjectRef, vm: &VirtualMachine) -> bool {
        vm.is_callable(&obj)
    }

    #[pyfunction]
    fn chr(i: PyIntRef, vm: &VirtualMachine) -> PyResult<String> {
        match i
            .try_to_primitive::<isize>(vm)?
            .to_u32()
            .and_then(char::from_u32)
        {
            Some(value) => Ok(value.to_string()),
            None => Err(vm.new_value_error("chr() arg not in range(0x110000)".to_owned())),
        }
    }

    #[derive(FromArgs)]
    #[allow(dead_code)]
    struct CompileArgs {
        source: PyObjectRef,
        filename: PyStrRef,
        mode: PyStrRef,
        #[pyarg(any, optional)]
        flags: OptionalArg<PyIntRef>,
        #[pyarg(any, optional)]
        dont_inherit: OptionalArg<bool>,
        #[pyarg(any, optional)]
        optimize: OptionalArg<PyIntRef>,
    }

    #[cfg(feature = "rustpython-compiler")]
    #[pyfunction]
    fn compile(args: CompileArgs, vm: &VirtualMachine) -> PyResult {
        #[cfg(not(feature = "rustpython-ast"))]
        {
            Err(vm.new_value_error("can't use compile() when the `compiler` and `parser` features of rustpython are disabled".to_owned()))
        }
        #[cfg(feature = "rustpython-ast")]
        {
            use crate::stdlib::ast;

            let mode_str = args.mode.as_str();

            if args.source.isinstance(&ast::AstNode::make_class(&vm.ctx)) {
                #[cfg(not(feature = "rustpython-compiler"))]
                {
                    return Err(vm.new_value_error("can't compile ast nodes when the `compiler` feature of rustpython is disabled"));
                }
                #[cfg(feature = "rustpython-compiler")]
                {
                    let mode = mode_str
                        .parse::<compile::Mode>()
                        .map_err(|err| vm.new_value_error(err.to_string()))?;
                    return ast::compile(vm, args.source, args.filename.as_str(), mode);
                }
            }

            #[cfg(not(feature = "rustpython-parser"))]
            {
                Err(vm.new_value_error(
                    "can't compile() a string when the `parser` feature of rustpython is disabled",
                ))
            }
            #[cfg(feature = "rustpython-parser")]
            {
                use rustpython_parser::parser;

                let source = Either::<PyStrRef, PyBytesRef>::try_from_object(vm, args.source)?;
                // TODO: compile::compile should probably get bytes
                let source = match &source {
                    Either::A(string) => string.as_str(),
                    Either::B(bytes) => std::str::from_utf8(bytes)
                        .map_err(|e| vm.new_unicode_decode_error(e.to_string()))?,
                };

                let flags = args.flags.map_or(Ok(0), |v| v.try_to_primitive(vm))?;

                if (flags & ast::PY_COMPILE_FLAG_AST_ONLY).is_zero() {
                    #[cfg(not(feature = "rustpython-compiler"))]
                    {
                        Err(vm.new_value_error("can't compile() a string to bytecode when the `compiler` feature of rustpython is disabled".to_owned()))
                    }
                    #[cfg(feature = "rustpython-compiler")]
                    {
                        let mode = mode_str
                            .parse::<compile::Mode>()
                            .map_err(|err| vm.new_value_error(err.to_string()))?;
                        let code = vm
                            .compile(source, mode, args.filename.as_str().to_owned())
                            .map_err(|err| vm.new_syntax_error(&err))?;
                        Ok(code.into())
                    }
                } else {
                    let mode = mode_str
                        .parse::<parser::Mode>()
                        .map_err(|err| vm.new_value_error(err.to_string()))?;
                    ast::parse(vm, source, mode)
                }
            }
        }
    }

    #[pyfunction]
    fn delattr(obj: PyObjectRef, attr: PyStrRef, vm: &VirtualMachine) -> PyResult<()> {
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

    #[cfg(feature = "rustpython-compiler")]
    #[derive(FromArgs)]
    struct ScopeArgs {
        #[pyarg(any, default)]
        globals: Option<PyDictRef>,
        #[pyarg(any, default)]
        locals: Option<PyMapping>,
    }

    #[cfg(feature = "rustpython-compiler")]
    impl ScopeArgs {
        fn make_scope(self, vm: &VirtualMachine) -> PyResult<Scope> {
            let (globals, locals) = match self.globals {
                Some(globals) => {
                    if !globals.contains_key("__builtins__", vm) {
                        let builtins_dict = vm.builtins.dict().into();
                        globals.set_item("__builtins__", builtins_dict, vm)?;
                    }
                    (
                        globals.clone(),
                        self.locals
                            .unwrap_or_else(|| PyMapping::new(globals.into())),
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

            let scope = Scope::with_builtins(Some(locals), globals, vm);
            Ok(scope)
        }
    }

    /// Implements `eval`.
    /// See also: https://docs.python.org/3/library/functions.html#eval
    #[cfg(feature = "rustpython-compiler")]
    #[pyfunction]
    fn eval(
        source: Either<PyStrRef, PyRef<PyCode>>,
        scope: ScopeArgs,
        vm: &VirtualMachine,
    ) -> PyResult {
        run_code(vm, source, scope, compile::Mode::Eval, "eval")
    }

    /// Implements `exec`
    /// https://docs.python.org/3/library/functions.html#exec
    #[cfg(feature = "rustpython-compiler")]
    #[pyfunction]
    fn exec(
        source: Either<PyStrRef, PyRef<PyCode>>,
        scope: ScopeArgs,
        vm: &VirtualMachine,
    ) -> PyResult {
        run_code(vm, source, scope, compile::Mode::Exec, "exec")
    }

    #[cfg(feature = "rustpython-compiler")]
    fn run_code(
        vm: &VirtualMachine,
        source: Either<PyStrRef, PyRef<PyCode>>,
        scope: ScopeArgs,
        mode: compile::Mode,
        func: &str,
    ) -> PyResult {
        let scope = scope.make_scope(vm)?;

        // Determine code object:
        let code_obj = match source {
            Either::A(string) => vm
                .compile(string.as_str(), mode, "<string>".to_owned())
                .map_err(|err| vm.new_syntax_error(&err))?,
            Either::B(code_obj) => code_obj,
        };

        if !code_obj.freevars.is_empty() {
            return Err(vm.new_type_error(format!(
                "code object passed to {}() may not contain free variables",
                func
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
        let format_spec = format_spec
            .into_option()
            .unwrap_or_else(|| PyStr::from("").into_ref(vm));

        vm.call_method(&value, "__format__", (format_spec,))?
            .downcast()
            .map_err(|obj| {
                vm.new_type_error(format!(
                    "__format__ must return a str, not {}",
                    obj.class().name()
                ))
            })
    }

    #[pyfunction]
    fn getattr(
        obj: PyObjectRef,
        attr: PyStrRef,
        default: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
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
    fn hasattr(obj: PyObjectRef, attr: PyStrRef, vm: &VirtualMachine) -> PyResult<bool> {
        Ok(vm.get_attribute_opt(obj, attr)?.is_some())
    }

    #[pyfunction]
    fn hash(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyHash> {
        obj.hash(vm)
    }

    // builtin_help

    #[pyfunction]
    fn hex(number: PyIntRef) -> String {
        let n = number.as_bigint();
        format!("{:#x}", n)
    }

    #[pyfunction]
    fn id(obj: PyObjectRef) -> usize {
        obj.get_id()
    }

    #[pyfunction]
    fn input(prompt: OptionalArg<PyStrRef>, vm: &VirtualMachine) -> PyResult {
        let stdin = sys::get_stdin(vm)?;
        let stdout = sys::get_stdout(vm)?;
        let stderr = sys::get_stderr(vm)?;

        let _ = vm.call_method(&stderr, "flush", ());

        let fd_matches = |obj, expected| {
            vm.call_method(obj, "fileno", ())
                .and_then(|o| i64::try_from_object(vm, o))
                .ok()
                .map_or(false, |fd| fd == expected)
        };

        // everything is normalish, we can just rely on rustyline to use stdin/stdout
        if fd_matches(&stdin, 0) && fd_matches(&stdout, 1) && atty::is(atty::Stream::Stdin) {
            let prompt = prompt.as_ref().map_or("", |s| s.as_str());
            let mut readline = Readline::new(());
            match readline.readline(prompt) {
                ReadlineResult::Line(s) => Ok(vm.ctx.new_str(s).into()),
                ReadlineResult::Eof => {
                    Err(vm.new_exception_empty(vm.ctx.exceptions.eof_error.clone()))
                }
                ReadlineResult::Interrupt => {
                    Err(vm.new_exception_empty(vm.ctx.exceptions.keyboard_interrupt.clone()))
                }
                ReadlineResult::Io(e) => Err(vm.new_os_error(e.to_string())),
                ReadlineResult::EncodingError => {
                    Err(vm.new_unicode_decode_error("Error decoding readline input".to_owned()))
                }
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
                .into_ref(vm)
                .into();
            Ok(PyIter::new(iterator))
        } else {
            iter_target.get_iter(vm)
        }
    }

    #[pyfunction]
    fn len(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
        obj.length(vm)
    }

    #[pyfunction]
    fn locals(vm: &VirtualMachine) -> PyResult<PyMapping> {
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
            std::cmp::Ordering::Greater => {
                if default.is_some() {
                    return Err(vm.new_type_error(format!(
                        "Cannot specify a default for {} with multiple positional arguments",
                        func_name
                    )));
                }
                args.args
            }
            std::cmp::Ordering::Equal => vm.extract_elements(&args.args[0])?,
            std::cmp::Ordering::Less => {
                // zero arguments means type error:
                return Err(vm.new_type_error("Expected 1 or more arguments".to_owned()));
            }
        };

        let mut candidates_iter = candidates.into_iter();
        let mut x = match candidates_iter.next() {
            Some(x) => x,
            None => {
                return default.ok_or_else(|| {
                    vm.new_value_error(format!("{} arg is an empty sequence", func_name))
                })
            }
        };

        let key_func = key_func.filter(|f| !vm.is_none(f));
        if let Some(ref key_func) = key_func {
            let mut x_key = vm.invoke(key_func, (x.clone(),))?;
            for y in candidates_iter {
                let y_key = vm.invoke(key_func, (y.clone(),))?;
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
        min_or_max(args, vm, "max()", PyComparisonOp::Gt)
    }

    #[pyfunction]
    fn min(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        min_or_max(args, vm, "min()", PyComparisonOp::Lt)
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
        PyIter::new(iterator).next(vm).map(|iret| match iret {
            PyIterReturn::Return(obj) => PyIterReturn::Return(obj),
            PyIterReturn::StopIteration(v) => {
                default_value.map_or(PyIterReturn::StopIteration(v), PyIterReturn::Return)
            }
        })
    }

    #[pyfunction]
    fn oct(number: PyIntRef, vm: &VirtualMachine) -> PyResult {
        let n = number.as_bigint();
        let s = if n.is_negative() {
            format!("-0o{:o}", n.abs())
        } else {
            format!("0o{:o}", n)
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
                        "ord() expected a character, but string of length {} found",
                        bytes_len
                    )));
                }
                Ok(u32::from(bytes[0]))
            }),
            Either::B(string) => {
                let string = string.as_str();
                let string_len = string.chars().count();
                if string_len != 1 {
                    return Err(vm.new_type_error(format!(
                        "ord() expected a character, but string of length {} found",
                        string_len
                    )));
                }
                match string.chars().next() {
                    Some(character) => Ok(character as u32),
                    None => Err(vm.new_type_error(
                        "ord() could not guess the integer representing this character".to_owned(),
                    )),
                }
            }
        }
    }

    #[derive(FromArgs)]
    struct PowArgs {
        base: PyObjectRef,
        exp: PyObjectRef,
        #[pyarg(any, optional, name = "mod")]
        modulus: Option<PyObjectRef>,
    }

    #[allow(clippy::suspicious_else_formatting)]
    #[pyfunction]
    fn pow(args: PowArgs, vm: &VirtualMachine) -> PyResult {
        let PowArgs {
            base: x,
            exp: y,
            modulus,
        } = args;
        match modulus {
            None => vm.call_or_reflection(&x, &y, "__pow__", "__rpow__", |vm, x, y| {
                Err(vm.new_unsupported_binop_error(x, y, "pow"))
            }),
            Some(z) => {
                let try_pow_value = |obj: &PyObject,
                                     args: (PyObjectRef, PyObjectRef, PyObjectRef)|
                 -> Option<PyResult> {
                    if let Some(method) = obj.get_class_attr("__pow__") {
                        let result = match vm.invoke(&method, args) {
                            Ok(x) => x,
                            Err(e) => return Some(Err(e)),
                        };
                        if let PyArithmeticValue::Implemented(x) =
                            PyArithmeticValue::from_object(vm, result)
                        {
                            return Some(Ok(x));
                        }
                    }
                    None
                };

                if let Some(val) = try_pow_value(&x, (x.clone(), y.clone(), z.clone())) {
                    return val;
                }

                if !x.class().is(&y.class()) {
                    if let Some(val) = try_pow_value(&y, (x.clone(), y.clone(), z.clone())) {
                        return val;
                    }
                }

                if !x.class().is(&z.class()) && !y.class().is(&z.class()) {
                    if let Some(val) = try_pow_value(&z, (x.clone(), y.clone(), z.clone())) {
                        return val;
                    }
                }

                Err(vm.new_unsupported_ternop_error(&x, &y, &z, "pow"))
            }
        }
    }

    #[pyfunction]
    pub fn exit(exit_code_arg: OptionalArg<PyObjectRef>, vm: &VirtualMachine) -> PyResult {
        let code = exit_code_arg.unwrap_or_else(|| vm.ctx.new_int(0).into());
        Err(vm.new_exception(vm.ctx.exceptions.system_exit.clone(), vec![code]))
    }

    #[derive(Debug, Default, FromArgs)]
    pub struct PrintOptions {
        #[pyarg(named, default)]
        sep: Option<PyStrRef>,
        #[pyarg(named, default)]
        end: Option<PyStrRef>,
        #[pyarg(named, default = "ArgIntoBool::FALSE")]
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

        let sep = options.sep.unwrap_or_else(|| PyStr::from(" ").into_ref(vm));

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
            .unwrap_or_else(|| PyStr::from("\n").into_ref(vm));
        write(end)?;

        if options.flush.to_bool() {
            vm.call_method(&file, "flush", ())?;
        }

        Ok(())
    }

    #[pyfunction]
    fn repr(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        obj.repr(vm)
    }

    #[pyfunction]
    fn reversed(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if let Some(reversed_method) = vm.get_method(obj.clone(), "__reversed__") {
            vm.invoke(&reversed_method?, ())
        } else {
            vm.get_method_or_type_error(obj.clone(), "__getitem__", || {
                "argument to reversed() must be a sequence".to_owned()
            })?;
            let len = obj.length(vm)?;
            let obj_iterator = PyReverseSequenceIterator::new(obj, len);
            Ok(obj_iterator.into_object(vm))
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
            .get_special_method(number, "__round__")?
            .map_err(|number| {
                vm.new_type_error(format!(
                    "type {} doesn't define __round__",
                    number.class().name()
                ))
            })?;
        match ndigits.flatten() {
            Some(obj) => {
                let ndigits = vm.to_index(&obj)?;
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
        attr: PyStrRef,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        obj.set_attr(attr, value, vm)?;
        Ok(())
    }

    // builtin_slice

    #[pyfunction]
    fn sorted(iterable: PyObjectRef, opts: SortOptions, vm: &VirtualMachine) -> PyResult<PyList> {
        let items = vm.extract_elements(&iterable)?;
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

    #[pyfunction]
    fn __import__(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        vm.invoke(&vm.import_func, args)
    }

    #[pyfunction]
    fn vars(obj: OptionalArg, vm: &VirtualMachine) -> PyResult {
        if let OptionalArg::Present(obj) = obj {
            obj.get_attr("__dict__", vm).map_err(|_| {
                vm.new_type_error("vars() argument must have __dict__ attribute".to_owned())
            })
        } else {
            Ok(vm.current_locals()?.into_object())
        }
    }

    #[pyfunction]
    pub fn __build_class__(
        function: PyFunctionRef,
        qualified_name: PyStrRef,
        bases: PosArgs,
        mut kwargs: KwArgs,
        vm: &VirtualMachine,
    ) -> PyResult {
        let name = qualified_name.as_str().split('.').next_back().unwrap();
        let name_obj = vm.ctx.new_str(name);

        // Update bases.
        let mut new_bases: Option<Vec<PyObjectRef>> = None;
        let bases = PyTuple::new_ref(bases.into_vec(), &vm.ctx);
        for (i, base) in bases.as_slice().iter().enumerate() {
            if base.isinstance(&vm.ctx.types.type_type) {
                if let Some(bases) = &mut new_bases {
                    bases.push(base.clone());
                }
                continue;
            }
            let mro_entries = vm.get_attribute_opt(base.clone(), "__mro_entries__")?;
            let entries = match mro_entries {
                Some(meth) => vm.invoke(&meth, (bases.clone(),))?,
                None => {
                    if let Some(bases) = &mut new_bases {
                        bases.push(base.clone());
                    }
                    continue;
                }
            };
            let entries: PyTupleRef = entries
                .downcast()
                .map_err(|_| vm.new_type_error("__mro_entries__ must return a tuple".to_owned()))?;
            let new_bases = new_bases.get_or_insert_with(|| bases.as_slice()[..i].to_vec());
            new_bases.extend_from_slice(entries.as_slice());
        }

        let new_bases = new_bases.map(|v| PyTuple::new_ref(v, &vm.ctx));
        let (orig_bases, bases) = match new_bases {
            Some(new) => (Some(bases), new),
            None => (None, bases),
        };

        // Use downcast_exact to keep ref to old object on error.
        let metaclass = kwargs
            .pop_kwarg("metaclass")
            .map(|metaclass| metaclass.downcast_exact::<PyType>(vm))
            .unwrap_or_else(|| Ok(vm.ctx.types.type_type.clone()));

        let (metaclass, meta_name) = match metaclass {
            Ok(mut metaclass) => {
                for base in bases.as_slice().iter() {
                    let base_class = base.class();
                    if base_class.issubclass(&metaclass) {
                        metaclass = base.clone_class();
                    } else if !metaclass.issubclass(&base_class) {
                        return Err(vm.new_type_error(
                            "metaclass conflict: the metaclass of a derived class must be a (non-strict) \
                            subclass of the metaclasses of all its bases"
                                .to_owned(),
                        ));
                    }
                }
                let meta_name = metaclass.slot_name();
                (metaclass.into(), meta_name)
            }
            Err(obj) => (obj, "<metaclass>".to_owned()),
        };

        let bases: PyObjectRef = bases.into();

        // Prepare uses full __getattribute__ resolution chain.
        let namespace = vm
            .get_attribute_opt(metaclass.clone(), "__prepare__")?
            .map_or(Ok(vm.ctx.new_dict().into()), |prepare| {
                vm.invoke(
                    &prepare,
                    FuncArgs::new(vec![name_obj.clone().into(), bases.clone()], kwargs.clone()),
                )
            })?;

        // Accept any PyMapping as namespace.
        let namespace = PyMapping::try_from_object(vm, namespace.clone()).map_err(|_| {
            vm.new_type_error(format!(
                "{}.__prepare__() must return a mapping, not {}",
                meta_name,
                namespace.class().name()
            ))
        })?;

        let classcell = function.invoke_with_locals(().into(), Some(namespace.clone()), vm)?;
        let classcell = <Option<PyCellRef>>::try_from_object(vm, classcell)?;

        if let Some(orig_bases) = orig_bases {
            namespace
                .as_object()
                .set_item("__orig_bases__", orig_bases.into(), vm)?;
        }

        let class = vm.invoke(
            &metaclass,
            FuncArgs::new(vec![name_obj.into(), bases, namespace.into()], kwargs),
        )?;

        if let Some(ref classcell) = classcell {
            classcell.set(Some(class.clone()));
        }

        Ok(class)
    }
}

pub use builtins::{ascii, print};

pub fn make_module(vm: &VirtualMachine, module: PyObjectRef) {
    let ctx = &vm.ctx;

    crate::protocol::VecBuffer::make_class(&vm.ctx);

    let _ = builtins::extend_module(vm, &module);

    let debug_mode: bool = vm.state.settings.optimize == 0;
    extend_module!(vm, module, {
        "__debug__" => ctx.new_bool(debug_mode),

        "bool" => ctx.types.bool_type.clone(),
        "bytearray" => ctx.types.bytearray_type.clone(),
        "bytes" => ctx.types.bytes_type.clone(),
        "classmethod" => ctx.types.classmethod_type.clone(),
        "complex" => ctx.types.complex_type.clone(),
        "dict" => ctx.types.dict_type.clone(),
        "enumerate" => ctx.types.enumerate_type.clone(),
        "float" => ctx.types.float_type.clone(),
        "frozenset" => ctx.types.frozenset_type.clone(),
        "filter" => ctx.types.filter_type.clone(),
        "int" => ctx.types.int_type.clone(),
        "list" => ctx.types.list_type.clone(),
        "map" => ctx.types.map_type.clone(),
        "memoryview" => ctx.types.memoryview_type.clone(),
        "object" => ctx.types.object_type.clone(),
        "property" => ctx.types.property_type.clone(),
        "range" => ctx.types.range_type.clone(),
        "set" => ctx.types.set_type.clone(),
        "slice" => ctx.types.slice_type.clone(),
        "staticmethod" => ctx.types.staticmethod_type.clone(),
        "str" => ctx.types.str_type.clone(),
        "super" => ctx.types.super_type.clone(),
        "tuple" => ctx.types.tuple_type.clone(),
        "type" => ctx.types.type_type.clone(),
        "zip" => ctx.types.zip_type.clone(),

        // Constants
        "None" => ctx.none(),
        "True" => ctx.new_bool(true),
        "False" => ctx.new_bool(false),
        "NotImplemented" => ctx.not_implemented(),
        "Ellipsis" => vm.ctx.ellipsis.clone(),

        // ordered by exception_hierarachy.txt
        // Exceptions:
        "BaseException" => ctx.exceptions.base_exception_type.clone(),
        "SystemExit" => ctx.exceptions.system_exit.clone(),
        "KeyboardInterrupt" => ctx.exceptions.keyboard_interrupt.clone(),
        "GeneratorExit" => ctx.exceptions.generator_exit.clone(),
        "Exception" => ctx.exceptions.exception_type.clone(),
        "StopIteration" => ctx.exceptions.stop_iteration.clone(),
        "StopAsyncIteration" => ctx.exceptions.stop_async_iteration.clone(),
        "ArithmeticError" => ctx.exceptions.arithmetic_error.clone(),
        "FloatingPointError" => ctx.exceptions.floating_point_error.clone(),
        "OverflowError" => ctx.exceptions.overflow_error.clone(),
        "ZeroDivisionError" => ctx.exceptions.zero_division_error.clone(),
        "AssertionError" => ctx.exceptions.assertion_error.clone(),
        "AttributeError" => ctx.exceptions.attribute_error.clone(),
        "BufferError" => ctx.exceptions.buffer_error.clone(),
        "EOFError" => ctx.exceptions.eof_error.clone(),
        "ImportError" => ctx.exceptions.import_error.clone(),
        "ModuleNotFoundError" => ctx.exceptions.module_not_found_error.clone(),
        "LookupError" => ctx.exceptions.lookup_error.clone(),
        "IndexError" => ctx.exceptions.index_error.clone(),
        "KeyError" => ctx.exceptions.key_error.clone(),
        "MemoryError" => ctx.exceptions.memory_error.clone(),
        "NameError" => ctx.exceptions.name_error.clone(),
        "UnboundLocalError" => ctx.exceptions.unbound_local_error.clone(),
        "OSError" => ctx.exceptions.os_error.clone(),
        // OSError alias
        "IOError" => ctx.exceptions.os_error.clone(),
        "EnvironmentError" => ctx.exceptions.os_error.clone(),
        "BlockingIOError" => ctx.exceptions.blocking_io_error.clone(),
        "ChildProcessError" => ctx.exceptions.child_process_error.clone(),
        "ConnectionError" => ctx.exceptions.connection_error.clone(),
        "BrokenPipeError" => ctx.exceptions.broken_pipe_error.clone(),
        "ConnectionAbortedError" => ctx.exceptions.connection_aborted_error.clone(),
        "ConnectionRefusedError" => ctx.exceptions.connection_refused_error.clone(),
        "ConnectionResetError" => ctx.exceptions.connection_reset_error.clone(),
        "FileExistsError" => ctx.exceptions.file_exists_error.clone(),
        "FileNotFoundError" => ctx.exceptions.file_not_found_error.clone(),
        "InterruptedError" => ctx.exceptions.interrupted_error.clone(),
        "IsADirectoryError" => ctx.exceptions.is_a_directory_error.clone(),
        "NotADirectoryError" => ctx.exceptions.not_a_directory_error.clone(),
        "PermissionError" => ctx.exceptions.permission_error.clone(),
        "ProcessLookupError" => ctx.exceptions.process_lookup_error.clone(),
        "TimeoutError" => ctx.exceptions.timeout_error.clone(),
        "ReferenceError" => ctx.exceptions.reference_error.clone(),
        "RuntimeError" => ctx.exceptions.runtime_error.clone(),
        "NotImplementedError" => ctx.exceptions.not_implemented_error.clone(),
        "RecursionError" => ctx.exceptions.recursion_error.clone(),
        "SyntaxError" =>  ctx.exceptions.syntax_error.clone(),
        "IndentationError" =>  ctx.exceptions.indentation_error.clone(),
        "TabError" =>  ctx.exceptions.tab_error.clone(),
        "SystemError" => ctx.exceptions.system_error.clone(),
        "TypeError" => ctx.exceptions.type_error.clone(),
        "ValueError" => ctx.exceptions.value_error.clone(),
        "UnicodeError" => ctx.exceptions.unicode_error.clone(),
        "UnicodeDecodeError" => ctx.exceptions.unicode_decode_error.clone(),
        "UnicodeEncodeError" => ctx.exceptions.unicode_encode_error.clone(),
        "UnicodeTranslateError" => ctx.exceptions.unicode_translate_error.clone(),

        // Warnings
        "Warning" => ctx.exceptions.warning.clone(),
        "DeprecationWarning" => ctx.exceptions.deprecation_warning.clone(),
        "PendingDeprecationWarning" => ctx.exceptions.pending_deprecation_warning.clone(),
        "RuntimeWarning" => ctx.exceptions.runtime_warning.clone(),
        "SyntaxWarning" => ctx.exceptions.syntax_warning.clone(),
        "UserWarning" => ctx.exceptions.user_warning.clone(),
        "FutureWarning" => ctx.exceptions.future_warning.clone(),
        "ImportWarning" => ctx.exceptions.import_warning.clone(),
        "UnicodeWarning" => ctx.exceptions.unicode_warning.clone(),
        "BytesWarning" => ctx.exceptions.bytes_warning.clone(),
        "ResourceWarning" => ctx.exceptions.resource_warning.clone(),
    });

    #[cfg(feature = "jit")]
    extend_module!(vm, module, {
        "JitError" => ctx.exceptions.jit_error.clone(),
    });
}

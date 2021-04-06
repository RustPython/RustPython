//! Builtin function definitions.
//!
//! Implements functions listed here: https://docs.python.org/3/library/builtins.html
use crate::pyobject::PyObjectRef;
use crate::vm::VirtualMachine;

/// Built-in functions, exceptions, and other objects.
///
/// Noteworthy: None is the `nil' object; Ellipsis represents `...' in slices.
#[pymodule(name = "builtins")]
mod decl {
    use crate::builtins::bytes::PyBytesRef;
    use crate::builtins::code::PyCodeRef;
    use crate::builtins::dict::PyDictRef;
    use crate::builtins::function::{PyCellRef, PyFunctionRef};
    use crate::builtins::int::PyIntRef;
    use crate::builtins::iter::{PyCallableIterator, PySequenceIterator};
    use crate::builtins::list::{PyList, SortOptions};
    use crate::builtins::pybool::IntoPyBool;
    use crate::builtins::pystr::{PyStr, PyStrRef};
    use crate::builtins::pytype::PyTypeRef;
    use crate::builtins::{PyByteArray, PyBytes};
    use crate::byteslike::PyBytesLike;
    use crate::common::{hash::PyHash, str::to_ascii};
    #[cfg(feature = "rustpython-compiler")]
    use crate::compile;
    use crate::function::{
        single_or_tuple_any, Args, FuncArgs, KwArgs, OptionalArg, OptionalOption,
    };
    use crate::iterator;
    use crate::pyobject::{
        BorrowValue, Either, IdProtocol, ItemProtocol, PyArithmaticValue, PyCallable, PyClassImpl,
        PyIterable, PyObjectRef, PyResult, PyValue, TryFromObject, TypeProtocol,
    };
    use crate::readline::{Readline, ReadlineResult};
    use crate::scope::Scope;
    use crate::slots::PyComparisonOp;
    use crate::vm::VirtualMachine;
    use crate::{py_io, sysmodule};
    use num_traits::{Signed, Zero};

    #[pyfunction]
    fn abs(x: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let method = vm.get_method_or_type_error(x.clone(), "__abs__", || {
            format!("bad operand type for abs(): '{}'", x.class().name)
        })?;
        vm.invoke(&method, ())
    }

    #[pyfunction]
    fn all(iterable: PyIterable<IntoPyBool>, vm: &VirtualMachine) -> PyResult<bool> {
        for item in iterable.iter(vm)? {
            if !item?.to_bool() {
                return Ok(false);
            }
        }
        Ok(true)
    }

    #[pyfunction]
    fn any(iterable: PyIterable<IntoPyBool>, vm: &VirtualMachine) -> PyResult<bool> {
        for item in iterable.iter(vm)? {
            if item?.to_bool() {
                return Ok(true);
            }
        }
        Ok(false)
    }

    #[pyfunction]
    pub fn ascii(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<String> {
        let repr = vm.to_repr(&obj)?;
        let ascii = to_ascii(repr.borrow_value());
        Ok(ascii)
    }

    #[pyfunction]
    fn bin(x: PyIntRef) -> String {
        let x = x.borrow_value();
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
    fn chr(i: u32, vm: &VirtualMachine) -> PyResult<String> {
        match std::char::from_u32(i) {
            Some(value) => Ok(value.to_string()),
            None => Err(vm.new_value_error("chr() arg not in range(0x110000)".to_owned())),
        }
    }

    #[derive(FromArgs)]
    #[allow(dead_code)]
    struct CompileArgs {
        #[pyarg(any)]
        source: PyObjectRef,
        #[pyarg(any)]
        filename: PyStrRef,
        #[pyarg(any)]
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

            let mode_str = args.mode.borrow_value();

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
                    return ast::compile(vm, args.source, args.filename.borrow_value(), mode);
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
                    Either::A(string) => string.borrow_value(),
                    Either::B(bytes) => std::str::from_utf8(bytes)
                        .map_err(|e| vm.new_unicode_decode_error(e.to_string()))?,
                };

                let flags = args
                    .flags
                    .map_or(Ok(0), |v| i32::try_from_object(vm, v.into_object()))?;

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

                        vm.compile(&source, mode, args.filename.borrow_value().to_owned())
                            .map(|o| o.into_object())
                            .map_err(|err| vm.new_syntax_error(&err))
                    }
                } else {
                    let mode = mode_str
                        .parse::<parser::Mode>()
                        .map_err(|err| vm.new_value_error(err.to_string()))?;
                    ast::parse(&vm, &source, mode)
                }
            }
        }
    }

    #[pyfunction]
    fn delattr(obj: PyObjectRef, attr: PyStrRef, vm: &VirtualMachine) -> PyResult<()> {
        vm.del_attr(&obj, attr.into_object())
    }

    #[pyfunction]
    fn dir(obj: OptionalArg<PyObjectRef>, vm: &VirtualMachine) -> PyResult<PyList> {
        let seq = match obj {
            OptionalArg::Present(obj) => vm
                .get_special_method(obj, "__dir__")?
                .map_err(|_obj| vm.new_type_error("object does not provide __dir__".to_owned()))?
                .invoke((), vm)?,
            OptionalArg::Missing => vm.call_method(vm.current_locals()?.as_object(), "keys", ())?,
        };
        let sorted = sorted(seq, Default::default(), vm)?;
        Ok(sorted)
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
        // TODO: support any mapping for `locals`
        #[pyarg(any, default)]
        locals: Option<PyDictRef>,
    }

    #[cfg(feature = "rustpython-compiler")]
    impl ScopeArgs {
        fn make_scope(self, vm: &VirtualMachine) -> PyResult<Scope> {
            let (globals, locals) = match self.globals {
                Some(globals) => {
                    if !globals.contains_key("__builtins__", vm) {
                        let builtins_dict = vm.builtins.dict().unwrap().into_object();
                        globals.set_item("__builtins__", builtins_dict, vm)?;
                    }
                    let locals = self.locals.unwrap_or_else(|| globals.clone());
                    (globals, locals)
                }
                None => {
                    let globals = vm.current_globals().clone();
                    let locals = match self.locals {
                        Some(l) => l,
                        None => vm.current_locals()?,
                    };
                    (globals, locals)
                }
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
        source: Either<PyStrRef, PyCodeRef>,
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
        source: Either<PyStrRef, PyCodeRef>,
        scope: ScopeArgs,
        vm: &VirtualMachine,
    ) -> PyResult {
        run_code(vm, source, scope, compile::Mode::Exec, "exec")
    }

    #[cfg(feature = "rustpython-compiler")]
    fn run_code(
        vm: &VirtualMachine,
        source: Either<PyStrRef, PyCodeRef>,
        scope: ScopeArgs,
        mode: compile::Mode,
        func: &str,
    ) -> PyResult {
        let scope = scope.make_scope(vm)?;

        // Determine code object:
        let code_obj = match source {
            Either::A(string) => vm
                .compile(string.borrow_value(), mode, "<string>".to_owned())
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
                    obj.class().name
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
            vm.get_attribute(obj, attr)
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
        vm._hash(&obj)
    }

    // builtin_help

    #[pyfunction]
    fn hex(number: PyIntRef) -> String {
        let n = number.borrow_value();
        format!("{:#x}", n)
    }

    #[pyfunction]
    fn id(obj: PyObjectRef) -> usize {
        obj.get_id()
    }

    #[pyfunction]
    fn input(prompt: OptionalArg<PyStrRef>, vm: &VirtualMachine) -> PyResult {
        let stdin = sysmodule::get_stdin(vm)?;
        let stdout = sysmodule::get_stdout(vm)?;
        let stderr = sysmodule::get_stderr(vm)?;

        let _ = vm.call_method(&stderr, "flush", ());

        let fd_matches = |obj, expected| {
            vm.call_method(obj, "fileno", ())
                .and_then(|o| i64::try_from_object(vm, o))
                .ok()
                .map_or(false, |fd| fd == expected)
        };

        // everything is normalish, we can just rely on rustyline to use stdin/stdout
        if fd_matches(&stdin, 0) && fd_matches(&stdout, 1) && atty::is(atty::Stream::Stdin) {
            let prompt = prompt.as_ref().map_or("", |s| s.borrow_value());
            let mut readline = Readline::new(());
            match readline.readline(prompt) {
                ReadlineResult::Line(s) => Ok(vm.ctx.new_str(s)),
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
    pub fn isinstance(obj: PyObjectRef, typ: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        single_or_tuple_any(
            typ,
            &|cls: &PyTypeRef| vm.isinstance(&obj, cls),
            &|o| {
                format!(
                    "isinstance() arg 2 must be a type or tuple of types, not {}",
                    o.class()
                )
            },
            vm,
        )
    }

    #[pyfunction]
    fn issubclass(subclass: PyTypeRef, typ: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        single_or_tuple_any(
            typ,
            &|cls: &PyTypeRef| vm.issubclass(&subclass, cls),
            &|o| {
                format!(
                    "issubclass() arg 2 must be a class or tuple of classes, not {}",
                    o.class()
                )
            },
            vm,
        )
    }

    #[pyfunction]
    fn iter(
        iter_target: PyObjectRef,
        sentinel: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        if let OptionalArg::Present(sentinel) = sentinel {
            let callable = PyCallable::try_from_object(vm, iter_target)?;
            Ok(PyCallableIterator::new(callable, sentinel)
                .into_ref(vm)
                .into_object())
        } else {
            iterator::get_iter(vm, iter_target)
        }
    }

    #[pyfunction]
    fn len(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
        vm.obj_len(&obj)
    }

    #[pyfunction]
    fn locals(vm: &VirtualMachine) -> PyResult<PyDictRef> {
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
                if vm.bool_cmp(&y_key, &x_key, op)? {
                    x = y;
                    x_key = y_key;
                }
            }
        } else {
            for y in candidates_iter {
                if vm.bool_cmp(&y, &x, op)? {
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
    ) -> PyResult {
        iterator::call_next(vm, &iterator).or_else(|err| {
            if err.isinstance(&vm.ctx.exceptions.stop_iteration) {
                match default_value {
                    OptionalArg::Missing => Err(err),
                    OptionalArg::Present(value) => Ok(value),
                }
            } else {
                Err(err)
            }
        })
    }

    #[pyfunction]
    fn oct(number: PyIntRef, vm: &VirtualMachine) -> PyResult {
        let n = number.borrow_value();
        let s = if n.is_negative() {
            format!("-0o{:o}", n.abs())
        } else {
            format!("0o{:o}", n)
        };

        Ok(vm.ctx.new_str(s))
    }

    #[pyfunction]
    fn ord(string: Either<PyBytesLike, PyStrRef>, vm: &VirtualMachine) -> PyResult<u32> {
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
                let string = string.borrow_value();
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
        #[pyarg(any)]
        base: PyObjectRef,
        #[pyarg(any)]
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
                let try_pow_value = |obj: &PyObjectRef,
                                     args: (PyObjectRef, PyObjectRef, PyObjectRef)|
                 -> Option<PyResult> {
                    if let Some(method) = obj.get_class_attr("__pow__") {
                        let result = match vm.invoke(&method, args) {
                            Ok(x) => x,
                            Err(e) => return Some(Err(e)),
                        };
                        if let PyArithmaticValue::Implemented(x) =
                            PyArithmaticValue::from_object(vm, result)
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
        let code = exit_code_arg.unwrap_or_else(|| vm.ctx.new_int(0));
        Err(vm.new_exception(vm.ctx.exceptions.system_exit.clone(), vec![code]))
    }

    #[derive(Debug, Default, FromArgs)]
    pub struct PrintOptions {
        #[pyarg(named, default)]
        sep: Option<PyStrRef>,
        #[pyarg(named, default)]
        end: Option<PyStrRef>,
        #[pyarg(named, default = "IntoPyBool::FALSE")]
        flush: IntoPyBool,
        #[pyarg(named, default)]
        file: Option<PyObjectRef>,
    }

    #[pyfunction]
    pub fn print(objects: Args, options: PrintOptions, vm: &VirtualMachine) -> PyResult<()> {
        let file = match options.file {
            Some(f) => f,
            None => sysmodule::get_stdout(vm)?,
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

            write(vm.to_str(&object)?)?;
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
        vm.to_repr(&obj)
    }

    #[pyfunction]
    fn reversed(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if let Some(reversed_method) = vm.get_method(obj.clone(), "__reversed__") {
            vm.invoke(&reversed_method?, ())
        } else {
            vm.get_method_or_type_error(obj.clone(), "__getitem__", || {
                "argument to reversed() must be a sequence".to_owned()
            })?;
            let len = vm.obj_len(&obj)? as isize;
            let obj_iterator = PySequenceIterator::new_reversed(obj, len);
            Ok(obj_iterator.into_object(vm))
        }
    }

    #[derive(FromArgs)]
    pub struct RoundArgs {
        #[pyarg(any)]
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
                    number.class().name
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
        vm.set_attr(&obj, attr.into_object(), value)?;
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
        iterable: PyIterable,
        #[pyarg(any, optional)]
        start: OptionalArg<PyObjectRef>,
    }

    #[pyfunction]
    fn sum(SumArgs { iterable, start }: SumArgs, vm: &VirtualMachine) -> PyResult {
        // Start with zero and add at will:
        let mut sum = start.into_option().unwrap_or_else(|| vm.ctx.new_int(0));

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
            sum = vm._add(&sum, &item?)?;
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
            vm.get_attribute(obj, "__dict__")
        } else {
            Ok(vm.current_locals()?.into_object())
        }
    }

    #[pyfunction]
    pub fn __build_class__(
        function: PyFunctionRef,
        qualified_name: PyStrRef,
        bases: Args<PyTypeRef>,
        mut kwargs: KwArgs,
        vm: &VirtualMachine,
    ) -> PyResult {
        let name = qualified_name
            .borrow_value()
            .split('.')
            .next_back()
            .unwrap();
        let name_obj = vm.ctx.new_str(name);

        let mut metaclass = if let Some(metaclass) = kwargs.pop_kwarg("metaclass") {
            PyTypeRef::try_from_object(vm, metaclass)?
        } else {
            vm.ctx.types.type_type.clone()
        };

        for base in bases.clone() {
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

        let bases = bases.into_tuple(vm);

        // Prepare uses full __getattribute__ resolution chain.
        let prepare = vm.get_attribute(metaclass.clone().into_object(), "__prepare__")?;
        let namespace = vm.invoke(
            &prepare,
            FuncArgs::new(vec![name_obj.clone(), bases.clone()], kwargs.clone()),
        )?;

        let namespace = PyDictRef::try_from_object(vm, namespace)?;

        let classcell = function.invoke_with_locals(().into(), Some(namespace.clone()), vm)?;
        let classcell = <Option<PyCellRef>>::try_from_object(vm, classcell)?;

        let class = vm.invoke(
            metaclass.as_object(),
            FuncArgs::new(vec![name_obj, bases, namespace.into_object()], kwargs),
        )?;

        if let Some(ref classcell) = classcell {
            classcell.set(Some(class.clone()));
        }

        Ok(class)
    }
}

pub use decl::{ascii, isinstance, print};

pub fn make_module(vm: &VirtualMachine, module: PyObjectRef) {
    let ctx = &vm.ctx;

    decl::extend_module(vm, &module);

    let debug_mode: bool = vm.state.settings.optimize == 0;
    extend_module!(vm, module, {
        "__debug__" => ctx.new_bool(debug_mode),
        //set __name__ fixes: https://github.com/RustPython/RustPython/issues/146
        "__name__" => ctx.new_str(String::from("__main__")),

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
        "TargetScopeError" =>  ctx.exceptions.target_scope_error.clone(),
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

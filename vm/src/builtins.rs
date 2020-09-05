//! Builtin function definitions.
//!
//! Implements functions listed here: https://docs.python.org/3/library/builtins.html
use crate::pyobject::PyObjectRef;
use crate::vm::VirtualMachine;

#[pymodule(name = "builtins")]
mod decl {
    use num_bigint::Sign;
    use num_traits::{Signed, ToPrimitive, Zero};
    #[cfg(feature = "rustpython-compiler")]
    use rustpython_compiler::compile;
    #[cfg(feature = "rustpython-parser")]
    use rustpython_parser::parser;

    use super::to_ascii;
    use crate::byteslike::PyBytesLike;
    use crate::exceptions::PyBaseExceptionRef;
    use crate::function::{single_or_tuple_any, Args, KwArgs, OptionalArg, PyFuncArgs};
    use crate::obj::objbool::{self, IntoPyBool};
    use crate::obj::objbytes::PyBytesRef;
    use crate::obj::objcode::PyCodeRef;
    use crate::obj::objdict::PyDictRef;
    use crate::obj::objfunction::PyFunctionRef;
    use crate::obj::objint::{self, PyIntRef};
    use crate::obj::objiter;
    use crate::obj::objlist::{PyList, SortOptions};
    use crate::obj::objsequence;
    use crate::obj::objstr::{PyString, PyStringRef};
    use crate::obj::objtype::{self, PyClassRef};
    use crate::pyobject::{
        BorrowValue, Either, IdProtocol, ItemProtocol, PyCallable, PyIterable, PyObjectRef,
        PyResult, PyValue, TryFromObject, TypeProtocol,
    };
    use crate::readline::{Readline, ReadlineResult};
    use crate::scope::Scope;
    #[cfg(feature = "rustpython-parser")]
    use crate::stdlib::ast;
    use crate::sysmodule;
    use crate::vm::VirtualMachine;
    use rustpython_common::hash::PyHash;

    #[pyfunction]
    fn abs(x: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let method = vm.get_method_or_type_error(x.clone(), "__abs__", || {
            format!("bad operand type for abs(): '{}'", x.lease_class().name)
        })?;
        vm.invoke(&method, PyFuncArgs::new(vec![], vec![]))
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
    fn ascii(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<String> {
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
        #[pyarg(positional_only, optional = false)]
        source: Either<PyStringRef, PyBytesRef>,
        #[pyarg(positional_only, optional = false)]
        filename: PyStringRef,
        #[pyarg(positional_only, optional = false)]
        mode: PyStringRef,
        #[pyarg(positional_or_keyword, optional = true)]
        flags: OptionalArg<PyIntRef>,
        #[pyarg(positional_or_keyword, optional = true)]
        dont_inherit: OptionalArg<bool>,
        #[pyarg(positional_or_keyword, optional = true)]
        optimize: OptionalArg<PyIntRef>,
    }

    #[cfg(feature = "rustpython-compiler")]
    #[pyfunction]
    fn compile(args: CompileArgs, vm: &VirtualMachine) -> PyResult {
        // TODO: compile::compile should probably get bytes
        let source = match &args.source {
            Either::A(string) => string.borrow_value(),
            Either::B(bytes) => std::str::from_utf8(bytes).unwrap(),
        };

        let mode_str = args.mode.borrow_value();

        let flags = args
            .flags
            .map_or(Ok(0), |v| i32::try_from_object(vm, v.into_object()))?;

        #[cfg(feature = "rustpython-parser")]
        {
            if (flags & ast::PY_COMPILE_FLAG_AST_ONLY).is_zero() {
                let mode = mode_str
                    .parse::<compile::Mode>()
                    .map_err(|err| vm.new_value_error(err.to_string()))?;

                vm.compile(&source, mode, args.filename.borrow_value().to_owned())
                    .map(|o| o.into_object())
                    .map_err(|err| vm.new_syntax_error(&err))
            } else {
                let mode = mode_str
                    .parse::<parser::Mode>()
                    .map_err(|err| vm.new_value_error(err.to_string()))?;
                ast::parse(&vm, &source, mode)
            }
        }
        #[cfg(not(feature = "rustpython-parser"))]
        {
            Err(vm.new_value_error(
                "PyCF_ONLY_AST flag is not supported without parser support".to_string(),
            ))
        }
    }

    #[pyfunction]
    fn delattr(obj: PyObjectRef, attr: PyStringRef, vm: &VirtualMachine) -> PyResult<()> {
        vm.del_attr(&obj, attr.into_object())
    }

    #[pyfunction]
    fn dir(obj: OptionalArg<PyObjectRef>, vm: &VirtualMachine) -> PyResult<PyList> {
        let seq = match obj {
            OptionalArg::Present(obj) => vm.call_method(&obj, "__dir__", vec![])?,
            OptionalArg::Missing => {
                vm.call_method(&vm.get_locals().into_object(), "keys", vec![])?
            }
        };
        let sorted = sorted(seq, Default::default(), vm)?;
        Ok(sorted)
    }

    #[pyfunction]
    fn divmod(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm.call_or_reflection(a, b, "__divmod__", "__rdivmod__", |vm, a, b| {
            Err(vm.new_unsupported_operand_error(a, b, "divmod"))
        })
    }

    #[cfg(feature = "rustpython-compiler")]
    #[derive(FromArgs)]
    struct ScopeArgs {
        #[pyarg(positional_or_keyword, default = "None")]
        globals: Option<PyDictRef>,
        // TODO: support any mapping for `locals`
        #[pyarg(positional_or_keyword, default = "None")]
        locals: Option<PyDictRef>,
    }

    /// Implements `eval`.
    /// See also: https://docs.python.org/3/library/functions.html#eval
    #[cfg(feature = "rustpython-compiler")]
    #[pyfunction]
    fn eval(
        source: Either<PyStringRef, PyCodeRef>,
        scope: ScopeArgs,
        vm: &VirtualMachine,
    ) -> PyResult {
        run_code(vm, source, scope, compile::Mode::Eval)
    }

    /// Implements `exec`
    /// https://docs.python.org/3/library/functions.html#exec
    #[cfg(feature = "rustpython-compiler")]
    #[pyfunction]
    fn exec(
        source: Either<PyStringRef, PyCodeRef>,
        scope: ScopeArgs,
        vm: &VirtualMachine,
    ) -> PyResult {
        run_code(vm, source, scope, compile::Mode::Exec)
    }

    #[cfg(feature = "rustpython-compiler")]
    fn run_code(
        vm: &VirtualMachine,
        source: Either<PyStringRef, PyCodeRef>,
        scope: ScopeArgs,
        mode: compile::Mode,
    ) -> PyResult {
        let scope = make_scope(vm, scope)?;

        // Determine code object:
        let code_obj = match source {
            Either::A(string) => vm
                .compile(string.borrow_value(), mode, "<string>".to_owned())
                .map_err(|err| vm.new_syntax_error(&err))?,
            Either::B(code_obj) => code_obj,
        };

        // Run the code:
        vm.run_code_obj(code_obj, scope)
    }

    #[cfg(feature = "rustpython-compiler")]
    fn make_scope(vm: &VirtualMachine, scope: ScopeArgs) -> PyResult<Scope> {
        let globals = scope.globals;
        let current_scope = vm.current_scope();
        let locals = match scope.locals {
            Some(dict) => Some(dict),
            None => {
                if globals.is_some() {
                    None
                } else {
                    current_scope.get_only_locals()
                }
            }
        };
        let globals = match globals {
            Some(dict) => {
                if !dict.contains_key("__builtins__", vm) {
                    let builtins_dict = vm.builtins.dict().unwrap().as_object().clone();
                    dict.set_item("__builtins__", builtins_dict, vm).unwrap();
                }
                dict
            }
            None => current_scope.globals.clone(),
        };

        let scope = Scope::with_builtins(locals, globals, vm);
        Ok(scope)
    }

    #[pyfunction]
    fn format(
        value: PyObjectRef,
        format_spec: OptionalArg<PyStringRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyStringRef> {
        let format_spec = format_spec
            .into_option()
            .unwrap_or_else(|| PyString::from("").into_ref(vm));

        vm.call_method(&value, "__format__", vec![format_spec.into_object()])?
            .downcast()
            .map_err(|obj| {
                vm.new_type_error(format!(
                    "__format__ must return a str, not {}",
                    obj.lease_class().name
                ))
            })
    }

    fn catch_attr_exception<T>(
        ex: PyBaseExceptionRef,
        default: T,
        vm: &VirtualMachine,
    ) -> PyResult<T> {
        if objtype::isinstance(&ex, &vm.ctx.exceptions.attribute_error) {
            Ok(default)
        } else {
            Err(ex)
        }
    }

    #[pyfunction]
    fn getattr(
        obj: PyObjectRef,
        attr: PyStringRef,
        default: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let ret = vm.get_attribute(obj, attr);
        if let OptionalArg::Present(default) = default {
            ret.or_else(|ex| catch_attr_exception(ex, default, vm))
        } else {
            ret
        }
    }

    #[pyfunction]
    fn globals(vm: &VirtualMachine) -> PyResult<PyDictRef> {
        Ok(vm.current_scope().globals.clone())
    }

    #[pyfunction]
    fn hasattr(obj: PyObjectRef, attr: PyStringRef, vm: &VirtualMachine) -> PyResult<bool> {
        if let Err(ex) = vm.get_attribute(obj, attr) {
            catch_attr_exception(ex, false, vm)
        } else {
            Ok(true)
        }
    }

    #[pyfunction]
    fn hash(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyHash> {
        vm._hash(&obj)
    }

    // builtin_help

    #[pyfunction]
    fn hex(number: PyIntRef, vm: &VirtualMachine) -> PyResult {
        let n = number.borrow_value();
        let s = if n.is_negative() {
            format!("-0x{:x}", -n)
        } else {
            format!("0x{:x}", n)
        };

        Ok(vm.ctx.new_str(s))
    }

    #[pyfunction]
    fn id(obj: PyObjectRef) -> usize {
        obj.get_id()
    }

    #[pyfunction]
    fn input(prompt: OptionalArg<PyStringRef>, vm: &VirtualMachine) -> PyResult<String> {
        let prompt = prompt.as_ref().map_or("", |s| s.borrow_value());
        let mut readline = Readline::new(());
        match readline.readline(prompt) {
            ReadlineResult::Line(s) => Ok(s),
            ReadlineResult::EOF => Err(vm.new_exception_empty(vm.ctx.exceptions.eof_error.clone())),
            ReadlineResult::Interrupt => {
                Err(vm.new_exception_empty(vm.ctx.exceptions.keyboard_interrupt.clone()))
            }
            ReadlineResult::IO(e) => Err(vm.new_os_error(e.to_string())),
            ReadlineResult::EncodingError => {
                Err(vm.new_unicode_decode_error("Error decoding readline input".to_owned()))
            }
            ReadlineResult::Other(e) => Err(vm.new_runtime_error(e.to_string())),
        }
    }

    #[pyfunction]
    pub fn isinstance(obj: PyObjectRef, typ: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        single_or_tuple_any(
            typ,
            &|cls: &PyClassRef| vm.isinstance(&obj, cls),
            &|o| {
                format!(
                    "isinstance() arg 2 must be a type or tuple of types, not {}",
                    o.lease_class()
                )
            },
            vm,
        )
    }

    #[pyfunction]
    fn issubclass(subclass: PyClassRef, typ: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        single_or_tuple_any(
            typ,
            &|cls: &PyClassRef| vm.issubclass(&subclass, cls),
            &|o| {
                format!(
                    "issubclass() arg 2 must be a class or tuple of classes, not {}",
                    o.lease_class()
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
            Ok(objiter::PyCallableIterator::new(callable, sentinel)
                .into_ref(vm)
                .into_object())
        } else {
            objiter::get_iter(vm, &iter_target)
        }
    }

    #[pyfunction]
    fn len(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
        objsequence::len(&obj, vm)
    }

    #[pyfunction]
    fn locals(vm: &VirtualMachine) -> PyDictRef {
        let locals = vm.get_locals();
        locals.copy().into_ref(vm)
    }

    #[pyfunction]
    fn max(vm: &VirtualMachine, mut args: PyFuncArgs) -> PyResult {
        let default = args.take_keyword("default");
        let key_func = args.take_keyword("key");
        if !args.kwargs.is_empty() {
            let invalid_keyword = args.kwargs.get_index(0).unwrap();
            return Err(vm.new_type_error(format!(
                "'{}' is an invalid keyword argument for max()",
                invalid_keyword.0
            )));
        }
        let candidates = match args.args.len().cmp(&1) {
            std::cmp::Ordering::Greater => {
                if default.is_some() {
                    return Err(vm.new_type_error(
                        "Cannot specify a default for max() with multiple positional arguments"
                            .to_owned(),
                    ));
                }
                args.args.clone()
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
                return default
                    .ok_or_else(|| vm.new_value_error("max() arg is an empty sequence".to_owned()))
            }
        };

        let key_func = key_func.filter(|f| !vm.is_none(f));
        if let Some(ref key_func) = key_func {
            let mut x_key = vm.invoke(key_func, x.clone())?;
            for y in candidates_iter {
                let y_key = vm.invoke(key_func, y.clone())?;
                let y_gt_x = objbool::boolval(vm, vm._gt(y_key.clone(), x_key.clone())?)?;
                if y_gt_x {
                    x = y;
                    x_key = y_key;
                }
            }
        } else {
            for y in candidates_iter {
                let y_gt_x = objbool::boolval(vm, vm._gt(y.clone(), x.clone())?)?;
                if y_gt_x {
                    x = y;
                }
            }
        }

        Ok(x)
    }

    #[pyfunction]
    fn min(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
        let candidates = match args.args.len().cmp(&1) {
            std::cmp::Ordering::Greater => args.args.clone(),
            std::cmp::Ordering::Equal => vm.extract_elements(&args.args[0])?,
            std::cmp::Ordering::Less => {
                // zero arguments means type error:
                return Err(vm.new_type_error("Expected 1 or more arguments".to_owned()));
            }
        };

        if candidates.is_empty() {
            let default = args.get_optional_kwarg("default");
            return default
                .ok_or_else(|| vm.new_value_error("min() arg is an empty sequence".to_owned()));
        }

        let key_func = args.get_optional_kwarg("key");

        let mut candidates_iter = candidates.into_iter();
        let mut x = candidates_iter.next().unwrap();
        // TODO: this key function looks pretty duplicate. Maybe we can create
        // a local function?
        let mut x_key = if let Some(ref f) = &key_func {
            vm.invoke(f, vec![x.clone()])?
        } else {
            x.clone()
        };

        for y in candidates_iter {
            let y_key = if let Some(ref f) = &key_func {
                vm.invoke(f, vec![y.clone()])?
            } else {
                y.clone()
            };
            let order = vm._gt(x_key.clone(), y_key.clone())?;

            if objbool::get_value(&order) {
                x = y.clone();
                x_key = y_key;
            }
        }

        Ok(x)
    }

    #[pyfunction]
    fn next(
        iterator: PyObjectRef,
        default_value: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        match vm.call_method(&iterator, "__next__", vec![]) {
            Ok(value) => Ok(value),
            Err(value) => {
                if objtype::isinstance(&value, &vm.ctx.exceptions.stop_iteration) {
                    match default_value {
                        OptionalArg::Missing => Err(value),
                        OptionalArg::Present(value) => Ok(value),
                    }
                } else {
                    Err(value)
                }
            }
        }
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
    fn ord(string: Either<PyBytesLike, PyStringRef>, vm: &VirtualMachine) -> PyResult<u32> {
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

    #[pyfunction]
    fn pow(
        x: PyObjectRef,
        y: PyObjectRef,
        mod_value: OptionalArg<PyIntRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        match mod_value {
            OptionalArg::Missing => {
                vm.call_or_reflection(x, y, "__pow__", "__rpow__", |vm, x, y| {
                    Err(vm.new_unsupported_operand_error(x, y, "pow"))
                })
            }
            OptionalArg::Present(m) => {
                // Check if the 3rd argument is defined and perform modulus on the result
                if !(objtype::isinstance(&x, &vm.ctx.types.int_type)
                    && objtype::isinstance(&y, &vm.ctx.types.int_type))
                {
                    return Err(vm.new_type_error(
                        "pow() 3rd argument not allowed unless all arguments are integers"
                            .to_owned(),
                    ));
                }
                let y = objint::get_value(&y);
                if y.sign() == Sign::Minus {
                    return Err(vm.new_value_error(
                        "pow() 2nd argument cannot be negative when 3rd argument specified"
                            .to_owned(),
                    ));
                }
                let m = m.borrow_value();
                if m.is_zero() {
                    return Err(vm.new_value_error("pow() 3rd argument cannot be 0".to_owned()));
                }
                let x = objint::get_value(&x);
                Ok(vm.ctx.new_int(x.modpow(&y, &m)))
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
        #[pyarg(keyword_only, default = "None")]
        sep: Option<PyStringRef>,
        #[pyarg(keyword_only, default = "None")]
        end: Option<PyStringRef>,
        #[pyarg(keyword_only, default = "IntoPyBool::FALSE")]
        flush: IntoPyBool,
        #[pyarg(keyword_only, default = "None")]
        file: Option<PyObjectRef>,
    }

    #[pyfunction]
    pub fn print(objects: Args, options: PrintOptions, vm: &VirtualMachine) -> PyResult<()> {
        let file = match options.file {
            Some(f) => f,
            None => sysmodule::get_stdout(vm)?,
        };
        let write = |obj: PyStringRef| vm.call_method(&file, "write", vec![obj.into_object()]);

        let sep = options
            .sep
            .unwrap_or_else(|| PyString::from(" ").into_ref(vm));

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
            .unwrap_or_else(|| PyString::from("\n").into_ref(vm));
        write(end)?;

        if options.flush.to_bool() {
            vm.call_method(&file, "flush", vec![])?;
        }

        Ok(())
    }

    #[pyfunction]
    fn repr(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyStringRef> {
        vm.to_repr(&obj)
    }

    #[pyfunction]
    fn reversed(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if let Some(reversed_method) = vm.get_method(obj.clone(), "__reversed__") {
            vm.invoke(&reversed_method?, PyFuncArgs::default())
        } else {
            vm.get_method_or_type_error(obj.clone(), "__getitem__", || {
                "argument to reversed() must be a sequence".to_owned()
            })?;
            let len = vm.call_method(&obj, "__len__", PyFuncArgs::default())?;
            let len = objint::get_value(&len).to_isize().unwrap();
            let obj_iterator = objiter::PySequenceIterator::new_reversed(obj, len);
            Ok(obj_iterator.into_object(vm))
        }
    }

    #[pyfunction]
    fn round(
        number: PyObjectRef,
        ndigits: OptionalArg<Option<PyIntRef>>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let rounded = match ndigits {
            OptionalArg::Present(ndigits) => match ndigits {
                Some(int) => {
                    let ndigits = vm.call_method(int.as_object(), "__int__", vec![])?;
                    vm.call_method(&number, "__round__", vec![ndigits])?
                }
                None => vm.call_method(&number, "__round__", vec![])?,
            },
            OptionalArg::Missing => {
                // without a parameter, the result type is coerced to int
                vm.call_method(&number, "__round__", vec![])?
            }
        };
        Ok(rounded)
    }

    #[pyfunction]
    fn setattr(
        obj: PyObjectRef,
        attr: PyStringRef,
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

    #[pyfunction]
    fn sum(iterable: PyIterable, start: OptionalArg, vm: &VirtualMachine) -> PyResult {
        // Start with zero and add at will:
        let mut sum = start.into_option().unwrap_or_else(|| vm.ctx.new_int(0));
        for item in iterable.iter(vm)? {
            sum = vm._add(sum, item?)?;
        }
        Ok(sum)
    }

    #[pyfunction]
    fn __import__(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
        vm.invoke(&vm.import_func, args)
    }

    #[pyfunction]
    fn vars(obj: OptionalArg, vm: &VirtualMachine) -> PyResult {
        if let OptionalArg::Present(obj) = obj {
            vm.get_attribute(obj, "__dict__")
        } else {
            Ok(vm.get_locals().into_object())
        }
    }

    #[pyfunction]
    pub fn __build_class__(
        function: PyFunctionRef,
        qualified_name: PyStringRef,
        bases: Args<PyClassRef>,
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
            PyClassRef::try_from_object(vm, metaclass)?
        } else {
            vm.ctx.types.type_type.clone()
        };

        for base in bases.clone() {
            let base_class = base.lease_class();
            if objtype::issubclass(&base_class, &metaclass) {
                metaclass = base.class();
            } else if !objtype::issubclass(&metaclass, &base_class) {
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
        let namespace = vm.invoke(&prepare, vec![name_obj.clone(), bases.clone()])?;

        let namespace: PyDictRef = TryFromObject::try_from_object(vm, namespace)?;

        let cells = vm.ctx.new_dict();

        let scope = function
            .scope()
            .new_child_scope_with_locals(cells.clone())
            .new_child_scope_with_locals(namespace.clone());

        function.invoke_with_scope(vec![].into(), &scope, vm)?;

        let class = vm.invoke(
            metaclass.as_object(),
            (
                Args::from(vec![name_obj, bases, namespace.into_object()]),
                kwargs,
            ),
        )?;
        cells.set_item("__class__", class.clone(), vm)?;
        Ok(class)
    }
}

pub use decl::isinstance as builtin_isinstance;
pub use decl::print as builtin_print;

/// Convert a string to ascii compatible, escaping unicodes into escape
/// sequences.
pub fn to_ascii(value: &str) -> String {
    let mut ascii = String::new();
    for c in value.chars() {
        if c.is_ascii() {
            ascii.push(c)
        } else {
            let c = c as i64;
            let hex = if c < 0x100 {
                format!("\\x{:02x}", c)
            } else if c < 0x10000 {
                format!("\\u{:04x}", c)
            } else {
                format!("\\U{:08x}", c)
            };
            ascii.push_str(&hex)
        }
    }
    ascii
}

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

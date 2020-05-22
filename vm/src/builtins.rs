//! Builtin function definitions.
//!
//! Implements functions listed here: https://docs.python.org/3/library/builtins.html

use std::char;
use std::str;

use num_bigint::Sign;
use num_traits::{Signed, ToPrimitive, Zero};
#[cfg(feature = "rustpython-compiler")]
use rustpython_compiler::compile;
#[cfg(feature = "rustpython-parser")]
use rustpython_parser::parser;

use crate::exceptions::PyBaseExceptionRef;
use crate::function::{single_or_tuple_any, Args, KwArgs, OptionalArg, PyFuncArgs};
use crate::obj::objbool::{self, IntoPyBool};
use crate::obj::objbyteinner::PyByteInner;
use crate::obj::objbytes::PyBytesRef;
use crate::obj::objcode::PyCodeRef;
use crate::obj::objdict::PyDictRef;
use crate::obj::objfunction::PyFunctionRef;
use crate::obj::objint::{self, PyIntRef};
use crate::obj::objiter;
use crate::obj::objsequence;
use crate::obj::objstr::{PyString, PyStringRef};
use crate::obj::objtype::{self, PyClassRef};
use crate::pyhash;
use crate::pyobject::{
    Either, IdProtocol, ItemProtocol, PyCallable, PyIterable, PyObjectRef, PyResult, PyValue,
    TryFromObject, TypeProtocol,
};
use crate::readline::{Readline, ReadlineResult};
use crate::scope::Scope;
#[cfg(feature = "rustpython-parser")]
use crate::stdlib::ast;
use crate::vm::VirtualMachine;

fn builtin_abs(x: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    let method = vm.get_method_or_type_error(x.clone(), "__abs__", || {
        format!("bad operand type for abs(): '{}'", x.class().name)
    })?;
    vm.invoke(&method, PyFuncArgs::new(vec![], vec![]))
}

fn builtin_all(iterable: PyIterable<IntoPyBool>, vm: &VirtualMachine) -> PyResult<bool> {
    for item in iterable.iter(vm)? {
        if !item?.to_bool() {
            return Ok(false);
        }
    }
    Ok(true)
}

fn builtin_any(iterable: PyIterable<IntoPyBool>, vm: &VirtualMachine) -> PyResult<bool> {
    for item in iterable.iter(vm)? {
        if item?.to_bool() {
            return Ok(true);
        }
    }
    Ok(false)
}

fn builtin_ascii(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<String> {
    let repr = vm.to_repr(&obj)?;
    let ascii = to_ascii(repr.as_str());
    Ok(ascii)
}

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

fn builtin_bin(x: PyIntRef) -> String {
    let x = x.as_bigint();
    if x.is_negative() {
        format!("-0b{:b}", x.abs())
    } else {
        format!("0b{:b}", x)
    }
}

// builtin_breakpoint

fn builtin_callable(obj: PyObjectRef, vm: &VirtualMachine) -> bool {
    vm.is_callable(&obj)
}

fn builtin_chr(i: u32, vm: &VirtualMachine) -> PyResult<String> {
    match char::from_u32(i) {
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
fn builtin_compile(args: CompileArgs, vm: &VirtualMachine) -> PyResult {
    // TODO: compile::compile should probably get bytes
    let source = match &args.source {
        Either::A(string) => string.as_str(),
        Either::B(bytes) => str::from_utf8(bytes).unwrap(),
    };

    let mode_str = args.mode.as_str();

    let flags = args
        .flags
        .map_or(Ok(0), |v| i32::try_from_object(vm, v.into_object()))?;

    #[cfg(feature = "rustpython-parser")]
    {
        if (flags & ast::PY_COMPILE_FLAG_AST_ONLY).is_zero() {
            let mode = mode_str
                .parse::<compile::Mode>()
                .map_err(|err| vm.new_value_error(err.to_string()))?;

            vm.compile(&source, mode, args.filename.as_str().to_owned())
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

fn builtin_delattr(obj: PyObjectRef, attr: PyStringRef, vm: &VirtualMachine) -> PyResult<()> {
    vm.del_attr(&obj, attr.into_object())
}

fn builtin_dir(obj: OptionalArg<PyObjectRef>, vm: &VirtualMachine) -> PyResult {
    let seq = match obj {
        OptionalArg::Present(obj) => vm.call_method(&obj, "__dir__", vec![])?,
        OptionalArg::Missing => vm.call_method(&vm.get_locals().into_object(), "keys", vec![])?,
    };
    let sorted = builtin_sorted(vm, PyFuncArgs::new(vec![seq], vec![]))?;
    Ok(sorted)
}

fn builtin_divmod(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    vm.call_or_reflection(
        a.clone(),
        b.clone(),
        "__divmod__",
        "__rdivmod__",
        |vm, a, b| Err(vm.new_unsupported_operand_error(a, b, "divmod")),
    )
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
fn builtin_eval(
    source: Either<PyStringRef, PyCodeRef>,
    scope: ScopeArgs,
    vm: &VirtualMachine,
) -> PyResult {
    run_code(vm, source, scope, compile::Mode::Eval)
}

/// Implements `exec`
/// https://docs.python.org/3/library/functions.html#exec
#[cfg(feature = "rustpython-compiler")]
fn builtin_exec(
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
            .compile(string.as_str(), mode, "<string>".to_owned())
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

fn builtin_format(
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
                obj.class().name
            ))
        })
}

fn catch_attr_exception<T>(ex: PyBaseExceptionRef, default: T, vm: &VirtualMachine) -> PyResult<T> {
    if objtype::isinstance(&ex, &vm.ctx.exceptions.attribute_error) {
        Ok(default)
    } else {
        Err(ex)
    }
}

fn builtin_getattr(
    obj: PyObjectRef,
    attr: PyStringRef,
    default: OptionalArg<PyObjectRef>,
    vm: &VirtualMachine,
) -> PyResult {
    let ret = vm.get_attribute(obj.clone(), attr);
    if let OptionalArg::Present(default) = default {
        ret.or_else(|ex| catch_attr_exception(ex, default, vm))
    } else {
        ret
    }
}

fn builtin_globals(vm: &VirtualMachine) -> PyResult<PyDictRef> {
    Ok(vm.current_scope().globals.clone())
}

fn builtin_hasattr(obj: PyObjectRef, attr: PyStringRef, vm: &VirtualMachine) -> PyResult<bool> {
    if let Err(ex) = vm.get_attribute(obj.clone(), attr) {
        catch_attr_exception(ex, false, vm)
    } else {
        Ok(true)
    }
}

fn builtin_hash(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<pyhash::PyHash> {
    vm._hash(&obj)
}

// builtin_help

fn builtin_hex(number: PyIntRef, vm: &VirtualMachine) -> PyResult {
    let n = number.as_bigint();
    let s = if n.is_negative() {
        format!("-0x{:x}", -n)
    } else {
        format!("0x{:x}", n)
    };

    Ok(vm.new_str(s))
}

fn builtin_id(obj: PyObjectRef) -> usize {
    obj.get_id()
}

fn builtin_input(prompt: OptionalArg<PyStringRef>, vm: &VirtualMachine) -> PyResult<String> {
    let prompt = prompt.as_ref().map_or("", |s| s.as_str());
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

pub fn builtin_isinstance(
    obj: PyObjectRef,
    typ: PyObjectRef,
    vm: &VirtualMachine,
) -> PyResult<bool> {
    single_or_tuple_any(
        typ,
        |cls: &PyClassRef| vm.isinstance(&obj, cls),
        |o| {
            format!(
                "isinstance() arg 2 must be a type or tuple of types, not {}",
                o.class()
            )
        },
        vm,
    )
}

fn builtin_issubclass(
    subclass: PyClassRef,
    typ: PyObjectRef,
    vm: &VirtualMachine,
) -> PyResult<bool> {
    single_or_tuple_any(
        typ,
        |cls: &PyClassRef| vm.issubclass(&subclass, cls),
        |o| {
            format!(
                "issubclass() arg 2 must be a class or tuple of classes, not {}",
                o.class()
            )
        },
        vm,
    )
}

fn builtin_iter(
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

fn builtin_len(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
    objsequence::len(&obj, vm)
}

fn builtin_locals(vm: &VirtualMachine) -> PyDictRef {
    let locals = vm.get_locals();
    locals.copy().into_ref(vm)
}

fn builtin_max(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
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
            .ok_or_else(|| vm.new_value_error("max() arg is an empty sequence".to_owned()));
    }

    let key_func = args.get_optional_kwarg("key");

    // Start with first assumption:
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

        if !objbool::get_value(&order) {
            x = y.clone();
            x_key = y_key;
        }
    }

    Ok(x)
}

fn builtin_min(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
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

fn builtin_next(
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
                    OptionalArg::Present(value) => Ok(value.clone()),
                }
            } else {
                Err(value)
            }
        }
    }
}

fn builtin_oct(number: PyIntRef, vm: &VirtualMachine) -> PyResult {
    let n = number.as_bigint();
    let s = if n.is_negative() {
        format!("-0o{:o}", n.abs())
    } else {
        format!("0o{:o}", n)
    };

    Ok(vm.new_str(s))
}

fn builtin_ord(string: Either<PyByteInner, PyStringRef>, vm: &VirtualMachine) -> PyResult<u32> {
    match string {
        Either::A(bytes) => {
            let bytes_len = bytes.elements.len();
            if bytes_len != 1 {
                return Err(vm.new_type_error(format!(
                    "ord() expected a character, but string of length {} found",
                    bytes_len
                )));
            }
            Ok(u32::from(bytes.elements[0]))
        }
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

fn builtin_pow(
    x: PyObjectRef,
    y: PyObjectRef,
    mod_value: OptionalArg<PyIntRef>,
    vm: &VirtualMachine,
) -> PyResult {
    match mod_value {
        OptionalArg::Missing => {
            vm.call_or_reflection(x.clone(), y.clone(), "__pow__", "__rpow__", |vm, x, y| {
                Err(vm.new_unsupported_operand_error(x, y, "pow"))
            })
        }
        OptionalArg::Present(m) => {
            // Check if the 3rd argument is defined and perform modulus on the result
            if !(objtype::isinstance(&x, &vm.ctx.int_type())
                && objtype::isinstance(&y, &vm.ctx.int_type()))
            {
                return Err(vm.new_type_error(
                    "pow() 3rd argument not allowed unless all arguments are integers".to_owned(),
                ));
            }
            let y = objint::get_value(&y);
            if y.sign() == Sign::Minus {
                return Err(vm.new_value_error(
                    "pow() 2nd argument cannot be negative when 3rd argument specified".to_owned(),
                ));
            }
            let m = m.as_bigint();
            if m.is_zero() {
                return Err(vm.new_value_error("pow() 3rd argument cannot be 0".to_owned()));
            }
            let x = objint::get_value(&x);
            Ok(vm.new_int(x.modpow(&y, &m)))
        }
    }
}

pub fn builtin_exit(exit_code_arg: OptionalArg<PyObjectRef>, vm: &VirtualMachine) -> PyResult {
    let code = exit_code_arg.unwrap_or_else(|| vm.new_int(0));
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

pub fn builtin_print(objects: Args, options: PrintOptions, vm: &VirtualMachine) -> PyResult<()> {
    let file = match options.file {
        Some(f) => f,
        None => vm.get_attribute(vm.sys_module.clone(), "stdout")?,
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

fn builtin_repr(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyStringRef> {
    vm.to_repr(&obj)
}

fn builtin_reversed(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    if let Some(reversed_method) = vm.get_method(obj.clone(), "__reversed__") {
        vm.invoke(&reversed_method?, PyFuncArgs::default())
    } else {
        vm.get_method_or_type_error(obj.clone(), "__getitem__", || {
            "argument to reversed() must be a sequence".to_owned()
        })?;
        let len = vm.call_method(&obj, "__len__", PyFuncArgs::default())?;
        let len = objint::get_value(&len).to_isize().unwrap();
        let obj_iterator = objiter::PySequenceIterator::new_reversed(obj, len);
        Ok(obj_iterator.into_ref(vm).into_object())
    }
}

fn builtin_round(
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

fn builtin_setattr(
    obj: PyObjectRef,
    attr: PyStringRef,
    value: PyObjectRef,
    vm: &VirtualMachine,
) -> PyResult<()> {
    vm.set_attr(&obj, attr.into_object(), value)?;
    Ok(())
}

// builtin_slice

fn builtin_sorted(vm: &VirtualMachine, mut args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(iterable, None)]);
    let items = vm.extract_elements(iterable)?;
    let lst = vm.ctx.new_list(items);

    args.shift();
    vm.call_method(&lst, "sort", args)?;
    Ok(lst)
}

fn builtin_sum(iterable: PyIterable, start: OptionalArg, vm: &VirtualMachine) -> PyResult {
    // Start with zero and add at will:
    let mut sum = start.into_option().unwrap_or_else(|| vm.ctx.new_int(0));
    for item in iterable.iter(vm)? {
        sum = vm._add(sum, item?)?;
    }
    Ok(sum)
}

// Should be renamed to builtin___import__?
fn builtin_import(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    vm.invoke(&vm.import_func, args)
}

fn builtin_vars(obj: OptionalArg, vm: &VirtualMachine) -> PyResult {
    if let OptionalArg::Present(obj) = obj {
        vm.get_attribute(obj, "__dict__")
    } else {
        Ok(vm.get_locals().into_object())
    }
}

// builtin_vars

pub fn make_module(vm: &VirtualMachine, module: PyObjectRef) {
    let ctx = &vm.ctx;

    #[cfg(feature = "rustpython-compiler")]
    {
        extend_module!(vm, module, {
            "eval" => ctx.new_function(builtin_eval),
            "exec" => ctx.new_function(builtin_exec),
            "compile" => ctx.new_function(builtin_compile),
        });
    }

    let debug_mode: bool = vm.state.settings.optimize == 0;
    extend_module!(vm, module, {
        "__debug__" => ctx.new_bool(debug_mode),
        //set __name__ fixes: https://github.com/RustPython/RustPython/issues/146
        "__name__" => ctx.new_str(String::from("__main__")),

        "abs" => ctx.new_function(builtin_abs),
        "all" => ctx.new_function(builtin_all),
        "any" => ctx.new_function(builtin_any),
        "ascii" => ctx.new_function(builtin_ascii),
        "bin" => ctx.new_function(builtin_bin),
        "bool" => ctx.bool_type(),
        "bytearray" => ctx.bytearray_type(),
        "bytes" => ctx.bytes_type(),
        "callable" => ctx.new_function(builtin_callable),
        "chr" => ctx.new_function(builtin_chr),
        "classmethod" => ctx.classmethod_type(),
        "complex" => ctx.complex_type(),
        "delattr" => ctx.new_function(builtin_delattr),
        "dict" => ctx.dict_type(),
        "divmod" => ctx.new_function(builtin_divmod),
        "dir" => ctx.new_function(builtin_dir),
        "enumerate" => ctx.enumerate_type(),
        "float" => ctx.float_type(),
        "frozenset" => ctx.frozenset_type(),
        "filter" => ctx.filter_type(),
        "format" => ctx.new_function(builtin_format),
        "getattr" => ctx.new_function(builtin_getattr),
        "globals" => ctx.new_function(builtin_globals),
        "hasattr" => ctx.new_function(builtin_hasattr),
        "hash" => ctx.new_function(builtin_hash),
        "hex" => ctx.new_function(builtin_hex),
        "id" => ctx.new_function(builtin_id),
        "input" => ctx.new_function(builtin_input),
        "int" => ctx.int_type(),
        "isinstance" => ctx.new_function(builtin_isinstance),
        "issubclass" => ctx.new_function(builtin_issubclass),
        "iter" => ctx.new_function(builtin_iter),
        "len" => ctx.new_function(builtin_len),
        "list" => ctx.list_type(),
        "locals" => ctx.new_function(builtin_locals),
        "map" => ctx.map_type(),
        "max" => ctx.new_function(builtin_max),
        "memoryview" => ctx.memoryview_type(),
        "min" => ctx.new_function(builtin_min),
        "object" => ctx.object(),
        "oct" => ctx.new_function(builtin_oct),
        "ord" => ctx.new_function(builtin_ord),
        "next" => ctx.new_function(builtin_next),
        "pow" => ctx.new_function(builtin_pow),
        "print" => ctx.new_function(builtin_print),
        "property" => ctx.property_type(),
        "range" => ctx.range_type(),
        "repr" => ctx.new_function(builtin_repr),
        "reversed" => ctx.new_function(builtin_reversed),
        "round" => ctx.new_function(builtin_round),
        "set" => ctx.set_type(),
        "setattr" => ctx.new_function(builtin_setattr),
        "sorted" => ctx.new_function(builtin_sorted),
        "slice" => ctx.slice_type(),
        "staticmethod" => ctx.staticmethod_type(),
        "str" => ctx.str_type(),
        "sum" => ctx.new_function(builtin_sum),
        "super" => ctx.super_type(),
        "tuple" => ctx.tuple_type(),
        "type" => ctx.type_type(),
        "vars" => ctx.new_function(builtin_vars),
        "zip" => ctx.zip_type(),
        "exit" => ctx.new_function(builtin_exit),
        "quit" => ctx.new_function(builtin_exit),
        "__import__" => ctx.new_function(builtin_import),
        "__build_class__" => ctx.new_function(builtin_build_class_),

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
}

pub fn builtin_build_class_(
    function: PyFunctionRef,
    qualified_name: PyStringRef,
    bases: Args<PyClassRef>,
    mut kwargs: KwArgs,
    vm: &VirtualMachine,
) -> PyResult {
    let name = qualified_name.as_str().split('.').next_back().unwrap();
    let name_obj = vm.new_str(name.to_owned());

    let mut metaclass = if let Some(metaclass) = kwargs.pop_kwarg("metaclass") {
        PyClassRef::try_from_object(vm, metaclass)?
    } else {
        vm.get_type()
    };

    for base in bases.clone() {
        if objtype::issubclass(&base.class(), &metaclass) {
            metaclass = base.class();
        } else if !objtype::issubclass(&metaclass, &base.class()) {
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

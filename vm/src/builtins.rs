//! Builtin function definitions.
//!
//! Implements functions listed here: https://docs.python.org/3/library/builtins.html

use std::char;
use std::io::{self, Write};
use std::path::PathBuf;

use num_bigint::Sign;
use num_traits::{Signed, Zero};

use crate::compile;
use crate::import::import_module;
use crate::obj::objbool;
use crate::obj::objcode::PyCodeRef;
use crate::obj::objdict::PyDictRef;
use crate::obj::objint::{self, PyIntRef};
use crate::obj::objiter;
use crate::obj::objstr::{self, PyString, PyStringRef};
use crate::obj::objtype::{self, PyClassRef};

use crate::frame::Scope;
use crate::function::{single_or_tuple_any, Args, KwArgs, OptionalArg, PyFuncArgs};
use crate::pyobject::{
    IdProtocol, IntoPyObject, ItemProtocol, PyIterable, PyObjectRef, PyResult, PyValue,
    TryFromObject, TypeProtocol,
};
use crate::vm::VirtualMachine;

#[cfg(not(target_arch = "wasm32"))]
use crate::stdlib::io::io_open;

fn builtin_abs(x: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    match vm.get_method(x.clone(), "__abs__") {
        Ok(attrib) => vm.invoke(attrib, PyFuncArgs::new(vec![], vec![])),
        Err(..) => Err(vm.new_type_error("bad operand for abs".to_string())),
    }
}

fn builtin_all(iterable: PyIterable<bool>, vm: &VirtualMachine) -> PyResult<bool> {
    for item in iterable.iter(vm)? {
        if !item? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn builtin_any(iterable: PyIterable<bool>, vm: &VirtualMachine) -> PyResult<bool> {
    for item in iterable.iter(vm)? {
        if item? {
            return Ok(true);
        }
    }
    Ok(false)
}

// builtin_ascii

fn builtin_bin(x: PyIntRef, _vm: &VirtualMachine) -> String {
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
        None => Err(vm.new_value_error("chr() arg not in range(0x110000)".to_string())),
    }
}

fn builtin_compile(
    source: PyStringRef,
    filename: PyStringRef,
    mode: PyStringRef,
    vm: &VirtualMachine,
) -> PyResult<PyCodeRef> {
    // TODO: fix this newline bug:
    let source = format!("{}\n", &source.value);

    let mode = {
        let mode = &mode.value;
        if mode == "exec" {
            compile::Mode::Exec
        } else if mode == "eval" {
            compile::Mode::Eval
        } else if mode == "single" {
            compile::Mode::Single
        } else {
            return Err(
                vm.new_value_error("compile() mode must be 'exec', 'eval' or single'".to_string())
            );
        }
    };

    compile::compile(vm, &source, &mode, filename.value.to_string())
        .map_err(|err| vm.new_syntax_error(&err))
}

fn builtin_delattr(obj: PyObjectRef, attr: PyStringRef, vm: &VirtualMachine) -> PyResult<()> {
    vm.del_attr(&obj, attr.into_object())
}

fn builtin_dir(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    if args.args.is_empty() {
        Ok(vm.get_locals().into_object())
    } else {
        let obj = args.args.into_iter().next().unwrap();
        let seq = vm.call_method(&obj, "__dir__", vec![])?;
        let sorted = builtin_sorted(vm, PyFuncArgs::new(vec![seq], vec![]))?;
        Ok(sorted)
    }
}

fn builtin_divmod(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(a, None), (b, None)]);
    vm.call_or_reflection(
        a.clone(),
        b.clone(),
        "__divmod__",
        "__rdivmod__",
        |vm, a, b| Err(vm.new_unsupported_operand_error(a, b, "divmod")),
    )
}

/// Implements `eval`.
/// See also: https://docs.python.org/3/library/functions.html#eval
fn builtin_eval(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(source, None)],
        optional = [(globals, None), (locals, Some(vm.ctx.dict_type()))]
    );

    let scope = make_scope(vm, globals, locals)?;

    // Determine code object:
    let code_obj = if let Ok(code_obj) = PyCodeRef::try_from_object(vm, source.clone()) {
        code_obj
    } else if objtype::isinstance(source, &vm.ctx.str_type()) {
        let mode = compile::Mode::Eval;
        let source = objstr::get_value(source);
        // TODO: fix this newline bug:
        let source = format!("{}\n", source);
        compile::compile(vm, &source, &mode, "<string>".to_string())
            .map_err(|err| vm.new_syntax_error(&err))?
    } else {
        return Err(vm.new_type_error("code argument must be str or code object".to_string()));
    };

    // Run the source:
    vm.run_code_obj(code_obj, scope)
}

/// Implements `exec`
/// https://docs.python.org/3/library/functions.html#exec
fn builtin_exec(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(source, None)],
        optional = [(globals, None), (locals, None)]
    );

    let scope = make_scope(vm, globals, locals)?;

    // Determine code object:
    let code_obj = if objtype::isinstance(source, &vm.ctx.str_type()) {
        let mode = compile::Mode::Exec;
        let source = objstr::get_value(source);
        // TODO: fix this newline bug:
        let source = format!("{}\n", source);
        compile::compile(vm, &source, &mode, "<string>".to_string())
            .map_err(|err| vm.new_syntax_error(&err))?
    } else if let Ok(code_obj) = PyCodeRef::try_from_object(vm, source.clone()) {
        code_obj
    } else {
        return Err(vm.new_type_error("source argument must be str or code object".to_string()));
    };

    // Run the code:
    vm.run_code_obj(code_obj, scope)
}

fn make_scope(
    vm: &VirtualMachine,
    globals: Option<&PyObjectRef>,
    locals: Option<&PyObjectRef>,
) -> PyResult<Scope> {
    let dict_type = vm.ctx.dict_type();
    let globals = match globals {
        Some(arg) => {
            if arg.is(&vm.get_none()) {
                None
            } else if vm.isinstance(arg, &dict_type)? {
                Some(arg)
            } else {
                let arg_typ = arg.class();
                let actual_type = vm.to_pystr(&arg_typ)?;
                let expected_type_name = vm.to_pystr(&dict_type)?;
                return Err(vm.new_type_error(format!(
                    "globals must be a {}, not {}",
                    expected_type_name, actual_type
                )));
            }
        }
        None => None,
    };
    let current_scope = vm.current_scope();
    let locals = match locals {
        Some(dict) => dict.clone().downcast().ok(),
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
            let dict: PyDictRef = dict.clone().downcast().unwrap();
            if !dict.contains_key("__builtins__", vm) {
                let builtins_dict = vm.builtins.dict.as_ref().unwrap().as_object();
                dict.set_item("__builtins__", builtins_dict.clone(), vm)
                    .unwrap();
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
    let format_spec = format_spec.into_option().unwrap_or_else(|| {
        PyString {
            value: "".to_string(),
        }
        .into_ref(vm)
    });

    vm.call_method(&value, "__format__", vec![format_spec.into_object()])?
        .downcast()
        .map_err(|obj| {
            vm.new_type_error(format!(
                "__format__ must return a str, not {}",
                obj.class().name
            ))
        })
}

fn catch_attr_exception<T>(ex: PyObjectRef, default: T, vm: &VirtualMachine) -> PyResult<T> {
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

fn builtin_hash(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(obj, None)]);
    vm._hash(obj).and_then(|v| Ok(vm.new_int(v)))
}

// builtin_help

fn builtin_hex(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(number, Some(vm.ctx.int_type()))]);

    let n = objint::get_value(number);
    let s = if n.is_negative() {
        format!("-0x{:x}", n.abs())
    } else {
        format!("0x{:x}", n)
    };

    Ok(vm.new_str(s))
}

fn builtin_id(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(obj, None)]);

    Ok(vm.context().new_int(obj.get_id()))
}

// builtin_input

fn builtin_isinstance(obj: PyObjectRef, typ: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
    single_or_tuple_any(
        typ,
        |cls: PyClassRef| vm.isinstance(&obj, &cls),
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
        |cls: PyClassRef| vm.issubclass(&subclass, &cls),
        |o| {
            format!(
                "issubclass() arg 2 must be a class or tuple of classes, not {}",
                o.class()
            )
        },
        vm,
    )
}

fn builtin_iter(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(iter_target, None)]);
    objiter::get_iter(vm, iter_target)
}

fn builtin_len(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(obj, None)]);
    let len_method_name = "__len__";
    match vm.get_method(obj.clone(), len_method_name) {
        Ok(value) => vm.invoke(value, PyFuncArgs::default()),
        Err(..) => Err(vm.new_type_error(format!(
            "object of type '{}' has no method {:?}",
            obj.class().name,
            len_method_name
        ))),
    }
}

fn builtin_locals(vm: &VirtualMachine) -> PyDictRef {
    vm.get_locals()
}

fn builtin_max(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    let candidates = if args.args.len() > 1 {
        args.args.clone()
    } else if args.args.len() == 1 {
        vm.extract_elements(&args.args[0])?
    } else {
        // zero arguments means type error:
        return Err(vm.new_type_error("Expected 1 or more arguments".to_string()));
    };

    if candidates.is_empty() {
        let default = args.get_optional_kwarg("default");
        if default.is_none() {
            return Err(vm.new_value_error("max() arg is an empty sequence".to_string()));
        } else {
            return Ok(default.unwrap());
        }
    }

    let key_func = args.get_optional_kwarg("key");

    // Start with first assumption:
    let mut candidates_iter = candidates.into_iter();
    let mut x = candidates_iter.next().unwrap();
    // TODO: this key function looks pretty duplicate. Maybe we can create
    // a local function?
    let mut x_key = if let Some(f) = &key_func {
        vm.invoke(f.clone(), vec![x.clone()])?
    } else {
        x.clone()
    };

    for y in candidates_iter {
        let y_key = if let Some(f) = &key_func {
            vm.invoke(f.clone(), vec![y.clone()])?
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
    let candidates = if args.args.len() > 1 {
        args.args.clone()
    } else if args.args.len() == 1 {
        vm.extract_elements(&args.args[0])?
    } else {
        // zero arguments means type error:
        return Err(vm.new_type_error("Expected 1 or more arguments".to_string()));
    };

    if candidates.is_empty() {
        let default = args.get_optional_kwarg("default");
        if default.is_none() {
            return Err(vm.new_value_error("min() arg is an empty sequence".to_string()));
        } else {
            return Ok(default.unwrap());
        }
    }

    let key_func = args.get_optional_kwarg("key");

    let mut candidates_iter = candidates.into_iter();
    let mut x = candidates_iter.next().unwrap();
    // TODO: this key function looks pretty duplicate. Maybe we can create
    // a local function?
    let mut x_key = if let Some(f) = &key_func {
        vm.invoke(f.clone(), vec![x.clone()])?
    } else {
        x.clone()
    };

    for y in candidates_iter {
        let y_key = if let Some(f) = &key_func {
            vm.invoke(f.clone(), vec![y.clone()])?
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

fn builtin_next(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(iterator, None)],
        optional = [(default_value, None)]
    );

    match vm.call_method(iterator, "__next__", vec![]) {
        Ok(value) => Ok(value),
        Err(value) => {
            if objtype::isinstance(&value, &vm.ctx.exceptions.stop_iteration) {
                match default_value {
                    None => Err(value),
                    Some(value) => Ok(value.clone()),
                }
            } else {
                Err(value)
            }
        }
    }
}

fn builtin_oct(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(number, Some(vm.ctx.int_type()))]);

    let n = objint::get_value(number);
    let s = if n.is_negative() {
        format!("-0o{:o}", n.abs())
    } else {
        format!("0o{:o}", n)
    };

    Ok(vm.new_str(s))
}

fn builtin_ord(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(string, Some(vm.ctx.str_type()))]);
    let string = objstr::borrow_value(string);
    let string_len = string.chars().count();
    if string_len != 1 {
        return Err(vm.new_type_error(format!(
            "ord() expected a character, but string of length {} found",
            string_len
        )));
    }
    match string.chars().next() {
        Some(character) => Ok(vm.context().new_int(character as i32)),
        None => Err(vm.new_type_error(
            "ord() could not guess the integer representing this character".to_string(),
        )),
    }
}

fn builtin_pow(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(x, None), (y, None)],
        optional = [(mod_value, Some(vm.ctx.int_type()))]
    );

    match mod_value {
        None => vm.call_or_reflection(x.clone(), y.clone(), "__pow__", "__rpow__", |vm, x, y| {
            Err(vm.new_unsupported_operand_error(x, y, "pow"))
        }),
        Some(m) => {
            // Check if the 3rd argument is defined and perform modulus on the result
            if !(objtype::isinstance(x, &vm.ctx.int_type())
                && objtype::isinstance(y, &vm.ctx.int_type()))
            {
                return Err(vm.new_type_error(
                    "pow() 3rd argument not allowed unless all arguments are integers".to_string(),
                ));
            }
            let y = objint::get_value(y);
            if y.sign() == Sign::Minus {
                return Err(vm.new_value_error(
                    "pow() 2nd argument cannot be negative when 3rd argument specified".to_string(),
                ));
            }
            let m = objint::get_value(m);
            if m.is_zero() {
                return Err(vm.new_value_error("pow() 3rd argument cannot be 0".to_string()));
            }
            let x = objint::get_value(x);
            Ok(vm.new_int(x.modpow(&y, &m)))
        }
    }
}

#[derive(Debug, FromArgs)]
pub struct PrintOptions {
    #[pyarg(keyword_only, default = "None")]
    sep: Option<PyStringRef>,
    #[pyarg(keyword_only, default = "None")]
    end: Option<PyStringRef>,
    #[pyarg(keyword_only, default = "false")]
    flush: bool,
    #[pyarg(keyword_only, default = "None")]
    file: Option<PyObjectRef>,
}

trait Printer {
    fn write(&mut self, vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<()>;
    fn flush(&mut self, vm: &VirtualMachine) -> PyResult<()>;
}

impl Printer for &'_ PyObjectRef {
    fn write(&mut self, vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<()> {
        vm.call_method(self, "write", vec![obj])?;
        Ok(())
    }

    fn flush(&mut self, vm: &VirtualMachine) -> PyResult<()> {
        vm.call_method(self, "flush", vec![])?;
        Ok(())
    }
}

impl Printer for std::io::StdoutLock<'_> {
    fn write(&mut self, vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<()> {
        let s = &vm.to_str(&obj)?.value;
        write!(self, "{}", s).unwrap();
        Ok(())
    }

    fn flush(&mut self, _vm: &VirtualMachine) -> PyResult<()> {
        <Self as std::io::Write>::flush(self).unwrap();
        Ok(())
    }
}

pub fn builtin_print(objects: Args, options: PrintOptions, vm: &VirtualMachine) -> PyResult<()> {
    let stdout = io::stdout();

    let mut printer: Box<dyn Printer> = if let Some(file) = &options.file {
        Box::new(file)
    } else {
        Box::new(stdout.lock())
    };

    let sep = options
        .sep
        .as_ref()
        .map_or(" ", |sep| &sep.value)
        .into_pyobject(vm)
        .unwrap();

    let mut first = true;
    for object in objects {
        if first {
            first = false;
        } else {
            printer.write(vm, sep.clone())?;
        }

        printer.write(vm, object)?;
    }

    let end = options
        .end
        .as_ref()
        .map_or("\n", |end| &end.value)
        .into_pyobject(vm)
        .unwrap();
    printer.write(vm, end)?;

    if options.flush {
        printer.flush(vm)?;
    }

    Ok(())
}

fn builtin_repr(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyStringRef> {
    vm.to_repr(&obj)
}

fn builtin_reversed(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(obj, None)]);

    match vm.get_method(obj.clone(), "__reversed__") {
        Ok(value) => vm.invoke(value, PyFuncArgs::default()),
        // TODO: fallback to using __len__ and __getitem__, if object supports sequence protocol
        Err(..) => {
            Err(vm.new_type_error(format!("'{}' object is not reversible", obj.class().name)))
        }
    }
}
// builtin_reversed

fn builtin_round(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(number, Some(vm.ctx.object()))],
        optional = [(ndigits, None)]
    );
    if let Some(ndigits) = ndigits {
        let ndigits = vm.call_method(ndigits, "__int__", vec![])?;
        let rounded = vm.call_method(number, "__round__", vec![ndigits])?;
        Ok(rounded)
    } else {
        // without a parameter, the result type is coerced to int
        let rounded = &vm.call_method(number, "__round__", vec![])?;
        Ok(vm.ctx.new_int(objint::get_value(rounded).clone()))
    }
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
    arg_check!(
        vm,
        args,
        required = [(name, Some(vm.ctx.str_type()))],
        optional = [
            (_globals, Some(vm.ctx.dict_type())),
            (_locals, Some(vm.ctx.dict_type()))
        ]
    );
    let current_path = {
        match vm.current_frame() {
            Some(frame) => {
                let mut source_pathbuf = PathBuf::from(&frame.code.source_path);
                source_pathbuf.pop();
                source_pathbuf
            }
            None => PathBuf::new(),
        }
    };

    import_module(vm, current_path, &objstr::get_value(name))
}

// builtin_vars

pub fn make_module(vm: &VirtualMachine, module: PyObjectRef) {
    let ctx = &vm.ctx;

    #[cfg(target_arch = "wasm32")]
    let open = vm.ctx.none();
    #[cfg(not(target_arch = "wasm32"))]
    let open = vm.ctx.new_rustfunc(io_open);

    extend_module!(vm, module, {
        //set __name__ fixes: https://github.com/RustPython/RustPython/issues/146
        "__name__" => ctx.new_str(String::from("__main__")),

        "abs" => ctx.new_rustfunc(builtin_abs),
        "all" => ctx.new_rustfunc(builtin_all),
        "any" => ctx.new_rustfunc(builtin_any),
        "bin" => ctx.new_rustfunc(builtin_bin),
        "bool" => ctx.bool_type(),
        "bytearray" => ctx.bytearray_type(),
        "bytes" => ctx.bytes_type(),
        "callable" => ctx.new_rustfunc(builtin_callable),
        "chr" => ctx.new_rustfunc(builtin_chr),
        "classmethod" => ctx.classmethod_type(),
        "compile" => ctx.new_rustfunc(builtin_compile),
        "complex" => ctx.complex_type(),
        "delattr" => ctx.new_rustfunc(builtin_delattr),
        "dict" => ctx.dict_type(),
        "divmod" => ctx.new_rustfunc(builtin_divmod),
        "dir" => ctx.new_rustfunc(builtin_dir),
        "enumerate" => ctx.enumerate_type(),
        "eval" => ctx.new_rustfunc(builtin_eval),
        "exec" => ctx.new_rustfunc(builtin_exec),
        "float" => ctx.float_type(),
        "frozenset" => ctx.frozenset_type(),
        "filter" => ctx.filter_type(),
        "format" => ctx.new_rustfunc(builtin_format),
        "getattr" => ctx.new_rustfunc(builtin_getattr),
        "globals" => ctx.new_rustfunc(builtin_globals),
        "hasattr" => ctx.new_rustfunc(builtin_hasattr),
        "hash" => ctx.new_rustfunc(builtin_hash),
        "hex" => ctx.new_rustfunc(builtin_hex),
        "id" => ctx.new_rustfunc(builtin_id),
        "int" => ctx.int_type(),
        "isinstance" => ctx.new_rustfunc(builtin_isinstance),
        "issubclass" => ctx.new_rustfunc(builtin_issubclass),
        "iter" => ctx.new_rustfunc(builtin_iter),
        "len" => ctx.new_rustfunc(builtin_len),
        "list" => ctx.list_type(),
        "locals" => ctx.new_rustfunc(builtin_locals),
        "map" => ctx.map_type(),
        "max" => ctx.new_rustfunc(builtin_max),
        "memoryview" => ctx.memoryview_type(),
        "min" => ctx.new_rustfunc(builtin_min),
        "object" => ctx.object(),
        "oct" => ctx.new_rustfunc(builtin_oct),
        "open" => open,
        "ord" => ctx.new_rustfunc(builtin_ord),
        "next" => ctx.new_rustfunc(builtin_next),
        "pow" => ctx.new_rustfunc(builtin_pow),
        "print" => ctx.new_rustfunc(builtin_print),
        "property" => ctx.property_type(),
        "range" => ctx.range_type(),
        "repr" => ctx.new_rustfunc(builtin_repr),
        "reversed" => ctx.new_rustfunc(builtin_reversed),
        "round" => ctx.new_rustfunc(builtin_round),
        "set" => ctx.set_type(),
        "setattr" => ctx.new_rustfunc(builtin_setattr),
        "sorted" => ctx.new_rustfunc(builtin_sorted),
        "slice" => ctx.slice_type(),
        "staticmethod" => ctx.staticmethod_type(),
        "str" => ctx.str_type(),
        "sum" => ctx.new_rustfunc(builtin_sum),
        "super" => ctx.super_type(),
        "tuple" => ctx.tuple_type(),
        "type" => ctx.type_type(),
        "zip" => ctx.zip_type(),
        "__import__" => ctx.new_rustfunc(builtin_import),

        // Constants
        "NotImplemented" => ctx.not_implemented(),

        // Exceptions:
        "BaseException" => ctx.exceptions.base_exception_type.clone(),
        "Exception" => ctx.exceptions.exception_type.clone(),
        "ArithmeticError" => ctx.exceptions.arithmetic_error.clone(),
        "AssertionError" => ctx.exceptions.assertion_error.clone(),
        "AttributeError" => ctx.exceptions.attribute_error.clone(),
        "NameError" => ctx.exceptions.name_error.clone(),
        "OverflowError" => ctx.exceptions.overflow_error.clone(),
        "RuntimeError" => ctx.exceptions.runtime_error.clone(),
        "ReferenceError" => ctx.exceptions.reference_error.clone(),
        "SyntaxError" =>  ctx.exceptions.syntax_error.clone(),
        "NotImplementedError" => ctx.exceptions.not_implemented_error.clone(),
        "TypeError" => ctx.exceptions.type_error.clone(),
        "ValueError" => ctx.exceptions.value_error.clone(),
        "IndexError" => ctx.exceptions.index_error.clone(),
        "ImportError" => ctx.exceptions.import_error.clone(),
        "FileNotFoundError" => ctx.exceptions.file_not_found_error.clone(),
        "StopIteration" => ctx.exceptions.stop_iteration.clone(),
        "ZeroDivisionError" => ctx.exceptions.zero_division_error.clone(),
        "KeyError" => ctx.exceptions.key_error.clone(),
        "OSError" => ctx.exceptions.os_error.clone(),

        // Warnings
        "Warning" => ctx.exceptions.warning.clone(),
        "BytesWarning" => ctx.exceptions.bytes_warning.clone(),
        "UnicodeWarning" => ctx.exceptions.unicode_warning.clone(),
        "DeprecationWarning" => ctx.exceptions.deprecation_warning.clone(),
        "PendingDeprecationWarning" => ctx.exceptions.pending_deprecation_warning.clone(),
        "FutureWarning" => ctx.exceptions.future_warning.clone(),
        "ImportWarning" => ctx.exceptions.import_warning.clone(),
        "SyntaxWarning" => ctx.exceptions.syntax_warning.clone(),
        "ResourceWarning" => ctx.exceptions.resource_warning.clone(),
        "RuntimeWarning" => ctx.exceptions.runtime_warning.clone(),
        "UserWarning" => ctx.exceptions.user_warning.clone(),
    });
}

pub fn builtin_build_class_(
    function: PyObjectRef,
    qualified_name: PyStringRef,
    bases: Args<PyClassRef>,
    mut kwargs: KwArgs,
    vm: &VirtualMachine,
) -> PyResult {
    let name = qualified_name.value.split('.').next_back().unwrap();
    let name_obj = vm.new_str(name.to_string());

    let mut metaclass = if let Some(metaclass) = kwargs.pop_kwarg("metaclass") {
        PyClassRef::try_from_object(vm, metaclass)?
    } else {
        vm.get_type()
    };

    for base in bases.clone() {
        if objtype::issubclass(&base.class(), &metaclass) {
            metaclass = base.class();
        } else if !objtype::issubclass(&metaclass, &base.class()) {
            return Err(vm.new_type_error("metaclass conflict: the metaclass of a derived class must be a (non-strict) subclass of the metaclasses of all its bases".to_string()));
        }
    }

    let bases = bases.into_tuple(vm);

    // Prepare uses full __getattribute__ resolution chain.
    let prepare = vm.get_attribute(metaclass.clone().into_object(), "__prepare__")?;
    let namespace = vm.invoke(prepare, vec![name_obj.clone(), bases.clone()])?;

    let namespace: PyDictRef = TryFromObject::try_from_object(vm, namespace)?;

    let cells = vm.ctx.new_dict();

    vm.invoke_with_locals(function, cells.clone(), namespace.clone())?;

    namespace.set_item("__name__", name_obj.clone(), vm)?;
    namespace.set_item("__qualname__", qualified_name.into_object(), vm)?;

    let class = vm.call_method(
        metaclass.as_object(),
        "__call__",
        vec![name_obj, bases, namespace.into_object()],
    )?;
    cells.set_item("__class__", class.clone(), vm)?;
    Ok(class)
}

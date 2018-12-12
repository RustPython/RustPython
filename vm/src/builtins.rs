//! Builtin function definitions.
//!
//! Implements functions listed here: https://docs.python.org/3/library/builtins.html

// use std::ops::Deref;
use std::char;
use std::io::{self, Write};

use super::compile;
use super::obj::objbool;
use super::obj::objdict;
use super::obj::objint;
use super::obj::objiter;
use super::obj::objstr;
use super::obj::objtype;
use super::pyobject::{
    AttributeProtocol, DictProtocol, IdProtocol, PyContext, PyFuncArgs, PyObject, PyObjectKind,
    PyObjectRef, PyResult, Scope, TypeProtocol,
};
use super::vm::VirtualMachine;
use num_bigint::ToBigInt;
use num_traits::{Signed, ToPrimitive, Zero};

fn get_locals(vm: &mut VirtualMachine) -> PyObjectRef {
    let d = vm.new_dict();
    // TODO: implement dict_iter_items?
    let locals = vm.get_locals();
    let key_value_pairs = objdict::get_key_value_pairs(vm, &locals);
    for (key, value) in key_value_pairs {
        objdict::set_item(&d, &key, &value);
    }
    d
}

fn dir_locals(vm: &mut VirtualMachine) -> PyObjectRef {
    get_locals(vm)
}

fn dir_object(vm: &mut VirtualMachine, obj: &PyObjectRef) -> PyObjectRef {
    // Gather all members here:
    let attributes = objtype::get_attributes(obj);
    let mut members: Vec<String> = attributes.into_iter().map(|(n, _o)| n).collect();

    // Sort members:
    members.sort();

    let members_pystr = members.into_iter().map(|m| vm.ctx.new_str(m)).collect();
    vm.ctx.new_list(members_pystr)
}

fn builtin_abs(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(x, None)]);
    match vm.get_method(x.clone(), "__abs__") {
        Ok(attrib) => vm.invoke(attrib, PyFuncArgs::new(vec![], vec![])),
        Err(..) => Err(vm.new_type_error("bad operand for abs".to_string())),
    }
}

fn builtin_all(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(iterable, None)]);
    let items = vm.extract_elements(iterable)?;
    for item in items {
        let result = objbool::boolval(vm, item)?;
        if !result {
            return Ok(vm.new_bool(false));
        }
    }
    Ok(vm.new_bool(true))
}

fn builtin_any(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(iterable, None)]);
    let items = vm.extract_elements(iterable)?;
    for item in items {
        let result = objbool::boolval(vm, item)?;
        if result {
            return Ok(vm.new_bool(true));
        }
    }
    Ok(vm.new_bool(false))
}

// builtin_ascii

fn builtin_bin(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(number, Some(vm.ctx.int_type()))]);

    let n = objint::get_value(number);
    let s = if n.is_negative() {
        format!("-0b{:b}", n.abs())
    } else {
        format!("0b{:b}", n)
    };

    Ok(vm.new_str(s))
}

// builtin_breakpoint

fn builtin_callable(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(obj, None)]);
    // TODO: is this a sufficiently thorough check?
    let is_callable = obj.has_attr("__call__");
    Ok(vm.new_bool(is_callable))
}

fn builtin_chr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(i, Some(vm.ctx.int_type()))]);

    let code_point = objint::get_value(i).to_u32().unwrap();

    let txt = match char::from_u32(code_point) {
        Some(value) => value.to_string(),
        None => '_'.to_string(),
    };

    Ok(vm.new_str(txt))
}

fn builtin_compile(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [
            (source, None),
            (filename, Some(vm.ctx.str_type())),
            (mode, Some(vm.ctx.str_type()))
        ]
    );
    let source = objstr::get_value(source);
    // TODO: fix this newline bug:
    let source = format!("{}\n", source);

    let mode = {
        let mode = objstr::get_value(mode);
        if mode == String::from("exec") {
            compile::Mode::Exec
        } else if mode == "eval".to_string() {
            compile::Mode::Eval
        } else if mode == "single".to_string() {
            compile::Mode::Single
        } else {
            return Err(
                vm.new_value_error("compile() mode must be 'exec', 'eval' or single'".to_string())
            );
        }
    };

    let filename = objstr::get_value(filename);

    compile::compile(vm, &source, mode, Some(filename))
}

fn builtin_delattr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(obj, None), (attr, Some(vm.ctx.str_type()))]
    );
    vm.del_attr(obj, attr.clone())
}

fn builtin_dir(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    if args.args.is_empty() {
        Ok(dir_locals(vm))
    } else {
        let obj = args.args.into_iter().next().unwrap();
        Ok(dir_object(vm, &obj))
    }
}

fn builtin_divmod(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(x, None), (y, None)]);
    match vm.get_method(x.clone(), "__divmod__") {
        Ok(attrib) => vm.invoke(attrib, PyFuncArgs::new(vec![y.clone()], vec![])),
        Err(..) => Err(vm.new_type_error("unsupported operand type(s) for divmod".to_string())),
    }
}

fn builtin_enumerate(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(iterable, None)],
        optional = [(start, None)]
    );
    let items = vm.extract_elements(iterable)?;
    let start = if let Some(start) = start {
        objint::get_value(start)
    } else {
        Zero::zero()
    };
    let mut new_items = vec![];
    for (i, item) in items.into_iter().enumerate() {
        let element = vm
            .ctx
            .new_tuple(vec![vm.ctx.new_int(i.to_bigint().unwrap() + &start), item]);
        new_items.push(element);
    }
    Ok(vm.ctx.new_list(new_items))
}

fn builtin_eval(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(source, None)],
        optional = [
            (_globals, Some(vm.ctx.dict_type())),
            (locals, Some(vm.ctx.dict_type()))
        ]
    );

    // Determine code object:
    let code_obj = if objtype::isinstance(source, &vm.ctx.code_type()) {
        source.clone()
    } else if objtype::isinstance(source, &vm.ctx.str_type()) {
        let mode = compile::Mode::Eval;
        let source = objstr::get_value(source);
        // TODO: fix this newline bug:
        let source = format!("{}\n", source);
        compile::compile(vm, &source, mode, None)?
    } else {
        return Err(vm.new_type_error("code argument must be str or code object".to_string()));
    };

    let locals = if let Some(locals) = locals {
        locals.clone()
    } else {
        vm.new_dict()
    };

    // TODO: handle optional globals
    // Construct new scope:
    let scope_inner = Scope {
        locals: locals,
        parent: None,
    };
    let scope = PyObject {
        kind: PyObjectKind::Scope { scope: scope_inner },
        typ: None,
    }
    .into_ref();

    // Run the source:
    vm.run_code_obj(code_obj.clone(), scope)
}

fn builtin_exec(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(source, None)],
        optional = [
            (_globals, Some(vm.ctx.dict_type())),
            (locals, Some(vm.ctx.dict_type()))
        ]
    );

    // Determine code object:
    let code_obj = if objtype::isinstance(source, &vm.ctx.str_type()) {
        let mode = compile::Mode::Exec;
        let source = objstr::get_value(source);
        // TODO: fix this newline bug:
        let source = format!("{}\n", source);
        compile::compile(vm, &source, mode, None)?
    } else if objtype::isinstance(source, &vm.ctx.code_type()) {
        source.clone()
    } else {
        return Err(vm.new_type_error("source argument must be str or code object".to_string()));
    };

    // handle optional global and locals
    let locals = if let Some(locals) = locals {
        locals.clone()
    } else {
        vm.new_dict()
    };

    // TODO: use globals

    // Construct new scope:
    let scope_inner = Scope {
        locals: locals,
        parent: None,
    };
    let scope = PyObject {
        kind: PyObjectKind::Scope { scope: scope_inner },
        typ: None,
    }
    .into_ref();

    // Run the code:
    vm.run_code_obj(code_obj, scope)
}

fn builtin_filter(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(function, None), (iterable, None)]);

    // TODO: process one element at a time from iterators.
    let iterable = vm.extract_elements(iterable)?;

    let mut new_items = vec![];
    for element in iterable {
        // apply function:
        let args = PyFuncArgs {
            args: vec![element.clone()],
            kwargs: vec![],
        };
        let result = vm.invoke(function.clone(), args)?;
        let result = objbool::boolval(vm, result)?;
        if result {
            new_items.push(element);
        }
    }

    Ok(vm.ctx.new_list(new_items))
}

// builtin_format

fn builtin_getattr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(obj, None), (attr, Some(vm.ctx.str_type()))]
    );
    vm.get_attribute(obj.clone(), attr.clone())
}

// builtin_globals

fn builtin_hasattr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(obj, None), (attr, Some(vm.ctx.str_type()))]
    );
    let has_attr = match vm.get_attribute(obj.clone(), attr.clone()) {
        Ok(..) => true,
        Err(..) => false,
    };
    Ok(vm.context().new_bool(has_attr))
}

fn builtin_hash(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(obj, None)]);

    vm.call_method(obj, "__hash__", vec![])
}

// builtin_help

fn builtin_hex(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(number, Some(vm.ctx.int_type()))]);

    let n = objint::get_value(number);
    let s = if n.is_negative() {
        format!("-0x{:x}", n.abs())
    } else {
        format!("0x{:x}", n)
    };

    Ok(vm.new_str(s))
}

fn builtin_id(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(obj, None)]);

    Ok(vm.context().new_int(obj.get_id().to_bigint().unwrap()))
}

// builtin_input

fn builtin_isinstance(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(obj, None), (typ, None)]);

    let isinstance = objtype::isinstance(obj, typ);
    Ok(vm.context().new_bool(isinstance))
}

fn builtin_issubclass(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    if args.args.len() != 2 {
        panic!("issubclass expects exactly two parameters");
    }

    let cls1 = &args.args[0];
    let cls2 = &args.args[1];

    Ok(vm.context().new_bool(objtype::issubclass(cls1, cls2)))
}

fn builtin_iter(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(iter_target, None)]);
    objiter::get_iter(vm, iter_target)
}

fn builtin_len(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(obj, None)]);
    let len_method_name = "__len__".to_string();
    match vm.get_method(obj.clone(), &len_method_name) {
        Ok(value) => vm.invoke(value, PyFuncArgs::default()),
        Err(..) => Err(vm.context().new_str(
            format!(
                "TypeError: object of this {:?} type has no method {:?}",
                obj, len_method_name
            )
            .to_string(),
        )),
    }
}

fn builtin_locals(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args);
    Ok(vm.get_locals())
}

fn builtin_map(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(function, None), (iter_target, None)]);
    let iterator = objiter::get_iter(vm, iter_target)?;
    let mut elements = vec![];
    loop {
        match vm.call_method(&iterator, "__next__", vec![]) {
            Ok(v) => {
                // Now apply function:
                let mapped_value = vm.invoke(
                    function.clone(),
                    PyFuncArgs {
                        args: vec![v],
                        kwargs: vec![],
                    },
                )?;
                elements.push(mapped_value);
            }
            Err(_) => break,
        }
    }

    trace!("Mapped elements: {:?}", elements);

    // TODO: when iterators are implemented, we can improve this function.
    Ok(vm.ctx.new_list(elements))
}

fn builtin_max(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    let candidates = if args.args.len() > 1 {
        args.args.clone()
    } else if args.args.len() == 1 {
        vm.extract_elements(&args.args[0])?
    } else {
        // zero arguments means type error:
        return Err(vm.new_type_error("Expected 1 or more arguments".to_string()));
    };

    if candidates.len() == 0 {
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
        let args = PyFuncArgs::new(vec![x.clone()], vec![]);
        vm.invoke(f.clone(), args)?
    } else {
        x.clone()
    };

    for y in candidates_iter {
        let y_key = if let Some(f) = &key_func {
            let args = PyFuncArgs::new(vec![y.clone()], vec![]);
            vm.invoke(f.clone(), args)?
        } else {
            y.clone()
        };
        let order = vm.call_method(&x_key, "__gt__", vec![y_key.clone()])?;

        if !objbool::get_value(&order) {
            x = y.clone();
            x_key = y_key;
        }
    }

    Ok(x)
}

// builtin_memoryview

fn builtin_min(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    let candidates = if args.args.len() > 1 {
        args.args.clone()
    } else if args.args.len() == 1 {
        vm.extract_elements(&args.args[0])?
    } else {
        // zero arguments means type error:
        return Err(vm.new_type_error("Expected 1 or more arguments".to_string()));
    };

    if candidates.len() == 0 {
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
        let args = PyFuncArgs::new(vec![x.clone()], vec![]);
        vm.invoke(f.clone(), args)?
    } else {
        x.clone()
    };

    for y in candidates_iter {
        let y_key = if let Some(f) = &key_func {
            let args = PyFuncArgs::new(vec![y.clone()], vec![]);
            vm.invoke(f.clone(), args)?
        } else {
            y.clone()
        };
        let order = vm.call_method(&x_key, "__gt__", vec![y_key.clone()])?;

        if objbool::get_value(&order) {
            x = y.clone();
            x_key = y_key;
        }
    }

    Ok(x)
}

fn builtin_next(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
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

fn builtin_oct(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(number, Some(vm.ctx.int_type()))]);

    let n = objint::get_value(number);
    let s = if n.is_negative() {
        format!("-0o{:o}", n.abs())
    } else {
        format!("0o{:o}", n)
    };

    Ok(vm.new_str(s))
}

// builtin_open

fn builtin_ord(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(string, Some(vm.ctx.str_type()))]);
    let string = objstr::get_value(string);
    let string_len = string.chars().count();
    if string_len > 1 {
        return Err(vm.new_type_error(
            format!(
                "ord() expected a character, but string of length {} found",
                string_len
            )
            .to_string(),
        ));
    }
    match string.chars().next() {
        Some(character) => Ok(vm
            .context()
            .new_int((character as i32).to_bigint().unwrap())),
        None => Err(vm.new_type_error(
            "ord() could not guess the integer representing this character".to_string(),
        )),
    }
}

fn builtin_pow(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(x, None), (y, None)],
        optional = [(mod_value, Some(vm.ctx.int_type()))]
    );
    let pow_method_name = "__pow__".to_string();
    let result = match vm.get_method(x.clone(), &pow_method_name) {
        Ok(attrib) => vm.invoke(attrib, PyFuncArgs::new(vec![y.clone()], vec![])),
        Err(..) => Err(vm.new_type_error("unsupported operand type(s) for pow".to_string())),
    };
    //Check if the 3rd argument is defined and perform modulus on the result
    //this should be optimized in the future to perform a "power-mod" algorithm in
    //order to improve performance
    match mod_value {
        Some(mod_value) => {
            let mod_method_name = "__mod__".to_string();
            match vm.get_method(
                result.expect("result not defined").clone(),
                &mod_method_name,
            ) {
                Ok(value) => vm.invoke(value, PyFuncArgs::new(vec![mod_value.clone()], vec![])),
                Err(..) => {
                    Err(vm.new_type_error("unsupported operand type(s) for mod".to_string()))
                }
            }
        }
        None => result,
    }
}

pub fn builtin_print(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    trace!("print called with {:?}", args);
    let mut first = true;
    for a in args.args {
        if first {
            first = false;
        } else {
            print!(" ");
        }
        let v = vm.to_str(&a)?;
        let s = objstr::get_value(&v);
        print!("{}", s);
    }
    println!();
    io::stdout().flush().unwrap();
    Ok(vm.get_none())
}

fn builtin_range(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(range, Some(vm.ctx.int_type()))]);
    let value = objint::get_value(range);
    let range_elements: Vec<PyObjectRef> = (0..value.to_i32().unwrap())
        .map(|num| vm.context().new_int(num.to_bigint().unwrap()))
        .collect();
    Ok(vm.context().new_list(range_elements))
}

fn builtin_repr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(obj, None)]);
    vm.to_repr(obj)
}
// builtin_reversed
// builtin_round

fn builtin_setattr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(obj, None), (attr, Some(vm.ctx.str_type())), (value, None)]
    );
    let name = objstr::get_value(attr);
    obj.clone().set_attr(&name, value.clone());
    Ok(vm.get_none())
}

// builtin_slice
// builtin_sorted

fn builtin_sum(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(iterable, None)]);
    let items = vm.extract_elements(iterable)?;

    // Start with zero and add at will:
    let mut sum = vm.ctx.new_int(Zero::zero());
    for item in items {
        sum = vm._add(sum, item)?;
    }
    Ok(sum)
}

// builtin_vars

fn builtin_zip(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    no_kwargs!(vm, args);

    // TODO: process one element at a time from iterators.
    let mut iterables = vec![];
    for iterable in args.args.iter() {
        let iterable = vm.extract_elements(iterable)?;
        iterables.push(iterable);
    }

    let minsize: usize = iterables.iter().map(|i| i.len()).min().unwrap_or(0);

    let mut new_items = vec![];
    for i in 0..minsize {
        let items = iterables
            .iter()
            .map(|iterable| iterable[i].clone())
            .collect();
        let element = vm.ctx.new_tuple(items);
        new_items.push(element);
    }

    Ok(vm.ctx.new_list(new_items))
}

// builtin___import__

pub fn make_module(ctx: &PyContext) -> PyObjectRef {
    let mod_name = "__builtins__".to_string();
    let py_mod = ctx.new_module(&mod_name, ctx.new_scope(None));
    //set __name__ fixes: https://github.com/RustPython/RustPython/issues/146
    py_mod.set_item("__name__", ctx.new_str(String::from("__main__")));

    py_mod.set_item("abs", ctx.new_rustfunc(builtin_abs));
    py_mod.set_item("all", ctx.new_rustfunc(builtin_all));
    py_mod.set_item("any", ctx.new_rustfunc(builtin_any));
    py_mod.set_item("bin", ctx.new_rustfunc(builtin_bin));
    py_mod.set_item("bool", ctx.bool_type());
    py_mod.set_item("bytearray", ctx.bytearray_type());
    py_mod.set_item("bytes", ctx.bytes_type());
    py_mod.set_item("callable", ctx.new_rustfunc(builtin_callable));
    py_mod.set_item("chr", ctx.new_rustfunc(builtin_chr));
    py_mod.set_item("classmethod", ctx.classmethod_type());
    py_mod.set_item("compile", ctx.new_rustfunc(builtin_compile));
    py_mod.set_item("complex", ctx.complex_type());
    py_mod.set_item("delattr", ctx.new_rustfunc(builtin_delattr));
    py_mod.set_item("dict", ctx.dict_type());
    py_mod.set_item("divmod", ctx.new_rustfunc(builtin_divmod));
    py_mod.set_item("dir", ctx.new_rustfunc(builtin_dir));
    py_mod.set_item("enumerate", ctx.new_rustfunc(builtin_enumerate));
    py_mod.set_item("eval", ctx.new_rustfunc(builtin_eval));
    py_mod.set_item("exec", ctx.new_rustfunc(builtin_exec));
    py_mod.set_item("float", ctx.float_type());
    py_mod.set_item("frozenset", ctx.frozenset_type());
    py_mod.set_item("filter", ctx.new_rustfunc(builtin_filter));
    py_mod.set_item("getattr", ctx.new_rustfunc(builtin_getattr));
    py_mod.set_item("hasattr", ctx.new_rustfunc(builtin_hasattr));
    py_mod.set_item("hash", ctx.new_rustfunc(builtin_hash));
    py_mod.set_item("hex", ctx.new_rustfunc(builtin_hex));
    py_mod.set_item("id", ctx.new_rustfunc(builtin_id));
    py_mod.set_item("int", ctx.int_type());
    py_mod.set_item("isinstance", ctx.new_rustfunc(builtin_isinstance));
    py_mod.set_item("issubclass", ctx.new_rustfunc(builtin_issubclass));
    py_mod.set_item("iter", ctx.new_rustfunc(builtin_iter));
    py_mod.set_item("len", ctx.new_rustfunc(builtin_len));
    py_mod.set_item("list", ctx.list_type());
    py_mod.set_item("locals", ctx.new_rustfunc(builtin_locals));
    py_mod.set_item("map", ctx.new_rustfunc(builtin_map));
    py_mod.set_item("max", ctx.new_rustfunc(builtin_max));
    py_mod.set_item("min", ctx.new_rustfunc(builtin_min));
    py_mod.set_item("object", ctx.object());
    py_mod.set_item("oct", ctx.new_rustfunc(builtin_oct));
    py_mod.set_item("ord", ctx.new_rustfunc(builtin_ord));
    py_mod.set_item("next", ctx.new_rustfunc(builtin_next));
    py_mod.set_item("pow", ctx.new_rustfunc(builtin_pow));
    py_mod.set_item("print", ctx.new_rustfunc(builtin_print));
    py_mod.set_item("property", ctx.property_type());
    py_mod.set_item("range", ctx.new_rustfunc(builtin_range));
    py_mod.set_item("repr", ctx.new_rustfunc(builtin_repr));
    py_mod.set_item("set", ctx.set_type());
    py_mod.set_item("setattr", ctx.new_rustfunc(builtin_setattr));
    py_mod.set_item("staticmethod", ctx.staticmethod_type());
    py_mod.set_item("str", ctx.str_type());
    py_mod.set_item("sum", ctx.new_rustfunc(builtin_sum));
    py_mod.set_item("super", ctx.super_type());
    py_mod.set_item("tuple", ctx.tuple_type());
    py_mod.set_item("type", ctx.type_type());
    py_mod.set_item("zip", ctx.new_rustfunc(builtin_zip));

    // Exceptions:
    py_mod.set_item("BaseException", ctx.exceptions.base_exception_type.clone());
    py_mod.set_item("Exception", ctx.exceptions.exception_type.clone());
    py_mod.set_item("AssertionError", ctx.exceptions.assertion_error.clone());
    py_mod.set_item("AttributeError", ctx.exceptions.attribute_error.clone());
    py_mod.set_item("NameError", ctx.exceptions.name_error.clone());
    py_mod.set_item("RuntimeError", ctx.exceptions.runtime_error.clone());
    py_mod.set_item(
        "NotImplementedError",
        ctx.exceptions.not_implemented_error.clone(),
    );
    py_mod.set_item("TypeError", ctx.exceptions.type_error.clone());
    py_mod.set_item("ValueError", ctx.exceptions.value_error.clone());

    py_mod
}

pub fn builtin_build_class_(vm: &mut VirtualMachine, mut args: PyFuncArgs) -> PyResult {
    let function = args.shift();
    let name_arg = args.shift();
    let bases = args.args.clone();
    let mut metaclass = args.get_kwarg("metaclass", vm.get_type());

    for base in bases.clone() {
        if objtype::issubclass(&base.typ(), &metaclass) {
            metaclass = base.typ();
        } else if !objtype::issubclass(&metaclass, &base.typ()) {
            return Err(vm.new_type_error("metaclass conflict: the metaclass of a derived class must be a (non-strict) subclass of the metaclasses of all its bases".to_string()));
        }
    }

    let bases = vm.context().new_tuple(bases);

    // Prepare uses full __getattribute__ resolution chain.
    let prepare_name = vm.new_str("__prepare__".to_string());
    let prepare = vm.get_attribute(metaclass.clone(), prepare_name)?;
    let namespace = vm.invoke(
        prepare,
        PyFuncArgs {
            args: vec![name_arg.clone(), bases.clone()],
            kwargs: vec![],
        },
    )?;

    vm.invoke(
        function,
        PyFuncArgs {
            args: vec![namespace.clone()],
            kwargs: vec![],
        },
    )?;

    vm.call_method(&metaclass, "__call__", vec![name_arg, bases, namespace])
}

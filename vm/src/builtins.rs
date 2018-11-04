//! Builtin function definitions.
//!
//! Implements functions listed here: https://docs.python.org/3/library/builtins.html

// use std::ops::Deref;
use std::char;
use std::collections::HashMap;
use std::io::{self, Write};

use super::compile;
use super::obj::objbool;
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
    match locals.borrow().kind {
        PyObjectKind::Dict { ref elements } => {
            for l in elements {
                d.set_item(l.0, l.1.clone());
            }
        }
        _ => {}
    };
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
// builtin_callable

fn builtin_chr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(i, Some(vm.ctx.int_type()))]);

    let code_point = objint::get_value(i).to_u32().unwrap();

    let txt = match char::from_u32(code_point) {
        Some(value) => value.to_string(),
        None => '_'.to_string(),
    };

    Ok(vm.new_str(txt))
}

// builtin_classmethod

fn builtin_compile(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(source, None)]);
    // TODO:
    let mode = compile::Mode::Eval;
    let source = source.borrow().str();

    compile::compile(vm, &source, mode, None)
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

// builtin_enumerate

fn builtin_eval(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [
            (source, None), // TODO: Use more specific type
            (_globals, Some(vm.ctx.dict_type())),
            (locals, Some(vm.ctx.dict_type()))
        ]
    );
    // TODO: handle optional global and locals

    let code_obj = source; // if source.borrow().kind

    // Construct new scope:
    let scope_inner = Scope {
        locals: locals.clone(),
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
        required = [
            (source, None),
            (_globals, Some(vm.ctx.dict_type())),
            (locals, Some(vm.ctx.dict_type()))
        ]
    );
    // TODO: handle optional global and locals

    // Determine code object:
    let code_obj = if objtype::isinstance(source, &vm.ctx.str_type()) {
        let mode = compile::Mode::Exec;
        let source = objstr::get_value(source);
        compile::compile(vm, &source, mode, None)?
    } else {
        source.clone()
    };

    // Construct new scope:
    let scope_inner = Scope {
        locals: locals.clone(),
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

// builtin_filter
// builtin_format
// builtin_frozenset

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
    arg_check!(vm, args, required = [(x, None), (y, None)]);

    let order = vm.call_method(x, "__gt__", vec![y.clone()])?;

    if objbool::get_value(&order) {
        Ok(x.clone())
    } else {
        Ok(y.clone())
    }
}

// builtin_memoryview

fn builtin_min(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(x, Some(vm.ctx.int_type())), (y, Some(vm.ctx.int_type()))]
    );

    let order = vm.call_method(x, "__gt__", vec![y.clone()])?;

    if objbool::get_value(&order) {
        Ok(y.clone())
    } else {
        Ok(x.clone())
    }
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

// builtin_property

fn builtin_range(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(range, Some(vm.ctx.int_type()))]);
    match range.borrow().kind {
        PyObjectKind::Integer { ref value } => {
            let range_elements: Vec<PyObjectRef> = (0..value.to_i32().unwrap())
                .map(|num| vm.context().new_int(num.to_bigint().unwrap()))
                .collect();
            Ok(vm.context().new_list(range_elements))
        }
        _ => panic!("argument checking failure: first argument to range must be an integer"),
    }
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
// builtin_staticmethod

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

// builtin_super
// builtin_vars
// builtin_zip
// builtin___import__

pub fn make_module(ctx: &PyContext) -> PyObjectRef {
    // scope[String::from("print")] = print;
    let mut dict = HashMap::new();
    //set __name__ fixes: https://github.com/RustPython/RustPython/issues/146
    dict.insert(
        String::from("__name__"),
        ctx.new_str(String::from("__main__")),
    );
    dict.insert(String::from("abs"), ctx.new_rustfunc(builtin_abs));
    dict.insert(String::from("all"), ctx.new_rustfunc(builtin_all));
    dict.insert(String::from("any"), ctx.new_rustfunc(builtin_any));
    dict.insert(String::from("bin"), ctx.new_rustfunc(builtin_bin));
    dict.insert(String::from("bool"), ctx.bool_type());
    dict.insert(String::from("bytearray"), ctx.bytearray_type());
    dict.insert(String::from("bytes"), ctx.bytes_type());
    dict.insert(String::from("chr"), ctx.new_rustfunc(builtin_chr));
    dict.insert(String::from("compile"), ctx.new_rustfunc(builtin_compile));
    dict.insert(String::from("complex"), ctx.complex_type());
    dict.insert(String::from("delattr"), ctx.new_rustfunc(builtin_delattr));
    dict.insert(String::from("dict"), ctx.dict_type());
    dict.insert(String::from("divmod"), ctx.new_rustfunc(builtin_divmod));
    dict.insert(String::from("dir"), ctx.new_rustfunc(builtin_dir));
    dict.insert(String::from("eval"), ctx.new_rustfunc(builtin_eval));
    dict.insert(String::from("exec"), ctx.new_rustfunc(builtin_exec));
    dict.insert(String::from("float"), ctx.float_type());
    dict.insert(String::from("getattr"), ctx.new_rustfunc(builtin_getattr));
    dict.insert(String::from("hasattr"), ctx.new_rustfunc(builtin_hasattr));
    dict.insert(String::from("hash"), ctx.new_rustfunc(builtin_hash));
    dict.insert(String::from("hex"), ctx.new_rustfunc(builtin_hex));
    dict.insert(String::from("id"), ctx.new_rustfunc(builtin_id));
    dict.insert(String::from("int"), ctx.int_type());
    dict.insert(
        String::from("isinstance"),
        ctx.new_rustfunc(builtin_isinstance),
    );
    dict.insert(
        String::from("issubclass"),
        ctx.new_rustfunc(builtin_issubclass),
    );
    dict.insert(String::from("iter"), ctx.new_rustfunc(builtin_iter));
    dict.insert(String::from("len"), ctx.new_rustfunc(builtin_len));
    dict.insert(String::from("list"), ctx.list_type());
    dict.insert(String::from("locals"), ctx.new_rustfunc(builtin_locals));
    dict.insert(String::from("map"), ctx.new_rustfunc(builtin_map));
    dict.insert(String::from("max"), ctx.new_rustfunc(builtin_max));
    dict.insert(String::from("min"), ctx.new_rustfunc(builtin_min));
    dict.insert(String::from("object"), ctx.object());
    dict.insert(String::from("oct"), ctx.new_rustfunc(builtin_oct));
    dict.insert(String::from("ord"), ctx.new_rustfunc(builtin_ord));
    dict.insert(String::from("next"), ctx.new_rustfunc(builtin_next));
    dict.insert(String::from("pow"), ctx.new_rustfunc(builtin_pow));
    dict.insert(String::from("print"), ctx.new_rustfunc(builtin_print));
    dict.insert(String::from("range"), ctx.new_rustfunc(builtin_range));
    dict.insert(String::from("repr"), ctx.new_rustfunc(builtin_repr));
    dict.insert(String::from("set"), ctx.set_type());
    dict.insert(String::from("setattr"), ctx.new_rustfunc(builtin_setattr));
    dict.insert(String::from("str"), ctx.str_type());
    dict.insert(String::from("sum"), ctx.new_rustfunc(builtin_sum));
    dict.insert(String::from("tuple"), ctx.tuple_type());
    dict.insert(String::from("type"), ctx.type_type());

    // Exceptions:
    dict.insert(
        String::from("BaseException"),
        ctx.exceptions.base_exception_type.clone(),
    );
    dict.insert(
        String::from("Exception"),
        ctx.exceptions.exception_type.clone(),
    );
    dict.insert(
        String::from("AssertionError"),
        ctx.exceptions.assertion_error.clone(),
    );
    dict.insert(
        String::from("AttributeError"),
        ctx.exceptions.attribute_error.clone(),
    );
    dict.insert(String::from("NameError"), ctx.exceptions.name_error.clone());
    dict.insert(
        String::from("RuntimeError"),
        ctx.exceptions.runtime_error.clone(),
    );
    dict.insert(
        String::from("NotImplementedError"),
        ctx.exceptions.not_implemented_error.clone(),
    );
    dict.insert(String::from("TypeError"), ctx.exceptions.type_error.clone());
    dict.insert(
        String::from("ValueError"),
        ctx.exceptions.value_error.clone(),
    );

    let d2 = PyObject::new(PyObjectKind::Dict { elements: dict }, ctx.type_type());
    let scope = PyObject::new(
        PyObjectKind::Scope {
            scope: Scope {
                locals: d2,
                parent: None,
            },
        },
        ctx.type_type(),
    );
    let obj = PyObject::new(
        PyObjectKind::Module {
            name: "__builtins__".to_string(),
            dict: scope,
        },
        ctx.type_type(),
    );
    obj
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

    &vm.invoke(
        function,
        PyFuncArgs {
            args: vec![namespace.clone()],
            kwargs: vec![],
        },
    );

    vm.call_method(&metaclass, "__call__", vec![name_arg, bases, namespace])
}

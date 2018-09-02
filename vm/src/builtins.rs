// use std::ops::Deref;
use std::char;
use std::collections::HashMap;
use std::io::{self, Write};

use super::compile;
use super::obj::objstr;
use super::obj::objtype;
use super::objbool;
use super::pyobject::{
    AttributeProtocol, DictProtocol, IdProtocol, PyContext, PyFuncArgs, PyObject, PyObjectKind,
    PyObjectRef, PyResult, Scope, TypeProtocol,
};
use super::vm::VirtualMachine;

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

fn dir_object(vm: &mut VirtualMachine, _obj: PyObjectRef) -> PyObjectRef {
    let d = vm.new_dict();
    // TODO: loop over dict of instance, next of class?
    // TODO: Implement dir for objects
    // for i in obj.iter_items() {
    //    d.set_item(k, v);
    // }
    d
}

// builtin_abs

fn builtin_all(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    for item in args.args {
        let result = objbool::boolval(vm, item)?;
        if !result {
            return Ok(vm.new_bool(false));
        }
    }
    Ok(vm.new_bool(true))
}

fn builtin_any(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    for item in args.args {
        let result = objbool::boolval(vm, item)?;
        if result {
            return Ok(vm.new_bool(true));
        }
    }
    Ok(vm.new_bool(false))
}

// builtin_ascii
// builtin_bin
// builtin_bool
// builtin_breakpoint
// builtin_bytearray
// builtin_bytes
// builtin_callable

fn builtin_chr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(i, Some(vm.ctx.int_type()))]);

    let code_point_obj = i.borrow();

    let code_point = *match code_point_obj.kind {
        PyObjectKind::Integer { ref value } => value,
        ref kind => panic!(
            "argument checking failure: chr not supported for {:?}",
            kind
        ),
    } as u32;

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

    match compile::compile(vm, &source, mode, None) {
        Ok(value) => Ok(value),
        Err(msg) => Err(vm.new_type_error(msg)),
    }
}

// builtin_complex
// builtin_delattr

fn builtin_dir(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    if args.args.is_empty() {
        Ok(dir_locals(vm))
    } else {
        let obj = args.args.into_iter().next().unwrap();
        Ok(dir_object(vm, obj))
    }
}

// builtin_divmod
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
    }.into_ref();

    // Run the source:
    vm.run_code_obj(code_obj.clone(), scope)
}

// builtin_exec
// builtin_filter
// builtin_float
// builtin_format
// builtin_frozenset

fn builtin_getattr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(obj, None), (attr, Some(vm.ctx.str_type()))]
    );
    if let PyObjectKind::String { ref value } = attr.borrow().kind {
        vm.get_attribute(obj.clone(), value)
    } else {
        panic!("argument checking failure: attr not string")
    }
}

// builtin_globals

fn builtin_hasattr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(obj, None), (attr, Some(vm.ctx.str_type()))]
    );
    if let PyObjectKind::String { ref value } = attr.borrow().kind {
        let has_attr = match vm.get_attribute(obj.clone(), value) {
            Ok(..) => true,
            Err(..) => false,
        };
        Ok(vm.context().new_bool(has_attr))
    } else {
        panic!("argument checking failure: attr not string")
    }
}

// builtin_hash
// builtin_help
// builtin_hex

fn builtin_id(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(obj, None)]);

    Ok(vm.context().new_int(obj.get_id() as i32))
}

// builtin_input
// builtin_int

fn builtin_isinstance(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(obj, None), (typ, None)]);

    let isinstance = objtype::isinstance(obj.clone(), typ.clone());
    Ok(vm.context().new_bool(isinstance))
}

fn builtin_issubclass(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    if args.args.len() != 2 {
        panic!("issubclass expects exactly two parameters");
    }

    let cls1 = args.args[0].clone();
    let cls2 = args.args[1].clone();

    Ok(vm.context().new_bool(objtype::issubclass(cls1, cls2)))
}

// builtin_iter

fn builtin_len(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(obj, None)]);
    match obj.borrow().kind {
        PyObjectKind::Dict { ref elements } => Ok(vm.context().new_int(elements.len() as i32)),
        PyObjectKind::Tuple { ref elements } => Ok(vm.context().new_int(elements.len() as i32)),
        _ => {
            let len_method_name = "__len__".to_string();
            match vm.get_attribute(obj.clone(), &len_method_name) {
                Ok(value) => vm.invoke(value, PyFuncArgs::default()),
                Err(..) => Err(vm.context().new_str(
                    format!(
                        "TypeError: object of this {:?} type has no method {:?}",
                        obj, len_method_name
                    ).to_string(),
                )),
            }
        }
    }
}

// builtin_list

fn builtin_locals(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args);
    Ok(vm.get_locals())
}

// builtin_map
// builtin_max
// builtin_memoryview
// builtin_min
// builtin_next
// builtin_object
// builtin_oct
// builtin_open
// builtin_ord
// builtin_pow

pub fn builtin_print(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    trace!("print called with {:?}", args);
    for a in args.args {
        let s = match vm.to_str(a) {
            Ok(v) => objstr::get_value(&v),
            Err(err) => return Err(err),
        };
        print!("{} ", s);
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
            let range_elements: Vec<PyObjectRef> =
                (0..*value).map(|num| vm.context().new_int(num)).collect();
            Ok(vm.context().new_list(range_elements))
        }
        _ => panic!("argument checking failure: first argument to range must be an integer"),
    }
}

// builtin_repr
fn builtin_repr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(obj, None)]);
    vm.to_repr(obj.clone())
}
// builtin_reversed
// builtin_round
// builtin_set

fn builtin_setattr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(obj, None), (attr, Some(vm.ctx.str_type())), (value, None)]
    );
    if let PyObjectKind::String { value: ref name } = attr.borrow().kind {
        obj.clone().set_attr(name, value.clone());
        Ok(vm.get_none())
    } else {
        panic!("argument checking failure: attr not string")
    }
}

// builtin_slice
// builtin_sorted
// builtin_staticmethod

// builtin_sum
// builtin_super
// builtin_vars
// builtin_zip
// builtin___import__

pub fn make_module(ctx: &PyContext) -> PyObjectRef {
    // scope[String::from("print")] = print;
    let mut dict = HashMap::new();
    dict.insert(String::from("all"), ctx.new_rustfunc(builtin_all));
    dict.insert(String::from("any"), ctx.new_rustfunc(builtin_any));
    dict.insert(String::from("bool"), ctx.bool_type());
    dict.insert(String::from("bytes"), ctx.bytes_type());
    dict.insert(String::from("chr"), ctx.new_rustfunc(builtin_chr));
    dict.insert(String::from("compile"), ctx.new_rustfunc(builtin_compile));
    dict.insert(String::from("dict"), ctx.dict_type());
    dict.insert(String::from("dir"), ctx.new_rustfunc(builtin_dir));
    dict.insert(String::from("eval"), ctx.new_rustfunc(builtin_eval));
    dict.insert(String::from("float"), ctx.float_type());
    dict.insert(String::from("getattr"), ctx.new_rustfunc(builtin_getattr));
    dict.insert(String::from("hasattr"), ctx.new_rustfunc(builtin_hasattr));
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
    dict.insert(String::from("len"), ctx.new_rustfunc(builtin_len));
    dict.insert(String::from("list"), ctx.list_type());
    dict.insert(String::from("locals"), ctx.new_rustfunc(builtin_locals));
    dict.insert(String::from("print"), ctx.new_rustfunc(builtin_print));
    dict.insert(String::from("range"), ctx.new_rustfunc(builtin_range));
    dict.insert(String::from("repr"), ctx.new_rustfunc(builtin_repr));
    dict.insert(String::from("setattr"), ctx.new_rustfunc(builtin_setattr));
    dict.insert(String::from("str"), ctx.str_type()); // new_rustfunc(builtin_str));
    dict.insert(String::from("tuple"), ctx.tuple_type());
    dict.insert(String::from("type"), ctx.type_type());
    dict.insert(String::from("object"), ctx.object());

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
    let name_arg_ref = name_arg.borrow();
    let name = match name_arg_ref.kind {
        PyObjectKind::String { ref value } => value,
        _ => panic!("Class name must by a string!"),
    };
    let mut bases = args.args.clone();
    bases.push(vm.context().object());
    let metaclass = vm.get_type();
    let namespace = vm.new_dict();
    &vm.invoke(
        function,
        PyFuncArgs {
            args: vec![namespace.clone()],
            kwargs: vec![],
        },
    );
    objtype::new(metaclass, name, bases, namespace)
}

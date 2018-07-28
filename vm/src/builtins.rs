// use std::ops::Deref;
use std::collections::HashMap;
use std::io::{self, Write};

use super::compile;
use super::pyobject::DictProtocol;
use super::pyobject::{Executor, PyContext, PyObject, PyObjectKind, PyObjectRef, PyResult, Scope};
use super::objbool;


fn get_locals(rt: &mut Executor) -> PyObjectRef {
    let mut d = rt.new_dict();
    // TODO: implement dict_iter_items?
    let locals = rt.get_locals();
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

fn dir_locals(rt: &mut Executor) -> PyObjectRef {
    get_locals(rt)
}

fn dir_object(rt: &mut Executor, obj: PyObjectRef) -> PyObjectRef {
    let d = rt.new_dict();
    d
}

pub fn builtin_dir(rt: &mut Executor, args: Vec<PyObjectRef>) -> PyResult {
    if args.is_empty() {
        Ok(dir_locals(rt))
    } else {
        let obj = args.into_iter().next().unwrap();
        Ok(dir_object(rt, obj))
    }
}

pub fn builtin_print(rt: &mut Executor, args: Vec<PyObjectRef>) -> PyResult {
    trace!("print called with {:?}", args);
    for a in args {
        print!("{} ", a.borrow().str());
    }
    println!();
    io::stdout().flush().unwrap();
    Ok(rt.get_none())
}

pub fn builtin_compile(rt: &mut Executor, args: Vec<PyObjectRef>) -> PyResult {
    if args.len() < 1 {
        return Err(rt.new_exception("Expected more arguments".to_string()))
    }
    // TODO:
    let mode = compile::Mode::Eval;
    let source = args[0].borrow().str();

    match compile::compile(rt, &source, mode) {
        Ok(value) => Ok(value),
        Err(msg) => Err(rt.new_exception(msg)),
    }
}

pub fn locals(rt: &mut Executor, args: Vec<PyObjectRef>) -> PyResult {
    Ok(rt.get_locals())
}

pub fn len(rt: &mut Executor, args: Vec<PyObjectRef>) -> PyResult {
    if args.len() != 1 {
        panic!("len(s) expects exactly one parameter");
    }
    let len = match args[0].borrow().kind {
        PyObjectKind::List { ref elements } => elements.len(),
        PyObjectKind::Tuple { ref elements } => elements.len(),
        PyObjectKind::String { ref value } => value.len(),
        _ => {
            return Err(rt.context()
                .new_str("TypeError: object of this type has no len()".to_string()))
        }
    };
    Ok(rt.context().new_int(len as i32))
}

pub fn make_module(ctx: &PyContext) -> PyObjectRef {
    // scope[String::from("print")] = print;
    let mut dict = HashMap::new();
    dict.insert(String::from("print"), ctx.new_rustfunc(builtin_print));
    dict.insert(String::from("type"), ctx.type_type.clone());
    dict.insert(String::from("int"), ctx.int_type.clone());
    dict.insert(String::from("all"), ctx.new_rustfunc(builtin_all));
    dict.insert(String::from("any"), ctx.new_rustfunc(builtin_any));
    dict.insert(String::from("dir"), ctx.new_rustfunc(builtin_dir));
    dict.insert(String::from("locals"), ctx.new_rustfunc(locals));
    dict.insert(String::from("compile"), ctx.new_rustfunc(builtin_compile));
    dict.insert("len".to_string(), ctx.new_rustfunc(len));
    let d2 = PyObject::new(PyObjectKind::Dict { elements: dict }, ctx.type_type.clone());
    let scope = PyObject::new(PyObjectKind::Scope { scope: Scope { locals: d2, parent: None} }, ctx.type_type.clone());
    let obj = PyObject::new(
        PyObjectKind::Module {
            name: "__builtins__".to_string(),
            dict: scope,
        },
        ctx.type_type.clone(),
    );
    obj
}

fn builtin_any(rt: &mut Executor, args: Vec<PyObjectRef>) -> PyResult {
    Ok(rt.new_bool(args.into_iter().any(|e| objbool::boolval(e))))
}

fn builtin_all(rt: &mut Executor, args: Vec<PyObjectRef>) -> PyResult {
    Ok(rt.new_bool(args.into_iter().all(|e| objbool::boolval(e))))
}

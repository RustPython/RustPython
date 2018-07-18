// use std::ops::Deref;
use std::collections::HashMap;
use std::io::{self, Write};

use super::pyobject::DictProtocol;
use super::pyobject::{Executor, PyContext, PyObject, PyObjectKind, PyObjectRef, PyResult};

/*
 * Original impl:
pub fn print(args: Vec<Rc<NativeType>>) -> NativeType {
    for elem in args {
        // TODO: figure out how python's print vectors
        match elem.deref() {
            &NativeType::NoneType => println!("None"),
            &NativeType::Boolean(ref b)=> {
                if *b {
                    println!("True");
                } else {
                    println!("False");
                }
            },
            &NativeType::Int(ref x)  => println!("{}", x),
            &NativeType::Float(ref x)  => println!("{}", x),
            &NativeType::Str(ref x)  => println!("{}", x),
            &NativeType::Unicode(ref x)  => println!("{}", x),
            _ => panic!("Print for {:?} not implemented yet", elem),
            /*
            List(Vec<NativeType>),
            Tuple(Vec<NativeType>),
            Iter(Vec<NativeType>), // TODO: use Iterator instead
            Code(PyCodeObject),
            Function(Function),
            #[serde(skip_serializing, skip_deserializing)]
            NativeFunction(fn(Vec<NativeType>) -> NativeType ),
            */
        }
    }
    NativeType::NoneType
}
*/

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

pub fn dir(rt: &mut Executor, args: Vec<PyObjectRef>) -> PyResult {
    if args.is_empty() {
        Ok(dir_locals(rt))
    } else {
        let obj = args.into_iter().next().unwrap();
        Ok(dir_object(rt, obj))
    }
}

pub fn print(rt: &mut Executor, args: Vec<PyObjectRef>) -> PyResult {
    // println!("Woot: {:?}", args);
    trace!("print called with {:?}", args);
    for a in args {
        print!("{} ", a.borrow().str());
    }
    println!();
    io::stdout().flush().unwrap();
    Ok(rt.get_none())
}

pub fn compile(rt: &mut Executor, args: Vec<PyObjectRef>) -> PyResult {
    // TODO
    Ok(rt.new_bool(true))
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
    dict.insert(String::from("print"), ctx.new_rustfunc(print));
    dict.insert(String::from("type"), ctx.type_type.clone());
    dict.insert(String::from("all"), ctx.new_rustfunc(all));
    dict.insert(String::from("any"), ctx.new_rustfunc(any));
    dict.insert(String::from("dir"), ctx.new_rustfunc(dir));
    dict.insert(String::from("locals"), ctx.new_rustfunc(locals));
    dict.insert("len".to_string(), ctx.new_rustfunc(len));
    let obj = PyObject::new(
        PyObjectKind::Module {
            name: "__builtins__".to_string(),
            dict: PyObject::new(PyObjectKind::Dict { elements: dict }, ctx.type_type.clone()),
        },
        ctx.type_type.clone(),
    );
    obj
}

fn any(rt: &mut Executor, args: Vec<PyObjectRef>) -> PyResult {
    // TODO
    Ok(rt.new_bool(true))
}

fn all(rt: &mut Executor, args: Vec<PyObjectRef>) -> PyResult {
    // TODO
    Ok(rt.new_bool(true))
}

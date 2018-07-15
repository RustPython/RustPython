// use std::ops::Deref;
use std::io::{self, Write};

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

pub fn print(rt: &mut Executor, args: Vec<PyObjectRef>) -> PyResult {
    // println!("Woot: {:?}", args);
    trace!("print called with {:?}", args);
    for a in args {
        print!("{} ", a.borrow_mut().str());
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
    // TODO
    Ok(rt.new_bool(true))
}

/*
 * TODO
pub fn len(args: Vec<Rc<NativeType>>) -> NativeType {
    if args.len() != 1 {
        panic!("len(s) expects exactly one parameter");
    }
    let len = match args[0].deref() {
        &NativeType::List(ref l) => l.borrow().len(),
        &NativeType::Tuple(ref t) => t.len(),
        &NativeType::Str(ref s) => s.len(),
        _ => panic!("TypeError: object of this type has no len()")
    };
    NativeType::Int(len as i32)
}
*/

pub fn make_module(ctx: &PyContext) -> PyObjectRef {
    // scope[String::from("print")] = print;
    let obj = PyObject::new(
        PyObjectKind::Module {
            name: "__builtins__".to_string(),
        },
        ctx.type_type.clone(),
    );
    obj.borrow_mut().dict.insert(
        String::from("print"),
        PyObject::new(
            PyObjectKind::RustFunction { function: print },
            ctx.type_type.clone(),
        ),
    );
    obj.borrow_mut()
        .dict
        .insert(String::from("type"), ctx.type_type.clone());
    obj.borrow_mut().dict.insert(
        String::from("all"),
        PyObject::new(
            PyObjectKind::RustFunction { function: all },
            ctx.type_type.clone(),
        ),
    );
    obj.borrow_mut().dict.insert(
        String::from("any"),
        PyObject::new(
            PyObjectKind::RustFunction { function: any },
            ctx.type_type.clone(),
        ),
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

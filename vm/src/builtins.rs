// use std::ops::Deref;
use std::char;
use std::collections::HashMap;
use std::io::{self, Write};

use super::compile;
use super::objbool;
use super::pyobject::DictProtocol;
use super::pyobject::{
    AttributeProtocol, IdProtocol, PyContext, PyFuncArgs, PyObject, PyObjectKind, PyObjectRef,
    PyResult, Scope,
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
    Ok(vm.new_bool(args.args.into_iter().all(|e| objbool::boolval(e))))
}

fn builtin_any(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    Ok(vm.new_bool(args.args.into_iter().any(|e| objbool::boolval(e))))
}

// builtin_ascii
// builtin_bin
// builtin_bool
// builtin_breakpoint
// builtin_bytearray
// builtin_bytes
// builtin_callable

fn builtin_chr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    if args.args.len() != 1 {
        return Err(vm.new_exception("Expected one arguments".to_string()));
    }

    let code_point_obj = args.args[0].borrow();

    let code_point = *match code_point_obj.kind {
        PyObjectKind::Integer { ref value } => value,
        ref kind => unimplemented!("{:?} not implemented for chr", kind),
    } as u32;

    let txt = match char::from_u32(code_point) {
        Some(value) => value.to_string(),
        None => '_'.to_string(),
    };

    Ok(vm.new_str(txt))
}

// builtin_classmethod

fn builtin_compile(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    if args.args.len() < 1 {
        return Err(vm.new_exception("Expected more arguments".to_string()));
    }
    // TODO:
    let mode = compile::Mode::Eval;
    let source = args.args[0].borrow().str();

    match compile::compile(vm, &source, mode) {
        Ok(value) => Ok(value),
        Err(msg) => Err(vm.new_exception(msg)),
    }
}

// builtin_complex
// builtin_delattr

fn builtin_dict(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    if !args.args.is_empty() {
        unimplemented!("only zero-arg version of dict is currently supported")
    }
    Ok(vm.new_dict())
}

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
    let args = args.args;
    if args.len() > 3 {
        return Err(vm.new_exception("Expected at maximum of 3 arguments".to_string()));
    } else if args.len() > 2 {
        // TODO: handle optional global and locals
    } else {
        return Err(vm.new_exception("Expected at least one argument".to_string()));
    }
    let source = args[0].clone();
    let _globals = args[1].clone();
    let locals = args[2].clone();

    let code_obj = source; // if source.borrow().kind

    // Construct new scope:
    let scope_inner = Scope {
        locals: locals,
        parent: None,
    };
    let scope = PyObject {
        kind: PyObjectKind::Scope { scope: scope_inner },
        typ: None,
    }.into_ref();

    // Run the source:
    vm.run_code_obj(code_obj, scope)
}

// builtin_exec
// builtin_filter
// builtin_float
// builtin_format
// builtin_frozenset

fn builtin_getattr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    let args = args.args;
    if args.len() == 2 {
        let obj = args[0].clone();
        let attr = args[1].borrow();
        if let PyObjectKind::String { ref value } = attr.kind {
            vm.get_attribute(obj, value)
        } else {
            Err(vm.new_exception("Attr can only be str for now".to_string()))
        }
    } else {
        Err(vm.new_exception("Expected 2 arguments".to_string()))
    }
}

// builtin_globals

fn builtin_hasattr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    let args = args.args;
    if args.len() == 2 {
        let obj = args[0].clone();
        let attr = args[1].borrow();
        if let PyObjectKind::String { ref value } = attr.kind {
            let has_attr = match vm.get_attribute(obj, value) {
                Ok(..) => true,
                Err(..) => false,
            };
            Ok(vm.context().new_bool(has_attr))
        } else {
            Err(vm.new_exception("Attr can only be str for now".to_string()))
        }
    } else {
        Err(vm.new_exception("Expected 2 arguments".to_string()))
    }
}

// builtin_hash
// builtin_help
// builtin_hex

fn builtin_id(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    if args.args.len() != 1 {
        return Err(vm.new_exception("Expected only one argument".to_string()));
    }

    Ok(vm.context().new_int(args.args[0].get_id() as i32))
}

// builtin_input
// builtin_int
// builtin_isinstance
// builtin_issubclass
// builtin_iter

fn builtin_len(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    if args.args.len() != 1 {
        panic!("len(s) expects exactly one parameter");
    }
    match args.args[0].borrow().kind {
        PyObjectKind::Dict { ref elements } => Ok(vm.context().new_int(elements.len() as i32)),
        PyObjectKind::Tuple { ref elements } => Ok(vm.context().new_int(elements.len() as i32)),
        PyObjectKind::String { ref value } => Ok(vm.context().new_int(value.len() as i32)),
        _ => {
            let len_method_name = "__len__".to_string();
            match vm.get_attribute(args.args[0].clone(), &len_method_name) {
                Ok(value) => vm.invoke(value, PyFuncArgs::default()),
                Err(..) => Err(vm.context().new_str(
                    format!(
                        "TypeError: object of this {:?} type has no method {:?}",
                        args.args[0], len_method_name
                    ).to_string(),
                )),
            }
        }
    }
}

// builtin_list

fn builtin_locals(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    if args.args.len() != 0 {
        panic!("locals() doesn't take any arguments");
    }
    Ok(vm.get_locals())
}

pub fn builtin_print(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    trace!("print called with {:?}", args);
    for a in args.args {
        print!("{} ", a.borrow().str());
    }
    println!();
    io::stdout().flush().unwrap();
    Ok(vm.get_none())
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
// builtin_print
// builtin_property

fn builtin_range(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    if args.args.len() != 1 {
        unimplemented!("only the single-parameter range is implemented");
    }
    match args.args[0].borrow().kind {
        PyObjectKind::Integer { ref value } => {
            let range_elements: Vec<PyObjectRef> =
                (0..*value).map(|num| vm.context().new_int(num)).collect();
            Ok(vm.context().new_list(Some(range_elements)))
        }
        _ => panic!("first argument to range must be an integer"),
    }
}

// builtin_repr
// builtin_reversed
// builtin_round
// builtin_set

fn builtin_setattr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    let args = args.args;
    if args.len() == 3 {
        let obj = args[0].clone();
        let attr = args[1].borrow();
        let value = args[2].clone();
        if let PyObjectKind::String { value: ref name } = attr.kind {
            obj.set_attr(name, value);
            Ok(vm.get_none())
        } else {
            Err(vm.new_exception("Attr can only be str for now".to_string()))
        }
    } else {
        Err(vm.new_exception("Expected 3 arguments".to_string()))
    }
}

// builtin_slice
// builtin_sorted
// builtin_staticmethod

// TODO: should with following format
// class str(object='')
// class str(object=b'', encoding='utf-8', errors='strict')
fn builtin_str(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    if args.args.len() != 1 {
        panic!("str expects exactly one parameter");
    };
    let s = match args.args[0].borrow().kind {
        PyObjectKind::Integer { value: _ } | PyObjectKind::String { value: _ } => {
            args.args[0].borrow().str()
        }
        _ => {
            return Err(vm
                .context()
                .new_str("TypeError: object of this type cannot be converted to str".to_string()))
        }
    };
    Ok(vm.new_str(s))
}
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
    dict.insert(String::from("chr"), ctx.new_rustfunc(builtin_chr));
    dict.insert(String::from("compile"), ctx.new_rustfunc(builtin_compile));
    // TODO: can we just insert dict here?
    dict.insert(String::from("dict"), ctx.new_rustfunc(builtin_dict));
    dict.insert(String::from("dir"), ctx.new_rustfunc(builtin_dir));
    dict.insert(String::from("eval"), ctx.new_rustfunc(builtin_eval));
    dict.insert(String::from("getattr"), ctx.new_rustfunc(builtin_getattr));
    dict.insert(String::from("hasattr"), ctx.new_rustfunc(builtin_hasattr));
    dict.insert(String::from("id"), ctx.new_rustfunc(builtin_id));
    dict.insert(String::from("int"), ctx.int_type.clone());
    dict.insert(String::from("len"), ctx.new_rustfunc(builtin_len));
    dict.insert(String::from("list"), ctx.list_type.clone());
    dict.insert(String::from("locals"), ctx.new_rustfunc(builtin_locals));
    dict.insert(String::from("print"), ctx.new_rustfunc(builtin_print));
    dict.insert(String::from("range"), ctx.new_rustfunc(builtin_range));
    dict.insert(String::from("setattr"), ctx.new_rustfunc(builtin_setattr));
    dict.insert(String::from("str"), ctx.new_rustfunc(builtin_str));
    dict.insert(String::from("tuple"), ctx.tuple_type.clone());
    dict.insert(String::from("type"), ctx.type_type.clone());
    dict.insert(String::from("object"), ctx.object.clone());
    let d2 = PyObject::new(PyObjectKind::Dict { elements: dict }, ctx.type_type.clone());
    let scope = PyObject::new(
        PyObjectKind::Scope {
            scope: Scope {
                locals: d2,
                parent: None,
            },
        },
        ctx.type_type.clone(),
    );
    let obj = PyObject::new(
        PyObjectKind::Module {
            name: "__builtins__".to_string(),
            dict: scope,
        },
        ctx.type_type.clone(),
    );
    obj
}

pub fn builtin_build_class_(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    let function = args.args[0].clone();
    let a1 = &*args.args[1].borrow();
    let name = match &a1.kind {
        PyObjectKind::String { value: name } => name,
        _ => panic!("Class name must be a string."),
    };

    let new_dict = vm.new_dict();
    &vm.invoke(
        function,
        PyFuncArgs {
            args: vec![new_dict.clone()],
        },
    );
    Ok(vm.new_class(name.to_string(), new_dict))
}

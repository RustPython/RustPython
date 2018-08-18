use super::pyobject::{
    AttributeProtocol, PyContext, PyFuncArgs, PyObject, PyObjectKind, PyObjectRef, PyResult,
    ToRust, TypeProtocol,
};
use super::vm::VirtualMachine;
use std::collections::HashMap;

/*
 * The magical type type
 */

pub fn create_type() -> PyObjectRef {
    let typ = PyObject {
        kind: PyObjectKind::None,
        typ: None,
    }.into_ref();

    let dict = PyObject::new(
        PyObjectKind::Dict {
            elements: HashMap::new(),
        },
        typ.clone(),
    );
    (*typ.borrow_mut()).kind = PyObjectKind::Class {
        name: String::from("type"),
        dict: dict,
        mro: vec![],
    };
    (*typ.borrow_mut()).typ = Some(typ.clone());
    typ
}

pub fn init(context: &mut PyContext) {
    context
        .type_type
        .set_attr(&String::from("__call__"), context.new_rustfunc(type_call));
    context
        .type_type
        .set_attr(&String::from("__new__"), context.new_rustfunc(type_new));

    context.type_type.set_attr(
        &String::from("__mro__"),
        context.new_member_descriptor(type_mro),
    );
    context.type_type.set_attr(
        &String::from("__class__"),
        context.new_member_descriptor(type_new),
    );
    context.type_type.set_attr(
        &String::from("__dict__"),
        context.new_member_descriptor(type_dict),
    );
}

fn type_mro(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    match args.args[0].borrow().kind {
        PyObjectKind::Class { ref mro, .. } => {
            let mut mro = mro.clone();
            mro.insert(0, args.args[0].clone());
            Ok(vm.context().new_tuple(mro.clone()))
        }
        _ => Err(vm.new_exception("Only classes have an MRO.".to_string())),
    }
}

fn type_dict(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    match args.args[0].borrow().kind {
        PyObjectKind::Class { ref dict, .. } => Ok(dict.clone()),
        _ => Err(vm.new_exception("type_dict must be called on a class.".to_string())),
    }
}

pub fn type_new(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    debug!("type.__new__{:?}", args);
    if args.args.len() == 2 {
        Ok(args.args[1].typ())
    } else if args.args.len() == 4 {
        let typ = args.args[0].clone();
        let name = args.args[1].to_str().unwrap();
        let bases = args.args[2].to_vec().unwrap();
        let dict = args.args[3].clone();
        new(typ, name, bases, dict)
    } else {
        Err(vm.new_exception(format!("TypeError: type_new: {:?}", args)))
    }
}

pub fn type_call(vm: &mut VirtualMachine, mut args: PyFuncArgs) -> PyResult {
    debug!("type_call: {:?}", args);
    let typ = args.shift();
    let new = typ.get_attr(&String::from("__new__"));
    let obj = vm.invoke(new, args.insert(typ.clone()))?;

    match get_attribute(vm, obj.typ(), &String::from("__init__")) {
        Ok(init) => {
            vm.invoke(init, args.insert(obj.clone()))?;
        }
        Err(_) => return Ok(obj),
    }

    Ok(obj)
}

pub fn get_attribute(vm: &mut VirtualMachine, obj: PyObjectRef, name: &String) -> PyResult {
    let cls = obj.typ();
    trace!("get_attribute: {:?}, {:?}, {:?}", cls, obj, name);
    if cls.has_attr(name) {
        let attr = cls.get_attr(name);
        let attr_class = attr.typ();
        if attr_class.has_attr(&String::from("__get__")) {
            return vm.invoke(
                attr_class.get_attr(&String::from("__get__")),
                PyFuncArgs {
                    args: vec![attr, obj, cls],
                },
            );
        }
    }

    if obj.has_attr(name) {
        Ok(obj.get_attr(name))
    } else if cls.has_attr(name) {
        Ok(cls.get_attr(name))
    } else {
        Err(vm.new_exception(format!(
            "AttributeError: {:?} object has no attribute {}",
            cls, name
        )))
    }
}

pub fn new(typ: PyObjectRef, name: String, bases: Vec<PyObjectRef>, dict: PyObjectRef) -> PyResult {
    Ok(PyObject::new(
        PyObjectKind::Class {
            name: name,
            dict: dict,
            mro: bases,
        },
        typ,
    ))
}

pub fn call(vm: &mut VirtualMachine, typ: PyObjectRef, args: PyFuncArgs) -> PyResult {
    let function = get_attribute(vm, typ, &String::from("__call__"))?;
    vm.invoke(function, args)
}

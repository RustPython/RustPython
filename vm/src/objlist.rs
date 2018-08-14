use super::objsequence::PySliceableSequence;
use super::pyobject::{PyFuncArgs, PyObject, PyObjectKind, PyObjectRef, PyResult};
use super::vm::VirtualMachine;
use std::collections::HashMap;

// set_item:
pub fn set_item(
    vm: &mut VirtualMachine,
    l: &mut Vec<PyObjectRef>,
    idx: PyObjectRef,
    obj: PyObjectRef,
) -> PyResult {
    match &(idx.borrow()).kind {
        PyObjectKind::Integer { value } => {
            let pos_index = l.get_pos(*value);
            l[pos_index] = obj;
            Ok(vm.get_none())
        }
        _ => panic!(
            "TypeError: indexing type {:?} with index {:?} is not supported (yet?)",
            l, idx
        ),
    }
}

fn append(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    trace!("list.append called with: {:?}", args);
    if args.args.len() == 2 {
        let l = args.args[0].clone();
        let o = args.args[1].clone();
        let mut list_obj = l.borrow_mut();
        if let PyObjectKind::List { ref mut elements } = list_obj.kind {
            elements.push(o);
            Ok(vm.get_none())
        } else {
            Err(vm.new_exception("list.append is called with no list".to_string()))
        }
    } else {
        Err(vm.new_exception("list.append requires two arguments".to_string()))
    }
}

fn clear(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    trace!("list.clear called with: {:?}", args);
    if args.args.len() == 1 {
        let l = args.args[0].clone();
        let mut list_obj = l.borrow_mut();
        if let PyObjectKind::List { ref mut elements } = list_obj.kind {
            elements.clear();
            Ok(vm.get_none())
        } else {
            Err(vm.new_exception("list.clear is called with no list".to_string()))
        }
    } else {
        Err(vm.new_exception("list.clear requires one arguments".to_string()))
    }
}

fn len(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    trace!("list.len called with: {:?}", args);
    // TODO: for this argument amount checking we could probably write some nice macro or templated function!
    if args.args.len() == 1 {
        let l = args.args[0].clone();
        let list_obj = l.borrow();
        if let PyObjectKind::List { ref elements } = list_obj.kind {
            Ok(vm.context().new_int(elements.len() as i32))
        } else {
            Err(vm.new_exception("list.len is called with no list".to_string()))
        }
    } else {
        Err(vm.new_exception("list.len requires one arguments".to_string()))
    }
}

fn reverse(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    trace!("list.reverse called with: {:?}", args);
    if args.args.len() == 1 {
        let l = args.args[0].clone();
        let mut list_obj = l.borrow_mut();
        if let PyObjectKind::List { ref mut elements } = list_obj.kind {
            elements.reverse();
            Ok(vm.get_none())
        } else {
            Err(vm.new_exception("list.reverse is called with no list".to_string()))
        }
    } else {
        Err(vm.new_exception("list.reverse requires one arguments".to_string()))
    }
}

pub fn create_type(type_type: PyObjectRef, method_type: PyObjectRef) -> PyObjectRef {
    let mut dict = HashMap::new();
    dict.insert(
        "__len__".to_string(),
        PyObject::new(
            PyObjectKind::RustFunction { function: len },
            method_type.clone(),
        ),
    );
    dict.insert(
        "append".to_string(),
        PyObject::new(
            PyObjectKind::RustFunction { function: append },
            method_type.clone(),
        ),
    );
    dict.insert(
        "clear".to_string(),
        PyObject::new(
            PyObjectKind::RustFunction { function: clear },
            method_type.clone(),
        ),
    );
    dict.insert(
        "reverse".to_string(),
        PyObject::new(
            PyObjectKind::RustFunction { function: reverse },
            method_type.clone(),
        ),
    );
    let typ = PyObject::new(
        PyObjectKind::Class {
            name: "list".to_string(),
            dict: PyObject::new(PyObjectKind::Dict { elements: dict }, type_type.clone()),
        },
        type_type.clone(),
    );
    typ
}

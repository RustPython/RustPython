/*
 * Builtin set type with a sequence of unique items.
 */

use super::super::pyobject::{
    AttributeProtocol, IdProtocol, PyContext, PyFuncArgs, PyObject, PyObjectKind, PyObjectRef,
    PyResult, TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objbool;
use super::objiter;
use super::objstr;
use super::objtype;
use num_bigint::ToBigInt;
use std::collections::HashMap;

pub fn get_elements(obj: &PyObjectRef) -> HashMap<usize, PyObjectRef> {
    if let PyObjectKind::Set { elements } = &obj.borrow().kind {
        elements.clone()
    } else {
        panic!("Cannot extract set elements from non-set");
    }
}

pub fn sequence_to_hashmap(iterable: &Vec<PyObjectRef>) -> HashMap<usize, PyObjectRef> {
    let mut elements = HashMap::new();
    for item in iterable {
        let key = item.get_id();
        elements.insert(key, item.clone());
    }
    elements
}

fn set_add(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    trace!("set.add called with: {:?}", args);
    arg_check!(
        vm,
        args,
        required = [(s, Some(vm.ctx.set_type())), (item, None)]
    );
    let mut mut_obj = s.borrow_mut();

    if let PyObjectKind::Set { ref mut elements } = mut_obj.kind {
        let key = item.get_id();
        elements.insert(key, item.clone());
        Ok(vm.get_none())
    } else {
        Err(vm.new_type_error("set.add is called with no list".to_string()))
    }
}

/* Create a new object of sub-type of set */
fn set_new(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(cls, None)],
        optional = [(iterable, None)]
    );

    if !objtype::issubclass(cls, &vm.ctx.set_type()) {
        return Err(vm.new_type_error(format!("{:?} is not a subtype of set", cls)));
    }

    let elements = match iterable {
        None => HashMap::new(),
        Some(iterable) => {
            let mut elements = HashMap::new();
            let iterator = objiter::get_iter(vm, iterable)?;
            loop {
                match vm.call_method(&iterator, "__next__", vec![]) {
                    Ok(v) => {
                        // TODO: should we use the hash function here?
                        let key = v.get_id();
                        elements.insert(key, v);
                    }
                    _ => break,
                }
            }
            elements
        }
    };

    Ok(PyObject::new(
        PyObjectKind::Set { elements: elements },
        cls.clone(),
    ))
}

fn set_len(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    trace!("set.len called with: {:?}", args);
    arg_check!(vm, args, required = [(s, Some(vm.ctx.set_type()))]);
    let elements = get_elements(s);
    Ok(vm.context().new_int(elements.len().to_bigint().unwrap()))
}

fn set_repr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(o, Some(vm.ctx.set_type()))]);

    let elements = get_elements(o);
    let mut str_parts = vec![];
    for elem in elements.values() {
        let part = vm.to_repr(elem)?;
        str_parts.push(objstr::get_value(&part));
    }

    let s = format!("{{ {} }}", str_parts.join(", "));
    Ok(vm.new_str(s))
}

pub fn set_contains(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(set, Some(vm.ctx.set_type())), (needle, None)]
    );
    for element in get_elements(set).iter() {
        match vm.call_method(needle, "__eq__", vec![element.1.clone()]) {
            Ok(value) => {
                if objbool::get_value(&value) {
                    return Ok(vm.new_bool(true));
                }
            }
            Err(_) => return Err(vm.new_type_error("".to_string())),
        }
    }

    Ok(vm.new_bool(false))
}

pub fn init(context: &PyContext) {
    let ref set_type = context.set_type;
    set_type.set_attr("__contains__", context.new_rustfunc(set_contains));
    set_type.set_attr("__len__", context.new_rustfunc(set_len));
    set_type.set_attr("__new__", context.new_rustfunc(set_new));
    set_type.set_attr("__repr__", context.new_rustfunc(set_repr));
    set_type.set_attr("add", context.new_rustfunc(set_add));
}

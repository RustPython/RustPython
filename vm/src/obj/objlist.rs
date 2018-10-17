use super::super::pyobject::{
    AttributeProtocol, PyContext, PyFuncArgs, PyObjectKind, PyObjectRef, PyResult, TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objbool;
use super::objiter;
use super::objsequence::{seq_equal, PySliceableSequence};
use super::objstr;
use super::objtype;

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

pub fn get_elements(obj: &PyObjectRef) -> Vec<PyObjectRef> {
    if let PyObjectKind::List { elements } = &obj.borrow().kind {
        elements.to_vec()
    } else {
        panic!("Cannot extract list elements from non-list");
    }
}

fn list_new(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(cls, None)],
        optional = [(iterable, None)]
    );

    if !objtype::issubclass(cls, vm.ctx.list_type()) {
        return Err(vm.new_type_error(format!("{:?} is not a subtype of list", cls)));
    }

    let elements = match iterable {
        None => vec![],
        Some(iterable) => {
            let mut elements = vec![];
            let iterator = objiter::get_iter(vm, iterable)?;
            loop {
                match vm.call_method(&iterator, "__next__", vec![]) {
                    Ok(v) => elements.push(v),
                    _ => break,
                }
            }
            elements
        }
    };

    Ok(vm.ctx.new_list(elements))
}

fn list_eq(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.list_type())), (other, None)]
    );

    let result = if objtype::isinstance(other, vm.ctx.list_type()) {
        let zelf = get_elements(zelf);
        let other = get_elements(other);
        seq_equal(vm, zelf, other)?
    } else {
        false
    };
    Ok(vm.ctx.new_bool(result))
}

fn list_add(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(o, Some(vm.ctx.list_type())), (o2, None)]
    );

    if objtype::isinstance(o2, vm.ctx.list_type()) {
        let e1 = get_elements(o);
        let e2 = get_elements(o2);
        let elements = e1.iter().chain(e2.iter()).map(|e| e.clone()).collect();
        Ok(vm.ctx.new_list(elements))
    } else {
        Err(vm.new_type_error(format!("Cannot add {:?} and {:?}", o, o2)))
    }
}

fn list_repr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(o, Some(vm.ctx.list_type()))]);

    let elements = get_elements(o);
    let mut str_parts = vec![];
    for elem in elements {
        match vm.to_repr(elem) {
            Ok(s) => str_parts.push(objstr::get_value(&s)),
            Err(err) => return Err(err),
        }
    }

    let s = format!("[{}]", str_parts.join(", "));
    Ok(vm.new_str(s))
}

pub fn append(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    trace!("list.append called with: {:?}", args);
    arg_check!(
        vm,
        args,
        required = [(list, Some(vm.ctx.list_type())), (x, None)]
    );
    let mut list_obj = list.borrow_mut();
    if let PyObjectKind::List { ref mut elements } = list_obj.kind {
        elements.push(x.clone());
        Ok(vm.get_none())
    } else {
        Err(vm.new_type_error("list.append is called with no list".to_string()))
    }
}

fn clear(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    trace!("list.clear called with: {:?}", args);
    arg_check!(vm, args, required = [(list, Some(vm.ctx.list_type()))]);
    let mut list_obj = list.borrow_mut();
    if let PyObjectKind::List { ref mut elements } = list_obj.kind {
        elements.clear();
        Ok(vm.get_none())
    } else {
        Err(vm.new_type_error("list.clear is called with no list".to_string()))
    }
}

fn list_len(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    trace!("list.len called with: {:?}", args);
    arg_check!(vm, args, required = [(list, Some(vm.ctx.list_type()))]);
    let elements = get_elements(list);
    Ok(vm.context().new_int(elements.len() as i32))
}

fn reverse(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    trace!("list.reverse called with: {:?}", args);
    arg_check!(vm, args, required = [(list, Some(vm.ctx.list_type()))]);
    let mut list_obj = list.borrow_mut();
    if let PyObjectKind::List { ref mut elements } = list_obj.kind {
        elements.reverse();
        Ok(vm.get_none())
    } else {
        Err(vm.new_type_error("list.reverse is called with no list".to_string()))
    }
}

fn list_contains(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    trace!("list.contains called with: {:?}", args);
    arg_check!(
        vm,
        args,
        required = [(list, Some(vm.ctx.list_type())), (needle, None)]
    );
    for element in get_elements(list).iter() {
        match vm.call_method(needle, "__eq__", vec![element.clone()]) {
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
    let ref list_type = context.list_type;
    list_type.set_attr("__add__", context.new_rustfunc(list_add));
    list_type.set_attr("__contains__", context.new_rustfunc(list_contains));
    list_type.set_attr("__eq__", context.new_rustfunc(list_eq));
    list_type.set_attr("__len__", context.new_rustfunc(list_len));
    list_type.set_attr("__new__", context.new_rustfunc(list_new));
    list_type.set_attr("__repr__", context.new_rustfunc(list_repr));
    list_type.set_attr("append", context.new_rustfunc(append));
    list_type.set_attr("clear", context.new_rustfunc(clear));
    list_type.set_attr("reverse", context.new_rustfunc(reverse));
}

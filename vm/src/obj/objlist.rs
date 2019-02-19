use super::super::pyobject::{
    IdProtocol, PyContext, PyFuncArgs, PyObject, PyObjectPayload, PyObjectRef, PyResult,
    TypeProtocol,
};
use super::super::vm::{ReprGuard, VirtualMachine};
use super::objbool;
use super::objint;
use super::objsequence::{
    get_elements, get_item, get_mut_elements, seq_equal, seq_ge, seq_gt, seq_le, seq_lt, seq_mul,
    PySliceableSequence,
};
use super::objstr;
use super::objtype;
use num_traits::ToPrimitive;

// set_item:
fn set_item(
    vm: &mut VirtualMachine,
    l: &mut Vec<PyObjectRef>,
    idx: PyObjectRef,
    obj: PyObjectRef,
) -> PyResult {
    if objtype::isinstance(&idx, &vm.ctx.int_type()) {
        let value = objint::get_value(&idx).to_i32().unwrap();
        if let Some(pos_index) = l.get_pos(value) {
            l[pos_index] = obj;
            Ok(vm.get_none())
        } else {
            Err(vm.new_index_error("list index out of range".to_string()))
        }
    } else {
        panic!(
            "TypeError: indexing type {:?} with index {:?} is not supported (yet?)",
            l, idx
        )
    }
}

fn list_new(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(cls, None)],
        optional = [(iterable, None)]
    );

    if !objtype::issubclass(cls, &vm.ctx.list_type()) {
        return Err(vm.new_type_error(format!("{:?} is not a subtype of list", cls)));
    }

    let elements = if let Some(iterable) = iterable {
        vm.extract_elements(iterable)?
    } else {
        vec![]
    };

    Ok(PyObject::new(
        PyObjectPayload::Sequence { elements },
        cls.clone(),
    ))
}

fn list_eq(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.list_type())), (other, None)]
    );

    if zelf.is(&other) {
        return Ok(vm.ctx.new_bool(true));
    }

    let result = if objtype::isinstance(other, &vm.ctx.list_type()) {
        let zelf = get_elements(zelf);
        let other = get_elements(other);
        seq_equal(vm, &zelf, &other)?
    } else {
        false
    };
    Ok(vm.ctx.new_bool(result))
}

fn list_lt(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.list_type())), (other, None)]
    );

    let result = if objtype::isinstance(other, &vm.ctx.list_type()) {
        let zelf = get_elements(zelf);
        let other = get_elements(other);
        seq_lt(vm, &zelf, &other)?
    } else {
        return Err(vm.new_type_error(format!(
            "Cannot compare {} and {} using '<'",
            zelf.borrow(),
            other.borrow()
        )));
    };

    Ok(vm.ctx.new_bool(result))
}

fn list_gt(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.list_type())), (other, None)]
    );

    let result = if objtype::isinstance(other, &vm.ctx.list_type()) {
        let zelf = get_elements(zelf);
        let other = get_elements(other);
        seq_gt(vm, &zelf, &other)?
    } else {
        return Err(vm.new_type_error(format!(
            "Cannot compare {} and {} using '>'",
            zelf.borrow(),
            other.borrow()
        )));
    };

    Ok(vm.ctx.new_bool(result))
}

fn list_ge(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.list_type())), (other, None)]
    );

    let result = if objtype::isinstance(other, &vm.ctx.list_type()) {
        let zelf = get_elements(zelf);
        let other = get_elements(other);
        seq_ge(vm, &zelf, &other)?
    } else {
        return Err(vm.new_type_error(format!(
            "Cannot compare {} and {} using '>='",
            zelf.borrow(),
            other.borrow()
        )));
    };

    Ok(vm.ctx.new_bool(result))
}

fn list_le(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.list_type())), (other, None)]
    );

    let result = if objtype::isinstance(other, &vm.ctx.list_type()) {
        let zelf = get_elements(zelf);
        let other = get_elements(other);
        seq_le(vm, &zelf, &other)?
    } else {
        return Err(vm.new_type_error(format!(
            "Cannot compare {} and {} using '<='",
            zelf.borrow(),
            other.borrow()
        )));
    };

    Ok(vm.ctx.new_bool(result))
}

fn list_add(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(o, Some(vm.ctx.list_type())), (o2, None)]
    );

    if objtype::isinstance(o2, &vm.ctx.list_type()) {
        let e1 = get_elements(o);
        let e2 = get_elements(o2);
        let elements = e1.iter().chain(e2.iter()).cloned().collect();
        Ok(vm.ctx.new_list(elements))
    } else {
        Err(vm.new_type_error(format!("Cannot add {} and {}", o.borrow(), o2.borrow())))
    }
}

fn list_iadd(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.list_type())), (other, None)]
    );

    if objtype::isinstance(other, &vm.ctx.list_type()) {
        get_mut_elements(zelf).extend_from_slice(&get_elements(other));
        Ok(zelf.clone())
    } else {
        Ok(vm.ctx.not_implemented())
    }
}

fn list_repr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(o, Some(vm.ctx.list_type()))]);

    let s = if let Some(_guard) = ReprGuard::enter(o) {
        let elements = get_elements(o);
        let mut str_parts = vec![];
        for elem in elements.iter() {
            let s = vm.to_repr(elem)?;
            str_parts.push(objstr::get_value(&s));
        }
        format!("[{}]", str_parts.join(", "))
    } else {
        "[...]".to_string()
    };

    Ok(vm.new_str(s))
}

pub fn list_append(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    trace!("list.append called with: {:?}", args);
    arg_check!(
        vm,
        args,
        required = [(list, Some(vm.ctx.list_type())), (x, None)]
    );
    let mut elements = get_mut_elements(list);
    elements.push(x.clone());
    Ok(vm.get_none())
}

fn list_clear(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    trace!("list.clear called with: {:?}", args);
    arg_check!(vm, args, required = [(list, Some(vm.ctx.list_type()))]);
    let mut elements = get_mut_elements(list);
    elements.clear();
    Ok(vm.get_none())
}

fn list_copy(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, Some(vm.ctx.list_type()))]);
    let elements = get_elements(zelf);
    Ok(vm.ctx.new_list(elements.clone()))
}

fn list_count(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.list_type())), (value, None)]
    );
    let elements = get_elements(zelf);
    let mut count: usize = 0;
    for element in elements.iter() {
        if value.is(&element) {
            count += 1;
        } else {
            let is_eq = vm._eq(element.clone(), value.clone())?;
            if objbool::boolval(vm, is_eq)? {
                count += 1;
            }
        }
    }
    Ok(vm.context().new_int(count))
}

pub fn list_extend(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(list, Some(vm.ctx.list_type())), (x, None)]
    );
    let mut new_elements = vm.extract_elements(x)?;
    let mut elements = get_mut_elements(list);
    elements.append(&mut new_elements);
    Ok(vm.get_none())
}

fn list_index(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    trace!("list.index called with: {:?}", args);
    arg_check!(
        vm,
        args,
        required = [(list, Some(vm.ctx.list_type())), (needle, None)]
    );
    for (index, element) in get_elements(list).iter().enumerate() {
        if needle.is(&element) {
            return Ok(vm.context().new_int(index));
        }
        let py_equal = vm._eq(needle.clone(), element.clone())?;
        if objbool::get_value(&py_equal) {
            return Ok(vm.context().new_int(index));
        }
    }
    let needle_str = objstr::get_value(&vm.to_str(needle).unwrap());
    Err(vm.new_value_error(format!("'{}' is not in list", needle_str)))
}

fn list_insert(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    trace!("list.insert called with: {:?}", args);
    arg_check!(
        vm,
        args,
        required = [
            (list, Some(vm.ctx.list_type())),
            (insert_position, Some(vm.ctx.int_type())),
            (element, None)
        ]
    );
    let int_position = match objint::get_value(insert_position).to_isize() {
        Some(i) => i,
        None => {
            return Err(
                vm.new_overflow_error("Python int too large to convert to Rust isize".to_string())
            );
        }
    };
    let mut vec = get_mut_elements(list);
    let vec_len = vec.len().to_isize().unwrap();
    // This unbounded position can be < 0 or > vec.len()
    let unbounded_position = if int_position < 0 {
        vec_len + int_position
    } else {
        int_position
    };
    // Bound it by [0, vec.len()]
    let position = unbounded_position.max(0).min(vec_len).to_usize().unwrap();
    vec.insert(position, element.clone());
    Ok(vm.get_none())
}

fn list_len(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    trace!("list.len called with: {:?}", args);
    arg_check!(vm, args, required = [(list, Some(vm.ctx.list_type()))]);
    let elements = get_elements(list);
    Ok(vm.context().new_int(elements.len()))
}

fn list_reverse(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    trace!("list.reverse called with: {:?}", args);
    arg_check!(vm, args, required = [(list, Some(vm.ctx.list_type()))]);
    let mut elements = get_mut_elements(list);
    elements.reverse();
    Ok(vm.get_none())
}

fn list_sort(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(list, Some(vm.ctx.list_type()))]);
    let mut _elements = get_mut_elements(list);
    unimplemented!("TODO: figure out how to invoke `sort_by` on a Vec");
    // elements.sort_by();
    // Ok(vm.get_none())
}

fn list_contains(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    trace!("list.contains called with: {:?}", args);
    arg_check!(
        vm,
        args,
        required = [(list, Some(vm.ctx.list_type())), (needle, None)]
    );
    for element in get_elements(list).iter() {
        if needle.is(&element) {
            return Ok(vm.new_bool(true));
        }
        match vm._eq(needle.clone(), element.clone()) {
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

fn list_getitem(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    trace!("list.getitem called with: {:?}", args);
    arg_check!(
        vm,
        args,
        required = [(list, Some(vm.ctx.list_type())), (needle, None)]
    );
    get_item(vm, list, &get_elements(list), needle.clone())
}

fn list_iter(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(list, Some(vm.ctx.list_type()))]);

    let iter_obj = PyObject::new(
        PyObjectPayload::Iterator {
            position: 0,
            iterated_obj: list.clone(),
        },
        vm.ctx.iter_type(),
    );

    // We are all good here:
    Ok(iter_obj)
}

fn list_setitem(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(list, Some(vm.ctx.list_type())), (key, None), (value, None)]
    );
    let mut elements = get_mut_elements(list);
    set_item(vm, &mut elements, key.clone(), value.clone())
}

fn list_mul(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [
            (list, Some(vm.ctx.list_type())),
            (product, Some(vm.ctx.int_type()))
        ]
    );

    let new_elements = seq_mul(&get_elements(list), product);

    Ok(vm.ctx.new_list(new_elements))
}

fn list_pop(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, Some(vm.ctx.list_type()))]);

    let mut elements = get_mut_elements(zelf);
    if let Some(result) = elements.pop() {
        Ok(result)
    } else {
        Err(vm.new_index_error("pop from empty list".to_string()))
    }
}

fn list_remove(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(list, Some(vm.ctx.list_type())), (needle, None)]
    );

    let mut ri: Option<usize> = None;
    for (index, element) in get_elements(list).iter().enumerate() {
        if needle.is(&element) {
            ri = Some(index);
            break;
        }
        let py_equal = vm._eq(needle.clone(), element.clone())?;
        if objbool::get_value(&py_equal) {
            ri = Some(index);
            break;
        }
    }

    if let Some(index) = ri {
        let mut elements = get_mut_elements(list);
        elements.remove(index);
        Ok(vm.get_none())
    } else {
        let needle_str = objstr::get_value(&vm.to_str(needle)?);
        Err(vm.new_value_error(format!("'{}' is not in list", needle_str)))
    }
}

pub fn init(context: &PyContext) {
    let list_type = &context.list_type;

    let list_doc = "Built-in mutable sequence.\n\n\
                    If no argument is given, the constructor creates a new empty list.\n\
                    The argument must be an iterable if specified.";

    context.set_attr(&list_type, "__add__", context.new_rustfunc(list_add));
    context.set_attr(&list_type, "__iadd__", context.new_rustfunc(list_iadd));
    context.set_attr(
        &list_type,
        "__contains__",
        context.new_rustfunc(list_contains),
    );
    context.set_attr(&list_type, "__eq__", context.new_rustfunc(list_eq));
    context.set_attr(&list_type, "__lt__", context.new_rustfunc(list_lt));
    context.set_attr(&list_type, "__gt__", context.new_rustfunc(list_gt));
    context.set_attr(&list_type, "__le__", context.new_rustfunc(list_le));
    context.set_attr(&list_type, "__ge__", context.new_rustfunc(list_ge));
    context.set_attr(
        &list_type,
        "__getitem__",
        context.new_rustfunc(list_getitem),
    );
    context.set_attr(&list_type, "__iter__", context.new_rustfunc(list_iter));
    context.set_attr(
        &list_type,
        "__setitem__",
        context.new_rustfunc(list_setitem),
    );
    context.set_attr(&list_type, "__mul__", context.new_rustfunc(list_mul));
    context.set_attr(&list_type, "__len__", context.new_rustfunc(list_len));
    context.set_attr(&list_type, "__new__", context.new_rustfunc(list_new));
    context.set_attr(&list_type, "__repr__", context.new_rustfunc(list_repr));
    context.set_attr(&list_type, "__doc__", context.new_str(list_doc.to_string()));
    context.set_attr(&list_type, "append", context.new_rustfunc(list_append));
    context.set_attr(&list_type, "clear", context.new_rustfunc(list_clear));
    context.set_attr(&list_type, "copy", context.new_rustfunc(list_copy));
    context.set_attr(&list_type, "count", context.new_rustfunc(list_count));
    context.set_attr(&list_type, "extend", context.new_rustfunc(list_extend));
    context.set_attr(&list_type, "index", context.new_rustfunc(list_index));
    context.set_attr(&list_type, "insert", context.new_rustfunc(list_insert));
    context.set_attr(&list_type, "reverse", context.new_rustfunc(list_reverse));
    context.set_attr(&list_type, "sort", context.new_rustfunc(list_sort));
    context.set_attr(&list_type, "pop", context.new_rustfunc(list_pop));
    context.set_attr(&list_type, "remove", context.new_rustfunc(list_remove));
}

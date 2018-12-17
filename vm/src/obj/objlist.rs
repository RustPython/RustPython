use super::super::pyobject::{
    PyContext, PyFuncArgs, PyObject, PyObjectKind, PyObjectRef, PyResult, TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objbool;
use super::objint;
use super::objsequence::{
    get_elements, get_item, get_mut_elements, seq_equal, PySliceableSequence,
};
use super::objstr;
use super::objtype;
use num_bigint::ToBigInt;
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
        let pos_index = l.get_pos(value);
        l[pos_index] = obj;
        Ok(vm.get_none())
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
        PyObjectKind::Sequence { elements: elements },
        cls.clone(),
    ))
}

fn list_eq(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.list_type())), (other, None)]
    );

    let result = if objtype::isinstance(other, &vm.ctx.list_type()) {
        let zelf = get_elements(zelf);
        let other = get_elements(other);
        seq_equal(vm, &zelf, &other)?
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

    if objtype::isinstance(o2, &vm.ctx.list_type()) {
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
    for elem in elements.iter() {
        let s = vm.to_repr(elem)?;
        str_parts.push(objstr::get_value(&s));
    }

    let s = format!("[{}]", str_parts.join(", "));
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

fn list_count(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.list_type())), (value, None)]
    );
    let elements = get_elements(zelf);
    let mut count: usize = 0;
    for element in elements.iter() {
        let is_eq = vm._eq(element, value.clone())?;
        if objbool::boolval(vm, is_eq)? {
            count = count + 1;
        }
    }
    Ok(vm.context().new_int(count.to_bigint().unwrap()))
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

fn list_len(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    trace!("list.len called with: {:?}", args);
    arg_check!(vm, args, required = [(list, Some(vm.ctx.list_type()))]);
    let elements = get_elements(list);
    Ok(vm.context().new_int(elements.len().to_bigint().unwrap()))
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
        PyObjectKind::Iterator {
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

    let counter = objint::get_value(&product).to_usize().unwrap();

    let elements = get_elements(list);
    let current_len = elements.len();
    let mut new_elements = Vec::with_capacity(counter * current_len);

    for _ in 0..counter {
        new_elements.extend(elements.clone());
    }

    Ok(PyObject::new(
        PyObjectKind::Sequence {
            elements: new_elements,
        },
        vm.ctx.list_type(),
    ))
}

pub fn init(context: &PyContext) {
    let ref list_type = context.list_type;
    context.set_attr(&list_type, "__add__", context.new_rustfunc(list_add));
    context.set_attr(
        &list_type,
        "__contains__",
        context.new_rustfunc(list_contains),
    );
    context.set_attr(&list_type, "__eq__", context.new_rustfunc(list_eq));
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
    context.set_attr(&list_type, "append", context.new_rustfunc(list_append));
    context.set_attr(&list_type, "clear", context.new_rustfunc(list_clear));
    context.set_attr(&list_type, "count", context.new_rustfunc(list_count));
    context.set_attr(&list_type, "extend", context.new_rustfunc(list_extend));
    context.set_attr(&list_type, "reverse", context.new_rustfunc(list_reverse));
    context.set_attr(&list_type, "sort", context.new_rustfunc(list_sort));
}

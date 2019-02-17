use super::super::pyobject::{
    PyContext, PyFuncArgs, PyObject, PyObjectPayload, PyResult, TypeProtocol,
};
use super::super::vm::{ReprGuard, VirtualMachine};
use super::objbool;
use super::objint;
use super::objsequence::{
    get_elements, get_item, seq_equal, seq_ge, seq_gt, seq_le, seq_lt, seq_mul,
};
use super::objstr;
use super::objtype;
use std::hash::{Hash, Hasher};

fn tuple_lt(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.tuple_type())), (other, None)]
    );

    let result = if objtype::isinstance(other, &vm.ctx.tuple_type()) {
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

fn tuple_gt(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.tuple_type())), (other, None)]
    );

    let result = if objtype::isinstance(other, &vm.ctx.tuple_type()) {
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

fn tuple_ge(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.tuple_type())), (other, None)]
    );

    let result = if objtype::isinstance(other, &vm.ctx.tuple_type()) {
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

fn tuple_le(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.tuple_type())), (other, None)]
    );

    let result = if objtype::isinstance(other, &vm.ctx.tuple_type()) {
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

fn tuple_add(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.tuple_type())), (other, None)]
    );

    if objtype::isinstance(other, &vm.ctx.tuple_type()) {
        let e1 = get_elements(zelf);
        let e2 = get_elements(other);
        let elements = e1.iter().chain(e2.iter()).cloned().collect();
        Ok(vm.ctx.new_tuple(elements))
    } else {
        Err(vm.new_type_error(format!(
            "Cannot add {} and {}",
            zelf.borrow(),
            other.borrow()
        )))
    }
}

fn tuple_count(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.tuple_type())), (value, None)]
    );
    let elements = get_elements(zelf);
    let mut count: usize = 0;
    for element in elements.iter() {
        let is_eq = vm._eq(element.clone(), value.clone())?;
        if objbool::boolval(vm, is_eq)? {
            count += 1;
        }
    }
    Ok(vm.context().new_int(count))
}

fn tuple_eq(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.tuple_type())), (other, None)]
    );

    let result = if objtype::isinstance(other, &vm.ctx.tuple_type()) {
        let zelf = get_elements(zelf);
        let other = get_elements(other);
        seq_equal(vm, &zelf, &other)?
    } else {
        false
    };
    Ok(vm.ctx.new_bool(result))
}

fn tuple_hash(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, Some(vm.ctx.tuple_type()))]);
    let elements = get_elements(zelf);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for element in elements.iter() {
        let element_hash = objint::get_value(&vm.call_method(element, "__hash__", vec![])?);
        element_hash.hash(&mut hasher);
    }
    let hash = hasher.finish();
    Ok(vm.ctx.new_int(hash))
}

fn tuple_iter(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(tuple, Some(vm.ctx.tuple_type()))]);

    let iter_obj = PyObject::new(
        PyObjectPayload::Iterator {
            position: 0,
            iterated_obj: tuple.clone(),
        },
        vm.ctx.iter_type(),
    );

    Ok(iter_obj)
}

fn tuple_len(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, Some(vm.ctx.tuple_type()))]);
    let elements = get_elements(zelf);
    Ok(vm.context().new_int(elements.len()))
}

fn tuple_new(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(cls, None)],
        optional = [(iterable, None)]
    );

    if !objtype::issubclass(cls, &vm.ctx.tuple_type()) {
        return Err(vm.new_type_error(format!("{} is not a subtype of tuple", cls.borrow())));
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

fn tuple_repr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, Some(vm.ctx.tuple_type()))]);

    let s = if let Some(_guard) = ReprGuard::enter(zelf) {
        let elements = get_elements(zelf);

        let mut str_parts = vec![];
        for elem in elements.iter() {
            let s = vm.to_repr(elem)?;
            str_parts.push(objstr::get_value(&s));
        }

        if str_parts.len() == 1 {
            format!("({},)", str_parts[0])
        } else {
            format!("({})", str_parts.join(", "))
        }
    } else {
        "(...)".to_string()
    };
    Ok(vm.new_str(s))
}

fn tuple_mul(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [
            (zelf, Some(vm.ctx.tuple_type())),
            (product, Some(vm.ctx.int_type()))
        ]
    );

    let new_elements = seq_mul(&get_elements(zelf), product);

    Ok(vm.ctx.new_tuple(new_elements))
}

fn tuple_getitem(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(tuple, Some(vm.ctx.tuple_type())), (needle, None)]
    );
    get_item(vm, tuple, &get_elements(&tuple), needle.clone())
}

pub fn tuple_index(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(tuple, Some(vm.ctx.tuple_type())), (needle, None)]
    );
    for (index, element) in get_elements(tuple).iter().enumerate() {
        let py_equal = vm._eq(needle.clone(), element.clone())?;
        if objbool::get_value(&py_equal) {
            return Ok(vm.context().new_int(index));
        }
    }
    Err(vm.new_value_error("tuple.index(x): x not in tuple".to_string()))
}

pub fn tuple_contains(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(tuple, Some(vm.ctx.tuple_type())), (needle, None)]
    );
    for element in get_elements(tuple).iter() {
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

pub fn init(context: &PyContext) {
    let tuple_type = &context.tuple_type;
    let tuple_doc = "tuple() -> empty tuple
tuple(iterable) -> tuple initialized from iterable's items

If the argument is a tuple, the return value is the same object.";
    context.set_attr(&tuple_type, "__add__", context.new_rustfunc(tuple_add));
    context.set_attr(&tuple_type, "__eq__", context.new_rustfunc(tuple_eq));
    context.set_attr(
        &tuple_type,
        "__contains__",
        context.new_rustfunc(tuple_contains),
    );
    context.set_attr(
        &tuple_type,
        "__getitem__",
        context.new_rustfunc(tuple_getitem),
    );
    context.set_attr(&tuple_type, "__hash__", context.new_rustfunc(tuple_hash));
    context.set_attr(&tuple_type, "__iter__", context.new_rustfunc(tuple_iter));
    context.set_attr(&tuple_type, "__len__", context.new_rustfunc(tuple_len));
    context.set_attr(&tuple_type, "__new__", context.new_rustfunc(tuple_new));
    context.set_attr(&tuple_type, "__mul__", context.new_rustfunc(tuple_mul));
    context.set_attr(&tuple_type, "__repr__", context.new_rustfunc(tuple_repr));
    context.set_attr(&tuple_type, "count", context.new_rustfunc(tuple_count));
    context.set_attr(&tuple_type, "__lt__", context.new_rustfunc(tuple_lt));
    context.set_attr(&tuple_type, "__le__", context.new_rustfunc(tuple_le));
    context.set_attr(&tuple_type, "__gt__", context.new_rustfunc(tuple_gt));
    context.set_attr(&tuple_type, "__ge__", context.new_rustfunc(tuple_ge));
    context.set_attr(
        &tuple_type,
        "__doc__",
        context.new_str(tuple_doc.to_string()),
    );
    context.set_attr(&tuple_type, "index", context.new_rustfunc(tuple_index));
}

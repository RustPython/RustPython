use super::super::pyobject::{
    PyContext, PyFuncArgs, PyObject, PyObjectKind, PyObjectRef, PyResult, TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objbool;
use super::objint;
use super::objsequence::{get_elements, get_item, seq_equal};
use super::objstr;
use super::objtype;
use num_bigint::ToBigInt;
use std::hash::{Hash, Hasher};

fn tuple_count(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.tuple_type())), (value, None)]
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
    Ok(vm.ctx.new_int(hash.to_bigint().unwrap()))
}

fn tuple_iter(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(tuple, Some(vm.ctx.tuple_type()))]);

    let iter_obj = PyObject::new(
        PyObjectKind::Iterator {
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
    Ok(vm.context().new_int(elements.len().to_bigint().unwrap()))
}

fn tuple_new(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(cls, None)],
        optional = [(iterable, None)]
    );

    if !objtype::issubclass(cls, &vm.ctx.tuple_type()) {
        return Err(vm.new_type_error(format!("{:?} is not a subtype of tuple", cls)));
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

fn tuple_repr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, Some(vm.ctx.tuple_type()))]);

    let elements = get_elements(zelf);

    let mut str_parts = vec![];
    for elem in elements.iter() {
        let s = vm.to_repr(elem)?;
        str_parts.push(objstr::get_value(&s));
    }

    let s = if str_parts.len() == 1 {
        format!("({},)", str_parts[0])
    } else {
        format!("({})", str_parts.join(", "))
    };
    Ok(vm.new_str(s))
}

fn tuple_getitem(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(tuple, Some(vm.ctx.tuple_type())), (needle, None)]
    );
    get_item(vm, tuple, &get_elements(&tuple), needle.clone())
}

pub fn tuple_contains(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(tuple, Some(vm.ctx.tuple_type())), (needle, None)]
    );
    for element in get_elements(tuple).iter() {
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
    let ref tuple_type = context.tuple_type;
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
    context.set_attr(&tuple_type, "__repr__", context.new_rustfunc(tuple_repr));
    context.set_attr(&tuple_type, "count", context.new_rustfunc(tuple_count));
}

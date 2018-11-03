use super::super::pyobject::{
    AttributeProtocol, PyContext, PyFuncArgs, PyObject, PyObjectKind, PyObjectRef, PyResult,
    TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objbool;
use super::objint;
use super::objsequence::{get_item, seq_equal};
use super::objstr;
use super::objtype;
use num_bigint::ToBigInt;
use num_traits::ToPrimitive;
use std::cell::Ref;
use std::ops::Deref;

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

    let mut x: usize = 0x345678;
    let elements = get_elements(zelf);
    let len: usize = elements.len();
    let mut mult = 0xf4243;

    for elem in elements.iter() {
        let y: usize = objint::get_value(&vm.call_method(elem, "__hash__", vec![])?)
            .to_usize()
            .unwrap();
        x = (x ^ y) * mult;
        mult = mult + 82520 + len * 2;
    }
    x += 97531;

    Ok(vm.ctx.new_int(x.to_bigint().unwrap()))
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
        PyObjectKind::Tuple { elements: elements },
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

pub fn get_elements<'a>(obj: &'a PyObjectRef) -> impl Deref<Target = Vec<PyObjectRef>> + 'a {
    Ref::map(obj.borrow(), |x| {
        if let PyObjectKind::Tuple { ref elements } = x.kind {
            elements
        } else {
            panic!("Cannot extract elements from non-tuple");
        }
    })
}

pub fn init(context: &PyContext) {
    let ref tuple_type = context.tuple_type;
    tuple_type.set_attr("__eq__", context.new_rustfunc(tuple_eq));
    tuple_type.set_attr("__contains__", context.new_rustfunc(tuple_contains));
    tuple_type.set_attr("__getitem__", context.new_rustfunc(tuple_getitem));
    tuple_type.set_attr("__hash__", context.new_rustfunc(tuple_hash));
    tuple_type.set_attr("__len__", context.new_rustfunc(tuple_len));
    tuple_type.set_attr("__new__", context.new_rustfunc(tuple_new));
    tuple_type.set_attr("__repr__", context.new_rustfunc(tuple_repr));
}

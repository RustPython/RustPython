use super::objsequence::PySliceableSequence;
use super::objtype;
use super::pyobject::{
    AttributeProtocol, PyContext, PyFuncArgs, PyObjectKind, PyObjectRef, PyResult, TypeProtocol,
};
use super::vm::VirtualMachine;

pub fn init(context: &PyContext) {
    let ref str_type = context.str_type;
    str_type.set_attr("__new__", context.new_rustfunc(str_new));
    str_type.set_attr("__str__", context.new_rustfunc(str_str));
    str_type.set_attr("__add__", context.new_rustfunc(str_add));
}

fn get_value(obj: PyObjectRef) -> String {
    if let PyObjectKind::String { value } = &obj.borrow().kind {
        value.to_string()
    } else {
        panic!("Inner error getting str");
    }
}

fn str_str(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(s, Some(vm.ctx.str_type()))]);
    Ok(s.clone())
}

fn str_add(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(s, Some(vm.ctx.str_type())), (s2, None)]
    );
    if objtype::isinstance(s2.clone(), vm.ctx.str_type()) {
        Ok(vm
            .ctx
            .new_str(format!("{}{}", get_value(s.clone()), get_value(s2.clone()))))
    } else {
        Err(vm.new_type_error(format!("Cannot add {:?} and {:?}", s, s2)))
    }
}

// TODO: should with following format
// class str(object='')
// class str(object=b'', encoding='utf-8', errors='strict')
fn str_new(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    if args.args.len() == 1 {
        return Ok(vm.new_str("".to_string()));
    }

    if args.args.len() > 2 {
        panic!("str expects exactly one parameter");
    };
    let s = args.args[1].borrow().str();
    Ok(vm.new_str(s))
}

impl PySliceableSequence for String {
    fn do_slice(&self, start: usize, stop: usize) -> Self {
        self[start..stop].to_string()
    }
    fn do_stepped_slice(&self, start: usize, stop: usize, step: usize) -> Self {
        self[start..stop].chars().step_by(step).collect()
    }
    fn len(&self) -> usize {
        self.len()
    }
}

pub fn subscript(vm: &mut VirtualMachine, value: &String, b: PyObjectRef) -> PyResult {
    // let value = a
    match &(*b.borrow()).kind {
        &PyObjectKind::Integer { value: ref pos } => {
            let idx = value.get_pos(*pos);
            Ok(vm.new_str(value[idx..idx + 1].to_string()))
        }
        &PyObjectKind::Slice {
            start: _,
            stop: _,
            step: _,
        } => Ok(vm.new_str(value.get_slice_items(&b))),
        _ => panic!(
            "TypeError: indexing type {:?} with index {:?} is not supported (yet?)",
            value, b
        ),
    }
}

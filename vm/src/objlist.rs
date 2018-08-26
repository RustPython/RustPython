use super::objsequence::PySliceableSequence;
use super::pyobject::{
    AttributeProtocol, PyContext, PyFuncArgs, PyObjectKind, PyObjectRef, PyResult,
};
use super::vm::VirtualMachine;

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

pub fn append(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    trace!("list.append called with: {:?}", args);
    if args.args.len() == 2 {
        let l = args.args[0].clone();
        let o = args.args[1].clone();
        let mut list_obj = l.borrow_mut();
        if let PyObjectKind::List { ref mut elements } = list_obj.kind {
            elements.push(o);
            Ok(vm.get_none())
        } else {
            Err(vm.new_type_error("list.append is called with no list".to_string()))
        }
    } else {
        Err(vm.new_type_error("list.append requires two arguments".to_string()))
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
            Err(vm.new_type_error("list.clear is called with no list".to_string()))
        }
    } else {
        Err(vm.new_type_error("list.clear requires one arguments".to_string()))
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
            Err(vm.new_type_error("list.len is called with no list".to_string()))
        }
    } else {
        Err(vm.new_type_error("list.len requires one arguments".to_string()))
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
            Err(vm.new_type_error("list.reverse is called with no list".to_string()))
        }
    } else {
        Err(vm.new_type_error("list.reverse requires one arguments".to_string()))
    }
}

pub fn init(context: &PyContext) {
    let ref list_type = context.list_type;
    list_type.set_attr("__len__", context.new_rustfunc(len));
    list_type.set_attr("append", context.new_rustfunc(append));
    list_type.set_attr("clear", context.new_rustfunc(clear));
    list_type.set_attr("reverse", context.new_rustfunc(reverse));
}

use super::super::pyobject::{
    PyContext, PyFuncArgs, PyObject, PyObjectRef, PyObjectWeakRef, PyObjectPayload, PyResult, TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objtype; // Required for arg_check! to use isinstance

fn ref_new(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    // TODO: check first argument for subclass of `ref`.
    arg_check!(vm, args, required = [(cls, Some(vm.ctx.type_type())), (referent, None)],
        optional = [(callback, None)]);
    let weak_referent = PyObjectRef::downgrade(referent);
    let weakref = PyObject::new(
        PyObjectPayload::WeakRef { referent: weak_referent, callback: callback.cloned() },
        cls.clone(),
    );
    referent.borrow_mut().add_weakref(&weakref);
    Ok(weakref)
}

/// Dereference the weakref, and check if we still refer something.
fn ref_call(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, Some(vm.ctx.weakref_type()))]);
    let referent = get_value(zelf);
    let py_obj = if let Some(obj) = referent.upgrade() {
        obj
    } else {
        vm.get_none()
    };
    Ok(py_obj)
}

fn get_value(obj: &PyObjectRef) -> PyObjectWeakRef {
    if let PyObjectPayload::WeakRef { referent, .. } = &obj.borrow().payload {
        referent.clone()
    } else {
        panic!("Inner error getting weak ref {:?}", obj);
    }
}

pub fn clear_weak_ref(obj: &PyObjectRef) {
    if let PyObjectPayload::WeakRef { ref mut referent, .. } = &mut obj.borrow_mut().payload {
        referent.clear();
    } else {
        panic!("Inner error getting weak ref {:?}", obj);
    }
}

pub fn notify_weak_ref(vm: &mut VirtualMachine, obj: &PyObjectRef) -> PyResult {
    if let PyObjectPayload::WeakRef { callback, .. } = &obj.borrow().payload {
        if let Some(callback) = callback {
            vm.invoke(callback.clone(), PyFuncArgs::empty())
        } else {
            Ok(vm.get_none())
        }
    } else {
        panic!("Inner error getting weak ref callback {:?}", obj);
    }
}

pub fn init(context: &PyContext) {
    let weakref_type = &context.weakref_type;
    context.set_attr(weakref_type, "__new__", context.new_rustfunc(ref_new));
    context.set_attr(weakref_type, "__call__", context.new_rustfunc(ref_call));
}

//! Implementation in line with the python `weakref` module.
//!
//! See also:
//! - [python weakref module](https://docs.python.org/3/library/weakref.html)
//! - [rust weak struct](https://doc.rust-lang.org/std/rc/struct.Weak.html)
//!

use super::super::obj::objtype;
use super::super::pyobject::{
    PyContext, PyFuncArgs, PyObject, PyObjectPayload, PyObjectRef, PyObjectWeakRef, PyResult,
    TypeProtocol,
};
use super::super::VirtualMachine;
use std::rc::Rc;

pub fn mk_module(ctx: &PyContext) -> PyObjectRef {
    let py_mod = ctx.new_module("_weakref", ctx.new_scope(None));

    let py_ref_class = ctx.new_class("ref", ctx.object());
    ctx.set_attr(&py_ref_class, "__new__", ctx.new_rustfunc(ref_new));
    ctx.set_attr(&py_ref_class, "__call__", ctx.new_rustfunc(ref_call));
    ctx.set_attr(&py_mod, "ref", py_ref_class);
    py_mod
}

fn ref_new(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    // TODO: check first argument for subclass of `ref`.
    arg_check!(vm, args, required = [(cls, None), (referent, None)]);
    let referent = Rc::downgrade(referent);
    Ok(PyObject::new(
        PyObjectPayload::WeakRef { referent },
        cls.clone(),
    ))
}

/// Dereference the weakref, and check if we still refer something.
fn ref_call(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    // TODO: check first argument for subclass of `ref`.
    arg_check!(vm, args, required = [(cls, None)]);
    let referent = get_value(cls);
    let py_obj = if let Some(obj) = referent.upgrade() {
        obj
    } else {
        vm.get_none()
    };
    Ok(py_obj)
}

fn get_value(obj: &PyObjectRef) -> PyObjectWeakRef {
    if let PyObjectPayload::WeakRef { referent } = &obj.borrow().payload {
        referent.clone()
    } else {
        panic!("Inner error getting weak ref {:?}", obj);
    }
}

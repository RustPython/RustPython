//! Implementation in line with the python `weakref` module.
//!
//! See also:
//! - [python weakref module](https://docs.python.org/3/library/weakref.html)
//! - [rust weak struct](https://doc.rust-lang.org/std/rc/struct.Weak.html)
//!

use crate::pyobject::PyObjectRef;
use crate::vm::VirtualMachine;
use rustpython_common::rc::PyRc;

fn weakref_getweakrefcount(obj: PyObjectRef) -> usize {
    PyRc::weak_count(&obj)
}

fn weakref_getweakrefs(_obj: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
    // TODO: implement this, may require a different gc
    vm.ctx.new_list(vec![])
}

fn weakref_remove_dead_weakref(_obj: PyObjectRef, _key: PyObjectRef) {
    // TODO
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    py_module!(vm, "_weakref", {
        "ref" => ctx.weakref_type(),
        "proxy" => ctx.weakproxy_type(),
        "getweakrefcount" => ctx.new_function(weakref_getweakrefcount),
        "getweakrefs" => ctx.new_function(weakref_getweakrefs),
        "ReferenceType" => ctx.weakref_type(),
        "ProxyType" => ctx.weakproxy_type(),
        "CallableProxyType" => ctx.weakproxy_type(),
        "_remove_dead_weakref" => ctx.new_function(weakref_remove_dead_weakref),
    })
}

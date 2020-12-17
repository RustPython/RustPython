//! Implementation in line with the python `weakref` module.
//!
//! See also:
//! - [python weakref module](https://docs.python.org/3/library/weakref.html)
//! - [rust weak struct](https://doc.rust-lang.org/std/rc/struct.Weak.html)
//!

use crate::pyobject::PyObjectRef;
use crate::vm::VirtualMachine;

fn _weakref_getweakrefcount(obj: PyObjectRef) -> usize {
    PyObjectRef::weak_count(&obj)
}

fn _weakref_getweakrefs(_obj: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
    // TODO: implement this, may require a different gc
    vm.ctx.new_list(vec![])
}

fn _weakref_remove_dead_weakref(_obj: PyObjectRef, _key: PyObjectRef) {
    // TODO
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    py_module!(vm, "_weakref", {
        "ref" => ctx.types.weakref_type.clone(),
        "proxy" => ctx.types.weakproxy_type.clone(),
        "getweakrefcount" => named_function!(ctx, _weakref, getweakrefcount),
        "getweakrefs" => named_function!(ctx, _weakref, getweakrefs),
        "ReferenceType" => ctx.types.weakref_type.clone(),
        "ProxyType" => ctx.types.weakproxy_type.clone(),
        "CallableProxyType" => ctx.types.weakproxy_type.clone(),
        "_remove_dead_weakref" => named_function!(ctx, _weakref, remove_dead_weakref),
    })
}

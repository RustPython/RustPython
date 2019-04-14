/// Implementation of the _thread module, currently noop implementation as RustPython doesn't yet
/// support threading
use super::super::pyobject::PyObjectRef;
use crate::function::PyFuncArgs;
use crate::pyobject::PyResult;
use crate::vm::VirtualMachine;

fn rlock_acquire(vm: &VirtualMachine, _args: PyFuncArgs) -> PyResult {
    Ok(vm.get_none())
}

fn rlock_release(_zelf: PyObjectRef, _vm: &VirtualMachine) {}

fn get_ident(_vm: &VirtualMachine) -> u32 {
    1
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    let rlock_type = py_class!(ctx, "_thread.RLock", ctx.object(), {
        "acquire" => ctx.new_rustfunc(rlock_acquire),
        "release" => ctx.new_rustfunc(rlock_release),
    });

    py_module!(vm, "_thread", {
        "RLock" => rlock_type,
        "get_ident" => ctx.new_rustfunc(get_ident)
    })
}

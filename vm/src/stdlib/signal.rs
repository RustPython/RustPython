use crate::obj::objfunction::PyFunctionRef;
use crate::obj::objint::PyIntRef;
use crate::pyobject::PyObjectRef;
use crate::pyobject::PyResult;
use crate::vm::{VirtualMachine, TRIGGERS};

use std::sync::atomic::Ordering;

use num_traits::cast::ToPrimitive;

use nix::sys::signal;

extern "C" fn run_signal(signum: i32) {
    unsafe {
        TRIGGERS[signum as usize].store(true, Ordering::Relaxed);
    }
}

fn signal(signalnum: PyIntRef, handler: PyFunctionRef, vm: &VirtualMachine) -> PyResult<()> {
    vm.signal_handlers.borrow_mut().insert(
        signalnum.as_bigint().to_i32().unwrap(),
        handler.into_object(),
    );
    let handler = nix::sys::signal::SigHandler::Handler(run_signal);
    let sig_action =
        signal::SigAction::new(handler, signal::SaFlags::empty(), signal::SigSet::empty());
    unsafe { signal::sigaction(signal::SIGINT, &sig_action) }.unwrap();
    Ok(())
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    py_module!(vm, "_signal", {
        "signal" => ctx.new_rustfunc(signal),
    })
}

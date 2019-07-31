use crate::obj::objint::PyIntRef;
use crate::pyobject::{IdProtocol, PyObjectRef, PyResult};
use crate::vm::VirtualMachine;

use std::sync::atomic::{AtomicBool, Ordering};

use num_traits::cast::ToPrimitive;

use nix::sys::signal;
use nix::unistd::alarm as sig_alarm;

// Signal triggers
// TODO: 64
const NSIG: usize = 15;

static mut TRIGGERS: [AtomicBool; NSIG] = [
    AtomicBool::new(false),
    AtomicBool::new(false),
    AtomicBool::new(false),
    AtomicBool::new(false),
    AtomicBool::new(false),
    AtomicBool::new(false),
    AtomicBool::new(false),
    AtomicBool::new(false),
    AtomicBool::new(false),
    AtomicBool::new(false),
    AtomicBool::new(false),
    AtomicBool::new(false),
    AtomicBool::new(false),
    AtomicBool::new(false),
    AtomicBool::new(false),
];

extern "C" fn run_signal(signum: i32) {
    unsafe {
        TRIGGERS[signum as usize].store(true, Ordering::Relaxed);
    }
}

fn signal(
    signalnum: PyIntRef,
    handler: PyObjectRef,
    vm: &VirtualMachine,
) -> PyResult<Option<PyObjectRef>> {
    if !vm.isinstance(&handler, &vm.ctx.function_type())?
        && !vm.isinstance(&handler, &vm.ctx.bound_method_type())?
        && !vm.isinstance(&handler, &vm.ctx.builtin_function_or_method_type())?
    {
        return Err(vm.new_type_error("Hanlder must be callable".to_string()));
    }
    let signal = vm.import("signal", &vm.ctx.new_tuple(vec![]), 0)?;
    let sig_dfl = vm.get_attribute(signal.clone(), "SIG_DFL")?;
    let sig_ign = vm.get_attribute(signal, "SIG_DFL")?;
    let signalnum = signalnum.as_bigint().to_i32().unwrap();
    let signal_enum = signal::Signal::from_c_int(signalnum).unwrap();
    let sig_handler = if handler.is(&sig_dfl) {
        signal::SigHandler::SigDfl
    } else if handler.is(&sig_ign) {
        signal::SigHandler::SigIgn
    } else {
        signal::SigHandler::Handler(run_signal)
    };
    let sig_action = signal::SigAction::new(
        sig_handler,
        signal::SaFlags::empty(),
        signal::SigSet::empty(),
    );
    check_signals(vm);
    unsafe { signal::sigaction(signal_enum, &sig_action) }.unwrap();
    let old_handler = vm.signal_handlers.borrow_mut().insert(signalnum, handler);
    Ok(old_handler)
}

fn getsignal(signalnum: PyIntRef, vm: &VirtualMachine) -> PyResult<Option<PyObjectRef>> {
    let signalnum = signalnum.as_bigint().to_i32().unwrap();
    Ok(vm
        .signal_handlers
        .borrow_mut()
        .get(&signalnum)
        .map(|x| x.clone()))
}

fn alarm(time: PyIntRef, _vm: &VirtualMachine) -> u32 {
    let time = time.as_bigint().to_u32().unwrap();
    let prev_time = if time == 0 {
        sig_alarm::cancel()
    } else {
        sig_alarm::set(time)
    };
    prev_time.unwrap_or(0)
}

pub fn check_signals(vm: &VirtualMachine) {
    for signum in 1..NSIG {
        let triggerd = unsafe { TRIGGERS[signum].swap(false, Ordering::Relaxed) };
        if triggerd {
            let handler = vm
                .signal_handlers
                .borrow()
                .get(&(signum as i32))
                .expect("Handler should be set")
                .clone();
            vm.invoke(handler, vec![vm.new_int(signum), vm.get_none()])
                .expect("Test");
        }
    }
}

fn stub_func(_vm: &VirtualMachine) -> PyResult {
    panic!("Do not use directly");
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    let sig_dfl = ctx.new_rustfunc(stub_func);
    let sig_ign = ctx.new_rustfunc(stub_func);

    py_module!(vm, "signal", {
        "signal" => ctx.new_rustfunc(signal),
        "getsignal" => ctx.new_rustfunc(getsignal),
        "alarm" => ctx.new_rustfunc(alarm),
        "SIG_DFL" => sig_dfl,
        "SIG_IGN" => sig_ign,
    })
}

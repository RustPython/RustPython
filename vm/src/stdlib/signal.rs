use crate::obj::objint::PyIntRef;
use crate::pyobject::{IdProtocol, PyObjectRef, PyResult};
use crate::vm::VirtualMachine;

use std::sync::atomic::{AtomicBool, Ordering};

use num_traits::cast::ToPrimitive;

use arr_macro::arr;

#[cfg(unix)]
use nix::sys::signal;
#[cfg(unix)]
use nix::unistd::alarm as sig_alarm;

const NSIG: usize = 64;

// We cannot use the NSIG const in the arr macro. This will fail compilation if NSIG is different.
static mut TRIGGERS: [AtomicBool; NSIG] = arr![AtomicBool::new(false); 64];

extern "C" fn run_signal(signum: i32) {
    unsafe {
        TRIGGERS[signum as usize].store(true, Ordering::Relaxed);
    }
}

#[derive(Debug)]
enum SigMode {
    Ign,
    Dfl,
    Handler,
}

#[cfg(unix)]
fn os_set_signal(signalnum: i32, mode: SigMode, _vm: &VirtualMachine) {
    let signal_enum = signal::Signal::from_c_int(signalnum).unwrap();
    let sig_handler = match mode {
        SigMode::Dfl => signal::SigHandler::SigDfl,
        SigMode::Ign => signal::SigHandler::SigIgn,
        SigMode::Handler => signal::SigHandler::Handler(run_signal),
    };
    let sig_action = signal::SigAction::new(
        sig_handler,
        signal::SaFlags::empty(),
        signal::SigSet::empty(),
    );
    unsafe { signal::sigaction(signal_enum, &sig_action) }.unwrap();
}

#[cfg(not(unix))]
fn os_set_signal(_signalnum: i32, _mode: SigMode, _vm: &VirtualMachine) {
    panic!("Not implemented");
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
    let sig_ign = vm.get_attribute(signal, "SIG_IGN")?;
    let signalnum = signalnum.as_bigint().to_i32().unwrap();
    check_signals(vm);
    let mode = if handler.is(&sig_dfl) {
        SigMode::Dfl
    } else if handler.is(&sig_ign) {
        SigMode::Ign
    } else {
        SigMode::Handler
    };
    os_set_signal(signalnum, mode, vm);
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

#[cfg(unix)]
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

    let module = py_module!(vm, "signal", {
        "signal" => ctx.new_rustfunc(signal),
        "getsignal" => ctx.new_rustfunc(getsignal),
        "SIG_DFL" => sig_dfl,
        "SIG_IGN" => sig_ign,
    });
    extend_module_platform_specific(vm, module)
}

#[cfg(unix)]
fn extend_module_platform_specific(vm: &VirtualMachine, module: PyObjectRef) -> PyObjectRef {
    let ctx = &vm.ctx;

    extend_module!(vm, module, {
        "alarm" => ctx.new_rustfunc(alarm),
        "SIGHUP" => ctx.new_int(signal::Signal::SIGHUP as u8),
        "SIGINT" => ctx.new_int(signal::Signal::SIGINT as u8),
        "SIGQUIT" => ctx.new_int(signal::Signal::SIGQUIT as u8),
        "SIGILL" => ctx.new_int(signal::Signal::SIGILL as u8),
        "SIGTRAP" => ctx.new_int(signal::Signal::SIGTRAP as u8),
        "SIGABRT" => ctx.new_int(signal::Signal::SIGABRT as u8),
        "SIGBUS" => ctx.new_int(signal::Signal::SIGBUS as u8),
        "SIGFPE" => ctx.new_int(signal::Signal::SIGFPE as u8),
        "SIGKILL" => ctx.new_int(signal::Signal::SIGKILL as u8),
        "SIGUSR1" => ctx.new_int(signal::Signal::SIGUSR1 as u8),
        "SIGSEGV" => ctx.new_int(signal::Signal::SIGSEGV as u8),
        "SIGUSR2" => ctx.new_int(signal::Signal::SIGUSR2 as u8),
        "SIGPIPE" => ctx.new_int(signal::Signal::SIGPIPE as u8),
        "SIGALRM" => ctx.new_int(signal::Signal::SIGALRM as u8),
        "SIGTERM" => ctx.new_int(signal::Signal::SIGTERM as u8),
        "SIGSTKFLT" => ctx.new_int(signal::Signal::SIGSTKFLT as u8),
        "SIGCHLD" => ctx.new_int(signal::Signal::SIGCHLD as u8),
        "SIGCONT" => ctx.new_int(signal::Signal::SIGCONT as u8),
        "SIGSTOP" => ctx.new_int(signal::Signal::SIGSTOP as u8),
        "SIGTSTP" => ctx.new_int(signal::Signal::SIGTSTP as u8),
        "SIGTTIN" => ctx.new_int(signal::Signal::SIGTTIN as u8),
        "SIGTTOU" => ctx.new_int(signal::Signal::SIGTTOU as u8),
        "SIGURG" => ctx.new_int(signal::Signal::SIGURG as u8),
        "SIGXCPU" => ctx.new_int(signal::Signal::SIGXCPU as u8),
        "SIGXFSZ" => ctx.new_int(signal::Signal::SIGXFSZ as u8),
        "SIGVTALRM" => ctx.new_int(signal::Signal::SIGVTALRM as u8),
        "SIGPROF" => ctx.new_int(signal::Signal::SIGPROF as u8),
        "SIGWINCH" => ctx.new_int(signal::Signal::SIGWINCH as u8),
        "SIGIO" => ctx.new_int(signal::Signal::SIGIO as u8),
        "SIGPWR" => ctx.new_int(signal::Signal::SIGPWR as u8),
        "SIGSYS" => ctx.new_int(signal::Signal::SIGSYS as u8),
    });

    module
}

#[cfg(not(unix))]
fn extend_module_platform_specific(_vm: &VirtualMachine, module: PyObjectRef) -> PyObjectRef {
    module
}

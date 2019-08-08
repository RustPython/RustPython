use crate::obj::objint::PyIntRef;
use crate::pyobject::{IdProtocol, PyObjectRef, PyResult};
use crate::vm::VirtualMachine;

use std::sync::atomic::{AtomicBool, Ordering};

use num_traits::cast::ToPrimitive;

use arr_macro::arr;

#[cfg(unix)]
use nix::unistd::alarm as sig_alarm;

use libc;

#[cfg(not(windows))]
use libc::{SIG_DFL, SIG_ERR, SIG_IGN};

#[cfg(windows)]
const SIG_DFL: libc::sighandler_t = 0;
#[cfg(windows)]
const SIG_IGN: libc::sighandler_t = 1;
#[cfg(windows)]
const SIG_ERR: libc::sighandler_t = !0;

const NSIG: usize = 64;

// We cannot use the NSIG const in the arr macro. This will fail compilation if NSIG is different.
static mut TRIGGERS: [AtomicBool; NSIG] = arr![AtomicBool::new(false); 64];

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
    let signal_module = vm.import("signal", &vm.ctx.new_tuple(vec![]), 0)?;
    let sig_dfl = vm.get_attribute(signal_module.clone(), "SIG_DFL")?;
    let sig_ign = vm.get_attribute(signal_module, "SIG_IGN")?;
    let signalnum = signalnum.as_bigint().to_i32().unwrap();
    check_signals(vm);
    let sig_handler = if handler.is(&sig_dfl) {
        SIG_DFL
    } else if handler.is(&sig_ign) {
        SIG_IGN
    } else {
        run_signal as libc::sighandler_t
    };
    let old = unsafe { libc::signal(signalnum, sig_handler) };
    if old == SIG_ERR {
        return Err(vm.new_os_error("Failed to set signal".to_string()));
    }
    let old_handler = vm.signal_handlers.borrow_mut().insert(signalnum, handler);
    Ok(old_handler)
}

fn getsignal(signalnum: PyIntRef, vm: &VirtualMachine) -> PyResult<Option<PyObjectRef>> {
    let signalnum = signalnum.as_bigint().to_i32().unwrap();
    Ok(vm.signal_handlers.borrow_mut().get(&signalnum).cloned())
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

#[allow(clippy::needless_range_loop)]
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
        "SIGABRT" => ctx.new_int(libc::SIGABRT as u8),
        "SIGFPE" => ctx.new_int(libc::SIGFPE as u8),
        "SIGILL" => ctx.new_int(libc::SIGILL as u8),
        "SIGINT" => ctx.new_int(libc::SIGINT as u8),
        "SIGSEGV" => ctx.new_int(libc::SIGSEGV as u8),
        "SIGTERM" => ctx.new_int(libc::SIGTERM as u8),
    });
    extend_module_platform_specific(vm, module)
}

#[cfg(unix)]
fn extend_module_platform_specific(vm: &VirtualMachine, module: PyObjectRef) -> PyObjectRef {
    let ctx = &vm.ctx;

    extend_module!(vm, module, {
        "alarm" => ctx.new_rustfunc(alarm),
        "SIGHUP" => ctx.new_int(libc::SIGHUP as u8),
        "SIGQUIT" => ctx.new_int(libc::SIGQUIT as u8),
        "SIGTRAP" => ctx.new_int(libc::SIGTRAP as u8),
        "SIGBUS" => ctx.new_int(libc::SIGBUS as u8),
        "SIGKILL" => ctx.new_int(libc::SIGKILL as u8),
        "SIGUSR1" => ctx.new_int(libc::SIGUSR1 as u8),
        "SIGUSR2" => ctx.new_int(libc::SIGUSR2 as u8),
        "SIGPIPE" => ctx.new_int(libc::SIGPIPE as u8),
        "SIGALRM" => ctx.new_int(libc::SIGALRM as u8),
        "SIGCHLD" => ctx.new_int(libc::SIGCHLD as u8),
        "SIGCONT" => ctx.new_int(libc::SIGCONT as u8),
        "SIGSTOP" => ctx.new_int(libc::SIGSTOP as u8),
        "SIGTSTP" => ctx.new_int(libc::SIGTSTP as u8),
        "SIGTTIN" => ctx.new_int(libc::SIGTTIN as u8),
        "SIGTTOU" => ctx.new_int(libc::SIGTTOU as u8),
        "SIGURG" => ctx.new_int(libc::SIGURG as u8),
        "SIGXCPU" => ctx.new_int(libc::SIGXCPU as u8),
        "SIGXFSZ" => ctx.new_int(libc::SIGXFSZ as u8),
        "SIGVTALRM" => ctx.new_int(libc::SIGVTALRM as u8),
        "SIGPROF" => ctx.new_int(libc::SIGPROF as u8),
        "SIGWINCH" => ctx.new_int(libc::SIGWINCH as u8),
        "SIGIO" => ctx.new_int(libc::SIGIO as u8),
        "SIGSYS" => ctx.new_int(libc::SIGSYS as u8),
    });

    #[cfg(not(target_os = "macos"))]
    {
        extend_module!(vm, module, {
            "SIGPWR" => ctx.new_int(libc::SIGPWR as u8),
            "SIGSTKFLT" => ctx.new_int(libc::SIGSTKFLT as u8),
        });
    }

    module
}

#[cfg(not(unix))]
fn extend_module_platform_specific(_vm: &VirtualMachine, module: PyObjectRef) -> PyObjectRef {
    module
}

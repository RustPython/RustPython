use crate::pyobject::{PyObjectRef, PyResult, TryFromObject};
use crate::vm::{VirtualMachine, NSIG};

use std::sync::atomic::{AtomicBool, Ordering};

use arr_macro::arr;

#[cfg(unix)]
use nix::unistd::alarm as sig_alarm;

#[cfg(not(windows))]
use libc::{SIG_DFL, SIG_ERR, SIG_IGN};

#[cfg(windows)]
const SIG_DFL: libc::sighandler_t = 0;
#[cfg(windows)]
const SIG_IGN: libc::sighandler_t = 1;
#[cfg(windows)]
const SIG_ERR: libc::sighandler_t = !0;

// We cannot use the NSIG const in the arr macro. This will fail compilation if NSIG is different.
static TRIGGERS: [AtomicBool; NSIG] = arr![AtomicBool::new(false); 64];

static ANY_TRIGGERED: AtomicBool = AtomicBool::new(false);

extern "C" fn run_signal(signum: i32) {
    ANY_TRIGGERED.store(true, Ordering::Relaxed);
    TRIGGERS[signum as usize].store(true, Ordering::Relaxed);
}

fn assert_in_range(signum: i32, vm: &VirtualMachine) -> PyResult<()> {
    if (1..NSIG as i32).contains(&signum) {
        Ok(())
    } else {
        Err(vm.new_value_error("signal number out of range".to_owned()))
    }
}

fn signal(signalnum: i32, handler: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    assert_in_range(signalnum, vm)?;
    let signal_handlers = vm
        .signal_handlers
        .as_ref()
        .ok_or_else(|| vm.new_value_error("signal only works in main thread".to_owned()))?;

    let sig_handler = match usize::try_from_object(vm, handler.clone()).ok() {
        Some(SIG_DFL) => SIG_DFL,
        Some(SIG_IGN) => SIG_IGN,
        None if vm.is_callable(&handler) => run_signal as libc::sighandler_t,
        _ => {
            return Err(vm.new_type_error(
                "signal handler must be signal.SIG_IGN, signal.SIG_DFL, or a callable object"
                    .to_owned(),
            ))
        }
    };
    check_signals(vm)?;

    let old = unsafe { libc::signal(signalnum, sig_handler) };
    if old == SIG_ERR {
        return Err(vm.new_os_error("Failed to set signal".to_owned()));
    }
    #[cfg(all(unix, not(target_os = "redox")))]
    {
        extern "C" {
            fn siginterrupt(sig: i32, flag: i32);
        }
        unsafe {
            siginterrupt(signalnum, 1);
        }
    }

    let mut old_handler = handler;
    std::mem::swap(
        &mut signal_handlers.borrow_mut()[signalnum as usize],
        &mut old_handler,
    );
    Ok(old_handler)
}

fn getsignal(signalnum: i32, vm: &VirtualMachine) -> PyResult {
    assert_in_range(signalnum, vm)?;
    let signal_handlers = vm
        .signal_handlers
        .as_ref()
        .ok_or_else(|| vm.new_value_error("getsignal only works in main thread".to_owned()))?;
    Ok(signal_handlers.borrow()[signalnum as usize].clone())
}

#[cfg(unix)]
fn alarm(time: u32) -> u32 {
    let prev_time = if time == 0 {
        sig_alarm::cancel()
    } else {
        sig_alarm::set(time)
    };
    prev_time.unwrap_or(0)
}

#[cfg_attr(feature = "flame-it", flame)]
pub fn check_signals(vm: &VirtualMachine) -> PyResult<()> {
    let signal_handlers = match vm.signal_handlers {
        Some(ref h) => h.borrow(),
        None => return Ok(()),
    };

    if !ANY_TRIGGERED.swap(false, Ordering::Relaxed) {
        return Ok(());
    }
    for (signum, trigger) in TRIGGERS.iter().enumerate().skip(1) {
        let triggerd = trigger.swap(false, Ordering::Relaxed);
        if triggerd {
            let handler = &signal_handlers[signum];
            if vm.is_callable(handler) {
                vm.invoke(handler, vec![vm.ctx.new_int(signum), vm.get_none()])?;
            }
        }
    }
    Ok(())
}

fn default_int_handler(_signum: PyObjectRef, _arg: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    Err(vm.new_exception_empty(vm.ctx.exceptions.keyboard_interrupt.clone()))
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    let int_handler = ctx.new_function(default_int_handler);

    let sig_dfl = ctx.new_int(SIG_DFL as u8);
    let sig_ign = ctx.new_int(SIG_IGN as u8);

    let module = py_module!(vm, "signal", {
        "signal" => ctx.new_function(signal),
        "getsignal" => ctx.new_function(getsignal),
        "SIG_DFL" => sig_dfl.clone(),
        "SIG_IGN" => sig_ign.clone(),
        "SIGABRT" => ctx.new_int(libc::SIGABRT as u8),
        "SIGFPE" => ctx.new_int(libc::SIGFPE as u8),
        "SIGILL" => ctx.new_int(libc::SIGILL as u8),
        "SIGINT" => ctx.new_int(libc::SIGINT as u8),
        "SIGSEGV" => ctx.new_int(libc::SIGSEGV as u8),
        "SIGTERM" => ctx.new_int(libc::SIGTERM as u8),
        "default_int_handler" => int_handler.clone(),
    });
    extend_module_platform_specific(vm, &module);

    for signum in 1..NSIG {
        let handler = unsafe { libc::signal(signum as i32, SIG_IGN) };
        if handler != SIG_ERR {
            unsafe { libc::signal(signum as i32, handler) };
        }
        let py_handler = if handler == SIG_DFL {
            sig_dfl.clone()
        } else if handler == SIG_IGN {
            sig_ign.clone()
        } else {
            vm.get_none()
        };
        vm.signal_handlers.as_ref().unwrap().borrow_mut()[signum] = py_handler;
    }

    signal(libc::SIGINT, int_handler, vm).expect("Failed to set sigint handler");

    module
}

#[cfg(unix)]
fn extend_module_platform_specific(vm: &VirtualMachine, module: &PyObjectRef) {
    let ctx = &vm.ctx;

    extend_module!(vm, module, {
        "alarm" => ctx.new_function(alarm),
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

    #[cfg(not(any(target_os = "macos", target_os = "openbsd")))]
    {
        extend_module!(vm, module, {
            "SIGPWR" => ctx.new_int(libc::SIGPWR as u8),
            "SIGSTKFLT" => ctx.new_int(libc::SIGSTKFLT as u8),
        });
    }
}

#[cfg(not(unix))]
fn extend_module_platform_specific(_vm: &VirtualMachine, _module: &PyObjectRef) {}

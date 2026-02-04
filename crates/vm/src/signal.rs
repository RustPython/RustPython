#![cfg_attr(target_os = "wasi", allow(dead_code))]
use crate::{PyResult, VirtualMachine};
use alloc::fmt;
#[cfg(windows)]
use core::sync::atomic::AtomicIsize;
use core::sync::atomic::{AtomicBool, Ordering};
use std::cell::Cell;
use std::sync::mpsc;

pub(crate) const NSIG: usize = 64;
static ANY_TRIGGERED: AtomicBool = AtomicBool::new(false);
// hack to get around const array repeat expressions, rust issue #79270
#[allow(clippy::declare_interior_mutable_const)]
const ATOMIC_FALSE: AtomicBool = AtomicBool::new(false);
pub(crate) static TRIGGERS: [AtomicBool; NSIG] = [ATOMIC_FALSE; NSIG];

#[cfg(windows)]
static SIGINT_EVENT: AtomicIsize = AtomicIsize::new(0);

thread_local! {
    /// Prevent recursive signal handler invocation. When a Python signal
    /// handler is running, new signals are deferred until it completes.
    static IN_SIGNAL_HANDLER: Cell<bool> = const { Cell::new(false) };
}

struct SignalHandlerGuard;

impl Drop for SignalHandlerGuard {
    fn drop(&mut self) {
        IN_SIGNAL_HANDLER.with(|h| h.set(false));
    }
}

// Reactivate EBR guard every N instructions to prevent epoch starvation.
// This allows GC to advance epochs even during long-running operations.
// 65536 instructions ≈ 1ms, much faster than CPython's 5ms GIL timeout
#[cfg(all(feature = "threading", feature = "gc"))]
const REACTIVATE_INTERVAL: u32 = 65536;

#[cfg(all(feature = "threading", feature = "gc"))]
thread_local! {
    static INSTRUCTION_COUNTER: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

#[cfg_attr(feature = "flame-it", flame)]
#[inline(always)]
pub fn check_signals(vm: &VirtualMachine) -> PyResult<()> {
    // Periodic EBR guard reactivation to prevent epoch starvation
    #[cfg(all(feature = "threading", feature = "gc"))]
    {
        INSTRUCTION_COUNTER.with(|counter| {
            let count = counter.get();
            if count >= REACTIVATE_INTERVAL {
                crate::vm::thread::reactivate_guard();
                counter.set(0);
            } else {
                counter.set(count + 1);
            }
        });
    }

    if vm.signal_handlers.is_none() {
        return Ok(());
    }

    if !ANY_TRIGGERED.swap(false, Ordering::Acquire) {
        return Ok(());
    }

    trigger_signals(vm)
}

#[inline(never)]
#[cold]
fn trigger_signals(vm: &VirtualMachine) -> PyResult<()> {
    if IN_SIGNAL_HANDLER.with(|h| h.replace(true)) {
        // Already inside a signal handler — defer pending signals
        set_triggered();
        return Ok(());
    }
    let _guard = SignalHandlerGuard;

    // unwrap should never fail since we check above
    let signal_handlers = vm.signal_handlers.as_ref().unwrap().borrow();
    for (signum, trigger) in TRIGGERS.iter().enumerate().skip(1) {
        let triggered = trigger.swap(false, Ordering::Relaxed);
        if triggered
            && let Some(handler) = &signal_handlers[signum]
            && let Some(callable) = handler.to_callable()
        {
            callable.invoke((signum, vm.ctx.none()), vm)?;
        }
    }
    if let Some(signal_rx) = &vm.signal_rx {
        for f in signal_rx.rx.try_iter() {
            f(vm)?;
        }
    }
    Ok(())
}

pub(crate) fn set_triggered() {
    ANY_TRIGGERED.store(true, Ordering::Release);
}

/// Reset all signal trigger state after fork in child process.
/// Stale triggers from the parent must not fire in the child.
#[cfg(unix)]
pub(crate) fn clear_after_fork() {
    ANY_TRIGGERED.store(false, Ordering::Release);
    for trigger in &TRIGGERS {
        trigger.store(false, Ordering::Relaxed);
    }
}

pub fn assert_in_range(signum: i32, vm: &VirtualMachine) -> PyResult<()> {
    if (1..NSIG as i32).contains(&signum) {
        Ok(())
    } else {
        Err(vm.new_value_error("signal number out of range"))
    }
}

/// Similar to `PyErr_SetInterruptEx` in CPython
///
/// Missing signal handler for the given signal number is silently ignored.
#[allow(dead_code)]
#[cfg(not(target_arch = "wasm32"))]
pub fn set_interrupt_ex(signum: i32, vm: &VirtualMachine) -> PyResult<()> {
    use crate::stdlib::signal::_signal::{SIG_DFL, SIG_IGN, run_signal};
    assert_in_range(signum, vm)?;

    match signum as usize {
        SIG_DFL | SIG_IGN => Ok(()),
        _ => {
            // interrupt the main thread with given signal number
            run_signal(signum);
            Ok(())
        }
    }
}

pub type UserSignal = Box<dyn FnOnce(&VirtualMachine) -> PyResult<()> + Send>;

#[derive(Clone, Debug)]
pub struct UserSignalSender {
    tx: mpsc::Sender<UserSignal>,
}

#[derive(Debug)]
pub struct UserSignalReceiver {
    rx: mpsc::Receiver<UserSignal>,
}

impl UserSignalSender {
    pub fn send(&self, sig: UserSignal) -> Result<(), UserSignalSendError> {
        self.tx
            .send(sig)
            .map_err(|mpsc::SendError(sig)| UserSignalSendError(sig))?;
        set_triggered();
        Ok(())
    }
}

pub struct UserSignalSendError(pub UserSignal);

impl fmt::Debug for UserSignalSendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UserSignalSendError")
            .finish_non_exhaustive()
    }
}

impl fmt::Display for UserSignalSendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("sending a signal to a exited vm")
    }
}

pub fn user_signal_channel() -> (UserSignalSender, UserSignalReceiver) {
    let (tx, rx) = mpsc::channel();
    (UserSignalSender { tx }, UserSignalReceiver { rx })
}

#[cfg(windows)]
pub fn set_sigint_event(handle: isize) {
    SIGINT_EVENT.store(handle, Ordering::Release);
}

#[cfg(windows)]
pub fn get_sigint_event() -> Option<isize> {
    let handle = SIGINT_EVENT.load(Ordering::Acquire);
    if handle == 0 { None } else { Some(handle) }
}

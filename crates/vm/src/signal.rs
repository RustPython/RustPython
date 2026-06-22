use core::{
    cell::{Cell, RefCell},
    fmt,
    ops::{Deref, DerefMut, Index, IndexMut, Range},
    sync::atomic::{AtomicBool, Ordering},
};
use std::sync::mpsc;

#[cfg(windows)]
use core::sync::atomic::AtomicIsize;

use crate::{PyObjectRef, PyResult, TryFromBorrowedObject, TryFromObject, VirtualMachine};

pub(crate) const NSIG: usize = 64;

static ANY_TRIGGERED: AtomicBool = AtomicBool::new(false);

#[expect(
    clippy::declare_interior_mutable_const,
    reason = "workaround for const array repeat limitation (rust issue #79270)"
)]
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

#[cfg_attr(feature = "flame-it", flame)]
#[inline(always)]
pub fn check_signals(vm: &VirtualMachine) -> PyResult<()> {
    if vm.signal_handlers.get().is_none() {
        return Ok(());
    }

    // Read-only check first: avoids cache-line invalidation on every
    // instruction when no signal is pending (the common case).
    if !ANY_TRIGGERED.load(Ordering::Relaxed) {
        return Ok(());
    }

    // Atomic RMW only when a signal is actually pending.
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

    let signal_handlers = vm
        .signal_handlers
        .get()
        .expect("should never fail since we check above")
        .borrow();

    for (signum, trigger) in TRIGGERS.iter().enumerate().skip(1) {
        let triggered = trigger.swap(false, Ordering::Relaxed);

        // SAFETY: TRIGGERS has the same length as the signal_handlers
        let signum = unsafe { SignalNum::new_unchecked(signum as i32) };

        if triggered
            && let Some(handler) = &signal_handlers[signum]
            && let Some(callable) = handler.to_callable()
        {
            callable.invoke((signum.as_i32(), vm.ctx.none()), vm)?;
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

#[inline(always)]
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn is_triggered() -> bool {
    ANY_TRIGGERED.load(Ordering::Relaxed)
}

/// Reset all signal trigger state after fork in child process.
/// Stale triggers from the parent must not fire in the child.
#[cfg(all(unix, feature = "host_env"))]
pub(crate) fn clear_after_fork() {
    ANY_TRIGGERED.store(false, Ordering::Release);
    for trigger in &TRIGGERS {
        trigger.store(false, Ordering::Relaxed);
    }
}

/// A valid signal number.
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub struct SignalNum(i32);

impl SignalNum {
    pub(crate) const VALID_RANGE: Range<i32> = 1..NSIG as i32;

    /// Alias for:
    /// ```rust
    /// # use rustpython_vm::signal::SignalNum;
    ///
    /// unsafe { SignalNum::new_unchecked(libc::SIGINT) };
    /// ```
    #[cfg(any(unix, windows))]
    #[allow(dead_code, reason = "Not used on all platforms")]
    pub(crate) const SIGINT: Self = Self(libc::SIGINT);

    /// Construct [`Self`] without any validation on the signalnum value.
    ///
    /// # Safety
    ///
    /// Caller's responsibility to ensure the signal num is valid.
    #[must_use]
    pub const unsafe fn new_unchecked(value: i32) -> Self {
        Self(value)
    }

    /// Get the self as an [`i32`].
    #[must_use]
    pub const fn as_i32(&self) -> i32 {
        self.0
    }

    /// Get the self as an [`usize`].
    #[must_use]
    pub const fn as_usize(&self) -> usize {
        self.0 as usize
    }
}

impl fmt::Display for SignalNum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl From<SignalNum> for i32 {
    fn from(signalnum: SignalNum) -> Self {
        signalnum.as_i32()
    }
}

impl TryFrom<i32> for SignalNum {
    type Error = String;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        let bounds = cfg_select! {
            all(windows, feature = "host_env") => rustpython_host_env::signal::VALID_SIGNALS,
            _ => Self::VALID_RANGE,
        };

        if bounds.contains(&value) {
            Ok(Self(value))
        } else {
            Err("signal number out of range".into())
        }
    }
}

impl TryFromObject for SignalNum {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        Self::try_from(i32::try_from_borrowed_object(vm, &obj)?)
            .map_err(|msg| vm.new_value_error(msg))
    }
}

/// Similar to `PyErr_SetInterruptEx` in CPython
///
/// Missing signal handler for the given signal number is silently ignored.
#[cfg(all(not(target_arch = "wasm32"), feature = "host_env"))]
pub fn set_interrupt_ex(signum: SignalNum) -> PyResult<()> {
    use crate::stdlib::_signal::_signal::{SIG_DFL, SIG_IGN, run_signal};

    match signum.as_usize() {
        SIG_DFL | SIG_IGN => Ok(()),
        _ => {
            // interrupt the main thread with given signal number
            run_signal(signum.into());
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

#[must_use]
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

pub struct SignalHandlersInner([Option<PyObjectRef>; NSIG]);

impl Default for SignalHandlersInner {
    fn default() -> Self {
        Self([const { None }; NSIG])
    }
}

impl Index<SignalNum> for SignalHandlersInner {
    type Output = Option<PyObjectRef>;

    fn index(&self, index: SignalNum) -> &Self::Output {
        &self.0[index.as_usize()]
    }
}

impl IndexMut<SignalNum> for SignalHandlersInner {
    fn index_mut(&mut self, index: SignalNum) -> &mut Self::Output {
        &mut self.0[index.as_usize()]
    }
}

pub struct SignalHandlers(Box<RefCell<SignalHandlersInner>>);

impl Default for SignalHandlers {
    fn default() -> Self {
        Self(Box::new(RefCell::new(SignalHandlersInner::default())))
    }
}

impl Deref for SignalHandlers {
    type Target = Box<RefCell<SignalHandlersInner>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for SignalHandlers {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

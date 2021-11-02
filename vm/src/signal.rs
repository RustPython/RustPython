#![cfg_attr(target_os = "wasi", allow(dead_code))]
use crate::{PyObjectRef, PyResult, VirtualMachine};
use std::sync::atomic::{AtomicBool, Ordering};

pub const NSIG: usize = 64;
static ANY_TRIGGERED: AtomicBool = AtomicBool::new(false);
// hack to get around const array repeat expressions, rust issue #79270
#[allow(clippy::declare_interior_mutable_const)]
const ATOMIC_FALSE: AtomicBool = AtomicBool::new(false);
pub(crate) static TRIGGERS: [AtomicBool; NSIG] = [ATOMIC_FALSE; NSIG];

#[cfg_attr(feature = "flame-it", flame)]
#[inline(always)]
pub fn check_signals(vm: &VirtualMachine) -> PyResult<()> {
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
    // unwrap should never fail since we check above
    let signal_handlers = vm.signal_handlers.as_ref().unwrap().borrow();
    for (signum, trigger) in TRIGGERS.iter().enumerate().skip(1) {
        let triggered = trigger.swap(false, Ordering::Relaxed);
        if triggered {
            if let Some(handler) = &signal_handlers[signum] {
                if vm.is_callable(handler) {
                    vm.invoke(handler, (signum, vm.ctx.none()))?;
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn set_triggered() {
    ANY_TRIGGERED.store(true, Ordering::Release);
}

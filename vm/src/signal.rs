use crate::{PyObjectRef, PyResult, VirtualMachine};
use std::sync::atomic::{AtomicBool, Ordering};

pub const NSIG: usize = 64;
pub(crate) static ANY_TRIGGERED: AtomicBool = AtomicBool::new(false);
// hack to get around const array repeat expressions, rust issue #79270
#[allow(clippy::declare_interior_mutable_const)]
const ATOMIC_FALSE: AtomicBool = AtomicBool::new(false);
pub(crate) static TRIGGERS: [AtomicBool; NSIG] = [ATOMIC_FALSE; NSIG];

#[cfg_attr(feature = "flame-it", flame)]
#[inline(always)]
pub fn check_signals(vm: &VirtualMachine) -> PyResult<()> {
    let signal_handlers = match &vm.signal_handlers {
        Some(h) => h,
        None => return Ok(()),
    };

    if !ANY_TRIGGERED.load(Ordering::Relaxed) {
        return Ok(());
    }
    ANY_TRIGGERED.store(false, Ordering::Relaxed);

    trigger_signals(&signal_handlers.borrow(), vm)
}
#[inline(never)]
#[cold]
fn trigger_signals(
    signal_handlers: &[Option<PyObjectRef>; NSIG],
    vm: &VirtualMachine,
) -> PyResult<()> {
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

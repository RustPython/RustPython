#![cfg(feature = "threading")]

// Keep this process-global signal-breaker regression in its own test binary:
// VM unit tests can clear or consume the breaker concurrently.
use rustpython_vm::Interpreter;

#[test]
fn receiverless_worker_preserves_user_signal_wakeup() {
    use core::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    let (sender, receiver) = rustpython_vm::signal::user_signal_channel();
    let interp = Interpreter::with_init(Default::default(), |vm| {
        vm.set_user_signal_channel(receiver);
    });
    let worker = interp.enter(|vm| vm.new_thread());
    let called = Arc::new(AtomicBool::new(false));
    let callback_called = called.clone();
    sender
        .send(Box::new(move |_| {
            callback_called.store(true, Ordering::Release);
            Ok(())
        }))
        .unwrap();
    std::thread::spawn(move || {
        worker.run(|vm| rustpython_vm::signal::check_signals(vm).unwrap());
    })
    .join()
    .unwrap();
    assert!(!called.load(Ordering::Acquire));
    interp.enter(|vm| rustpython_vm::signal::check_signals(vm).unwrap());
    assert!(called.load(Ordering::Acquire));
}

#![cfg(test)]
#![cfg(feature = "threading")]

extern crate alloc;

// Keep this process-global signal-breaker regression in its own test binary:
// VM unit tests can clear or consume the breaker concurrently.
use rustpython_vm::Interpreter;

#[test]
fn unrelated_thread_states_preserve_user_signal_wakeup() {
    use alloc::sync::Arc;
    use core::sync::atomic::{AtomicBool, Ordering};

    let (sender, receiver) = rustpython_vm::signal::user_signal_channel();
    let interp = Interpreter::with_init(Default::default(), |vm| {
        vm.set_user_signal_channel(receiver);
    });
    let worker = interp.enter(|vm| vm.new_thread());
    let helper = interp.enter(|vm| vm.new_thread());
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
    // Like exec_interpreter(main=False), this helper runs on the native main
    // thread and may dispatch OS signals, but owns no user-signal receiver.
    helper.run(|vm| rustpython_vm::signal::check_signals(vm).unwrap());
    assert!(!called.load(Ordering::Acquire));
    interp.enter(|vm| rustpython_vm::signal::check_signals(vm).unwrap());
    assert!(called.load(Ordering::Acquire));

    // A different receiver must not consume this channel's wakeup either.
    let (other_sender, other_receiver) = rustpython_vm::signal::user_signal_channel();
    let other = Interpreter::with_init(Default::default(), |vm| {
        vm.set_user_signal_channel(other_receiver);
    });
    called.store(false, Ordering::Release);
    let callback_called = called.clone();
    sender
        .send(Box::new(move |_| {
            callback_called.store(true, Ordering::Release);
            Ok(())
        }))
        .unwrap();
    other.enter(|vm| rustpython_vm::signal::check_signals(vm).unwrap());
    assert!(!called.load(Ordering::Acquire));
    interp.enter(|vm| rustpython_vm::signal::check_signals(vm).unwrap());
    assert!(called.load(Ordering::Acquire));
    drop(other);
    assert!(other_sender.send(Box::new(|_| Ok(()))).is_err());

    // A failing callback must leave the rest of its queue pending.
    called.store(false, Ordering::Release);
    sender
        .send(Box::new(|vm| Err(vm.new_runtime_error("callback failed"))))
        .unwrap();
    let callback_called = called.clone();
    sender
        .send(Box::new(move |_| {
            callback_called.store(true, Ordering::Release);
            Ok(())
        }))
        .unwrap();
    assert!(interp.enter(rustpython_vm::signal::check_signals).is_err());
    assert!(!called.load(Ordering::Acquire));
    interp.enter(|vm| rustpython_vm::signal::check_signals(vm).unwrap());
    assert!(called.load(Ordering::Acquire));

    // The bytecode fast path must see the channel-local hint without an OS
    // signal. Direct check_signals calls alone would not exercise that path.
    #[cfg(feature = "compiler")]
    {
        called.store(false, Ordering::Release);
        let callback_called = called.clone();
        sender
            .send(Box::new(move |_| {
                callback_called.store(true, Ordering::Release);
                Ok(())
            }))
            .unwrap();
        interp.enter(|vm| {
            vm.run_string(vm.new_scope_with_builtins(), "pass", "<signal-wakeup>")
                .unwrap();
        });
        assert!(called.load(Ordering::Acquire));
    }
}

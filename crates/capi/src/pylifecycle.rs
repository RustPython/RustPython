use core::ffi::c_int;
use rustpython_vm::Interpreter;
use std::cell::RefCell;
use std::mem::ManuallyDrop;

thread_local! {
    pub static INTERP: RefCell<Option<ManuallyDrop<Interpreter>>> = const { RefCell::new(None) };
}

#[unsafe(no_mangle)]
pub extern "C" fn Py_IsInitialized() -> c_int {
    INTERP.with(|interp| interp.borrow().is_some() as c_int)
}

#[unsafe(no_mangle)]
pub extern "C" fn Py_Initialize() {
    Py_InitializeEx(0);
}

#[unsafe(no_mangle)]
pub extern "C" fn Py_InitializeEx(_initsigs: c_int) {
    if INTERP.with(|interp| interp.borrow().is_none()) {
        let interp = Interpreter::with_init(Default::default(), |_vm| {});

        INTERP.with(|interp_ref| {
            *interp_ref.borrow_mut() = Some(ManuallyDrop::new(interp));
        });
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn Py_Finalize() {
    let _ = Py_FinalizeEx();
}

#[unsafe(no_mangle)]
pub extern "C" fn Py_FinalizeEx() -> c_int {
    INTERP.with(|interp_ref| {
        let interp = ManuallyDrop::into_inner(
            interp_ref
                .borrow_mut()
                .take()
                .expect("Py_FinalizeEx called without an active interpreter"),
        );
        interp.finalize(None)
    }) as _
}

#[unsafe(no_mangle)]
pub extern "C" fn Py_IsFinalizing() -> c_int {
    0
}

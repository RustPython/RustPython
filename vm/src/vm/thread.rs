use crate::{AsObject, PyObject, VirtualMachine};
use itertools::Itertools;
use std::{cell::RefCell, ptr::NonNull, thread_local};

thread_local! {
    pub(super) static VM_STACK: RefCell<Vec<NonNull<VirtualMachine>>> = Vec::with_capacity(1).into();
}

pub fn enter_vm<R>(vm: &VirtualMachine, f: impl FnOnce() -> R) -> R {
    VM_STACK.with(|vms| {
        vms.borrow_mut().push(vm.into());
        let ret = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        vms.borrow_mut().pop();
        ret.unwrap_or_else(|e| std::panic::resume_unwind(e))
    })
}

pub fn with_vm<F, R>(obj: &PyObject, f: F) -> Option<R>
where
    F: Fn(&VirtualMachine) -> R,
{
    let vm_owns_obj = |intp: NonNull<VirtualMachine>| {
        // SAFETY: all references in VM_STACK should be valid
        let vm = unsafe { intp.as_ref() };
        obj.fast_isinstance(&vm.ctx.types.object_type)
    };
    VM_STACK.with(|vms| {
        let intp = match vms.borrow().iter().copied().exactly_one() {
            Ok(x) => {
                debug_assert!(vm_owns_obj(x));
                x
            }
            Err(mut others) => others.find(|x| vm_owns_obj(*x))?,
        };
        // SAFETY: all references in VM_STACK should be valid, and should not be changed or moved
        // at least until this function returns and the stack unwinds to an enter_vm() call
        let vm = unsafe { intp.as_ref() };
        Some(f(vm))
    })
}

#[must_use = "PyThread does nothing unless you move it to another thread and call .run()"]
#[cfg(feature = "threading")]
pub struct PyThread {
    pub(super) thread_vm: VirtualMachine,
}

#[cfg(feature = "threading")]
impl PyThread {
    /// Create a `FnOnce()` that can easily be passed to a function like [`std::thread::Builder::spawn`]
    ///
    /// # Note
    ///
    /// If you return a `PyObjectRef` (or a type that contains one) from `F`, and don't `join()`
    /// on the thread this `FnOnce` runs in, there is a possibility that that thread will panic
    /// as `PyObjectRef`'s `Drop` implementation tries to run the `__del__` destructor of a
    /// Python object but finds that it's not in the context of any vm.
    pub fn make_spawn_func<F, R>(self, f: F) -> impl FnOnce() -> R
    where
        F: FnOnce(&VirtualMachine) -> R,
    {
        move || self.run(f)
    }

    /// Run a function in this thread context
    ///
    /// # Note
    ///
    /// If you return a `PyObjectRef` (or a type that contains one) from `F`, and don't return the object
    /// to the parent thread and then `join()` on the `JoinHandle` (or similar), there is a possibility that
    /// the current thread will panic as `PyObjectRef`'s `Drop` implementation tries to run the `__del__`
    /// destructor of a python object but finds that it's not in the context of any vm.
    pub fn run<F, R>(self, f: F) -> R
    where
        F: FnOnce(&VirtualMachine) -> R,
    {
        let vm = &self.thread_vm;
        enter_vm(vm, || f(vm))
    }
}

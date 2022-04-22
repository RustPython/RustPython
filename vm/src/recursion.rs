use crate::{AsObject, PyObject, VirtualMachine};

pub struct ReprGuard<'vm> {
    vm: &'vm VirtualMachine,
    id: usize,
}

/// A guard to protect repr methods from recursion into itself,
impl<'vm> ReprGuard<'vm> {
    /// Returns None if the guard against 'obj' is still held otherwise returns the guard. The guard
    /// which is released if dropped.
    pub fn enter(vm: &'vm VirtualMachine, obj: &PyObject) -> Option<Self> {
        let mut guards = vm.repr_guards.borrow_mut();

        // Should this be a flag on the obj itself? putting it in a global variable for now until it
        // decided the form of PyObject. https://github.com/RustPython/RustPython/issues/371
        let id = obj.get_id();
        if guards.contains(&id) {
            return None;
        }
        guards.insert(id);
        Some(ReprGuard { vm, id })
    }
}

impl<'vm> Drop for ReprGuard<'vm> {
    fn drop(&mut self) {
        self.vm.repr_guards.borrow_mut().remove(&self.id);
    }
}

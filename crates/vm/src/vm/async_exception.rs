use super::VirtualMachine;
use crate::{
    AsObject, PyAtomicRef, PyObject, PyObjectRef, PyResult,
    builtins::{PyBaseException, PyType},
    common::wtf8::Wtf8Buf,
};
use core::cell::Cell;

/// Owned by one execution context, like `PyThreadState.async_exc`.
/// This is an external GC root, not an edge owned by a Python object.
/// The VM is not Sync: only its owning execution thread accesses these flags.
/// Cross-thread requests would require a separate synchronized API and wakeup.
pub(super) struct PendingAsyncException {
    exception: PyAtomicRef<Option<PyObject>>,
    pending: Cell<bool>,
    closed: Cell<bool>,
}

impl Default for PendingAsyncException {
    fn default() -> Self {
        Self {
            exception: PyAtomicRef::new_empty(),
            pending: Cell::new(false),
            closed: Cell::new(false),
        }
    }
}

impl PendingAsyncException {
    pub(super) fn is_pending(&self) -> bool {
        self.pending.get()
    }

    pub(super) fn has_reference(&self) -> bool {
        !self.exception.load_ptr().is_null()
    }

    fn replace(&self, exception: Option<PyObjectRef>) -> Option<Option<PyObjectRef>> {
        if self.closed.get() {
            return None;
        }
        Some(self.exception.store(exception))
    }

    fn notify(&self) {
        if !self.closed.get() {
            self.pending.set(true);
        }
    }

    fn take(&self) -> Option<PyObjectRef> {
        if !self.pending.replace(false) {
            return None;
        }
        self.exception.store(None)
    }

    fn close(&self) {
        // A cleared thread state cannot accept another request. Rejecting
        // reentrant requests makes this a single clear, not a finalizer drain.
        self.closed.set(true);
        self.pending.set(false);
        drop(self.exception.store(None));
    }
}

impl VirtualMachine {
    /// Replace the currently attached thread state's asynchronous exception.
    /// The outer `None` means that this VM is inactive or is being cleared.
    /// The old owner is returned so callers can release it outside all locks,
    /// either before or after notification as required by their API contract.
    /// `None` as the input cancels; validation happens only during delivery.
    pub(crate) fn replace_async_exception(
        &self,
        exception: Option<PyObjectRef>,
    ) -> Option<Option<PyObjectRef>> {
        if !super::thread::try_with_current_vm(|vm| core::ptr::eq(vm, self)).unwrap_or(false) {
            return None;
        }
        self.async_exception.replace(exception)
    }

    pub(crate) fn notify_async_exception(&self) {
        self.async_exception.notify();
    }

    /// Called only at bytecode safepoints, after pending signals and GC.
    pub(crate) fn raise_async_exception(&self) -> PyResult<()> {
        let Some(exception) = self.async_exception.take() else {
            return Ok(());
        };
        let class = match exception.downcast::<PyType>() {
            Ok(class) if class.fast_issubclass(self.ctx.exceptions.base_exception_type) => class,
            object => {
                let object: PyObjectRef = match object {
                    Ok(class) => class.into(),
                    Err(object) => object,
                };
                let mut message = Wtf8Buf::from("_PyErr_SetObject: exception ");
                message.push_wtf8(object.repr(self)?.as_wtf8());
                message.push_str(" is not a BaseException subclass");
                return Err(self.new_system_error(message));
            }
        };

        // `_PyErr_SetNone` invokes the class through its metaclass, including
        // user __call__/__new__/__init__, rather than allocating a bare payload.
        let result = class.as_object().call((), self).and_then(|object| {
            object.downcast::<PyBaseException>().map_err(|object| {
                let mut message = Wtf8Buf::from("calling ");
                match class.as_object().repr(self) {
                    Ok(repr) => message.push_wtf8(repr.as_wtf8()),
                    Err(error) => return error,
                }
                message.push_str(" should have returned an instance of BaseException, not ");
                message.push_str(&object.class().slot_name());
                self.new_type_error(message)
            })
        });
        let exception = match result {
            Ok(exception) => exception,
            Err(error) => {
                // CPython represents the NULL value passed by SetNone as
                // '<NULL>'; failure in repr's signal check uses '<unknown>'.
                #[cfg(not(target_arch = "wasm32"))]
                let args = if crate::signal::check_signals(self).is_ok() {
                    "<NULL>"
                } else {
                    "<unknown>"
                };
                #[cfg(target_arch = "wasm32")]
                let args = "<NULL>";
                let note = self.ctx.new_str(format!(
                    "Normalization failed: type={} args={args}",
                    class.slot_name()
                ));
                // A failed attempt to append the note must not replace the
                // exception raised by the class constructor.
                let _ = error.add_note(note, self);
                error
            }
        };
        Err(exception)
    }

    /// Clear a thread-state root while its VM and frame slot are still usable.
    pub(crate) fn clear_async_exception(&self) {
        self.async_exception.close();
    }
}

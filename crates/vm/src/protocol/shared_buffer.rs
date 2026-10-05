//! Cross-interpreter buffers retain native storage, never foreign Python payloads.

use super::{BufferDescriptor, PyBuffer};
use crate::{
    PyResult, VirtualMachine,
    common::{
        borrow::{BorrowedValue, BorrowedValueMut},
        rc::PyRc,
    },
    object::PyThreadingConstraint,
    vm::roots::RootLease,
};
use core::fmt;

/// Storage with no Python references or callbacks, independently owned by an export.
///
/// # Safety
/// The storage must remain valid, with stable length, for this export's lifetime,
/// including after its interpreter is destroyed. Reading, writing and dropping it
/// must not access Python objects. Mutable access must synchronize with every other
/// reader/writer of the same storage, including the original exporter.
pub unsafe trait SharedBufferStorage: fmt::Debug + PyThreadingConstraint {
    fn read(&self) -> BorrowedValue<'_, [u8]>;
    fn write(&self) -> BorrowedValueMut<'_, [u8]>;
}

#[derive(Clone)]
pub struct SharedBuffer {
    pub desc: BufferDescriptor,
    pub(crate) storage: PyRc<dyn SharedBufferStorage>,
    // Releasing a Python buffer can call __release_buffer__. Its acquisition
    // remains rooted on the owner; the receiving VM sees only this opaque lease.
    _lease: PyRc<RootLease>,
}

impl fmt::Debug for SharedBuffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SharedBuffer")
            .field("desc", &self.desc)
            .finish_non_exhaustive()
    }
}

impl SharedBuffer {
    pub(crate) fn new(buffer: PyBuffer, vm: &VirtualMachine) -> PyResult<Self> {
        // Forwarding a foreign export must not build a chain of owner roots.
        if let Some(mut shared) = crate::stdlib::_interpreters::clone_shared_buffer(&buffer.obj) {
            shared.desc = buffer.desc.clone();
            return Ok(shared);
        }
        let storage = buffer.shared_storage().ok_or_else(|| {
            vm.new_buffer_error("buffer exporter does not provide interpreter-independent storage")
        })?;
        let desc = buffer.desc.clone();
        let lease = vm
            .state
            .roots
            .insert_buffer(buffer)
            .ok_or_else(|| vm.new_runtime_error("interpreter is closing"))?;
        Ok(Self {
            desc,
            storage,
            _lease: lease,
        })
    }

    pub(crate) fn local_buffer(&self, vm: &VirtualMachine) -> Option<PyBuffer> {
        let mut buffer = self._lease.buffer(vm)?;
        buffer.desc = self.desc.clone();
        Some(buffer)
    }

    pub(crate) fn read(&self) -> BorrowedValue<'_, [u8]> {
        self.storage.read()
    }
    pub(crate) fn write(&self) -> BorrowedValueMut<'_, [u8]> {
        self.storage.write()
    }
}

#[derive(Debug)]
pub(crate) struct ImmutableBuffer(pub(crate) Box<[u8]>);

// SAFETY: owns immutable Rust bytes and cannot retain or execute Python.
unsafe impl SharedBufferStorage for ImmutableBuffer {
    fn read(&self) -> BorrowedValue<'_, [u8]> {
        self.0.as_ref().into()
    }
    fn write(&self) -> BorrowedValueMut<'_, [u8]> {
        panic!("immutable buffer is not writable")
    }
}

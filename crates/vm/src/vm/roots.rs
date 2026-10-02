//! Persistent native roots released only while their interpreter is attached.

use crate::common::{lock::PyMutex, rc::PyRc};
#[cfg(any(feature = "threading", test))]
use core::any::Any;
use core::sync::atomic::{AtomicBool, Ordering};

#[cfg(feature = "threading")]
type Resource = dyn Any + Send + Sync;
#[cfg(all(not(feature = "threading"), test))]
type Resource = dyn Any;

enum Root {
    Buffer(crate::protocol::PyBuffer),
    #[cfg(any(feature = "threading", test))]
    Resource {
        _value: Box<Resource>,
    },
}

struct Slot {
    generation: u64,
    value: Option<Root>,
}

#[derive(Clone, Copy)]
struct Key {
    index: usize,
    generation: u64,
}

#[derive(Default)]
struct Table {
    slots: Vec<Slot>,
    free: Vec<usize>,
    pending: Vec<Key>,
    closed: bool,
}

pub(crate) struct Roots {
    pub(crate) owner: i64,
    table: PyMutex<Table>,
    pending: AtomicBool,
    draining: AtomicBool,
    closed: AtomicBool,
}

/// Contains no directly accessible Python reference. Clones share one table entry.
pub(crate) struct RootLease {
    roots: PyRc<Roots>,
    key: Key,
}

impl Drop for RootLease {
    fn drop(&mut self) {
        // Closed root tables may outlive the runtime and its fork snapshot.
        // Never acquire their inherited mutex after native retirement.
        if self.roots.is_closed() {
            return;
        }
        {
            let mut table = self.roots.table.lock();
            if table.closed {
                return;
            }
            table.pending.push(self.key);
            self.roots.pending.store(true, Ordering::Release);
        }
        // The normal path can release immediately. A foreign thread or a TLS
        // destructor only queues work, without dereferencing a Python object.
        super::thread::try_with_attached_vm(|vm| {
            if vm.state.interpreter_id == self.roots.owner {
                self.roots.drain_pending();
            }
        });
    }
}

impl RootLease {
    pub(crate) fn buffer(&self, vm: &crate::VirtualMachine) -> Option<crate::protocol::PyBuffer> {
        if self.owner() != vm.state.interpreter_id {
            return None;
        }
        assert!(super::thread::is_current_attached(vm));
        let table = self.roots.table.lock();
        let slot = table.slots.get(self.key.index)?;
        if slot.generation != self.key.generation {
            return None;
        }
        match slot.value.as_ref()? {
            Root::Buffer(buffer) => Some(buffer.clone()),
            #[cfg(any(feature = "threading", test))]
            Root::Resource { .. } => unreachable!("non-buffer root used as a buffer"),
        }
    }

    /// Retire one owner-local cleanup action immediately, even from a release
    /// callback. The generation invalidates a later queued drop of this lease.
    #[cfg(feature = "threading")]
    pub(crate) fn release_now(&self, vm: &crate::VirtualMachine) {
        assert_eq!(self.owner(), vm.state.interpreter_id);
        assert!(super::thread::is_current_attached(vm));
        let retired = retire(&mut self.roots.table.lock(), self.key);
        drop(retired);
    }

    pub(crate) fn owner(&self) -> i64 {
        self.roots.owner
    }
}

impl Roots {
    pub(crate) fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    pub(crate) fn has_pending(&self) -> bool {
        self.pending.load(Ordering::Relaxed)
    }

    // Detached native threads can enqueue the last handle release without a
    // Python attachment. Stabilize the Vec itself across fork, not just its lock.
    #[cfg(all(unix, feature = "threading"))]
    pub(super) fn prepare_fork(&self) -> impl Drop + '_ {
        self.table.lock()
    }

    #[cfg(all(unix, feature = "threading"))]
    pub(crate) unsafe fn reinit_after_fork(&self) {
        // The fork coordinator held and released the directory guard in both
        // processes. No interrupted table mutation can survive the syscall.
        self.draining.store(false, Ordering::Relaxed);
    }

    pub(crate) fn new(owner: i64) -> Self {
        Self {
            owner,
            table: PyMutex::default(),
            pending: AtomicBool::new(false),
            draining: AtomicBool::new(false),
            closed: AtomicBool::new(false),
        }
    }

    fn insert(self: &PyRc<Self>, value: Root) -> Option<PyRc<RootLease>> {
        let key = {
            let mut table = self.table.lock();
            if table.closed {
                return None;
            }
            if let Some(index) = table.free.pop() {
                let slot = &mut table.slots[index];
                debug_assert!(slot.value.is_none());
                slot.value = Some(value);
                Key {
                    index,
                    generation: slot.generation,
                }
            } else {
                let index = table.slots.len();
                table.slots.push(Slot {
                    generation: 0,
                    value: Some(value),
                });
                Key {
                    index,
                    generation: 0,
                }
            }
        };
        Some(PyRc::new(RootLease {
            roots: self.clone(),
            key,
        }))
    }

    pub(crate) fn insert_buffer(
        self: &PyRc<Self>,
        buffer: crate::protocol::PyBuffer,
    ) -> Option<PyRc<RootLease>> {
        self.insert(Root::Buffer(buffer))
    }

    /// Retain an export or another native resource with an owner-local destructor.
    #[cfg(any(feature = "threading", test))]
    pub(crate) fn insert_resource<T>(self: &PyRc<Self>, resource: T) -> Option<PyRc<RootLease>>
    where
        T: Any + crate::object::PyThreadingConstraint,
    {
        self.insert(Root::Resource {
            _value: Box::new(resource),
        })
    }

    /// Detach roots under the directory lock; arbitrary Python runs only after unlocking.
    pub(crate) fn drain_pending(&self) {
        if !self.pending.load(Ordering::Acquire) || self.draining.swap(true, Ordering::Acquire) {
            return;
        }
        scopeguard::defer! { self.draining.store(false, Ordering::Release); }
        let retired = {
            let mut table = self.table.lock();
            self.pending.store(false, Ordering::Relaxed);
            let pending = core::mem::take(&mut table.pending);
            let mut retired = Vec::with_capacity(pending.len());
            for key in pending {
                if let Some(value) = retire(&mut table, key) {
                    retired.push(value);
                }
            }
            retired
        };
        drop(retired);
    }

    /// Invalidate every external handle before running any of their destructors.
    pub(crate) fn close(&self) {
        let slots = {
            let mut table = self.table.lock();
            table.closed = true;
            self.closed.store(true, Ordering::Release);
            table.pending.clear();
            self.pending.store(false, Ordering::Relaxed);
            table.free.clear();
            core::mem::take(&mut table.slots)
        };
        drop(slots);
    }
}

fn retire(table: &mut Table, key: Key) -> Option<Root> {
    let slot = table.slots.get_mut(key.index)?;
    if slot.generation != key.generation {
        return None;
    }
    let value = slot.value.take()?;
    // A wrapped generation is never reused, including stale queued releases.
    if let Some(next) = slot.generation.checked_add(1) {
        slot.generation = next;
        table.free.push(key.index);
    }
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "threading")]
    struct Callback(Box<dyn Fn() + Send + Sync>);

    #[cfg(feature = "threading")]
    impl Drop for Callback {
        fn drop(&mut self) {
            (self.0)();
        }
    }

    #[test]
    fn retired_keys_cannot_release_a_reused_slot_and_close_rejects_reentry() {
        let roots = PyRc::new(Roots::new(1));
        let first = roots.insert_resource(()).unwrap();
        let old_key = first.key;
        drop(first);
        roots.drain_pending();
        let second = roots.insert_resource(()).unwrap();
        assert_eq!(second.key.index, old_key.index);
        assert_ne!(second.key.generation, old_key.generation);
        roots.table.lock().pending.push(old_key);
        roots.pending.store(true, Ordering::Release);
        roots.drain_pending();
        assert!(roots.table.lock().slots[second.key.index].value.is_some());
        roots.close();
        assert!(roots.insert_resource(()).is_none());
        drop(second);
        assert!(roots.table.lock().pending.is_empty());
    }

    #[cfg(feature = "threading")]
    #[test]
    fn native_resources_are_released_attached_and_outside_the_directory_lock() {
        use crate::vm::thread;
        let interpreter = crate::Interpreter::without_stdlib(Default::default());
        let roots = interpreter.enter_raw(|vm| vm.state.roots.clone());
        let weak = PyRc::downgrade(&roots);
        let count = PyRc::new(core::sync::atomic::AtomicUsize::new(0));
        let released = count.clone();
        let owner = interpreter.id();
        let lease = roots
            .insert_resource(Callback(Box::new(move || {
                assert_eq!(thread::with_current_vm(|vm| vm.state.interpreter_id), owner);
                assert!(weak.upgrade().unwrap().table.try_lock().is_some());
                released.fetch_add(1, Ordering::Relaxed);
            })))
            .unwrap();
        std::thread::spawn(move || drop(lease)).join().unwrap();
        assert_eq!(count.load(Ordering::Relaxed), 0);
        interpreter.enter(|_| {});
        assert_eq!(count.load(Ordering::Relaxed), 1);
    }
}

use lock_api::{
    GetThreadId, RawMutex, RawRwLock, RawRwLockDowngrade, RawRwLockRecursive, RawRwLockUpgrade,
    RawRwLockUpgradeDowngrade,
};
use std::cell::Cell;
use std::num::NonZeroUsize;

pub struct RawCellMutex {
    locked: Cell<bool>,
}

unsafe impl RawMutex for RawCellMutex {
    #[allow(clippy::declare_interior_mutable_const)]
    const INIT: Self = RawCellMutex {
        locked: Cell::new(false),
    };

    type GuardMarker = lock_api::GuardNoSend;

    #[inline]
    fn lock(&self) {
        if self.is_locked() {
            deadlock("", "Mutex")
        }
        self.locked.set(true)
    }

    #[inline]
    fn try_lock(&self) -> bool {
        if self.is_locked() {
            false
        } else {
            self.locked.set(true);
            true
        }
    }

    unsafe fn unlock(&self) {
        self.locked.set(false)
    }

    #[inline]
    fn is_locked(&self) -> bool {
        self.locked.get()
    }
}

const WRITER_BIT: usize = 0b01;
const ONE_READER: usize = 0b10;

pub struct RawCellRwLock {
    state: Cell<usize>,
}

impl RawCellRwLock {
    #[inline]
    fn is_exclusive(&self) -> bool {
        self.state.get() & WRITER_BIT != 0
    }
}

unsafe impl RawRwLock for RawCellRwLock {
    #[allow(clippy::declare_interior_mutable_const)]
    const INIT: Self = RawCellRwLock {
        state: Cell::new(0),
    };

    type GuardMarker = <RawCellMutex as RawMutex>::GuardMarker;

    #[inline]
    fn lock_shared(&self) {
        if !self.try_lock_shared() {
            deadlock("sharedly ", "RwLock")
        }
    }

    #[inline]
    fn try_lock_shared(&self) -> bool {
        // TODO: figure out whether this is realistic; could maybe help
        // debug deadlocks from 2+ read() in the same thread?
        // if self.is_locked() {
        //     false
        // } else {
        //     self.state.set(ONE_READER);
        //     true
        // }
        self.try_lock_shared_recursive()
    }

    #[inline]
    unsafe fn unlock_shared(&self) {
        self.state.set(self.state.get() - ONE_READER)
    }

    #[inline]
    fn lock_exclusive(&self) {
        if !self.try_lock_exclusive() {
            deadlock("exclusively ", "RwLock")
        }
        self.state.set(WRITER_BIT)
    }

    #[inline]
    fn try_lock_exclusive(&self) -> bool {
        if self.is_locked() {
            false
        } else {
            self.state.set(WRITER_BIT);
            true
        }
    }

    unsafe fn unlock_exclusive(&self) {
        self.state.set(0)
    }

    fn is_locked(&self) -> bool {
        self.state.get() != 0
    }
}

unsafe impl RawRwLockDowngrade for RawCellRwLock {
    unsafe fn downgrade(&self) {
        self.state.set(ONE_READER);
    }
}

unsafe impl RawRwLockUpgrade for RawCellRwLock {
    #[inline]
    fn lock_upgradable(&self) {
        if !self.try_lock_upgradable() {
            deadlock("upgradably+sharedly ", "RwLock")
        }
    }

    #[inline]
    fn try_lock_upgradable(&self) -> bool {
        // defer to normal -- we can always try to upgrade
        self.try_lock_shared()
    }

    #[inline]
    unsafe fn unlock_upgradable(&self) {
        self.unlock_shared()
    }

    #[inline]
    unsafe fn upgrade(&self) {
        if !self.try_upgrade() {
            deadlock("upgrade ", "RwLock")
        }
    }

    #[inline]
    unsafe fn try_upgrade(&self) -> bool {
        if self.state.get() == ONE_READER {
            self.state.set(WRITER_BIT);
            true
        } else {
            false
        }
    }
}

unsafe impl RawRwLockUpgradeDowngrade for RawCellRwLock {
    #[inline]
    unsafe fn downgrade_upgradable(&self) {
        // no-op -- we're always upgradable
    }

    #[inline]
    unsafe fn downgrade_to_upgradable(&self) {
        self.state.set(ONE_READER);
    }
}

unsafe impl RawRwLockRecursive for RawCellRwLock {
    #[inline]
    fn lock_shared_recursive(&self) {
        if !self.try_lock_shared_recursive() {
            deadlock("recursively+sharedly ", "RwLock")
        }
    }

    #[inline]
    fn try_lock_shared_recursive(&self) -> bool {
        if self.is_exclusive() {
            false
        } else if let Some(new) = self.state.get().checked_add(ONE_READER) {
            self.state.set(new);
            true
        } else {
            false
        }
    }
}

#[cold]
#[inline(never)]
fn deadlock(lockkind: &str, ty: &str) -> ! {
    panic!("deadlock: tried to {}lock a Cell{} twice", lockkind, ty)
}

pub struct SingleThreadId(());
unsafe impl GetThreadId for SingleThreadId {
    const INIT: Self = SingleThreadId(());
    fn nonzero_thread_id(&self) -> NonZeroUsize {
        NonZeroUsize::new(1).unwrap()
    }
}

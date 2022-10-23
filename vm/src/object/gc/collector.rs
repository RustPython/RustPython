use rustpython_common::{
    lock::{PyMutex, PyMutexGuard, PyRwLock, PyRwLockReadGuard, PyRwLockWriteGuard},
    rc::PyRc,
};
use std::{cell::Cell, ptr::NonNull, time::Instant};

#[cfg(feature = "threading")]
use std::time::Duration;

use crate::{
    object::gc::{Color, GcResult, Trace},
    PyObject,
};

use super::{GcObj, GcObjRef, GcStatus};

#[cfg(feature = "threading")]
pub static LOCK_TIMEOUT: Duration = Duration::from_secs(5);

/// The global cycle collector, which collect cycle references for PyInner<T>
#[cfg(feature = "threading")]
pub static GLOBAL_COLLECTOR: once_cell::sync::Lazy<PyRc<Collector>> =
    once_cell::sync::Lazy::new(|| PyRc::new(Default::default()));

#[cfg(not(feature = "threading"))]
thread_local! {
    pub static GLOBAL_COLLECTOR: PyRc<Collector> = PyRc::new(Default::default());
}

/// only use for roots's pointer to object, mark `NonNull` as safe to send
#[repr(transparent)]
struct WrappedPtr(NonNull<PyObject>);
unsafe impl Send for WrappedPtr {}
unsafe impl Sync for WrappedPtr {}
impl std::ops::Deref for WrappedPtr {
    type Target = NonNull<PyObject>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::fmt::Debug for WrappedPtr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

impl From<NonNull<PyObject>> for WrappedPtr {
    fn from(ptr: NonNull<PyObject>) -> Self {
        Self(ptr)
    }
}

impl From<WrappedPtr> for NonNull<PyObject> {
    fn from(w: WrappedPtr) -> Self {
        w.0
    }
}

pub struct Collector {
    roots: PyMutex<Vec<WrappedPtr>>,
    /// for stop the world, will be try to check lock every time deref ObjecteRef
    /// to achive pausing
    pause: PyRwLock<()>,
    last_gc_time: PyMutex<Instant>,
    is_enabled: PyMutex<bool>,
    /// acquire this to prevent a new gc to happen before this gc is completed
    cleanup_cycle: PyMutex<()>,
}

impl std::fmt::Debug for Collector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CcSync")
            .field(
                "roots",
                &format!("[{} objects in buffer]", self.roots_len()),
            )
            .field("pause", &self.pause)
            .field("last_gc_time", &self.last_gc_time)
            .finish()
    }
}

impl Default for Collector {
    fn default() -> Self {
        Self {
            roots: Default::default(),
            pause: Default::default(),
            last_gc_time: PyMutex::new(Instant::now()),
            is_enabled: PyMutex::new(true),
            cleanup_cycle: PyMutex::new(()),
        }
    }
}

// core of gc algorithm
impl Collector {
    /*
    unsafe fn drop_dealloc(obj: NonNull<PyObject>) {
        PyObject::drop_slow(obj)
    }
     */
    fn collect_cycles(&self, force: bool) -> GcResult {
        if Self::IS_GC_THREAD.with(|v| v.get()) {
            return (0, 0).into();
            // already call collect_cycle() once
        }
        if self.roots_len() == 0 || self.cleanup_cycle.is_locked() {
            return (0, 0).into();
        }

        // acquire stop-the-world lock
        let lock = {
            #[cfg(feature = "threading")]
            {
                // if can't access pause lock for a second, return because gc is not that emergency,
                // also normal call to `gc.collect()` can usually acquire that lock unless something is wrong
                if force {
                    // if is forced to gc, wait a while for write lock
                    match self.pause.try_write_for(std::time::Duration::from_secs(1)) {
                        Some(v) => v,
                        None => {
                            warn!("Can't acquire lock to stop the world.");
                            return (0, 0).into();
                        }
                    }
                } else {
                    // if not forced to gc, a non-blocking check to see if gc is possible
                    match self.pause.try_write() {
                        Some(v) => v,
                        None => {
                            warn!("Fast GC fail to acquire write lock.");
                            return (0, 0).into();
                        }
                    }
                }
            }
            // also when no threading, there is actually no need to get a lock,(because every thread have it's own gc)
            // but for the sake of using same code(and defendsive), we acquire one anyway
            #[cfg(not(feature = "threading"))]
            {
                // when not threading, no deadlock should occur?
                let _force = force;
                self.pause.try_write().unwrap()
            }
        };
        // order of acquire lock and check IS_GC_THREAD here is important
        // This prevent set multiple IS_GC_THREAD thread local variable to true
        // using write() to gain exclusive access
        Self::IS_GC_THREAD.with(|v| v.set(true));

        // the three main step of GC
        // 1. mark roots: which get trial DECREF object so cycles get zero rc
        // 2. scan roots: get non-cycle object back to normal values
        // 3. collect roots: collect cycles starting from roots
        let freed = Self::mark_roots(&mut *self.roots.lock());
        Self::scan_roots(&mut *self.roots.lock());
        let ret_cycle = self.collect_roots(self.roots.lock(), lock);
        (freed, ret_cycle).into()
    }

    fn mark_roots<R>(mut roots: R) -> usize
    where
        R: AsMut<Vec<WrappedPtr>>,
    {
        let mut freed = 0;
        let old_roots: Vec<_> = { roots.as_mut().drain(..).collect() };
        let mut new_roots = old_roots
            .into_iter()
            .filter(|ptr| {
                let obj = unsafe { ptr.as_ref() };
                if obj.header().color() == Color::Purple {
                    Self::mark_gray(obj);
                    true
                } else {
                    obj.header().set_buffered(false);
                    if obj.header().color() == Color::Black && obj.header().rc() == 0 {
                        debug_assert!(obj.header().is_drop() && !obj.header().is_dealloc());
                        freed += 1;
                        unsafe {
                            // only dealloc here, because already drop(only) in Object's impl Drop
                            // PyObject::dealloc_only(ptr.cast::<PyObject>());
                            let ret = PyObject::dealloc_only(**ptr);
                            debug_assert!(ret);
                            // obj is dangling after this line?
                        }
                    }
                    false
                }
            })
            .collect();
        roots.as_mut().append(&mut new_roots);
        freed
    }

    fn mark_gray(obj: GcObjRef) {
        if obj.header().color() != Color::Gray {
            obj.header().set_color(Color::Gray);
            obj.trace(&mut |ch| {
                if ch.header().is_leaked() {
                    return;
                }
                ch.header().dec();
                Self::mark_gray(ch);
            });
        }
    }

    fn scan_roots<R>(mut roots: R)
    where
        R: AsMut<Vec<WrappedPtr>>,
    {
        roots
            .as_mut()
            .iter()
            .map(|ptr| {
                let obj = unsafe { ptr.as_ref() };
                Self::scan(obj);
            })
            .count();
    }

    fn scan(obj: GcObjRef) {
        if obj.header().color() == Color::Gray {
            if obj.header().rc() > 0 {
                Self::scan_black(obj)
            } else {
                obj.header().set_color(Color::White);
                obj.trace(&mut |ch| {
                    if ch.header().is_leaked() {
                        return;
                    }
                    Self::scan(ch);
                });
            }
        }
    }

    fn scan_black(obj: GcObjRef) {
        obj.header().set_color(Color::Black);
        obj.trace(&mut |ch| {
            if ch.header().is_leaked() {
                return;
            }
            ch.header().inc();
            if ch.header().color() != Color::Black {
                debug_assert!(
                    ch.header().color() == Color::Gray || ch.header().color() == Color::White
                );
                Self::scan_black(ch)
            }
        });
    }

    /// TODO: change to use weak_ref count to prevent premature dealloc in cycles
    /// free everything in white, safe to use even when those objects form cycle refs
    fn free_cycles(&self, white: Vec<NonNull<PyObject>>) -> usize {
        // TODO: maybe never run __del__ anyway, for running a __del__ function is an implementation detail!!!!
        // TODO: impl PEP 442
        // 0. count&mark cycle with indexies
        // 0.5. add back one ref for all thing in white
        // 1. clear weakref
        // 2. run del
        // 3. check if cycle is still isolated(using mark_roots&scan_roots), remember to acquire gc's exclusive lock to prevent graph from change
        // (atomic op required, maybe acquire a lock on them?
        //or if a object dead immediate before even incref, it will be wrongly revived, but if rc is added back, that should be ok)
        // 4. drop the still isolated cycle(which is confirmed garbage), then dealloc them

        // Run drop on each of nodes.
        white.iter().for_each(|i| {
            // Calling drop() will decrement the reference count on any of our live children.
            // However, during trial deletion the reference count was already decremented
            // so we'll end up decrementing twice. To avoid that, we increment the count
            // just before calling drop() so that it balances out. This is another difference
            // from the original paper caused by having destructors that we need to run.

            let obj = unsafe { i.as_ref() };
            obj.trace(&mut |ch| {
                if ch.header().is_leaked() {
                    return;
                }
                debug_assert!(!ch.header().is_dealloc());
                if ch.header().rc() > 0 {
                    ch.header().inc();
                }
            });
        });
        // drop all for once at seperate loop to avoid certain cycle ref double drop bug
        let can_deallocs: Vec<_> = white
            .iter()
            .map(|i| unsafe { PyObject::drop_clr_wr(*i) })
            .collect();
        // drop first, deallocate later so to avoid heap corruption
        // cause by circular ref and therefore
        // access pointer of already dropped value's memory region
        white
            .iter()
            .zip(can_deallocs)
            .map(|(i, can_dealloc)| {
                if can_dealloc {
                    let ret = unsafe { PyObject::dealloc_only(*i) };
                    debug_assert!(ret);
                }
            })
            .count();
        info!("Cyclic garbage collected, count={}", white.len());
        white.len()
    }

    fn collect_roots(
        &self,
        mut roots_lock: PyMutexGuard<Vec<WrappedPtr>>,
        lock: PyRwLockWriteGuard<()>,
    ) -> usize {
        // Collecting the nodes into this Vec is difference from the original
        // Bacon-Rajan paper. We need this because we have destructors(RAII) and
        // running them during traversal will cause cycles to be broken which
        // ruins the rest of our traversal.
        let mut white = Vec::new();
        let roots: Vec<_> = { roots_lock.drain(..).collect() };
        // future inc/dec will accesss roots so drop lock in here
        drop(roots_lock);
        // release gc pause lock in here, for after this line no white garbage will be access by mutator
        roots
            .into_iter()
            .map(|ptr| {
                let obj = unsafe { ptr.as_ref() };
                obj.header().set_buffered(false);
                Self::collect_white(obj, &mut white);
            })
            .count();
        let len_white = white.len();
        if !white.is_empty() {
            info!("Cyclic garbage collected, count={}", white.len());
        }

        // mark the end of GC, but another gc can only begin after acquire cleanup_cycle lock
        // because a dead cycle can't actively change object graph anymore
        let _cleanup_lock = self.cleanup_cycle.lock();
        // unlock fair so high freq gc wouldn't stop the world forever
        #[cfg(feature = "threading")]
        PyRwLockWriteGuard::unlock_fair(lock);
        #[cfg(not(feature = "threading"))]
        drop(lock);
        Self::IS_GC_THREAD.with(|v| v.set(false));

        self.free_cycles(white);

        len_white
    }
    fn collect_white(obj: GcObjRef, white: &mut Vec<NonNull<GcObj>>) {
        if obj.header().color() == Color::White && !obj.header().buffered() {
            obj.header().set_color(Color::Black);
            obj.header().set_in_cycle(true);
            obj.trace(&mut |ch| {
                if ch.header().is_leaked() {
                    return;
                }
                Self::collect_white(ch, white)
            });
            white.push(NonNull::from(obj));
        }
    }

    // basic inc/dec operation
    pub fn increment(&self, obj: &PyObject) {
        if obj.header().is_leaked() {
            return;
        }
        // acquire exclusive access to obj's header

        // prevent starting a gc in the middle of change header state
        let _lock_gc = obj.header().try_pausing();
        #[cfg(feature = "threading")]
        let _lock = obj.header().exclusive();
        obj.header().inc();
        obj.header().set_color(Color::Black);
    }

    /// if the last ref to a object call decrement() on object,
    /// then this object should be considered freed.
    pub fn decrement(&self, obj: &PyObject) -> GcStatus {
        if obj.header().is_leaked() {
            // a leaked object should always keep
            return GcStatus::ShouldKeep;
        }

        // prevent starting a gc in the middle of decrement
        let _lock_gc = obj.header().try_pausing();

        // acquire exclusive access to obj's header, so no decrement in the middle of increment of vice versa
        #[cfg(feature = "threading")]
        let _lock_obj = obj.header().exclusive();
        // prevent RAII Drop to drop below zero
        if obj.header().rc() > 0 {
            debug_assert!(!obj.header().is_drop());
            let rc = obj.header().dec();
            if rc == 0 {
                self.release(obj)
            } else if obj.is_traceable() && !obj.header().is_leaked() {
                // only buffer traceable(and not leaked) object for that is where we can detect cycles
                self.possible_root(obj);
                GcStatus::ShouldKeep
            } else {
                // if is not traceable, which could be actually acyclic or not, keep them anyway
                GcStatus::ShouldKeep
            }
        } else {
            // FIXME(discord9): confirm if already rc==0 then should do nothing
            GcStatus::DoNothing
        }
    }

    fn release(&self, obj: &PyObject) -> GcStatus {
        // because drop obj itself will drop all ObjRef store by object itself once more,
        // so balance out in here
        // by doing nothing
        // instead of minus one and do:
        // ```ignore
        // obj.trace(&mut |ch| {
        //   self.decrement(ch);
        // });
        //```
        obj.header().set_color(Color::Black);

        // TODO(discord9): just drop in here, not by the caller, which is cleaner
        // before it is free in here,
        // but now change to passing message to allow it to drop outside
        match (obj.header().buffered(), obj.header().is_in_cycle()) {
            (true, _) => GcStatus::BufferedDrop,
            (_, true) => GcStatus::GarbageCycle,
            (false, false) => GcStatus::ShouldDrop,
        }
    }

    fn possible_root(&self, obj: &PyObject) {
        if obj.header().color() != Color::Purple {
            obj.header().set_color(Color::Purple);
            // prevent add to buffer for multiple times
            let mut roots = self.roots.lock();
            let header = obj.header();
            if !header.buffered() {
                header.set_buffered(true);
                roots.push(NonNull::from(obj).into());
            }
        }
    }

    // methods about gc condition

    #[inline]
    fn roots_len(&self) -> usize {
        self.roots.lock().len()
    }

    #[inline]
    pub(crate) fn is_enabled(&self) -> bool {
        *self.is_enabled.lock()
    }
    #[inline]
    pub(crate) fn enable(&self) {
        *self.is_enabled.lock() = true
    }
    #[inline]
    pub(crate) fn disable(&self) {
        *self.is_enabled.lock() = false
    }

    #[inline]
    pub fn force_gc(&self) -> GcResult {
        self.collect_cycles(true)
    }

    #[inline]
    #[allow(unreachable_code)]
    pub fn should_gc(&self) -> bool {
        // TODO: use "Optimal Heap Limits for Reducing Browser Memory Use"(http://arxiv.org/abs/2204.10455)
        // to optimize gc condition
        if !self.is_enabled() {
            return false;
        }
        // if can't acquire lock, some other thread is already in gc
        if self.pause.is_locked_exclusive() {
            return false;
        }
        // FIXME(discord9): better condition, could be important
        if self.roots_len() > 10007 {
            if Self::IS_GC_THREAD.with(|v| v.get()) {
                // Already in gc, return early
                return false;
            }
            let mut last_gc_time = self.last_gc_time.lock();
            if last_gc_time.elapsed().as_secs() >= 1 {
                *last_gc_time = Instant::now();
                true
            } else {
                false
            }
        } else {
            false
        }
    }

    /// _suggest_(may or may not) collector to collect garbage. return number of cyclic garbage being collected
    #[inline]
    pub fn fast_try_gc(&self) -> GcResult {
        // so that if other thread is gcing, this thread can be stopped in time
        self.do_pausing();
        if self.should_gc() {
            self.collect_cycles(false)
        } else {
            (0, 0).into()
        }
    }

    // methods about stopping the world

    thread_local! {
        /// only set to true when start a gc in thread, assume any drop() impl doesn't create new thread, so gc only work in this one thread.
        pub static IS_GC_THREAD: Cell<bool> = Cell::new(false);
    }

    #[inline]
    pub fn is_gcing(&self) -> bool {
        Self::IS_GC_THREAD.with(|v| v.get())
    }

    #[cfg(feature = "threading")]
    fn loop_wait_with_warning(&self) -> Option<PyRwLockReadGuard<()>> {
        if Self::IS_GC_THREAD.with(|v| v.get()) {
            // if is same thread, then this thread is already stop by gc itself,
            // no need to block.
            // and any call to do_pausing is probably from drop() or what so allow it to continue execute.
            return None;
        }
        if self.cleanup_cycle.is_locked() {
            // the latest gc is not done, but the world can continue for collect known cycle doesn't require a static object graph?
            // this might prevent high freq gc call to pause the world forever?
            return None;
        }

        let mut gc_wait = 0;
        loop {
            gc_wait += 1;
            let res = self.pause.try_read_recursive_for(LOCK_TIMEOUT);
            match res {
                Some(res) => break Some(res),
                None => {
                    warn!(
                        "Wait GC lock for {} secs",
                        (gc_wait * LOCK_TIMEOUT).as_secs_f32()
                    );
                    if gc_wait == 12 {
                        // a minutes is too long for gc
                        warn!(
                            "GC Pause is too long: {} s",
                            (gc_wait * LOCK_TIMEOUT).as_secs_f32()
                        );
                        // deadlock is happening, better terminate program instead of waitting
                        std::process::exit(1);
                    }
                }
            }
        }
    }

    /// This function will block if is a garbage collect is happening
    pub fn do_pausing(&self) {
        // if there is no multi-thread, there is no need to pause,
        // for running different vm in different thread will make sure them have no depend whatsoever
        #[cfg(feature = "threading")]
        {
            // however when gc-ing the object graph must stay the same so check and try to lock until gc is done
            // timeout is to prevent dead lock(which is worse than panic?)

            let _lock = self.loop_wait_with_warning();
        }
        // when not threading, one could still run multiple vm on multiple thread(which have a GC per thread)
        // but when call `gc()`, it automatically pause the world for this thread.
        // so nothing need to be done, pausing is only useful for threading
    }

    /// similar to do_pausing,
    ///
    /// but instead return a ReadGuard for covering more critical region if needed
    pub fn try_pausing(&self) -> Option<PyRwLockReadGuard<()>> {
        #[cfg(feature = "threading")]
        {
            self.loop_wait_with_warning()
        }
        #[cfg(not(feature = "threading"))]
        return None;
    }
}

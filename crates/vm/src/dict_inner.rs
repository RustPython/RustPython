//! Ordered dictionary implementation.
//! Inspired by: <https://morepypy.blogspot.com/2015/01/faster-more-memory-efficient-and-more.html>
//! And: <https://www.youtube.com/watch?v=p33CVV29OG8>
//! And: <http://code.activestate.com/recipes/578375/>

use crate::{
    AsObject, Py, PyExact, PyObject, PyObjectRef, PyRefExact, PyResult, VirtualMachine,
    builtins::{PyBytes, PyInt, PyStr, PyStrInterned, PyStrRef, PyUtf8Str, PyUtf8StrRef},
    convert::ToPyObject,
};
use crate::{
    common::{
        hash,
        lock::{PyRwLock, PyRwLockReadGuard, PyRwLockWriteGuard},
        wtf8::{Wtf8, Wtf8Buf},
    },
    object::{Traverse, TraverseFn},
};
use alloc::fmt;
use core::mem::size_of;
use core::ops::ControlFlow;
use core::sync::atomic::{
    AtomicU32, AtomicU64,
    Ordering::{AcqRel, Acquire, Relaxed, Release},
};
use num_traits::ToPrimitive;

// HashIndex is intended to be same size with hash::PyHash
// but it doesn't mean the values are compatible with actual PyHash value

/// hash value of an object returned by __hash__
type HashValue = hash::PyHash;
/// index calculated by resolving collision
type HashIndex = hash::PyHash;
/// index into dict.indices
type IndexIndex = usize;
/// index into dict.entries
type EntryIndex = usize;

pub(crate) struct Dict<T = PyObjectRef> {
    inner: PyRwLock<DictInner<T>>,
    version: AtomicU64,
    /// Keys-version stamp, assigned lazily by `assign_keys_version` and
    /// reset to 0 whenever the key set changes. Value-only updates keep it.
    ///
    /// A nonzero stamp identifies either a *shape* — an exact hole-free
    /// entry sequence of interned string keys, shared by every dict with
    /// that layout — or, when no shape is derivable, this dict's key set
    /// frozen at assignment time. Either way a stamp match guarantees the
    /// entry layout is exactly the one the stamp was issued for, so entry
    /// indexes cached against a stamp stay valid wherever the stamp matches.
    keys_version: AtomicU32,
}

/// Source of keys-version stamps. Allocated globally so a shape stamp and a
/// dict-unique stamp can never collide.
static KEYS_VERSION: AtomicU32 = AtomicU32::new(0);

/// Allocate a new keys-version stamp. Returns 0 once the stamp space is
/// exhausted; stamps are only allocated on specialization, so exhaustion is
/// unrealistic in practice.
fn next_keys_version() -> u32 {
    KEYS_VERSION
        .fetch_update(Relaxed, Relaxed, |v| v.checked_add(1))
        .map_or(0, |v| v + 1)
}

/// Largest key count eligible for a shared shape stamp.
const SHAPE_MAX_KEYS: usize = 32;
/// Shape registry slot count (power of two).
const SHAPE_TABLE_SIZE: usize = 1 << 12;
/// Linear-probe limit before giving up on registering a shape.
const SHAPE_MAX_PROBE: usize = 8;

/// A registered shape: the ordered interned-key-pointer sequence of a
/// hole-free dict, plus the stamp shared by every dict with that layout.
/// Interned strings are never freed, so the addresses are stable identities.
struct ShapeData {
    keys: Box<[usize]>,
    stamp: u32,
}

/// Lock-free registry mapping shapes to shared stamps. Fixed-size open
/// addressing; slots are installed with CAS and never removed, so a stamp
/// permanently means "exactly this entry sequence". Registered `ShapeData`
/// is intentionally leaked (bounded by the table size). Lock-free makes the
/// registry safe across fork() without reinitialization.
static SHAPE_TABLE: std::sync::LazyLock<Box<[core::sync::atomic::AtomicPtr<ShapeData>]>> =
    std::sync::LazyLock::new(|| {
        (0..SHAPE_TABLE_SIZE)
            .map(|_| core::sync::atomic::AtomicPtr::new(core::ptr::null_mut()))
            .collect()
    });

fn shape_stamp(shape: &[usize]) -> Option<u32> {
    use core::sync::atomic::AtomicPtr;
    // The hasher seed must be process-stable so equal shapes always probe
    // the same slots.
    static SHAPE_HASHER: std::sync::LazyLock<rapidhash::quality::RandomState> =
        std::sync::LazyLock::new(Default::default);
    let hash = {
        use core::hash::{BuildHasher, Hash, Hasher};
        let mut hasher = SHAPE_HASHER.build_hasher();
        shape.hash(&mut hasher);
        hasher.finish() as usize
    };
    let mut candidate: *mut ShapeData = core::ptr::null_mut();
    let mut result = None;
    for probe in 0..SHAPE_MAX_PROBE {
        let slot: &AtomicPtr<ShapeData> = &SHAPE_TABLE[(hash + probe) & (SHAPE_TABLE_SIZE - 1)];
        let mut installed = slot.load(Acquire);
        if installed.is_null() {
            if candidate.is_null() {
                let stamp = next_keys_version();
                if stamp == 0 {
                    break;
                }
                candidate = Box::into_raw(Box::new(ShapeData {
                    keys: shape.into(),
                    stamp,
                }));
            }
            match slot.compare_exchange(core::ptr::null_mut(), candidate, AcqRel, Acquire) {
                Ok(_) => {
                    // SAFETY: candidate was just leaked into the table.
                    result = Some(unsafe { (*candidate).stamp });
                    candidate = core::ptr::null_mut();
                    break;
                }
                Err(current) => installed = current,
            }
        }
        // SAFETY: non-null slots reference leaked ShapeData, never freed.
        let data = unsafe { &*installed };
        if *data.keys == *shape {
            result = Some(data.stamp);
            break;
        }
    }
    if !candidate.is_null() {
        // SAFETY: the candidate lost the race and was never shared.
        drop(unsafe { Box::from_raw(candidate) });
    }
    result
}

unsafe impl<T: Traverse> Traverse for Dict<T> {
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        self.inner.traverse(tracer_fn);
    }
}

impl<T> fmt::Debug for Dict<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Debug").finish()
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(transparent)]
struct IndexEntry(i64);

impl IndexEntry {
    const FREE: Self = Self(-1);
    const DUMMY: Self = Self(-2);

    /// # Safety
    /// idx must not be one of FREE or DUMMY
    const unsafe fn from_index_unchecked(idx: usize) -> Self {
        debug_assert!((idx as isize) >= 0);
        Self(idx as i64)
    }

    const fn index(self) -> Option<usize> {
        if self.0 >= 0 {
            Some(self.0 as usize)
        } else {
            None
        }
    }
}

#[derive(Clone)]
struct DictInner<T> {
    used: usize,
    filled: usize,
    indices: Vec<IndexEntry>,
    entries: Vec<Option<DictEntry<T>>>,
}

unsafe impl<T: Traverse> Traverse for DictInner<T> {
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        self.entries
            .iter()
            .map(|v| {
                if let Some(v) = v {
                    v.key.traverse(tracer_fn);
                    v.value.traverse(tracer_fn);
                }
            })
            .count();
    }
}

impl<T: Clone> Clone for Dict<T> {
    fn clone(&self) -> Self {
        Self {
            inner: PyRwLock::new(self.inner.read().clone()),
            version: AtomicU64::new(0),
            keys_version: AtomicU32::new(0),
        }
    }
}

impl<T> Default for Dict<T> {
    fn default() -> Self {
        Self {
            inner: PyRwLock::new(DictInner {
                used: 0,
                filled: 0,
                indices: vec![IndexEntry::FREE; 8],
                entries: Vec::new(),
            }),
            version: AtomicU64::new(0),
            keys_version: AtomicU32::new(0),
        }
    }
}

#[derive(Clone)]
struct DictEntry<T> {
    hash: HashValue,
    key: PyObjectRef,
    index: IndexIndex,
    value: T,
}
static_assertions::assert_eq_size!(DictEntry<PyObjectRef>, Option<DictEntry<PyObjectRef>>);

#[derive(Debug, PartialEq, Eq)]
pub struct DictSize {
    indices_size: usize,
    pub entries_size: usize,
    pub used: usize,
    filled: usize,
}

struct GenIndexes {
    idx: HashIndex,
    perturb: HashValue,
    mask: HashIndex,
}

impl GenIndexes {
    const fn new(hash: HashValue, mask: HashIndex) -> Self {
        let hash = hash.abs();
        Self {
            idx: hash,
            perturb: hash,
            mask,
        }
    }

    const fn next(&mut self) -> usize {
        let prev = self.idx;
        self.idx = prev
            .wrapping_mul(5)
            .wrapping_add(self.perturb)
            .wrapping_add(1);
        self.perturb >>= 5;
        (prev & self.mask) as usize
    }
}

impl<T> DictInner<T> {
    fn resize(&mut self, new_size: usize) {
        let new_size = {
            let mut i = 1;
            while i < new_size {
                i <<= 1;
            }
            i
        };
        self.indices = vec![IndexEntry::FREE; new_size];
        let mask = (new_size - 1) as i64;
        for (entry_idx, entry) in self.entries.iter_mut().enumerate() {
            if let Some(entry) = entry {
                let mut idxs = GenIndexes::new(entry.hash, mask);
                loop {
                    let index_index = idxs.next();
                    unsafe {
                        // Safety: index is always valid here
                        // index_index is generated by idxs
                        // entry_idx is saved one
                        let idx = self.indices.get_unchecked_mut(index_index);
                        if *idx == IndexEntry::FREE {
                            *idx = IndexEntry::from_index_unchecked(entry_idx);
                            entry.index = index_index;
                            break;
                        }
                    }
                }
            } else {
                //removed entry
            }
        }
        self.filled = self.used;
    }

    fn unchecked_push(
        &mut self,
        index: IndexIndex,
        hash_value: HashValue,
        key: PyObjectRef,
        value: T,
        index_entry: IndexEntry,
    ) {
        let entry = DictEntry {
            hash: hash_value,
            key,
            value,
            index,
        };
        let entry_index = self.entries.len();
        self.entries.push(Some(entry));
        self.indices[index] = unsafe {
            // SAFETY: entry_index is self.entries.len(). it never can
            // grow to `usize-2` because hash tables cannot full its index
            IndexEntry::from_index_unchecked(entry_index)
        };
        self.used += 1;
        if let IndexEntry::FREE = index_entry {
            self.filled += 1;
            if let Some(new_size) = self.should_resize() {
                self.resize(new_size)
            }
        }
    }

    const fn size(&self) -> DictSize {
        DictSize {
            indices_size: self.indices.len(),
            entries_size: self.entries.len(),
            used: self.used,
            filled: self.filled,
        }
    }

    #[inline]
    const fn should_resize(&self) -> Option<usize> {
        if self.filled * 3 > self.indices.len() * 2 {
            Some(self.used * 2)
        } else {
            None
        }
    }

    #[inline]
    fn get_entry_checked(&self, idx: EntryIndex, index_index: IndexIndex) -> Option<&DictEntry<T>> {
        match self.entries.get(idx) {
            Some(Some(entry)) if entry.index == index_index => Some(entry),
            _ => None,
        }
    }
}

type PopInnerResult<T> = ControlFlow<Option<DictEntry<T>>>;

impl<T: Clone> Dict<T> {
    /// Monotonically increasing version counter for mutation tracking.
    pub(crate) fn version(&self) -> u64 {
        self.version.load(Acquire)
    }

    /// Bump the version counter after any mutation.
    fn bump_version(&self) {
        self.version.fetch_add(1, Release);
    }

    /// Current keys-version stamp, or 0 if none has been assigned since the
    /// last key-set change. Equal nonzero stamps guarantee an unchanged key
    /// set (values may differ).
    pub(crate) fn keys_version(&self) -> u32 {
        self.keys_version.load(Acquire)
    }

    /// Return the current keys-version stamp, assigning one if none is set.
    /// Returns 0 only if no stamp could be allocated.
    ///
    /// When the dict is hole-free and all keys are interned strings, the
    /// stamp is the *shared shape stamp* for that exact key sequence, so
    /// dicts with identical layouts (e.g. instances of the same class built
    /// by the same `__init__`) carry equal stamps and one cached stamp or
    /// entry index serves them all. Otherwise a dict-unique stamp is used.
    ///
    /// The shape inspection and the stamp install happen under the inner
    /// read lock. Key-set changes reset the stamp under the write lock, so
    /// an installed stamp always attests the layout it was derived from.
    pub(crate) fn assign_keys_version(&self) -> u32 {
        let version = self.keys_version.load(Acquire);
        if version != 0 {
            return version;
        }
        let inner = self.read();
        // Re-check under the lock: a concurrent assign may have won.
        let version = self.keys_version.load(Acquire);
        if version != 0 {
            return version;
        }
        let new_version = Self::derive_shape_stamp(&inner).unwrap_or_else(next_keys_version);
        if new_version == 0 {
            return 0;
        }
        // Only install over 0 so an already-valid stamp is never replaced.
        match self
            .keys_version
            .compare_exchange(0, new_version, AcqRel, Acquire)
        {
            Ok(_) => new_version,
            Err(current) => current,
        }
    }

    /// Compute the shared shape stamp for the current layout, if it
    /// qualifies: hole-free entries, bounded size, all keys interned strings.
    fn derive_shape_stamp(inner: &DictInner<T>) -> Option<u32> {
        if inner.entries.len() != inner.used || inner.used > SHAPE_MAX_KEYS {
            return None;
        }
        let shape = inner
            .entries
            .iter()
            .map(|entry| {
                let key = &entry.as_ref()?.key;
                key.is_interned()
                    .then(|| key.as_ref() as *const PyObject as usize)
            })
            .collect::<Option<Vec<usize>>>()?;
        shape_stamp(&shape)
    }

    /// Reset the keys-version stamp on a key-set change (insert of a new
    /// key, deletion, or clear). Value-only updates keep the stamp.
    ///
    /// Must be called while holding the write lock, *before* the key set is
    /// modified: a lock-free stamp reader that still observes the old stamp
    /// then provably ran before the change became visible, so acting on the
    /// old key set is linearizable. A stamp assigned concurrently (between
    /// this reset and the mutation) can only be trusted by a caller whose
    /// subsequent probe serializes after the mutation through the inner
    /// lock, which then reflects the new key set. Since stamps are never
    /// reused, a cached stamp can never spuriously match again.
    fn invalidate_keys_version(&self) {
        self.keys_version.store(0, Release);
    }

    fn read(&self) -> PyRwLockReadGuard<'_, DictInner<T>> {
        self.inner.read()
    }

    fn write(&self) -> PyRwLockWriteGuard<'_, DictInner<T>> {
        self.inner.write()
    }

    /// Store a key
    pub(crate) fn insert<K>(&self, vm: &VirtualMachine, key: &K, value: T) -> PyResult<()>
    where
        K: DictKey + ?Sized,
    {
        let hash = key.key_hash(vm)?;
        let _removed = loop {
            let (entry_index, index_index) = self.lookup(vm, key, hash, None)?;
            let mut inner = self.write();
            if let Some(index) = entry_index.index() {
                // Update existing key
                if let Some(entry) = inner.entries.get_mut(index) {
                    let Some(entry) = entry.as_mut() else {
                        // The dict was changed since we did lookup. Let's try again.
                        // this is very rare to happen
                        // (and seems only happen with very high freq gc, and about one time in 10000 iters)
                        // but still possible
                        continue;
                    };
                    #[expect(
                        clippy::redundant_else,
                        reason = "Keeping the empty `else` block here for documentation"
                    )]
                    if entry.index == index_index {
                        let removed = core::mem::replace(&mut entry.value, value);
                        self.bump_version();
                        // defer dec RC
                        break Some(removed);
                    } else {
                        // stuff shifted around, let's try again
                    }
                } else {
                    // The dict was changed since we did lookup. Let's try again.
                }
            } else {
                // New key - validate slot is still what lookup found
                if inner.indices.get(index_index) != Some(&entry_index) {
                    // Dict was resized since lookup, retry
                    continue;
                }
                self.invalidate_keys_version();
                inner.unchecked_push(index_index, hash, key.to_pyobject(vm), value, entry_index);
                self.bump_version();
                break None;
            }
        };
        Ok(())
    }

    pub(crate) fn contains<K: DictKey + ?Sized>(
        &self,
        vm: &VirtualMachine,
        key: &K,
    ) -> PyResult<bool> {
        let key_hash = key.key_hash(vm)?;
        let (entry, _) = self.lookup(vm, key, key_hash, None)?;
        Ok(entry.index().is_some())
    }

    /// Retrieve a key
    #[cfg_attr(feature = "flame-it", flame("Dict"))]
    pub(crate) fn get<K: DictKey + ?Sized>(
        &self,
        vm: &VirtualMachine,
        key: &K,
    ) -> PyResult<Option<T>> {
        let hash = key.key_hash(vm)?;
        self._get_inner(vm, key, hash)
    }

    /// Return a stable entry hint for `key` if present.
    ///
    /// The hint is the internal entry index and can be used with
    /// [`Self::get_hint`]. It is invalidated by dict mutations.
    pub(crate) fn hint_for_key<K: DictKey + ?Sized>(
        &self,
        vm: &VirtualMachine,
        key: &K,
    ) -> PyResult<Option<u16>> {
        let hash = key.key_hash(vm)?;
        let (entry, _) = self.lookup(vm, key, hash, None)?;
        let Some(index) = entry.index() else {
            return Ok(None);
        };
        Ok(u16::try_from(index).ok())
    }

    /// Retrieve a key along with its entry index, for hint caching.
    ///
    /// Same as [`Self::get`], but on a hit also returns the entry index
    /// usable as a `hint` for [`Self::get_hint`] (`None` if it doesn't fit).
    pub(crate) fn get_with_hint<K: DictKey + ?Sized>(
        &self,
        vm: &VirtualMachine,
        key: &K,
    ) -> PyResult<Option<(T, Option<u16>)>> {
        let hash = key.key_hash(vm)?;
        let ret = loop {
            let (entry, index_index) = self.lookup(vm, key, hash, None)?;
            if let Some(index) = entry.index() {
                let inner = self.read();
                if let Some(entry) = inner.get_entry_checked(index, index_index) {
                    // The dict was not changed since we did lookup
                    break Some((entry.value.clone(), u16::try_from(index).ok()));
                }
                // The dict was changed since we did lookup. Let's try again.
            } else {
                break None;
            }
        };
        Ok(ret)
    }

    /// Replace the value at entry index `hint` if that entry's key is
    /// identical to `key`, otherwise fall back to a full probing store.
    ///
    /// On a hint miss, returns a refreshed hint for the key (`None` when the
    /// hint hit or no hint is representable).
    pub(crate) fn insert_with_hint<K: DictKey + ?Sized>(
        &self,
        vm: &VirtualMachine,
        key: &K,
        hint: usize,
        value: T,
    ) -> PyResult<Option<u16>> {
        let value = {
            let mut inner = self.write();
            match inner.entries.get_mut(hint) {
                Some(Some(entry)) if key.key_is(&entry.key) => {
                    let removed = core::mem::replace(&mut entry.value, value);
                    self.bump_version();
                    drop(inner);
                    // defer dec RC until after the lock is released
                    drop(removed);
                    return Ok(None);
                }
                _ => value,
            }
        };
        self.insert(vm, key, value)?;
        self.hint_for_key(vm, key)
    }

    /// Fast path lookup using a cached entry index (`hint`).
    ///
    /// Returns `None` if the hint is stale or the key no longer matches.
    pub(crate) fn get_hint<K: DictKey + ?Sized>(
        &self,
        vm: &VirtualMachine,
        key: &K,
        hint: usize,
    ) -> PyResult<Option<T>> {
        let (entry_key, entry_value) = {
            let inner = self.read();
            let Some(Some(entry)) = inner.entries.get(hint) else {
                return Ok(None);
            };
            if key.key_is(&entry.key) {
                return Ok(Some(entry.value.clone()));
            }
            (entry.key.clone(), entry.value.clone())
        };
        // key_eq may run Python __eq__, so must be outside the lock.
        if key.key_eq(vm, &entry_key)? {
            Ok(Some(entry_value))
        } else {
            Ok(None)
        }
    }

    fn _get_inner<K: DictKey + ?Sized>(
        &self,
        vm: &VirtualMachine,
        key: &K,
        hash: HashValue,
    ) -> PyResult<Option<T>> {
        let ret = loop {
            let (entry, index_index) = self.lookup(vm, key, hash, None)?;
            if let Some(index) = entry.index() {
                let inner = self.read();
                if let Some(entry) = inner.get_entry_checked(index, index_index) {
                    // The dict was not changed since we did lookup
                    break Some(entry.value.clone());
                }

                // The dict was changed since we did lookup. Let's try again.
            } else {
                break None;
            }
        };
        Ok(ret)
    }

    pub(crate) fn get_chain<K: DictKey + ?Sized>(
        &self,
        other: &Self,
        vm: &VirtualMachine,
        key: &K,
    ) -> PyResult<Option<T>> {
        let hash = key.key_hash(vm)?;
        if let Some(x) = self._get_inner(vm, key, hash)? {
            Ok(Some(x))
        } else {
            other._get_inner(vm, key, hash)
        }
    }

    pub(crate) fn clear(&self) {
        let _removed = {
            let mut inner = self.write();
            self.invalidate_keys_version();
            inner.indices.clear();
            inner.indices.resize(8, IndexEntry::FREE);
            inner.used = 0;
            inner.filled = 0;
            self.bump_version();
            // defer dec rc
            core::mem::take(&mut inner.entries)
        };
    }

    /// Delete a key
    pub(crate) fn delete<K>(&self, vm: &VirtualMachine, key: &K) -> PyResult<()>
    where
        K: DictKey + ?Sized,
    {
        if self.remove_if_exists(vm, key)?.is_some() {
            Ok(())
        } else {
            Err(vm.new_key_error(key.to_pyobject(vm)))
        }
    }

    pub(crate) fn delete_if_exists<K>(&self, vm: &VirtualMachine, key: &K) -> PyResult<bool>
    where
        K: DictKey + ?Sized,
    {
        self.remove_if_exists(vm, key).map(|opt| opt.is_some())
    }

    pub(crate) fn delete_if<K, F>(&self, vm: &VirtualMachine, key: &K, pred: F) -> PyResult<bool>
    where
        K: DictKey + ?Sized,
        F: Fn(&T) -> PyResult<bool>,
    {
        self.remove_if(vm, key, pred).map(|opt| opt.is_some())
    }

    pub(crate) fn remove_if_exists<K>(&self, vm: &VirtualMachine, key: &K) -> PyResult<Option<T>>
    where
        K: DictKey + ?Sized,
    {
        self.remove_if(vm, key, |_| Ok(true))
    }

    /// pred should be VERY CAREFUL about what it does as it is called while
    /// the dict's internal mutex is held
    pub(crate) fn remove_if<K, F>(
        &self,
        vm: &VirtualMachine,
        key: &K,
        pred: F,
    ) -> PyResult<Option<T>>
    where
        K: DictKey + ?Sized,
        F: Fn(&T) -> PyResult<bool>,
    {
        let hash = key.key_hash(vm)?;
        let removed = loop {
            let lookup = self.lookup(vm, key, hash, None)?;
            match self.pop_inner_if(lookup, &pred)? {
                ControlFlow::Break(entry) => break entry,
                ControlFlow::Continue(()) => continue,
            }
        };
        Ok(removed.map(|entry| entry.value))
    }

    pub(crate) fn delete_or_insert(
        &self,
        vm: &VirtualMachine,
        key: &PyObject,
        value: T,
    ) -> PyResult<()> {
        let hash = key.key_hash(vm)?;
        let _removed = loop {
            let lookup = self.lookup(vm, key, hash, None)?;
            let (entry, index_index) = lookup;
            if entry.index().is_some() {
                match self.pop_inner(lookup) {
                    ControlFlow::Break(Some(entry)) => break Some(entry),
                    _ => continue,
                }
            }

            let mut inner = self.write();
            if inner.indices.get(index_index) != Some(&entry) {
                continue;
            }
            self.invalidate_keys_version();
            inner.unchecked_push(index_index, hash, key.to_owned(), value, entry);
            self.bump_version();
            break None;
        };
        Ok(())
    }

    pub(crate) fn setdefault<K, F>(&self, vm: &VirtualMachine, key: &K, default: F) -> PyResult<T>
    where
        K: DictKey + ?Sized,
        F: FnOnce() -> T,
    {
        let hash = key.key_hash(vm)?;
        let mut default = Some(default);
        loop {
            let (index_entry, index_index) = self.lookup(vm, key, hash, None)?;
            if let Some(index) = index_entry.index() {
                let inner = self.read();
                if let Some(entry) = inner.get_entry_checked(index, index_index) {
                    return Ok(entry.value.clone());
                }
                continue;
            }
            let mut inner = self.write();
            if inner.indices.get(index_index) != Some(&index_entry) {
                continue;
            }
            let value = default
                .take()
                .expect("default must only be computed on insertion")();
            self.invalidate_keys_version();
            inner.unchecked_push(
                index_index,
                hash,
                key.to_pyobject(vm),
                value.clone(),
                index_entry,
            );
            self.bump_version();
            return Ok(value);
        }
    }

    #[allow(dead_code)]
    pub(crate) fn setdefault_entry<K, F>(
        &self,
        vm: &VirtualMachine,
        key: &K,
        default: F,
    ) -> PyResult<(PyObjectRef, T)>
    where
        K: DictKey + ?Sized,
        F: FnOnce() -> T,
    {
        let hash = key.key_hash(vm)?;
        let mut default = Some(default);
        loop {
            let (index_entry, index_index) = self.lookup(vm, key, hash, None)?;
            if let Some(index) = index_entry.index() {
                let inner = self.read();
                if let Some(entry) = inner.get_entry_checked(index, index_index) {
                    return Ok((entry.key.clone(), entry.value.clone()));
                }
                continue;
            }
            let mut inner = self.write();
            if inner.indices.get(index_index) != Some(&index_entry) {
                continue;
            }
            let value = default
                .take()
                .expect("default must only be computed on insertion")();
            let key_obj = key.to_pyobject(vm);
            let ret = (key_obj.clone(), value.clone());
            self.invalidate_keys_version();
            inner.unchecked_push(index_index, hash, key_obj, value, index_entry);
            self.bump_version();
            return Ok(ret);
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.read().used
    }

    pub(crate) fn size(&self) -> DictSize {
        self.read().size()
    }

    pub(crate) fn next_entry(&self, mut position: EntryIndex) -> Option<(usize, PyObjectRef, T)> {
        let inner = self.read();
        loop {
            let entry = inner.entries.get(position)?;
            position += 1;
            if let Some(entry) = entry {
                break Some((position, entry.key.clone(), entry.value.clone()));
            }
        }
    }

    pub(crate) fn prev_entry(&self, mut position: EntryIndex) -> Option<(usize, PyObjectRef, T)> {
        let inner = self.read();
        loop {
            let entry = inner.entries.get(position)?;
            position = position.saturating_sub(1);
            if let Some(entry) = entry {
                break Some((position, entry.key.clone(), entry.value.clone()));
            }
        }
    }

    pub(crate) fn len_from_entry_index(&self, position: EntryIndex) -> usize {
        self.read().entries.len().saturating_sub(position)
    }

    pub(crate) fn has_changed_size(&self, old: &DictSize) -> bool {
        let current = self.read().size();
        current != *old
    }

    pub(crate) fn keys(&self) -> Vec<PyObjectRef> {
        self.read()
            .entries
            .iter()
            .filter_map(|v| v.as_ref().map(|v| v.key.clone()))
            .collect()
    }

    pub(crate) fn values(&self) -> Vec<T> {
        self.read()
            .entries
            .iter()
            .filter_map(|v| v.as_ref().map(|v| v.value.clone()))
            .collect()
    }

    pub(crate) fn items(&self) -> Vec<(PyObjectRef, T)> {
        self.read()
            .entries
            .iter()
            .filter_map(|v| v.as_ref().map(|v| (v.key.clone(), v.value.clone())))
            .collect()
    }

    pub(crate) fn try_fold_keys<Acc, Fold>(&self, init: Acc, f: Fold) -> PyResult<Acc>
    where
        Fold: FnMut(Acc, &PyObject) -> PyResult<Acc>,
    {
        self.read()
            .entries
            .iter()
            .filter_map(|v| v.as_ref().map(|v| v.key.as_object()))
            .try_fold(init, f)
    }

    /// Lookup the index for the given key.
    #[cfg_attr(feature = "flame-it", flame("Dict"))]
    fn lookup<K: DictKey + ?Sized>(
        &self,
        vm: &VirtualMachine,
        key: &K,
        hash_value: HashValue,
        mut lock: Option<PyRwLockReadGuard<'_, DictInner<T>>>,
    ) -> PyResult<LookupResult> {
        let mut idxs = None;
        let mut free_slot = None;
        let ret = 'outer: loop {
            let (entry_key, ret) = {
                let inner = lock.take().unwrap_or_else(|| self.read());
                let mask = (inner.indices.len() - 1) as i64;
                let idxs = idxs.get_or_insert_with(|| GenIndexes::new(hash_value, mask));
                if idxs.mask != mask {
                    // Dict was resized since last probe, restart
                    *idxs = GenIndexes::new(hash_value, mask);
                    free_slot = None;
                }
                loop {
                    let index_index = idxs.next();
                    let index_entry = *unsafe {
                        // Safety: index_index is generated
                        inner.indices.get_unchecked(index_index)
                    };
                    match index_entry {
                        IndexEntry::DUMMY => {
                            if free_slot.is_none() {
                                free_slot = Some(index_index);
                            }
                        }
                        IndexEntry::FREE => {
                            let idxs = match free_slot {
                                Some(free) => (IndexEntry::DUMMY, free),
                                None => (IndexEntry::FREE, index_index),
                            };
                            return Ok(idxs);
                        }
                        idx => {
                            let entry = unsafe {
                                // Safety: DUMMY and FREE are already handled above.
                                // i is always valid and entry always exists.
                                let i = idx.index().unwrap_unchecked();
                                inner.entries.get_unchecked(i).as_ref().unwrap_unchecked()
                            };
                            let ret = (idx, index_index);

                            #[expect(
                                clippy::redundant_else,
                                reason = "Keeping the empty `else` block here for documentation"
                            )]
                            if key.key_is(&entry.key) {
                                break 'outer ret;
                            } else if entry.hash == hash_value {
                                break (entry.key.clone(), ret);
                            } else {
                                // entry mismatch
                            }
                        }
                    }
                    // warn!("Perturb value: {}", i);
                }
            };

            #[expect(
                clippy::redundant_else,
                reason = "Keeping the empty `else` block here for documentation"
            )]
            // This comparison needs to be done outside the lock.
            if key.key_eq(vm, &entry_key)? {
                break 'outer ret;
            } else {
                // hash collision
            }

            // warn!("Perturb value: {}", i);
        };
        Ok(ret)
    }

    // returns Err(()) if changed since lookup
    fn pop_inner(&self, lookup: LookupResult) -> PopInnerResult<T> {
        self.pop_inner_if(lookup, |_| Ok::<_, core::convert::Infallible>(true))
            .unwrap_or_else(|x| match x {})
    }

    fn pop_inner_if<E>(
        &self,
        lookup: LookupResult,
        pred: impl Fn(&T) -> Result<bool, E>,
    ) -> Result<PopInnerResult<T>, E> {
        let (entry_index, index_index) = lookup;
        let Some(entry_index) = entry_index.index() else {
            return Ok(ControlFlow::Break(None));
        };
        let inner = &mut *self.write();
        let slot = if let Some(slot) = inner.entries.get_mut(entry_index) {
            slot
        } else {
            // The dict was changed since we did lookup. Let's try again.
            return Ok(ControlFlow::Continue(()));
        };
        match slot {
            Some(entry) if entry.index == index_index => {
                if !pred(&entry.value)? {
                    return Ok(ControlFlow::Break(None));
                }
            }
            // The dict was changed since we did lookup. Let's try again.
            _ => return Ok(ControlFlow::Continue(())),
        }
        self.invalidate_keys_version();
        *unsafe {
            // index_index is result of lookup
            inner.indices.get_unchecked_mut(index_index)
        } = IndexEntry::DUMMY;
        inner.used -= 1;
        let removed = slot.take();
        self.bump_version();
        Ok(ControlFlow::Break(removed))
    }

    /// Retrieve and delete a key
    pub(crate) fn pop<K: DictKey + ?Sized>(
        &self,
        vm: &VirtualMachine,
        key: &K,
    ) -> PyResult<Option<T>> {
        let hash_value = key.key_hash(vm)?;
        let removed = loop {
            let lookup = self.lookup(vm, key, hash_value, None)?;
            match self.pop_inner(lookup) {
                ControlFlow::Break(entry) => break entry.map(|e| e.value),
                ControlFlow::Continue(()) => continue,
            }
        };
        Ok(removed)
    }

    pub(crate) fn pop_back(&self) -> Option<(PyObjectRef, T)> {
        let inner = &mut *self.write();
        let entry = loop {
            let entry = inner.entries.pop()?;
            if let Some(entry) = entry {
                break entry;
            }
        };
        self.invalidate_keys_version();
        inner.used -= 1;
        *unsafe {
            // entry.index always refers valid index
            inner.indices.get_unchecked_mut(entry.index)
        } = IndexEntry::DUMMY;
        self.bump_version();
        Some((entry.key, entry.value))
    }

    pub(crate) fn sizeof(&self) -> usize {
        let inner = self.read();
        size_of::<Self>()
            + size_of::<DictInner<T>>()
            + inner.indices.len() * size_of::<i64>()
            + inner.entries.len() * size_of::<DictEntry<T>>()
    }

    /// Pop all entries from the dict, returning (key, value) pairs.
    /// This is used for circular reference resolution in GC.
    /// Requires &mut self to avoid lock contention.
    pub(crate) fn drain_entries(&mut self) -> impl Iterator<Item = (PyObjectRef, T)> + '_ {
        self.keys_version.store(0, Release);
        let inner = self.inner.get_mut();
        inner.used = 0;
        inner.filled = 0;
        inner.indices.iter_mut().for_each(|i| *i = IndexEntry::FREE);
        inner.entries.drain(..).flatten().map(|e| (e.key, e.value))
    }
}

type LookupResult = (IndexEntry, IndexIndex);

/// Types implementing this trait can be used to index
/// the dictionary. Typical use-cases are:
/// - PyObjectRef -> arbitrary python type used as key
/// - str -> string reference used as key, this is often used internally
pub trait DictKey {
    type Owned: ToPyObject;
    fn _to_owned(&self, vm: &VirtualMachine) -> Self::Owned;
    fn to_pyobject(&self, vm: &VirtualMachine) -> PyObjectRef {
        self._to_owned(vm).to_pyobject(vm)
    }
    fn key_hash(&self, vm: &VirtualMachine) -> PyResult<HashValue>;
    fn key_is(&self, other: &PyObject) -> bool;
    fn key_eq(&self, vm: &VirtualMachine, other_key: &PyObject) -> PyResult<bool>;
    fn key_as_isize(&self, vm: &VirtualMachine) -> PyResult<isize>;
}

/// Implement trait for PyObjectRef such that we can use python objects
/// to index dictionaries.
impl DictKey for PyObject {
    type Owned = PyObjectRef;
    #[inline(always)]
    fn _to_owned(&self, _vm: &VirtualMachine) -> Self::Owned {
        self.to_owned()
    }

    #[inline(always)]
    fn key_hash(&self, vm: &VirtualMachine) -> PyResult<HashValue> {
        self.hash(vm)
    }

    #[inline(always)]
    fn key_is(&self, other: &PyObject) -> bool {
        self.is(other)
    }

    #[inline(always)]
    fn key_eq(&self, vm: &VirtualMachine, other_key: &PyObject) -> PyResult<bool> {
        vm.identical_or_equal(self, other_key)
    }

    #[inline]
    fn key_as_isize(&self, vm: &VirtualMachine) -> PyResult<isize> {
        self.try_index(vm)?.try_to_primitive(vm)
    }
}

impl DictKey for Py<PyStr> {
    type Owned = PyStrRef;
    #[inline(always)]
    fn _to_owned(&self, _vm: &VirtualMachine) -> Self::Owned {
        self.to_owned()
    }

    #[inline]
    fn key_hash(&self, vm: &VirtualMachine) -> PyResult<HashValue> {
        Ok(self.hash(vm))
    }

    #[inline(always)]
    fn key_is(&self, other: &PyObject) -> bool {
        self.is(other)
    }

    fn key_eq(&self, vm: &VirtualMachine, other_key: &PyObject) -> PyResult<bool> {
        if self.is(other_key) {
            Ok(true)
        } else if let Some(pystr) = other_key.downcast_ref_if_exact::<PyStr>(vm) {
            Ok(self.as_wtf8() == pystr.as_wtf8())
        } else {
            vm.bool_eq(self.as_object(), other_key)
        }
    }

    #[inline(always)]
    fn key_as_isize(&self, vm: &VirtualMachine) -> PyResult<isize> {
        self.as_object().key_as_isize(vm)
    }
}

impl DictKey for Py<PyUtf8Str> {
    type Owned = PyUtf8StrRef;
    #[inline(always)]
    fn _to_owned(&self, _vm: &VirtualMachine) -> Self::Owned {
        self.to_owned()
    }

    #[inline]
    fn key_hash(&self, vm: &VirtualMachine) -> PyResult<HashValue> {
        self.as_pystr().key_hash(vm)
    }

    #[inline(always)]
    fn key_is(&self, other: &PyObject) -> bool {
        self.as_pystr().key_is(other)
    }

    fn key_eq(&self, vm: &VirtualMachine, other_key: &PyObject) -> PyResult<bool> {
        self.as_pystr().key_eq(vm, other_key)
    }

    #[inline(always)]
    fn key_as_isize(&self, vm: &VirtualMachine) -> PyResult<isize> {
        self.as_pystr().key_as_isize(vm)
    }
}

impl DictKey for PyStrInterned {
    type Owned = PyRefExact<PyStr>;

    #[inline]
    fn _to_owned(&self, _vm: &VirtualMachine) -> Self::Owned {
        let zelf: &'static Self = unsafe { &*(self as *const _) };
        zelf.to_exact()
    }

    #[inline]
    fn key_hash(&self, vm: &VirtualMachine) -> PyResult<HashValue> {
        (**self).key_hash(vm)
    }

    #[inline]
    fn key_is(&self, other: &PyObject) -> bool {
        (**self).key_is(other)
    }

    #[inline]
    fn key_eq(&self, vm: &VirtualMachine, other_key: &PyObject) -> PyResult<bool> {
        (**self).key_eq(vm, other_key)
    }

    #[inline]
    fn key_as_isize(&self, vm: &VirtualMachine) -> PyResult<isize> {
        (**self).key_as_isize(vm)
    }
}

impl DictKey for PyExact<PyStr> {
    type Owned = PyRefExact<PyStr>;

    #[inline]
    fn _to_owned(&self, _vm: &VirtualMachine) -> Self::Owned {
        self.to_owned()
    }

    #[inline(always)]
    fn key_hash(&self, vm: &VirtualMachine) -> PyResult<HashValue> {
        (**self).key_hash(vm)
    }

    #[inline(always)]
    fn key_is(&self, other: &PyObject) -> bool {
        (**self).key_is(other)
    }

    #[inline(always)]
    fn key_eq(&self, vm: &VirtualMachine, other_key: &PyObject) -> PyResult<bool> {
        (**self).key_eq(vm, other_key)
    }

    #[inline(always)]
    fn key_as_isize(&self, vm: &VirtualMachine) -> PyResult<isize> {
        (**self).key_as_isize(vm)
    }
}

// AsRef<str> fit this case but not possible in rust 1.46

/// Implement trait for the str type, so that we can use strings
/// to index dictionaries.
impl DictKey for str {
    type Owned = String;

    #[inline(always)]
    fn _to_owned(&self, _vm: &VirtualMachine) -> Self::Owned {
        self.to_owned()
    }

    #[inline]
    fn key_hash(&self, vm: &VirtualMachine) -> PyResult<HashValue> {
        // follow a similar route as the hashing of PyStrRef
        Ok(vm.state.hash_secret.hash_str(self))
    }

    #[inline(always)]
    fn key_is(&self, _other: &PyObject) -> bool {
        // No matter who the other pyobject is, we are never the same thing, since
        // we are a str, not a pyobject.
        false
    }

    fn key_eq(&self, vm: &VirtualMachine, other_key: &PyObject) -> PyResult<bool> {
        if let Some(pystr) = other_key.downcast_ref_if_exact::<PyStr>(vm) {
            Ok(pystr.as_wtf8() == self)
        } else {
            // Fall back to PyObjectRef implementation.
            let s = vm.ctx.new_str(self);
            s.key_eq(vm, other_key)
        }
    }

    fn key_as_isize(&self, vm: &VirtualMachine) -> PyResult<isize> {
        Err(vm.new_type_error("'str' object cannot be interpreted as an integer"))
    }
}

impl DictKey for String {
    type Owned = Self;

    #[inline]
    fn _to_owned(&self, _vm: &VirtualMachine) -> Self::Owned {
        self.clone()
    }

    fn key_hash(&self, vm: &VirtualMachine) -> PyResult<HashValue> {
        self.as_str().key_hash(vm)
    }

    fn key_is(&self, other: &PyObject) -> bool {
        self.as_str().key_is(other)
    }

    fn key_eq(&self, vm: &VirtualMachine, other_key: &PyObject) -> PyResult<bool> {
        self.as_str().key_eq(vm, other_key)
    }

    fn key_as_isize(&self, vm: &VirtualMachine) -> PyResult<isize> {
        self.as_str().key_as_isize(vm)
    }
}

impl DictKey for Wtf8 {
    type Owned = Wtf8Buf;

    #[inline(always)]
    fn _to_owned(&self, _vm: &VirtualMachine) -> Self::Owned {
        self.to_owned()
    }

    #[inline]
    fn key_hash(&self, vm: &VirtualMachine) -> PyResult<HashValue> {
        // follow a similar route as the hashing of PyStrRef
        Ok(vm.state.hash_secret.hash_bytes(self.as_bytes()))
    }

    #[inline(always)]
    fn key_is(&self, _other: &PyObject) -> bool {
        // No matter who the other pyobject is, we are never the same thing, since
        // we are a str, not a pyobject.
        false
    }

    fn key_eq(&self, vm: &VirtualMachine, other_key: &PyObject) -> PyResult<bool> {
        if let Some(pystr) = other_key.downcast_ref_if_exact::<PyStr>(vm) {
            Ok(pystr.as_wtf8() == self)
        } else {
            // Fall back to PyObjectRef implementation.
            let s = vm.ctx.new_str(self);
            s.key_eq(vm, other_key)
        }
    }

    fn key_as_isize(&self, vm: &VirtualMachine) -> PyResult<isize> {
        Err(vm.new_type_error("'str' object cannot be interpreted as an integer"))
    }
}

impl DictKey for Wtf8Buf {
    type Owned = Self;

    #[inline]
    fn _to_owned(&self, _vm: &VirtualMachine) -> Self::Owned {
        self.clone()
    }

    fn key_hash(&self, vm: &VirtualMachine) -> PyResult<HashValue> {
        (**self).key_hash(vm)
    }

    fn key_is(&self, other: &PyObject) -> bool {
        (**self).key_is(other)
    }

    fn key_eq(&self, vm: &VirtualMachine, other_key: &PyObject) -> PyResult<bool> {
        (**self).key_eq(vm, other_key)
    }

    fn key_as_isize(&self, vm: &VirtualMachine) -> PyResult<isize> {
        (**self).key_as_isize(vm)
    }
}

impl DictKey for [u8] {
    type Owned = Vec<u8>;

    #[inline(always)]
    fn _to_owned(&self, _vm: &VirtualMachine) -> Self::Owned {
        self.to_owned()
    }

    #[inline]
    fn key_hash(&self, vm: &VirtualMachine) -> PyResult<HashValue> {
        // follow a similar route as the hashing of PyStrRef
        Ok(vm.state.hash_secret.hash_bytes(self))
    }

    #[inline(always)]
    fn key_is(&self, _other: &PyObject) -> bool {
        // No matter who the other pyobject is, we are never the same thing, since
        // we are a str, not a pyobject.
        false
    }

    fn key_eq(&self, vm: &VirtualMachine, other_key: &PyObject) -> PyResult<bool> {
        if let Some(pystr) = other_key.downcast_ref_if_exact::<PyBytes>(vm) {
            Ok(pystr.as_bytes() == self)
        } else {
            // Fall back to PyObjectRef implementation.
            let s = vm.ctx.new_bytes(self.to_vec());
            s.key_eq(vm, other_key)
        }
    }

    fn key_as_isize(&self, vm: &VirtualMachine) -> PyResult<isize> {
        Err(vm.new_type_error("'str' object cannot be interpreted as an integer"))
    }
}

impl DictKey for Vec<u8> {
    type Owned = Self;

    #[inline]
    fn _to_owned(&self, _vm: &VirtualMachine) -> Self::Owned {
        self.clone()
    }

    fn key_hash(&self, vm: &VirtualMachine) -> PyResult<HashValue> {
        self.as_slice().key_hash(vm)
    }

    fn key_is(&self, other: &PyObject) -> bool {
        self.as_slice().key_is(other)
    }

    fn key_eq(&self, vm: &VirtualMachine, other_key: &PyObject) -> PyResult<bool> {
        self.as_slice().key_eq(vm, other_key)
    }

    fn key_as_isize(&self, vm: &VirtualMachine) -> PyResult<isize> {
        self.as_slice().key_as_isize(vm)
    }
}

impl DictKey for usize {
    type Owned = Self;

    #[inline]
    fn _to_owned(&self, _vm: &VirtualMachine) -> Self::Owned {
        *self
    }

    fn key_hash(&self, _vm: &VirtualMachine) -> PyResult<HashValue> {
        Ok(hash::hash_usize(*self))
    }

    fn key_is(&self, _other: &PyObject) -> bool {
        false
    }

    fn key_eq(&self, vm: &VirtualMachine, other_key: &PyObject) -> PyResult<bool> {
        if let Some(int) = other_key.downcast_ref_if_exact::<PyInt>(vm) {
            if let Some(i) = int.as_bigint().to_usize() {
                Ok(i == *self)
            } else {
                Ok(false)
            }
        } else {
            let int = vm.ctx.new_int(*self);
            vm.bool_eq(int.as_ref(), other_key)
        }
    }

    fn key_as_isize(&self, _vm: &VirtualMachine) -> PyResult<isize> {
        Ok(*self as isize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Interpreter, common::ascii};

    #[test]
    fn insert_basic() {
        Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let dict = Dict::default();
            assert_eq!(0, dict.len());

            let key1 = vm.new_pyobj(true);
            let value1 = vm.new_pyobj(ascii!("abc"));
            dict.insert(vm, &*key1, value1).unwrap();
            assert_eq!(1, dict.len());

            let key2 = vm.new_pyobj(ascii!("x"));
            let value2 = vm.new_pyobj(ascii!("def"));
            dict.insert(vm, &*key2, value2.clone()).unwrap();
            assert_eq!(2, dict.len());

            dict.insert(vm, &*key1, value2.clone()).unwrap();
            assert_eq!(2, dict.len());

            dict.delete(vm, &*key1).unwrap();
            assert_eq!(1, dict.len());

            dict.insert(vm, &*key1, value2.clone()).unwrap();
            assert_eq!(2, dict.len());

            assert!(dict.contains(vm, &*key1).unwrap());
            assert!(dict.contains(vm, "x").unwrap());

            let val = dict.get(vm, "x").unwrap().unwrap();
            vm.bool_eq(&val, &value2)
                .expect("retrieved value must be equal to inserted value.");
        })
    }

    macro_rules! hash_tests {
        ($($name:ident: $example_hash:expr,)*) => {
            $(
                #[test]
                fn $name() {
                    check_hash_equivalence($example_hash);
                }
            )*
        }
    }

    hash_tests! {
        abc: "abc",
        x: "x",
    }

    fn check_hash_equivalence(text: &str) {
        Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let value1 = text;
            let value2 = vm.new_pyobj(value1.to_owned());

            let hash1 = value1.key_hash(vm).expect("Hash should not fail.");
            let hash2 = value2.key_hash(vm).expect("Hash should not fail.");
            assert_eq!(hash1, hash2);
        })
    }
}

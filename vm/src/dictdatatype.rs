/// Ordered dictionary implementation.
/// Inspired by: https://morepypy.blogspot.com/2015/01/faster-more-memory-efficient-and-more.html
/// And: https://www.youtube.com/watch?v=p33CVV29OG8
/// And: http://code.activestate.com/recipes/578375/
use crate::common::cell::{PyRwLock, PyRwLockReadGuard, PyRwLockWriteGuard};
use crate::obj::objstr::{PyString, PyStringRef};
use crate::pyobject::{BorrowValue, IdProtocol, IntoPyObject, PyObjectRef, PyResult};
use crate::vm::VirtualMachine;
use rustpython_common::hash;
use std::collections::HashMap;
use std::mem::size_of;

// HashIndex is intended to be same size with hash::PyHash
// but it doesn't mean the values are compatible with actual pyhash value

/// hash value of an object returned by __hash__
type HashValue = hash::PyHash;
/// index calculated by resolving collision
type HashIndex = hash::PyHash;
/// entry index mapped in indices
type EntryIndex = usize;

pub struct Dict<T = PyObjectRef> {
    inner: PyRwLock<DictInner<T>>,
}

struct DictInner<T> {
    size: usize,
    indices: HashMap<HashIndex, EntryIndex>,
    entries: Vec<Option<DictEntry<T>>>,
}

impl<T: Clone> Clone for DictInner<T> {
    fn clone(&self) -> Self {
        DictInner {
            size: self.size,
            indices: self.indices.clone(),
            entries: self.entries.clone(),
        }
    }
}

impl<T: Clone> Clone for Dict<T> {
    fn clone(&self) -> Self {
        Dict {
            inner: PyRwLock::new(self.inner.read().clone()),
        }
    }
}

impl<T> Default for Dict<T> {
    fn default() -> Self {
        Dict {
            inner: PyRwLock::new(DictInner {
                size: 0,
                indices: HashMap::new(),
                entries: Vec::new(),
            }),
        }
    }
}

struct DictEntry<T> {
    hash: HashValue,
    key: PyObjectRef,
    value: T,
}

impl<T: Clone> Clone for DictEntry<T> {
    fn clone(&self) -> Self {
        DictEntry {
            hash: self.hash,
            key: self.key.clone(),
            value: self.value.clone(),
        }
    }
}

#[derive(Debug)]
pub struct DictSize {
    size: usize,
    entries_size: usize,
}

impl<T: Clone> Dict<T> {
    fn borrow_value(&self) -> PyRwLockReadGuard<'_, DictInner<T>> {
        self.inner.read()
    }

    fn borrow_value_mut(&self) -> PyRwLockWriteGuard<'_, DictInner<T>> {
        self.inner.write()
    }

    fn resize(&self) {
        let mut inner = self.borrow_value_mut();
        let mut new_indices = HashMap::with_capacity(inner.size);
        let mut new_entries = Vec::with_capacity(inner.size);
        for maybe_entry in inner.entries.drain(0..) {
            if let Some(entry) = maybe_entry {
                let mut hash_index = entry.hash;
                // Faster version of lookup. No equality checks here.
                // We assume dict doesn't contatins any duplicate keys
                while new_indices.contains_key(&hash_index) {
                    hash_index = Self::next_index(entry.hash, hash_index);
                }
                new_indices.insert(hash_index, new_entries.len());
                new_entries.push(Some(entry));
            }
        }
        inner.indices = new_indices;
        inner.entries = new_entries;
    }

    fn unchecked_push(
        &self,
        hash_index: HashIndex,
        hash_value: HashValue,
        key: PyObjectRef,
        value: T,
    ) {
        let entry = DictEntry {
            hash: hash_value,
            key,
            value,
        };
        let mut inner = self.borrow_value_mut();
        let entry_index = inner.entries.len();
        inner.entries.push(Some(entry));
        inner.indices.insert(hash_index, entry_index);
        inner.size += 1;
    }

    /// Store a key
    pub fn insert<K>(&self, vm: &VirtualMachine, key: K, value: T) -> PyResult<()>
    where
        K: DictKey,
    {
        // This does not need to be accurate so we can take the lock mutiple times.
        let (indices_len, size) = {
            let borrowed = self.borrow_value();
            (borrowed.indices.len(), borrowed.size)
        };
        if indices_len > 2 * size {
            self.resize();
        }
        let _removed = loop {
            match self.lookup(vm, &key)? {
                LookupResult::Existing(index) => {
                    let mut inner = self.borrow_value_mut();
                    // Update existing key
                    if let Some(ref mut entry) = inner.entries[index] {
                        // They entry might have changed since we did lookup. Should we update the key?
                        let removed = std::mem::replace(&mut entry.value, value);
                        // defer dec RC
                        break Some(removed);
                    } else {
                        // The dict was changed since we did lookup. Let's try again.
                        continue;
                    }
                }
                LookupResult::NewIndex {
                    hash_index,
                    hash_value,
                } => {
                    // New key:
                    self.unchecked_push(hash_index, hash_value, key.into_pyobject(vm), value);
                    break None;
                }
            }
        };
        Ok(())
    }

    pub fn contains<K: DictKey>(&self, vm: &VirtualMachine, key: &K) -> PyResult<bool> {
        if let LookupResult::Existing(_) = self.lookup(vm, key)? {
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Retrieve a key
    #[cfg_attr(feature = "flame-it", flame("Dict"))]
    pub fn get<K: DictKey>(&self, vm: &VirtualMachine, key: &K) -> PyResult<Option<T>> {
        loop {
            if let LookupResult::Existing(index) = self.lookup(vm, key)? {
                if let Some(entry) = &self.borrow_value().entries[index] {
                    break Ok(Some(entry.value.clone()));
                } else {
                    // The dict was changed since we did lookup. Let's try again.
                    continue;
                }
            } else {
                break Ok(None);
            }
        }
    }

    pub fn clear(&self) {
        let _removed = {
            let mut inner = self.borrow_value_mut();
            inner.indices.clear();
            inner.size = 0;
            // defer dec rc
            std::mem::replace(&mut inner.entries, Vec::new())
        };
    }

    /// Delete a key
    pub fn delete<K>(&self, vm: &VirtualMachine, key: K) -> PyResult<()>
    where
        K: DictKey,
    {
        if self.delete_if_exists(vm, &key)? {
            Ok(())
        } else {
            Err(vm.new_key_error(key.into_pyobject(vm)))
        }
    }

    pub fn delete_if_exists<K>(&self, vm: &VirtualMachine, key: &K) -> PyResult<bool>
    where
        K: DictKey,
    {
        let deleted = loop {
            if let LookupResult::Existing(entry_index) = self.lookup(vm, key)? {
                let mut inner = self.borrow_value_mut();
                let entry = inner.entries.get_mut(entry_index).unwrap();
                if entry.is_some() {
                    // defer rc out of borrow
                    let deleted = std::mem::take(entry);
                    inner.size -= 1;
                    break deleted;
                } else {
                    // The dict was changed since we did lookup. Let's try again.
                    continue;
                }
            } else {
                break None;
            }
        };
        Ok(deleted.is_some())
    }

    pub fn delete_or_insert(
        &self,
        vm: &VirtualMachine,
        key: &PyObjectRef,
        value: T,
    ) -> PyResult<()> {
        let _removed = loop {
            match self.lookup(vm, key)? {
                LookupResult::Existing(entry_index) => {
                    let mut inner = self.borrow_value_mut();
                    let entry = inner.entries.get_mut(entry_index).unwrap();
                    if entry.is_some() {
                        // defer dec RC
                        let entry = std::mem::take(entry);
                        inner.size -= 1;
                        break entry;
                    } else {
                        // The dict was changed since we did lookup. Let's try again.
                        continue;
                    }
                }
                LookupResult::NewIndex {
                    hash_value,
                    hash_index,
                } => {
                    self.unchecked_push(hash_index, hash_value, key.clone(), value);
                    break None;
                }
            };
        };
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.borrow_value().size
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn size(&self) -> DictSize {
        let inner = self.borrow_value();
        DictSize {
            size: inner.size,
            entries_size: inner.entries.len(),
        }
    }

    pub fn next_entry(&self, position: &mut EntryIndex) -> Option<(PyObjectRef, T)> {
        self.borrow_value().entries[*position..]
            .iter()
            .find_map(|entry| {
                *position += 1;
                entry
                    .as_ref()
                    .map(|DictEntry { key, value, .. }| (key.clone(), value.clone()))
            })
    }

    pub fn len_from_entry_index(&self, position: EntryIndex) -> usize {
        self.borrow_value().entries[position..]
            .iter()
            .flatten()
            .count()
    }

    pub fn has_changed_size(&self, position: &DictSize) -> bool {
        let inner = self.borrow_value();
        position.size != inner.size || inner.entries.len() != position.entries_size
    }

    pub fn keys(&self) -> Vec<PyObjectRef> {
        self.borrow_value()
            .entries
            .iter()
            .filter_map(|v| v.as_ref().map(|v| v.key.clone()))
            .collect()
    }

    /// Lookup the index for the given key.
    #[cfg_attr(feature = "flame-it", flame("Dict"))]
    fn lookup<K: DictKey>(&self, vm: &VirtualMachine, key: &K) -> PyResult<LookupResult> {
        let hash_value = key.key_hash(vm)?;
        let perturb = hash_value;
        let mut hash_index: HashIndex = hash_value;
        'outer: loop {
            let (entry, index) = loop {
                let inner = self.borrow_value();
                if inner.indices.contains_key(&hash_index) {
                    // Now we have an index, lets check the key.
                    let index = inner.indices[&hash_index];
                    if let Some(entry) = &inner.entries[index] {
                        // Okay, we have an entry at this place
                        if key.key_is(&entry.key) {
                            // Literally the same object
                            break 'outer Ok(LookupResult::Existing(index));
                        } else if entry.hash == hash_value {
                            break (entry.clone(), index);
                        } else {
                            // entry mismatch.
                        }
                    } else {
                        // Removed entry, continue search...
                    }
                } else {
                    // Hash not in table, we are at free slot now.
                    break 'outer Ok(LookupResult::NewIndex {
                        hash_value,
                        hash_index,
                    });
                }
                // Update i to next probe location:
                hash_index = Self::next_index(perturb, hash_index)
                // warn!("Perturb value: {}", i);
            };
            // This comparison needs to be done outside the lock.
            if key.key_eq(vm, &entry.key)? {
                break Ok(LookupResult::Existing(index));
            } else {
                // entry mismatch.
            }

            // Update i to next probe location:
            hash_index = Self::next_index(perturb, hash_index)
            // warn!("Perturb value: {}", i);
        }
    }

    fn next_index(perturb: HashValue, hash_index: HashIndex) -> HashIndex {
        hash_index
            .wrapping_mul(5)
            .wrapping_add(perturb)
            .wrapping_add(1)
    }

    /// Retrieve and delete a key
    pub fn pop<K: DictKey>(&self, vm: &VirtualMachine, key: &K) -> PyResult<Option<T>> {
        let removed = loop {
            if let LookupResult::Existing(index) = self.lookup(vm, key)? {
                let mut inner = self.borrow_value_mut();
                if let Some(entry) = inner.entries.get_mut(index) {
                    let popped = std::mem::take(entry);
                    inner.size -= 1;
                    break Some(popped.unwrap().value);
                } else {
                    // The dict was changed since we did lookup. Let's try again.
                    continue;
                }
            } else {
                break None;
            }
        };
        Ok(removed)
    }

    pub fn pop_front(&self) -> Option<(PyObjectRef, T)> {
        let mut position = 0;
        let mut inner = self.borrow_value_mut();
        let first_item = inner.entries.iter().find_map(|entry| {
            position += 1;
            entry
                .as_ref()
                .map(|DictEntry { key, value, .. }| (key.clone(), value.clone()))
        });
        if let Some(item) = first_item {
            inner.entries[position - 1] = None;
            inner.size -= 1;
            Some(item)
        } else {
            None
        }
    }

    pub fn sizeof(&self) -> usize {
        size_of::<Self>() + self.borrow_value().size * size_of::<DictEntry<T>>()
    }
}

enum LookupResult {
    NewIndex {
        hash_value: HashValue,
        hash_index: HashIndex,
    }, // return not found, index into indices
    Existing(EntryIndex), // Existing record, index into entries
}

/// Types implementing this trait can be used to index
/// the dictionary. Typical usecases are:
/// - PyObjectRef -> arbitrary python type used as key
/// - str -> string reference used as key, this is often used internally
pub trait DictKey: IntoPyObject {
    fn key_hash(&self, vm: &VirtualMachine) -> PyResult<HashValue>;
    fn key_is(&self, other: &PyObjectRef) -> bool;
    fn key_eq(&self, vm: &VirtualMachine, other_key: &PyObjectRef) -> PyResult<bool>;
}

/// Implement trait for PyObjectRef such that we can use python objects
/// to index dictionaries.
impl DictKey for PyObjectRef {
    fn key_hash(&self, vm: &VirtualMachine) -> PyResult<HashValue> {
        vm._hash(self)
    }

    fn key_is(&self, other: &PyObjectRef) -> bool {
        self.is(other)
    }

    fn key_eq(&self, vm: &VirtualMachine, other_key: &PyObjectRef) -> PyResult<bool> {
        vm.identical_or_equal(self, other_key)
    }
}

impl DictKey for PyStringRef {
    fn key_hash(&self, vm: &VirtualMachine) -> PyResult<HashValue> {
        Ok(self.hash(vm))
    }

    fn key_is(&self, other: &PyObjectRef) -> bool {
        self.is(other)
    }

    fn key_eq(&self, vm: &VirtualMachine, other_key: &PyObjectRef) -> PyResult<bool> {
        if self.is(other_key) {
            Ok(true)
        } else if let Some(py_str_value) = other_key.payload::<PyString>() {
            Ok(py_str_value.borrow_value() == self.borrow_value())
        } else {
            vm.bool_eq(self.clone().into_object(), other_key.clone())
        }
    }
}

// AsRef<str> fit this case but not possible in rust 1.46

/// Implement trait for the str type, so that we can use strings
/// to index dictionaries.
impl DictKey for &str {
    fn key_hash(&self, vm: &VirtualMachine) -> PyResult<HashValue> {
        // follow a similar route as the hashing of PyStringRef
        Ok(vm.state.hash_secret.hash_str(*self))
    }

    fn key_is(&self, _other: &PyObjectRef) -> bool {
        // No matter who the other pyobject is, we are never the same thing, since
        // we are a str, not a pyobject.
        false
    }

    fn key_eq(&self, vm: &VirtualMachine, other_key: &PyObjectRef) -> PyResult<bool> {
        if let Some(py_str_value) = other_key.payload::<PyString>() {
            Ok(py_str_value.borrow_value() == *self)
        } else {
            // Fall back to PyObjectRef implementation.
            let s = vm.ctx.new_str(*self);
            s.key_eq(vm, other_key)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Dict, DictKey, VirtualMachine};

    #[test]
    fn test_insert() {
        let vm: VirtualMachine = Default::default();
        let dict = Dict::default();
        assert_eq!(0, dict.len());

        let key1 = vm.ctx.new_bool(true);
        let value1 = vm.ctx.new_str("abc");
        dict.insert(&vm, key1.clone(), value1.clone()).unwrap();
        assert_eq!(1, dict.len());

        let key2 = vm.ctx.new_str("x");
        let value2 = vm.ctx.new_str("def");
        dict.insert(&vm, key2.clone(), value2.clone()).unwrap();
        assert_eq!(2, dict.len());

        dict.insert(&vm, key1.clone(), value2.clone()).unwrap();
        assert_eq!(2, dict.len());

        dict.delete(&vm, key1.clone()).unwrap();
        assert_eq!(1, dict.len());

        dict.insert(&vm, key1.clone(), value2.clone()).unwrap();
        assert_eq!(2, dict.len());

        assert_eq!(true, dict.contains(&vm, &key1).unwrap());
        assert_eq!(true, dict.contains(&vm, &"x").unwrap());

        let val = dict.get(&vm, &"x").unwrap().unwrap();
        vm.bool_eq(val, value2)
            .expect("retrieved value must be equal to inserted value.");
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
        test_abc: "abc",
        test_x: "x",
    }

    fn check_hash_equivalence(text: &str) {
        let vm: VirtualMachine = Default::default();
        let value1 = text;
        let value2 = vm.ctx.new_str(value1.to_owned());

        let hash1 = value1.key_hash(&vm).expect("Hash should not fail.");
        let hash2 = value2.key_hash(&vm).expect("Hash should not fail.");
        assert_eq!(hash1, hash2);
    }
}

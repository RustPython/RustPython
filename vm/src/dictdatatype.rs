use crate::obj::objstr::PyString;
use crate::pyhash;
use crate::pyobject::{IdProtocol, IntoPyObject, PyObjectRef, PyResult};
use crate::vm::VirtualMachine;
use num_bigint::ToBigInt;
/// Ordered dictionary implementation.
/// Inspired by: https://morepypy.blogspot.com/2015/01/faster-more-memory-efficient-and-more.html
/// And: https://www.youtube.com/watch?v=p33CVV29OG8
/// And: http://code.activestate.com/recipes/578375/
use std::collections::{hash_map::DefaultHasher, HashMap};
use std::hash::{Hash, Hasher};
use std::mem::size_of;

/// hash value of an object returned by __hash__
type HashValue = pyhash::PyHash;
/// index calculated by resolving collision
type HashIndex = pyhash::PyHash;
/// entry index mapped in indices
type EntryIndex = usize;

#[derive(Clone)]
pub struct Dict<T = PyObjectRef> {
    size: usize,
    indices: HashMap<HashIndex, EntryIndex>,
    entries: Vec<Option<DictEntry<T>>>,
}

impl<T> Default for Dict<T> {
    fn default() -> Self {
        Dict {
            size: 0,
            indices: HashMap::new(),
            entries: Vec::new(),
        }
    }
}

#[derive(Clone)]
struct DictEntry<T> {
    hash: HashValue,
    key: PyObjectRef,
    value: T,
}

#[derive(Debug)]
pub struct DictSize {
    size: usize,
    entries_size: usize,
}

impl<T: Clone> Dict<T> {
    fn resize(&mut self) {
        let mut new_indices = HashMap::with_capacity(self.size);
        let mut new_entries = Vec::with_capacity(self.size);
        for maybe_entry in self.entries.drain(0..) {
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
        self.indices = new_indices;
        self.entries = new_entries;
    }

    fn unchecked_push(
        &mut self,
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
        let entry_index = self.entries.len();
        self.entries.push(Some(entry));
        self.indices.insert(hash_index, entry_index);
        self.size += 1;
    }

    fn unchecked_delete(&mut self, entry_index: EntryIndex) {
        self.entries[entry_index] = None;
        self.size -= 1;
    }

    /// Store a key
    pub fn insert<K: DictKey + IntoPyObject + Copy>(
        &mut self,
        vm: &VirtualMachine,
        key: K,
        value: T,
    ) -> PyResult<()> {
        if self.indices.len() > 2 * self.size {
            self.resize();
        }
        match self.lookup(vm, key)? {
            LookupResult::Existing(index) => {
                // Update existing key
                if let Some(ref mut entry) = self.entries[index] {
                    entry.value = value;
                    Ok(())
                } else {
                    panic!("Lookup returned invalid index into entries!");
                }
            }
            LookupResult::NewIndex {
                hash_index,
                hash_value,
            } => {
                // New key:
                self.unchecked_push(hash_index, hash_value, key.into_pyobject(vm)?, value);
                Ok(())
            }
        }
    }

    pub fn contains<K: DictKey + Copy>(&self, vm: &VirtualMachine, key: K) -> PyResult<bool> {
        if let LookupResult::Existing(_) = self.lookup(vm, key)? {
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn unchecked_get(&self, index: EntryIndex) -> T {
        if let Some(entry) = &self.entries[index] {
            entry.value.clone()
        } else {
            panic!("Lookup returned invalid index into entries!");
        }
    }

    /// Retrieve a key
    #[cfg_attr(feature = "flame-it", flame("Dict"))]
    pub fn get<K: DictKey + Copy>(&self, vm: &VirtualMachine, key: K) -> PyResult<Option<T>> {
        if let LookupResult::Existing(index) = self.lookup(vm, key)? {
            Ok(Some(self.unchecked_get(index)))
        } else {
            Ok(None)
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.indices.clear();
        self.size = 0
    }

    /// Delete a key
    pub fn delete(&mut self, vm: &VirtualMachine, key: &PyObjectRef) -> PyResult<()> {
        if self.delete_if_exists(vm, key)? {
            Ok(())
        } else {
            Err(vm.new_key_error(key.clone()))
        }
    }

    pub fn delete_if_exists(&mut self, vm: &VirtualMachine, key: &PyObjectRef) -> PyResult<bool> {
        if let LookupResult::Existing(entry_index) = self.lookup(vm, key)? {
            self.unchecked_delete(entry_index);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn delete_or_insert(
        &mut self,
        vm: &VirtualMachine,
        key: &PyObjectRef,
        value: T,
    ) -> PyResult<()> {
        match self.lookup(vm, key)? {
            LookupResult::Existing(entry_index) => self.unchecked_delete(entry_index),
            LookupResult::NewIndex {
                hash_value,
                hash_index,
            } => self.unchecked_push(hash_index, hash_value, key.clone(), value),
        };
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.size
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn size(&self) -> DictSize {
        DictSize {
            size: self.size,
            entries_size: self.entries.len(),
        }
    }

    pub fn next_entry(&self, position: &mut EntryIndex) -> Option<(&PyObjectRef, &T)> {
        while *position < self.entries.len() {
            if let Some(DictEntry { key, value, .. }) = &self.entries[*position] {
                *position += 1;
                return Some((key, value));
            }
            *position += 1;
        }
        None
    }

    pub fn has_changed_size(&self, position: &DictSize) -> bool {
        position.size != self.size || self.entries.len() != position.entries_size
    }

    pub fn keys<'a>(&'a self) -> Box<dyn Iterator<Item = PyObjectRef> + 'a> {
        Box::new(
            self.entries
                .iter()
                .filter_map(|v| v.as_ref().map(|v| v.key.clone())),
        )
    }

    /// Lookup the index for the given key.
    #[cfg_attr(feature = "flame-it", flame("Dict"))]
    fn lookup<K: DictKey + Copy>(&self, vm: &VirtualMachine, key: K) -> PyResult<LookupResult> {
        let hash_value = key.do_hash(vm)?;
        let perturb = hash_value;
        let mut hash_index: HashIndex = hash_value;
        loop {
            if self.indices.contains_key(&hash_index) {
                // Now we have an index, lets check the key.
                let index = self.indices[&hash_index];
                if let Some(entry) = &self.entries[index] {
                    // Okay, we have an entry at this place
                    if key.do_is(&entry.key) {
                        // Literally the same object
                        break Ok(LookupResult::Existing(index));
                    } else if entry.hash == hash_value {
                        if key.do_eq(vm, &entry.key)? {
                            break Ok(LookupResult::Existing(index));
                        } else {
                            // entry mismatch.
                        }
                    } else {
                        // entry mismatch.
                    }
                } else {
                    // Removed entry, continue search...
                }
            } else {
                // Hash not in table, we are at free slot now.
                break Ok(LookupResult::NewIndex {
                    hash_value,
                    hash_index,
                });
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
    pub fn pop<K: DictKey + Copy>(&mut self, vm: &VirtualMachine, key: K) -> PyResult<Option<T>> {
        if let LookupResult::Existing(index) = self.lookup(vm, key)? {
            let value = self.unchecked_get(index);
            self.unchecked_delete(index);
            Ok(Some(value))
        } else {
            Ok(None)
        }
    }

    pub fn pop_front(&mut self) -> Option<(PyObjectRef, T)> {
        let mut entry_index = 0;
        match self.next_entry(&mut entry_index) {
            Some((key, value)) => {
                let item = (key.clone(), value.clone());
                self.unchecked_delete(entry_index - 1);
                Some(item)
            }
            None => None,
        }
    }

    pub fn sizeof(&self) -> usize {
        size_of::<Self>() + self.size * size_of::<DictEntry<T>>()
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
pub trait DictKey {
    fn do_hash(self, vm: &VirtualMachine) -> PyResult<HashValue>;
    fn do_is(self, other: &PyObjectRef) -> bool;
    fn do_eq(self, vm: &VirtualMachine, other_key: &PyObjectRef) -> PyResult<bool>;
}

/// Implement trait for PyObjectRef such that we can use python objects
/// to index dictionaries.
impl DictKey for &PyObjectRef {
    fn do_hash(self, vm: &VirtualMachine) -> PyResult<HashValue> {
        let raw_hash = vm._hash(self)?;
        let mut hasher = DefaultHasher::new();
        raw_hash.hash(&mut hasher);
        Ok(hasher.finish() as HashValue)
    }

    fn do_is(self, other: &PyObjectRef) -> bool {
        self.is(other)
    }

    fn do_eq(self, vm: &VirtualMachine, other_key: &PyObjectRef) -> PyResult<bool> {
        vm.identical_or_equal(self, other_key)
    }
}

/// Implement trait for the str type, so that we can use strings
/// to index dictionaries.
impl DictKey for &str {
    fn do_hash(self, _vm: &VirtualMachine) -> PyResult<HashValue> {
        // follow a similar route as the hashing of PyStringRef
        let raw_hash = pyhash::hash_value(&self.to_string()).to_bigint().unwrap();
        let raw_hash = pyhash::hash_bigint(&raw_hash);
        let mut hasher = DefaultHasher::new();
        raw_hash.hash(&mut hasher);
        Ok(hasher.finish() as HashValue)
    }

    fn do_is(self, _other: &PyObjectRef) -> bool {
        // No matter who the other pyobject is, we are never the same thing, since
        // we are a str, not a pyobject.
        false
    }

    fn do_eq(self, vm: &VirtualMachine, other_key: &PyObjectRef) -> PyResult<bool> {
        if let Some(py_str_value) = other_key.payload::<PyString>() {
            Ok(py_str_value.as_str() == self)
        } else {
            // Fall back to PyString implementation.
            let s = vm.new_str(self.to_string());
            s.do_eq(vm, other_key)
        }
    }
}

impl DictKey for &String {
    fn do_hash(self, _vm: &VirtualMachine) -> PyResult<HashValue> {
        // follow a similar route as the hashing of PyStringRef
        let raw_hash = pyhash::hash_value(self).to_bigint().unwrap();
        let raw_hash = pyhash::hash_bigint(&raw_hash);
        let mut hasher = DefaultHasher::new();
        raw_hash.hash(&mut hasher);
        Ok(hasher.finish() as HashValue)
    }

    fn do_is(self, _other: &PyObjectRef) -> bool {
        // No matter who the other pyobject is, we are never the same thing, since
        // we are a str, not a pyobject.
        false
    }

    fn do_eq(self, vm: &VirtualMachine, other_key: &PyObjectRef) -> PyResult<bool> {
        if let Some(py_str_value) = other_key.payload::<PyString>() {
            Ok(py_str_value.as_str() == self)
        } else {
            // Fall back to PyString implementation.
            let s = vm.new_str(self.to_string());
            s.do_eq(vm, other_key)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Dict, DictKey, VirtualMachine};

    #[test]
    fn test_insert() {
        let vm: VirtualMachine = Default::default();
        let mut dict = Dict::default();
        assert_eq!(0, dict.len());

        let key1 = vm.new_bool(true);
        let value1 = vm.new_str("abc".to_string());
        dict.insert(&vm, &key1, value1.clone()).unwrap();
        assert_eq!(1, dict.len());

        let key2 = vm.new_str("x".to_string());
        let value2 = vm.new_str("def".to_string());
        dict.insert(&vm, &key2, value2.clone()).unwrap();
        assert_eq!(2, dict.len());

        dict.insert(&vm, &key1, value2.clone()).unwrap();
        assert_eq!(2, dict.len());

        dict.delete(&vm, &key1).unwrap();
        assert_eq!(1, dict.len());

        dict.insert(&vm, &key1, value2.clone()).unwrap();
        assert_eq!(2, dict.len());

        assert_eq!(true, dict.contains(&vm, &key1).unwrap());
        assert_eq!(true, dict.contains(&vm, "x").unwrap());

        let val = dict.get(&vm, "x").unwrap().unwrap();
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
        let value2 = vm.new_str(value1.to_string());

        let hash1 = value1.do_hash(&vm).expect("Hash should not fail.");
        let hash2 = value2.do_hash(&vm).expect("Hash should not fail.");
        assert_eq!(hash1, hash2);
    }
}

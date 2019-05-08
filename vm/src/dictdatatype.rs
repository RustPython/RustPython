use crate::obj::objbool;
use crate::pyhash;
use crate::pyobject::{IdProtocol, PyObjectRef, PyResult};
use crate::vm::VirtualMachine;
/// Ordered dictionary implementation.
/// Inspired by: https://morepypy.blogspot.com/2015/01/faster-more-memory-efficient-and-more.html
/// And: https://www.youtube.com/watch?v=p33CVV29OG8
/// And: http://code.activestate.com/recipes/578375/
use std::collections::{hash_map::DefaultHasher, HashMap};
use std::hash::{Hash, Hasher};

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
    fn unchecked_push(
        &mut self,
        hash_index: HashIndex,
        hash_value: HashValue,
        key: &PyObjectRef,
        value: T,
    ) {
        let entry = DictEntry {
            hash: hash_value,
            key: key.clone(),
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
    pub fn insert(&mut self, vm: &VirtualMachine, key: &PyObjectRef, value: T) -> PyResult<()> {
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
                self.unchecked_push(hash_index, hash_value, key, value);
                Ok(())
            }
        }
    }

    pub fn contains(&self, vm: &VirtualMachine, key: &PyObjectRef) -> PyResult<bool> {
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
    pub fn get(&self, vm: &VirtualMachine, key: &PyObjectRef) -> PyResult<Option<T>> {
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
            let key_repr = vm.to_pystr(key)?;
            Err(vm.new_key_error(format!("Key not found: {}", key_repr)))
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

    /// Lookup the index for the given key.
    fn lookup(&self, vm: &VirtualMachine, key: &PyObjectRef) -> PyResult<LookupResult> {
        let hash_value = collection_hash(vm, key)?;
        let perturb = hash_value;
        let mut hash_index: HashIndex = hash_value;
        loop {
            if self.indices.contains_key(&hash_index) {
                // Now we have an index, lets check the key.
                let index = self.indices[&hash_index];
                if let Some(entry) = &self.entries[index] {
                    // Okay, we have an entry at this place
                    if entry.key.is(key) {
                        // Literally the same object
                        break Ok(LookupResult::Existing(index));
                    } else if entry.hash == hash_value {
                        if do_eq(vm, &entry.key, key)? {
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
            hash_index = hash_index
                .wrapping_mul(5)
                .wrapping_add(perturb)
                .wrapping_add(1);
            // warn!("Perturb value: {}", i);
        }
    }

    /// Retrieve and delete a key
    pub fn pop(&mut self, vm: &VirtualMachine, key: &PyObjectRef) -> PyResult<T> {
        if let LookupResult::Existing(index) = self.lookup(vm, key)? {
            let value = self.unchecked_get(index);
            self.unchecked_delete(index);
            Ok(value)
        } else {
            let key_repr = vm.to_pystr(key)?;
            Err(vm.new_key_error(format!("Key not found: {}", key_repr)))
        }
    }
}

enum LookupResult {
    NewIndex {
        hash_value: HashValue,
        hash_index: HashIndex,
    }, // return not found, index into indices
    Existing(EntryIndex), // Existing record, index into entries
}

fn collection_hash(vm: &VirtualMachine, object: &PyObjectRef) -> PyResult<HashValue> {
    let raw_hash = vm._hash(object)?;
    let mut hasher = DefaultHasher::new();
    raw_hash.hash(&mut hasher);
    Ok(hasher.finish() as HashValue)
}

/// Invoke __eq__ on two keys
fn do_eq(vm: &VirtualMachine, key1: &PyObjectRef, key2: &PyObjectRef) -> Result<bool, PyObjectRef> {
    let result = vm._eq(key1.clone(), key2.clone())?;
    objbool::boolval(vm, result)
}

#[cfg(test)]
mod tests {
    use super::{Dict, VirtualMachine};

    #[test]
    fn test_insert() {
        let mut vm = VirtualMachine::new();
        let mut dict = Dict::default();
        assert_eq!(0, dict.len());

        let key1 = vm.new_bool(true);
        let value1 = vm.new_str("abc".to_string());
        dict.insert(&mut vm, &key1, value1.clone()).unwrap();
        assert_eq!(1, dict.len());

        let key2 = vm.new_str("x".to_string());
        let value2 = vm.new_str("def".to_string());
        dict.insert(&mut vm, &key2, value2.clone()).unwrap();
        assert_eq!(2, dict.len());

        dict.insert(&mut vm, &key1, value2.clone()).unwrap();
        assert_eq!(2, dict.len());

        dict.delete(&mut vm, &key1).unwrap();
        assert_eq!(1, dict.len());

        dict.insert(&mut vm, &key1, value2).unwrap();
        assert_eq!(2, dict.len());

        assert_eq!(true, dict.contains(&mut vm, &key1).unwrap());
    }
}

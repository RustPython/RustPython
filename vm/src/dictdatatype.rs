use super::obj::objbool;
use super::obj::objint;
use super::pyobject::{IdProtocol, PyObjectRef, PyResult};
use super::vm::VirtualMachine;
use num_traits::ToPrimitive;
/// Ordered dictionary implementation.
/// Inspired by: https://morepypy.blogspot.com/2015/01/faster-more-memory-efficient-and-more.html
/// And: https://www.youtube.com/watch?v=p33CVV29OG8
/// And: http://code.activestate.com/recipes/578375/
use std::collections::HashMap;

pub struct Dict {
    size: usize,
    indices: HashMap<usize, usize>,
    entries: Vec<Option<DictEntry>>,
}

struct DictEntry {
    hash: usize,
    key: PyObjectRef,
    value: PyObjectRef,
}

impl Dict {
    pub fn new() -> Self {
        Dict {
            size: 0,
            indices: HashMap::new(),
            entries: Vec::new(),
        }
    }

    /// Store a key
    pub fn insert(
        &mut self,
        vm: &mut VirtualMachine,
        key: &PyObjectRef,
        value: PyObjectRef,
    ) -> Result<(), PyObjectRef> {
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
                let entry = DictEntry {
                    hash: hash_value,
                    key: key.clone(),
                    value,
                };
                let index = self.entries.len();
                self.entries.push(Some(entry));
                self.indices.insert(hash_index, index);
                self.size += 1;
                Ok(())
            }
        }
    }

    pub fn contains(
        &self,
        vm: &mut VirtualMachine,
        key: &PyObjectRef,
    ) -> Result<bool, PyObjectRef> {
        if let LookupResult::Existing(_index) = self.lookup(vm, key)? {
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Retrieve a key
    pub fn get(&self, vm: &mut VirtualMachine, key: &PyObjectRef) -> PyResult {
        if let LookupResult::Existing(index) = self.lookup(vm, key)? {
            if let Some(entry) = &self.entries[index] {
                Ok(entry.value.clone())
            } else {
                panic!("Lookup returned invalid index into entries!");
            }
        } else {
            let key_repr = vm.to_pystr(key)?;
            Err(vm.new_value_error(format!("Key not found: {}", key_repr)))
        }
    }

    /// Delete a key
    pub fn delete(
        &mut self,
        vm: &mut VirtualMachine,
        key: &PyObjectRef,
    ) -> Result<(), PyObjectRef> {
        if let LookupResult::Existing(index) = self.lookup(vm, key)? {
            self.entries[index] = None;
            self.size -= 1;
            Ok(())
        } else {
            let key_repr = vm.to_pystr(key)?;
            Err(vm.new_value_error(format!("Key not found: {}", key_repr)))
        }
    }

    pub fn len(&self) -> usize {
        self.size
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn get_items(&self) -> Vec<(PyObjectRef, PyObjectRef)> {
        self.entries
            .iter()
            .filter(|e| e.is_some())
            .map(|e| e.as_ref().unwrap())
            .map(|e| (e.key.clone(), e.value.clone()))
            .collect()
    }

    /// Lookup the index for the given key.
    fn lookup(
        &self,
        vm: &mut VirtualMachine,
        key: &PyObjectRef,
    ) -> Result<LookupResult, PyObjectRef> {
        let hash_value = calc_hash(vm, key)?;
        let perturb = hash_value;
        let mut hash_index: usize = hash_value;
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
}

enum LookupResult {
    NewIndex {
        hash_value: usize,
        hash_index: usize,
    }, // return not found, index into indices
    Existing(usize), // Existing record, index into entries
}

fn calc_hash(vm: &mut VirtualMachine, key: &PyObjectRef) -> Result<usize, PyObjectRef> {
    let hash = vm.call_method(key, "__hash__", vec![])?;
    Ok(objint::get_value(&hash).to_usize().unwrap())
}

/// Invoke __eq__ on two keys
fn do_eq(
    vm: &mut VirtualMachine,
    key1: &PyObjectRef,
    key2: &PyObjectRef,
) -> Result<bool, PyObjectRef> {
    let result = vm._eq(key1, key2.clone())?;
    Ok(objbool::get_value(&result))
}

#[cfg(test)]
mod tests {
    use super::{Dict, VirtualMachine};

    #[test]
    fn test_insert() {
        let mut vm = VirtualMachine::new();
        let mut dict = Dict::new();
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

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fmt;
use std::ops::{Deref, DerefMut};

use crate::function::{OptionalArg, PyFuncArgs};
use crate::pyobject::{
    PyAttributes, PyContext, PyIteratorValue, PyObjectRef, PyRef, PyResult, PyValue, TypeProtocol,
};
use crate::vm::{ReprGuard, VirtualMachine};

use super::objiter;
use super::objstr::{self, PyStringRef};
use super::objtype;

pub type DictContentType = HashMap<String, (PyObjectRef, PyObjectRef)>;

#[derive(Default)]
pub struct PyDict {
    // TODO: should be private
    pub entries: RefCell<DictContentType>,
}
pub type PyDictRef = PyRef<PyDict>;

impl fmt::Debug for PyDict {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // TODO: implement more detailed, non-recursive Debug formatter
        f.write_str("dict")
    }
}

impl PyValue for PyDict {
    fn class(vm: &VirtualMachine) -> Vec<PyObjectRef> {
        vec![vm.ctx.dict_type()]
    }
}

pub fn get_elements<'a>(obj: &'a PyObjectRef) -> impl Deref<Target = DictContentType> + 'a {
    obj.payload::<PyDict>().unwrap().entries.borrow()
}

pub fn get_mut_elements<'a>(obj: &'a PyObjectRef) -> impl DerefMut<Target = DictContentType> + 'a {
    obj.payload::<PyDict>().unwrap().entries.borrow_mut()
}

pub fn set_item(
    dict: &PyObjectRef,
    _vm: &VirtualMachine,
    needle: &PyObjectRef,
    value: &PyObjectRef,
) {
    // TODO: use vm to call eventual __hash__ and __eq__methods.
    let mut elements = get_mut_elements(dict);
    set_item_in_content(&mut elements, needle, value);
}

pub fn set_item_in_content(
    elements: &mut DictContentType,
    needle: &PyObjectRef,
    value: &PyObjectRef,
) {
    // XXX: Currently, we only support String keys, so we have to unwrap the
    // PyObject (and ensure it is a String).

    // TODO: invoke __hash__ function here!
    let needle_str = objstr::get_value(needle);
    elements.insert(needle_str, (needle.clone(), value.clone()));
}

pub fn get_key_value_pairs(dict: &PyObjectRef) -> Vec<(PyObjectRef, PyObjectRef)> {
    let dict_elements = get_elements(dict);
    get_key_value_pairs_from_content(&dict_elements)
}

pub fn get_key_value_pairs_from_content(
    dict_content: &DictContentType,
) -> Vec<(PyObjectRef, PyObjectRef)> {
    let mut pairs: Vec<(PyObjectRef, PyObjectRef)> = Vec::new();
    for (_str_key, pair) in dict_content.iter() {
        let (key, obj) = pair;
        pairs.push((key.clone(), obj.clone()));
    }
    pairs
}

pub fn get_item(dict: &PyObjectRef, key: &PyObjectRef) -> Option<PyObjectRef> {
    let needle_str = objstr::get_value(key);
    get_key_str(dict, &needle_str)
}

// Special case for the case when requesting a str key from a dict:
pub fn get_key_str(dict: &PyObjectRef, key: &str) -> Option<PyObjectRef> {
    let elements = get_elements(dict);
    content_get_key_str(&elements, key)
}

/// Retrieve a key from dict contents:
pub fn content_get_key_str(elements: &DictContentType, key: &str) -> Option<PyObjectRef> {
    // TODO: let hash: usize = key;
    match elements.get(key) {
        Some(v) => Some(v.1.clone()),
        None => None,
    }
}

pub fn contains_key_str(dict: &PyObjectRef, key: &str) -> bool {
    let elements = get_elements(dict);
    content_contains_key_str(&elements, key)
}

pub fn content_contains_key_str(elements: &DictContentType, key: &str) -> bool {
    // TODO: let hash: usize = key;
    elements.get(key).is_some()
}

/// Take a python dictionary and convert it to attributes.
pub fn py_dict_to_attributes(dict: &PyObjectRef) -> PyAttributes {
    let mut attrs = PyAttributes::new();
    for (key, value) in get_key_value_pairs(dict) {
        let key = objstr::get_value(&key);
        attrs.insert(key, value);
    }
    attrs
}

pub fn attributes_to_py_dict(vm: &VirtualMachine, attributes: PyAttributes) -> PyResult {
    let dict = vm.ctx.new_dict();
    for (key, value) in attributes {
        let key = vm.ctx.new_str(key);
        set_item(&dict, vm, &key, &value);
    }
    Ok(dict)
}

// Python dict methods:
fn dict_new(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(_ty, Some(vm.ctx.type_type()))],
        optional = [(dict_obj, None)]
    );
    let dict = vm.ctx.new_dict();
    if let Some(dict_obj) = dict_obj {
        if objtype::isinstance(&dict_obj, &vm.ctx.dict_type()) {
            for (needle, value) in get_key_value_pairs(&dict_obj) {
                set_item(&dict, vm, &needle, &value);
            }
        } else {
            let iter = objiter::get_iter(vm, dict_obj)?;
            loop {
                fn err(vm: &VirtualMachine) -> PyObjectRef {
                    vm.new_type_error("Iterator must have exactly two elements".to_string())
                }
                let element = match objiter::get_next_object(vm, &iter)? {
                    Some(obj) => obj,
                    None => break,
                };
                let elem_iter = objiter::get_iter(vm, &element)?;
                let needle = objiter::get_next_object(vm, &elem_iter)?.ok_or_else(|| err(vm))?;
                let value = objiter::get_next_object(vm, &elem_iter)?.ok_or_else(|| err(vm))?;
                if objiter::get_next_object(vm, &elem_iter)?.is_some() {
                    return Err(err(vm));
                }
                set_item(&dict, vm, &needle, &value);
            }
        }
    }
    for (needle, value) in args.kwargs {
        let py_needle = vm.new_str(needle);
        set_item(&dict, vm, &py_needle, &value);
    }
    Ok(dict)
}

impl PyDictRef {
    fn len(self, _vm: &VirtualMachine) -> usize {
        self.entries.borrow().len()
    }

    fn repr(self, vm: &VirtualMachine) -> PyResult {
        let s = if let Some(_guard) = ReprGuard::enter(self.as_object()) {
            let elements = get_key_value_pairs(self.as_object());
            let mut str_parts = vec![];
            for (key, value) in elements {
                let key_repr = vm.to_repr(&key)?;
                let value_repr = vm.to_repr(&value)?;
                str_parts.push(format!("{}: {}", key_repr.value, value_repr.value));
            }

            format!("{{{}}}", str_parts.join(", "))
        } else {
            "{...}".to_string()
        };
        Ok(vm.new_str(s))
    }

    fn contains(self, key: PyStringRef, _vm: &VirtualMachine) -> bool {
        self.entries.borrow().contains_key(&key.value)
    }

    fn delitem(self, key: PyStringRef, vm: &VirtualMachine) -> PyResult<()> {
        let key = &key.value;
        // Delete the item:
        let mut elements = self.entries.borrow_mut();
        match elements.remove(key) {
            Some(_) => Ok(()),
            None => Err(vm.new_value_error(format!("Key not found: {}", key))),
        }
    }

    fn clear(self, _vm: &VirtualMachine) {
        self.entries.borrow_mut().clear()
    }

    /// When iterating over a dictionary, we iterate over the keys of it.
    fn iter(self, vm: &VirtualMachine) -> PyIteratorValue {
        let keys = self
            .entries
            .borrow()
            .values()
            .map(|(k, _v)| k.clone())
            .collect();
        let key_list = vm.ctx.new_list(keys);

        PyIteratorValue {
            position: Cell::new(0),
            iterated_obj: key_list,
        }
    }

    fn values(self, vm: &VirtualMachine) -> PyIteratorValue {
        let values = self
            .entries
            .borrow()
            .values()
            .map(|(_k, v)| v.clone())
            .collect();
        let values_list = vm.ctx.new_list(values);

        PyIteratorValue {
            position: Cell::new(0),
            iterated_obj: values_list,
        }
    }

    fn items(self, vm: &VirtualMachine) -> PyIteratorValue {
        let items = self
            .entries
            .borrow()
            .values()
            .map(|(k, v)| vm.ctx.new_tuple(vec![k.clone(), v.clone()]))
            .collect();
        let items_list = vm.ctx.new_list(items);

        PyIteratorValue {
            position: Cell::new(0),
            iterated_obj: items_list,
        }
    }

    fn setitem(self, needle: PyObjectRef, value: PyObjectRef, _vm: &VirtualMachine) {
        let mut elements = self.entries.borrow_mut();
        set_item_in_content(&mut elements, &needle, &value)
    }

    fn getitem(self, key: PyStringRef, vm: &VirtualMachine) -> PyResult {
        let key = &key.value;

        // What we are looking for:
        let elements = self.entries.borrow();
        if elements.contains_key(key) {
            Ok(elements[key].1.clone())
        } else {
            Err(vm.new_value_error(format!("Key not found: {}", key)))
        }
    }

    fn get(
        self,
        key: PyStringRef,
        default: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyObjectRef {
        // What we are looking for:
        let key = &key.value;

        let elements = self.entries.borrow();
        if elements.contains_key(key) {
            elements[key].1.clone()
        } else {
            match default {
                OptionalArg::Present(value) => value,
                OptionalArg::Missing => vm.ctx.none(),
            }
        }
    }
}

pub fn init(context: &PyContext) {
    extend_class!(context, &context.dict_type, {
        "__len__" => context.new_rustfunc(PyDictRef::len),
        "__contains__" => context.new_rustfunc(PyDictRef::contains),
        "__delitem__" => context.new_rustfunc(PyDictRef::delitem),
        "__getitem__" => context.new_rustfunc(PyDictRef::getitem),
        "__iter__" => context.new_rustfunc(PyDictRef::iter),
        "__new__" => context.new_rustfunc(dict_new),
        "__repr__" => context.new_rustfunc(PyDictRef::repr),
        "__setitem__" => context.new_rustfunc(PyDictRef::setitem),
        "clear" => context.new_rustfunc(PyDictRef::clear),
        "values" => context.new_rustfunc(PyDictRef::values),
        "items" => context.new_rustfunc(PyDictRef::items),
        "keys" => context.new_rustfunc(PyDictRef::iter),
        "get" => context.new_rustfunc(PyDictRef::get),
    });
}

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fmt;

use crate::function::{KwArgs, OptionalArg};
use crate::pyobject::{
    DictProtocol, PyAttributes, PyContext, PyObjectRef, PyRef, PyResult, PyValue,
};
use crate::vm::{ReprGuard, VirtualMachine};

use super::objiter;
use super::objlist::PyListIterator;
use super::objstr::{self, PyStringRef};
use super::objtype;
use crate::obj::objtype::PyClassRef;

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
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.dict_type()
    }
}

pub fn get_key_value_pairs(dict: &PyObjectRef) -> Vec<(PyObjectRef, PyObjectRef)> {
    let dict_elements = dict.payload::<PyDict>().unwrap().entries.borrow();
    let mut pairs: Vec<(PyObjectRef, PyObjectRef)> = Vec::new();
    for (_str_key, pair) in dict_elements.iter() {
        let (key, obj) = pair;
        pairs.push((key.clone(), obj.clone()));
    }
    pairs
}

// Python dict methods:
impl PyDictRef {
    fn new(
        _class: PyClassRef, // TODO Support subclasses of int.
        dict_obj: OptionalArg<PyObjectRef>,
        kwargs: KwArgs,
        vm: &VirtualMachine,
    ) -> PyResult<PyDictRef> {
        let dict = vm.ctx.new_dict();
        if let OptionalArg::Present(dict_obj) = dict_obj {
            if objtype::isinstance(&dict_obj, &vm.ctx.dict_type()) {
                for (needle, value) in get_key_value_pairs(&dict_obj) {
                    dict.set_item(&vm.ctx, &objstr::get_value(&needle), value);
                }
            } else {
                let iter = objiter::get_iter(vm, &dict_obj)?;
                loop {
                    fn err(vm: &VirtualMachine) -> PyObjectRef {
                        vm.new_type_error("Iterator must have exactly two elements".to_string())
                    }
                    let element = match objiter::get_next_object(vm, &iter)? {
                        Some(obj) => obj,
                        None => break,
                    };
                    let elem_iter = objiter::get_iter(vm, &element)?;
                    let needle =
                        objiter::get_next_object(vm, &elem_iter)?.ok_or_else(|| err(vm))?;
                    let value = objiter::get_next_object(vm, &elem_iter)?.ok_or_else(|| err(vm))?;
                    if objiter::get_next_object(vm, &elem_iter)?.is_some() {
                        return Err(err(vm));
                    }
                    dict.set_item(&vm.ctx, &objstr::get_value(&needle), value);
                }
            }
        }
        for (needle, value) in kwargs.into_iter() {
            let py_needle = vm.new_str(needle);
            dict.set_item(&vm.ctx, &objstr::get_value(&py_needle), value);
        }
        Ok(dict)
    }

    fn bool(self, _vm: &VirtualMachine) -> bool {
        !self.entries.borrow().is_empty()
    }

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
    fn iter(self, vm: &VirtualMachine) -> PyListIterator {
        // TODO: separate type, not a list iterator
        let keys = self
            .entries
            .borrow()
            .values()
            .map(|(k, _v)| k.clone())
            .collect();
        let key_list = vm.ctx.new_list(keys);

        PyListIterator {
            position: Cell::new(0),
            list: key_list.downcast().unwrap(),
        }
    }

    fn values(self, vm: &VirtualMachine) -> PyListIterator {
        // TODO: separate type. `values` should be a live view over the collection, not an iterator.
        let values = self
            .entries
            .borrow()
            .values()
            .map(|(_k, v)| v.clone())
            .collect();
        let values_list = vm.ctx.new_list(values);

        PyListIterator {
            position: Cell::new(0),
            list: values_list.downcast().unwrap(),
        }
    }

    fn items(self, vm: &VirtualMachine) -> PyListIterator {
        // TODO: separate type. `items` should be a live view over the collection, not an iterator.
        let items = self
            .entries
            .borrow()
            .values()
            .map(|(k, v)| vm.ctx.new_tuple(vec![k.clone(), v.clone()]))
            .collect();
        let items_list = vm.ctx.new_list(items);

        PyListIterator {
            position: Cell::new(0),
            list: items_list.downcast().unwrap(),
        }
    }

    fn setitem(self, needle: PyObjectRef, value: PyObjectRef, vm: &VirtualMachine) {
        self.set_item(&vm.ctx, &objstr::get_value(&needle), value)
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

    /// Take a python dictionary and convert it to attributes.
    pub fn to_attributes(self) -> PyAttributes {
        let mut attrs = PyAttributes::new();
        for (key, value) in self.get_key_value_pairs() {
            let key = objstr::get_value(&key);
            attrs.insert(key, value);
        }
        attrs
    }
}

impl DictProtocol for PyDictRef {
    fn contains_key(&self, k: &str) -> bool {
        self.entries.borrow().get(k).is_some()
    }

    fn get_item(&self, k: &str) -> Option<PyObjectRef> {
        match self.entries.borrow().get(k) {
            Some(v) => Some(v.1.clone()),
            None => None,
        }
    }

    fn get_key_value_pairs(&self) -> Vec<(PyObjectRef, PyObjectRef)> {
        get_key_value_pairs(self.as_object())
    }

    // Item set/get:
    fn set_item(&self, ctx: &PyContext, key_str: &str, v: PyObjectRef) {
        let key = ctx.new_str(key_str.to_string());
        let elements = &mut self.entries.borrow_mut();
        elements.insert(key_str.to_string(), (key.clone(), v.clone()));
    }

    fn del_item(&self, key: &str) {
        let elements = &mut self.entries.borrow_mut();
        elements.remove(key).unwrap();
    }
}

pub fn init(context: &PyContext) {
    extend_class!(context, &context.dict_type, {
        "__bool__" => context.new_rustfunc(PyDictRef::bool),
        "__len__" => context.new_rustfunc(PyDictRef::len),
        "__contains__" => context.new_rustfunc(PyDictRef::contains),
        "__delitem__" => context.new_rustfunc(PyDictRef::delitem),
        "__getitem__" => context.new_rustfunc(PyDictRef::getitem),
        "__iter__" => context.new_rustfunc(PyDictRef::iter),
        "__new__" => context.new_rustfunc(PyDictRef::new),
        "__repr__" => context.new_rustfunc(PyDictRef::repr),
        "__setitem__" => context.new_rustfunc(PyDictRef::setitem),
        "clear" => context.new_rustfunc(PyDictRef::clear),
        "values" => context.new_rustfunc(PyDictRef::values),
        "items" => context.new_rustfunc(PyDictRef::items),
        // TODO: separate type. `keys` should be a live view over the collection, not an iterator.
        "keys" => context.new_rustfunc(PyDictRef::iter),
        "get" => context.new_rustfunc(PyDictRef::get),
    });
}

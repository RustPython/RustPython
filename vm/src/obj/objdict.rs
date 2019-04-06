use std::cell::{Cell, RefCell};
use std::fmt;

use crate::function::{KwArgs, OptionalArg};
use crate::pyobject::{
    IntoPyObject, ItemProtocol, PyAttributes, PyContext, PyObjectRef, PyRef, PyResult, PyValue,
};
use crate::vm::{ReprGuard, VirtualMachine};

use crate::dictdatatype;

use super::objiter;
use super::objlist::PyListIterator;
use super::objstr;
use crate::obj::objtype::PyClassRef;

pub type DictContentType = dictdatatype::Dict;

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

// Python dict methods:
impl PyDictRef {
    fn new(
        class: PyClassRef,
        dict_obj: OptionalArg<PyObjectRef>,
        kwargs: KwArgs,
        vm: &VirtualMachine,
    ) -> PyResult<PyDictRef> {
        let mut dict = DictContentType::default();

        if let OptionalArg::Present(dict_obj) = dict_obj {
            let dicted: PyResult<PyDictRef> = dict_obj.clone().downcast();
            if let Ok(dict_obj) = dicted {
                for (key, value) in dict_obj.get_key_value_pairs() {
                    dict.insert(vm, &key, value)?;
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
                    let key = objiter::get_next_object(vm, &elem_iter)?.ok_or_else(|| err(vm))?;
                    let value = objiter::get_next_object(vm, &elem_iter)?.ok_or_else(|| err(vm))?;
                    if objiter::get_next_object(vm, &elem_iter)?.is_some() {
                        return Err(err(vm));
                    }
                    dict.insert(vm, &key, value)?;
                }
            }
        }
        for (key, value) in kwargs.into_iter() {
            dict.insert(vm, &vm.new_str(key), value)?;
        }
        PyDict {
            entries: RefCell::new(dict),
        }
        .into_ref_with_type(vm, class)
    }

    fn bool(self, _vm: &VirtualMachine) -> bool {
        !self.entries.borrow().is_empty()
    }

    fn len(self, _vm: &VirtualMachine) -> usize {
        self.entries.borrow().len()
    }

    fn repr(self, vm: &VirtualMachine) -> PyResult<String> {
        let s = if let Some(_guard) = ReprGuard::enter(self.as_object()) {
            let mut str_parts = vec![];
            for (key, value) in self.get_key_value_pairs() {
                let key_repr = vm.to_repr(&key)?;
                let value_repr = vm.to_repr(&value)?;
                str_parts.push(format!("{}: {}", key_repr.value, value_repr.value));
            }

            format!("{{{}}}", str_parts.join(", "))
        } else {
            "{...}".to_string()
        };
        Ok(s)
    }

    fn contains(self, key: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        self.entries.borrow().contains(vm, &key)
    }

    fn inner_delitem(self, key: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.entries.borrow_mut().delete(vm, &key)
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
            .get_items()
            .iter()
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
            .get_items()
            .iter()
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
            .get_items()
            .iter()
            .map(|(k, v)| vm.ctx.new_tuple(vec![k.clone(), v.clone()]))
            .collect();
        let items_list = vm.ctx.new_list(items);

        PyListIterator {
            position: Cell::new(0),
            list: items_list.downcast().unwrap(),
        }
    }

    pub fn get_key_value_pairs(&self) -> Vec<(PyObjectRef, PyObjectRef)> {
        self.entries.borrow().get_items()
    }

    fn inner_setitem(
        self,
        key: PyObjectRef,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        self.entries.borrow_mut().insert(vm, &key, value)
    }

    fn inner_getitem(self, key: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if let Some(value) = self.entries.borrow().get(vm, &key)? {
            return Ok(value);
        }

        if let Ok(method) = vm.get_method(self.clone().into_object(), "__missing__") {
            return vm.invoke(method, vec![key]);
        }

        Err(vm.new_key_error(format!("Key not found: {}", vm.to_pystr(&key)?)))
    }

    fn get(
        self,
        key: PyObjectRef,
        default: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        match self.entries.borrow().get(vm, &key)? {
            Some(value) => Ok(value),
            None => match default {
                OptionalArg::Present(value) => Ok(value),
                OptionalArg::Missing => Ok(vm.ctx.none()),
            },
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

    fn hash(self, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_type_error("unhashable type".to_string()))
    }

    pub fn contains_key<T: IntoPyObject>(&self, key: T, vm: &VirtualMachine) -> bool {
        let key = key.into_pyobject(vm).unwrap();
        self.entries.borrow().contains(vm, &key).unwrap()
    }
}

impl ItemProtocol for PyDictRef {
    fn get_item<T: IntoPyObject>(&self, key: T, vm: &VirtualMachine) -> PyResult {
        self.as_object().get_item(key, vm)
    }

    fn set_item<T: IntoPyObject>(
        &self,
        key: T,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult {
        self.as_object().set_item(key, value, vm)
    }

    fn del_item<T: IntoPyObject>(&self, key: T, vm: &VirtualMachine) -> PyResult {
        self.as_object().del_item(key, vm)
    }
}

pub fn init(context: &PyContext) {
    extend_class!(context, &context.dict_type, {
        "__bool__" => context.new_rustfunc(PyDictRef::bool),
        "__len__" => context.new_rustfunc(PyDictRef::len),
        "__contains__" => context.new_rustfunc(PyDictRef::contains),
        "__delitem__" => context.new_rustfunc(PyDictRef::inner_delitem),
        "__getitem__" => context.new_rustfunc(PyDictRef::inner_getitem),
        "__iter__" => context.new_rustfunc(PyDictRef::iter),
        "__new__" => context.new_rustfunc(PyDictRef::new),
        "__repr__" => context.new_rustfunc(PyDictRef::repr),
        "__setitem__" => context.new_rustfunc(PyDictRef::inner_setitem),
        "__hash__" => context.new_rustfunc(PyDictRef::hash),
        "clear" => context.new_rustfunc(PyDictRef::clear),
        "values" => context.new_rustfunc(PyDictRef::values),
        "items" => context.new_rustfunc(PyDictRef::items),
        // TODO: separate type. `keys` should be a live view over the collection, not an iterator.
        "keys" => context.new_rustfunc(PyDictRef::iter),
        "get" => context.new_rustfunc(PyDictRef::get),
    });
}

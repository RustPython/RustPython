use std::cell::{Cell, RefCell};
use std::fmt;

use crate::function::{KwArgs, OptionalArg};
use crate::pyobject::{
    IdProtocol, IntoPyObject, ItemProtocol, PyAttributes, PyContext, PyObjectRef, PyRef, PyResult,
    PyValue,
};
use crate::vm::{ReprGuard, VirtualMachine};

use super::objiter;
use super::objstr;
use crate::dictdatatype;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::PyClassImpl;

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
        let dict = DictContentType::default();

        let entries = RefCell::new(dict);
        // it's unfortunate that we can't abstract over RefCall, as we should be able to use dict
        // directly here, but that would require generic associated types
        PyDictRef::merge(&entries, dict_obj, kwargs, vm)?;

        PyDict { entries }.into_ref_with_type(vm, class)
    }

    fn merge(
        dict: &RefCell<DictContentType>,
        dict_obj: OptionalArg<PyObjectRef>,
        kwargs: KwArgs,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        if let OptionalArg::Present(dict_obj) = dict_obj {
            let dicted: PyResult<PyDictRef> = dict_obj.clone().downcast();
            if let Ok(dict_obj) = dicted {
                let mut dict_borrowed = dict.borrow_mut();
                for (key, value) in dict_obj.entries.borrow().iter_items() {
                    dict_borrowed.insert(vm, &key, value)?;
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
                    dict.borrow_mut().insert(vm, &key, value)?;
                }
            }
        }
        let mut dict_borrowed = dict.borrow_mut();
        for (key, value) in kwargs.into_iter() {
            dict_borrowed.insert(vm, &vm.new_str(key), value)?;
        }
        Ok(())
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

    fn iter(self, _vm: &VirtualMachine) -> PyDictKeyIterator {
        PyDictKeyIterator::new(self)
    }

    fn keys(self, _vm: &VirtualMachine) -> PyDictKeys {
        PyDictKeys::new(self)
    }

    fn values(self, _vm: &VirtualMachine) -> PyDictValues {
        PyDictValues::new(self)
    }

    fn items(self, _vm: &VirtualMachine) -> PyDictItems {
        PyDictItems::new(self)
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

    fn copy(self, _vm: &VirtualMachine) -> PyDict {
        PyDict {
            entries: self.entries.clone(),
        }
    }

    fn update(
        self,
        mut dict_obj: OptionalArg<PyObjectRef>,
        kwargs: KwArgs,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        if let OptionalArg::Present(ref other) = dict_obj {
            if self.is(other) {
                // Updating yourself is a noop, and this avoids a borrow error
                dict_obj = OptionalArg::Missing;
            }
        }

        PyDictRef::merge(&self.entries, dict_obj, kwargs, vm)
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

macro_rules! dict_iterator {
    ( $name: ident, $iter_name: ident, $class: ident, $iter_class: ident, $class_name: literal, $iter_class_name: literal, $result_fn: expr) => {
        #[pyclass(name = $class_name, __inside_vm)]
        #[derive(Debug)]
        struct $name {
            pub dict: PyDictRef,
        }

        #[pyimpl(__inside_vm)]
        impl $name {
            fn new(dict: PyDictRef) -> Self {
                $name { dict: dict }
            }

            #[pymethod(name = "__iter__")]
            fn iter(&self, _vm: &VirtualMachine) -> $iter_name {
                $iter_name::new(self.dict.clone())
            }

            #[pymethod(name = "__len__")]
            fn len(&self, vm: &VirtualMachine) -> usize {
                self.dict.clone().len(vm)
            }
        }

        impl PyValue for $name {
            fn class(vm: &VirtualMachine) -> PyClassRef {
                vm.ctx.$class.clone()
            }
        }

        #[pyclass(name = $iter_class_name, __inside_vm)]
        #[derive(Debug)]
        struct $iter_name {
            pub dict: PyDictRef,
            pub position: Cell<usize>,
        }

        #[pyimpl(__inside_vm)]
        impl $iter_name {
            fn new(dict: PyDictRef) -> Self {
                $iter_name {
                    position: Cell::new(0),
                    dict,
                }
            }

            #[pymethod(name = "__next__")]
            fn next(&self, vm: &VirtualMachine) -> PyResult {
                match self.dict.entries.borrow().next_entry(self.position.get()) {
                    Some((new_position, key, value)) => {
                        self.position.set(new_position);
                        Ok($result_fn(vm, key, value))
                    }
                    None => Err(objiter::new_stop_iteration(vm)),
                }
            }

            #[pymethod(name = "__iter__")]
            fn iter(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyRef<Self> {
                zelf
            }
        }

        impl PyValue for $iter_name {
            fn class(vm: &VirtualMachine) -> PyClassRef {
                vm.ctx.$iter_class.clone()
            }
        }
    };
}

dict_iterator! {
    PyDictKeys,
    PyDictKeyIterator,
    dictkeys_type,
    dictkeyiterator_type,
    "dictkeys",
    "dictkeyiterator",
    |_vm: &VirtualMachine, key: &PyObjectRef, _value: &PyObjectRef| key.clone()
}

dict_iterator! {
    PyDictValues,
    PyDictValueIterator,
    dictvalues_type,
    dictvalueiterator_type,
    "dictvalues",
    "dictvalueiterator",
    |_vm: &VirtualMachine, _key: &PyObjectRef, value: &PyObjectRef| value.clone()
}

dict_iterator! {
    PyDictItems,
    PyDictItemIterator,
    dictitems_type,
    dictitemiterator_type,
    "dictitems",
    "dictitemiterator",
    |vm: &VirtualMachine, key: &PyObjectRef, value: &PyObjectRef|
        vm.ctx.new_tuple(vec![key.clone(), value.clone()])
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
        "keys" => context.new_rustfunc(PyDictRef::keys),
        "get" => context.new_rustfunc(PyDictRef::get),
        "copy" => context.new_rustfunc(PyDictRef::copy),
        "update" => context.new_rustfunc(PyDictRef::update),
    });

    PyDictKeys::extend_class(context, &context.dictkeys_type);
    PyDictKeyIterator::extend_class(context, &context.dictkeyiterator_type);
    PyDictValues::extend_class(context, &context.dictvalues_type);
    PyDictValueIterator::extend_class(context, &context.dictvalueiterator_type);
    PyDictItems::extend_class(context, &context.dictitems_type);
    PyDictItemIterator::extend_class(context, &context.dictitemiterator_type);
}

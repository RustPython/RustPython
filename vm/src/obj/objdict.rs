use std::cell::{Cell, RefCell};
use std::fmt;

use super::objiter;
use super::objstr;
use super::objtype::{self, PyClassRef};
use crate::dictdatatype::{self, DictKey};
use crate::function::{KwArgs, OptionalArg};
use crate::pyobject::{
    IdProtocol, IntoPyObject, ItemProtocol, PyAttributes, PyClassImpl, PyContext, PyIterable,
    PyObjectRef, PyRef, PyResult, PyValue,
};
use crate::vm::{ReprGuard, VirtualMachine};

use std::mem::size_of;

pub type DictContentType = dictdatatype::Dict;

#[derive(Default)]
pub struct PyDict {
    entries: RefCell<DictContentType>,
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
                for (key, value) in dict_obj {
                    dict.borrow_mut().insert(vm, &key, value)?;
                }
            } else if let Some(keys) = vm.get_method(dict_obj.clone(), "keys") {
                let keys = objiter::get_iter(vm, &vm.invoke(&keys?, vec![])?)?;
                while let Some(key) = objiter::get_next_object(vm, &keys)? {
                    let val = dict_obj.get_item(&key, vm)?;
                    dict.borrow_mut().insert(vm, &key, val)?;
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

    fn fromkeys(
        class: PyClassRef,
        iterable: PyIterable,
        value: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyDictRef> {
        let mut dict = DictContentType::default();
        let value = value.unwrap_or_else(|| vm.ctx.none());
        for elem in iterable.iter(vm)? {
            let elem = elem?;
            dict.insert(vm, &elem, value.clone())?;
        }
        let entries = RefCell::new(dict);
        PyDict { entries }.into_ref_with_type(vm, class)
    }

    fn bool(self, _vm: &VirtualMachine) -> bool {
        !self.entries.borrow().is_empty()
    }

    fn inner_eq(self, other: &PyDict, vm: &VirtualMachine) -> PyResult<bool> {
        if other.entries.borrow().len() != self.entries.borrow().len() {
            return Ok(false);
        }
        for (k, v1) in self {
            match other.entries.borrow().get(vm, &k)? {
                Some(v2) => {
                    if v1.is(&v2) {
                        continue;
                    }
                    if !vm.bool_eq(v1, v2)? {
                        return Ok(false);
                    }
                }
                None => {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    fn eq(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if let Some(other) = other.payload::<PyDict>() {
            let eq = self.inner_eq(other, vm)?;
            Ok(vm.ctx.new_bool(eq))
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    fn ne(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if let Some(other) = other.payload::<PyDict>() {
            let neq = !self.inner_eq(other, vm)?;
            Ok(vm.ctx.new_bool(neq))
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    fn len(self, _vm: &VirtualMachine) -> usize {
        self.entries.borrow().len()
    }

    fn sizeof(self, _vm: &VirtualMachine) -> usize {
        size_of::<Self>() + self.entries.borrow().sizeof()
    }

    fn repr(self, vm: &VirtualMachine) -> PyResult<String> {
        let s = if let Some(_guard) = ReprGuard::enter(self.as_object()) {
            let mut str_parts = vec![];
            for (key, value) in self {
                let key_repr = vm.to_repr(&key)?;
                let value_repr = vm.to_repr(&value)?;
                str_parts.push(format!("{}: {}", key_repr.as_str(), value_repr.as_str()));
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

    fn inner_setitem(
        self,
        key: PyObjectRef,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        self.inner_setitem_fast(&key, value, vm)
    }

    /// Set item variant which can be called with multiple
    /// key types, such as str to name a notable one.
    fn inner_setitem_fast<K: DictKey + IntoPyObject + Copy>(
        &self,
        key: K,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        self.entries.borrow_mut().insert(vm, key, value)
    }

    #[cfg_attr(feature = "flame-it", flame("PyDictRef"))]
    fn inner_getitem(self, key: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if let Some(value) = self.inner_getitem_option(&key, vm)? {
            Ok(value)
        } else {
            Err(vm.new_key_error(key.clone()))
        }
    }

    /// Return an optional inner item, or an error (can be key error as well)
    fn inner_getitem_option<K: DictKey + IntoPyObject + Copy>(
        &self,
        key: K,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>> {
        if let Some(value) = self.entries.borrow().get(vm, key)? {
            return Ok(Some(value));
        }

        if let Some(method_or_err) = vm.get_method(self.clone().into_object(), "__missing__") {
            let method = method_or_err?;
            Ok(Some(vm.invoke(&method, vec![key.into_pyobject(vm)?])?))
        } else {
            Ok(None)
        }
    }

    fn get(
        self,
        key: PyObjectRef,
        default: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        match self.entries.borrow().get(vm, &key)? {
            Some(value) => Ok(value),
            None => Ok(default.unwrap_or_else(|| vm.ctx.none())),
        }
    }

    fn setdefault(
        self,
        key: PyObjectRef,
        default: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let mut entries = self.entries.borrow_mut();
        match entries.get(vm, &key)? {
            Some(value) => Ok(value),
            None => {
                let set_value = default.unwrap_or_else(|| vm.ctx.none());
                entries.insert(vm, &key, set_value.clone())?;
                Ok(set_value)
            }
        }
    }

    pub fn copy(self, _vm: &VirtualMachine) -> PyDict {
        PyDict {
            entries: self.entries.clone(),
        }
    }

    fn update(
        self,
        dict_obj: OptionalArg<PyObjectRef>,
        kwargs: KwArgs,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        PyDictRef::merge(&self.entries, dict_obj, kwargs, vm)
    }

    fn pop(
        self,
        key: PyObjectRef,
        default: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        match self.entries.borrow_mut().pop(vm, &key)? {
            Some(value) => Ok(value),
            None => match default {
                OptionalArg::Present(default) => Ok(default),
                OptionalArg::Missing => Err(vm.new_key_error(key.clone())),
            },
        }
    }

    fn popitem(self, vm: &VirtualMachine) -> PyResult {
        let mut entries = self.entries.borrow_mut();
        if let Some((key, value)) = entries.pop_front() {
            Ok(vm.ctx.new_tuple(vec![key, value]))
        } else {
            let err_msg = vm.new_str("popitem(): dictionary is empty".to_string());
            Err(vm.new_key_error(err_msg))
        }
    }

    /// Take a python dictionary and convert it to attributes.
    pub fn to_attributes(self) -> PyAttributes {
        let mut attrs = PyAttributes::new();
        for (key, value) in self {
            let key = objstr::get_value(&key);
            attrs.insert(key, value);
        }
        attrs
    }

    pub fn from_attributes(attrs: PyAttributes, vm: &VirtualMachine) -> PyResult<Self> {
        let mut dict = DictContentType::default();

        for (key, value) in attrs {
            dict.insert(vm, &vm.ctx.new_str(key), value)?;
        }

        let entries = RefCell::new(dict);
        Ok(PyDict { entries }.into_ref(vm))
    }

    fn hash(self, vm: &VirtualMachine) -> PyResult<()> {
        Err(vm.new_type_error("unhashable type".to_string()))
    }

    pub fn contains_key<T: IntoPyObject>(&self, key: T, vm: &VirtualMachine) -> bool {
        let key = key.into_pyobject(vm).unwrap();
        self.entries.borrow().contains(vm, &key).unwrap()
    }

    pub fn size(&self) -> dictdatatype::DictSize {
        self.entries.borrow().size()
    }

    /// This function can be used to get an item without raising the
    /// KeyError, so we can simply check upon the result being Some
    /// python value, or None.
    /// Note that we can pass any type which implements the DictKey
    /// trait. Notable examples are String and PyObjectRef.
    pub fn get_item_option<T: IntoPyObject + DictKey + Copy>(
        &self,
        key: T,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>> {
        // Test if this object is a true dict, or mabye a subclass?
        // If it is a dict, we can directly invoke inner_get_item_option,
        // and prevent the creation of the KeyError exception.
        // Also note, that we prevent the creation of a full PyString object
        // if we lookup local names (which happens all of the time).
        if self.typ().is(&vm.ctx.dict_type()) {
            // We can take the short path here!
            match self.inner_getitem_option(key, vm) {
                Err(exc) => {
                    if objtype::isinstance(&exc, &vm.ctx.exceptions.key_error) {
                        Ok(None)
                    } else {
                        Err(exc)
                    }
                }
                Ok(x) => Ok(x),
            }
        } else {
            // Fall back to full get_item with KeyError checking

            match self.get_item(key, vm) {
                Ok(value) => Ok(Some(value)),
                Err(exc) => {
                    if objtype::isinstance(&exc, &vm.ctx.exceptions.key_error) {
                        Ok(None)
                    } else {
                        Err(exc)
                    }
                }
            }
        }
    }
}

impl ItemProtocol for PyDictRef {
    fn get_item<T: IntoPyObject + DictKey + Copy>(&self, key: T, vm: &VirtualMachine) -> PyResult {
        self.as_object().get_item(key, vm)
    }

    fn set_item<T: IntoPyObject + DictKey + Copy>(
        &self,
        key: T,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult {
        if self.typ().is(&vm.ctx.dict_type()) {
            self.inner_setitem_fast(key, value, vm)
                .map(|_| vm.ctx.none())
        } else {
            // Fall back to slow path if we are in a dict subclass:
            self.as_object().set_item(key, value, vm)
        }
    }

    fn del_item<T: IntoPyObject + DictKey + Copy>(&self, key: T, vm: &VirtualMachine) -> PyResult {
        self.as_object().del_item(key, vm)
    }
}

// Implement IntoIterator so that we can easily iterate dictionaries from rust code.
impl IntoIterator for PyDictRef {
    type Item = (PyObjectRef, PyObjectRef);
    type IntoIter = DictIter;

    fn into_iter(self) -> Self::IntoIter {
        DictIter::new(self)
    }
}

impl IntoIterator for &PyDictRef {
    type Item = (PyObjectRef, PyObjectRef);
    type IntoIter = DictIter;

    fn into_iter(self) -> Self::IntoIter {
        DictIter::new(self.clone())
    }
}

pub struct DictIter {
    dict: PyDictRef,
    position: usize,
}

impl DictIter {
    pub fn new(dict: PyDictRef) -> DictIter {
        DictIter { dict, position: 0 }
    }
}

impl Iterator for DictIter {
    type Item = (PyObjectRef, PyObjectRef);

    fn next(&mut self) -> Option<Self::Item> {
        match self.dict.entries.borrow().next_entry(&mut self.position) {
            Some((key, value)) => Some((key.clone(), value.clone())),
            None => None,
        }
    }
}

macro_rules! dict_iterator {
    ( $name: ident, $iter_name: ident, $class: ident, $iter_class: ident, $class_name: literal, $iter_class_name: literal, $result_fn: expr) => {
        #[pyclass(name = $class_name)]
        #[derive(Debug)]
        struct $name {
            pub dict: PyDictRef,
        }

        #[pyimpl]
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
                vm.ctx.types.$class.clone()
            }
        }

        #[pyclass(name = $iter_class_name)]
        #[derive(Debug)]
        struct $iter_name {
            pub dict: PyDictRef,
            pub size: dictdatatype::DictSize,
            pub position: Cell<usize>,
        }

        #[pyimpl]
        impl $iter_name {
            fn new(dict: PyDictRef) -> Self {
                $iter_name {
                    position: Cell::new(0),
                    size: dict.size(),
                    dict,
                }
            }

            #[pymethod(name = "__next__")]
            #[allow(clippy::redundant_closure_call)]
            fn next(&self, vm: &VirtualMachine) -> PyResult {
                let mut position = self.position.get();
                let dict = self.dict.entries.borrow();
                if dict.has_changed_size(&self.size) {
                    return Err(vm.new_exception(
                        vm.ctx.exceptions.runtime_error.clone(),
                        "dictionary changed size during iteration".to_string(),
                    ));
                }
                match dict.next_entry(&mut position) {
                    Some((key, value)) => {
                        self.position.set(position);
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
                vm.ctx.types.$iter_class.clone()
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
    extend_class!(context, &context.types.dict_type, {
        "__bool__" => context.new_rustfunc(PyDictRef::bool),
        "__len__" => context.new_rustfunc(PyDictRef::len),
        "__sizeof__" => context.new_rustfunc(PyDictRef::sizeof),
        "__contains__" => context.new_rustfunc(PyDictRef::contains),
        "__delitem__" => context.new_rustfunc(PyDictRef::inner_delitem),
        "__eq__" => context.new_rustfunc(PyDictRef::eq),
        "__ne__" => context.new_rustfunc(PyDictRef::ne),
        "__getitem__" => context.new_rustfunc(PyDictRef::inner_getitem),
        "__iter__" => context.new_rustfunc(PyDictRef::iter),
        (slot new) => PyDictRef::new,
        "__repr__" => context.new_rustfunc(PyDictRef::repr),
        "__setitem__" => context.new_rustfunc(PyDictRef::inner_setitem),
        "__hash__" => context.new_rustfunc(PyDictRef::hash),
        "clear" => context.new_rustfunc(PyDictRef::clear),
        "values" => context.new_rustfunc(PyDictRef::values),
        "items" => context.new_rustfunc(PyDictRef::items),
        "keys" => context.new_rustfunc(PyDictRef::keys),
        "fromkeys" => context.new_classmethod(PyDictRef::fromkeys),
        "get" => context.new_rustfunc(PyDictRef::get),
        "setdefault" => context.new_rustfunc(PyDictRef::setdefault),
        "copy" => context.new_rustfunc(PyDictRef::copy),
        "update" => context.new_rustfunc(PyDictRef::update),
        "pop" => context.new_rustfunc(PyDictRef::pop),
        "popitem" => context.new_rustfunc(PyDictRef::popitem),
    });

    PyDictKeys::extend_class(context, &context.types.dictkeys_type);
    PyDictKeyIterator::extend_class(context, &context.types.dictkeyiterator_type);
    PyDictValues::extend_class(context, &context.types.dictvalues_type);
    PyDictValueIterator::extend_class(context, &context.types.dictvalueiterator_type);
    PyDictItems::extend_class(context, &context.types.dictitems_type);
    PyDictItemIterator::extend_class(context, &context.types.dictitemiterator_type);
}

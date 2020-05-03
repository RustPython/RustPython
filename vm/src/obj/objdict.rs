use std::cell::Cell;
use std::fmt;

use super::objiter;
use super::objstr;
use super::objtype::{self, PyClassRef};
use crate::dictdatatype::{self, DictKey};
use crate::exceptions::PyBaseExceptionRef;
use crate::function::{KwArgs, OptionalArg, PyFuncArgs};
use crate::pyobject::{
    IdProtocol, IntoPyObject, ItemProtocol, PyAttributes, PyClassImpl, PyContext, PyIterable,
    PyObjectRef, PyRef, PyResult, PyValue, ThreadSafe,
};
use crate::vm::{ReprGuard, VirtualMachine};

use std::mem::size_of;

pub type DictContentType = dictdatatype::Dict;

#[pyclass]
#[derive(Default)]
pub struct PyDict {
    entries: DictContentType,
}
pub type PyDictRef = PyRef<PyDict>;
impl ThreadSafe for PyDict {}

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
#[pyimpl(flags(BASETYPE))]
impl PyDictRef {
    #[pyslot]
    fn tp_new(class: PyClassRef, _args: PyFuncArgs, vm: &VirtualMachine) -> PyResult<PyDictRef> {
        PyDict {
            entries: DictContentType::default(),
        }
        .into_ref_with_type(vm, class)
    }

    #[pymethod(magic)]
    fn init(
        self,
        dict_obj: OptionalArg<PyObjectRef>,
        kwargs: KwArgs,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        PyDictRef::merge(&self.entries, dict_obj, kwargs, vm)
    }

    fn merge(
        dict: &DictContentType,
        dict_obj: OptionalArg<PyObjectRef>,
        kwargs: KwArgs,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        if let OptionalArg::Present(dict_obj) = dict_obj {
            let dicted: Result<PyDictRef, _> = dict_obj.clone().downcast();
            if let Ok(dict_obj) = dicted {
                for (key, value) in dict_obj {
                    dict.insert(vm, &key, value)?;
                }
            } else if let Some(keys) = vm.get_method(dict_obj.clone(), "keys") {
                let keys = objiter::get_iter(vm, &vm.invoke(&keys?, vec![])?)?;
                while let Some(key) = objiter::get_next_object(vm, &keys)? {
                    let val = dict_obj.get_item(&key, vm)?;
                    dict.insert(vm, &key, val)?;
                }
            } else {
                let iter = objiter::get_iter(vm, &dict_obj)?;
                loop {
                    fn err(vm: &VirtualMachine) -> PyBaseExceptionRef {
                        vm.new_type_error("Iterator must have exactly two elements".to_owned())
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
        Ok(())
    }

    fn merge_dict(
        dict: &DictContentType,
        dict_other: PyDictRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        for (key, value) in dict_other {
            dict.insert(vm, &key, value)?;
        }
        Ok(())
    }

    #[pyclassmethod]
    fn fromkeys(
        class: PyClassRef,
        iterable: PyIterable,
        value: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyDictRef> {
        let dict = DictContentType::default();
        let value = value.unwrap_or_else(|| vm.ctx.none());
        for elem in iterable.iter(vm)? {
            let elem = elem?;
            dict.insert(vm, &elem, value.clone())?;
        }
        PyDict { entries: dict }.into_ref_with_type(vm, class)
    }

    #[pymethod(magic)]
    fn bool(self) -> bool {
        !self.entries.is_empty()
    }

    fn inner_eq(self, other: &PyDict, vm: &VirtualMachine) -> PyResult<bool> {
        if other.entries.len() != self.entries.len() {
            return Ok(false);
        }
        for (k, v1) in self {
            match other.entries.get(vm, &k)? {
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

    #[pymethod(magic)]
    fn eq(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if let Some(other) = other.payload::<PyDict>() {
            let eq = self.inner_eq(other, vm)?;
            Ok(vm.ctx.new_bool(eq))
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    #[pymethod(magic)]
    fn ne(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if let Some(other) = other.payload::<PyDict>() {
            let neq = !self.inner_eq(other, vm)?;
            Ok(vm.ctx.new_bool(neq))
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    #[pymethod(magic)]
    fn len(self) -> usize {
        self.entries.len()
    }

    #[pymethod(magic)]
    fn sizeof(self) -> usize {
        size_of::<Self>() + self.entries.sizeof()
    }

    #[pymethod(magic)]
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
            "{...}".to_owned()
        };
        Ok(s)
    }

    #[pymethod(magic)]
    fn contains(self, key: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        self.entries.contains(vm, &key)
    }

    #[pymethod(magic)]
    fn delitem(self, key: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.entries.delete(vm, &key)
    }

    #[pymethod]
    fn clear(self) {
        self.entries.clear()
    }

    #[pymethod(magic)]
    fn iter(self) -> PyDictKeyIterator {
        PyDictKeyIterator::new(self)
    }

    #[pymethod]
    fn keys(self) -> PyDictKeys {
        PyDictKeys::new(self)
    }

    #[pymethod]
    fn values(self) -> PyDictValues {
        PyDictValues::new(self)
    }

    #[pymethod]
    fn items(self) -> PyDictItems {
        PyDictItems::new(self)
    }

    #[pymethod(magic)]
    fn setitem(self, key: PyObjectRef, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
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
        self.entries.insert(vm, key, value)
    }

    #[pymethod(magic)]
    #[cfg_attr(feature = "flame-it", flame("PyDictRef"))]
    fn getitem(self, key: PyObjectRef, vm: &VirtualMachine) -> PyResult {
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
        if let Some(value) = self.entries.get(vm, key)? {
            return Ok(Some(value));
        }

        if let Some(method_or_err) = vm.get_method(self.clone().into_object(), "__missing__") {
            let method = method_or_err?;
            Ok(Some(vm.invoke(&method, vec![key.into_pyobject(vm)?])?))
        } else {
            Ok(None)
        }
    }

    #[pymethod]
    fn get(
        self,
        key: PyObjectRef,
        default: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        match self.entries.get(vm, &key)? {
            Some(value) => Ok(value),
            None => Ok(default.unwrap_or_else(|| vm.ctx.none())),
        }
    }

    #[pymethod]
    fn setdefault(
        self,
        key: PyObjectRef,
        default: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        match self.entries.get(vm, &key)? {
            Some(value) => Ok(value),
            None => {
                let set_value = default.unwrap_or_else(|| vm.ctx.none());
                self.entries.insert(vm, &key, set_value.clone())?;
                Ok(set_value)
            }
        }
    }

    #[pymethod]
    pub fn copy(self) -> PyDict {
        PyDict {
            entries: self.entries.clone(),
        }
    }

    #[pymethod]
    fn update(
        self,
        dict_obj: OptionalArg<PyObjectRef>,
        kwargs: KwArgs,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        PyDictRef::merge(&self.entries, dict_obj, kwargs, vm)
    }

    #[pymethod(name = "__ior__")]
    fn ior(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let dicted: Result<PyDictRef, _> = other.clone().downcast();
        if let Ok(other) = dicted {
            PyDictRef::merge_dict(&self.entries, other, vm)?;
            return Ok(self.into_object());
        }
        Err(vm.new_type_error("__ior__ not implemented for non-dict type".to_owned()))
    }

    #[pymethod(name = "__ror__")]
    fn ror(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyDict> {
        let dicted: Result<PyDictRef, _> = other.clone().downcast();
        if let Ok(other) = dicted {
            let other_cp = other.copy();
            PyDictRef::merge_dict(&other_cp.entries, self, vm)?;
            return Ok(other_cp);
        }
        Err(vm.new_type_error("__ror__ not implemented for non-dict type".to_owned()))
    }

    #[pymethod(name = "__or__")]
    fn or(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyDict> {
        let dicted: Result<PyDictRef, _> = other.clone().downcast();
        if let Ok(other) = dicted {
            let self_cp = self.copy();
            PyDictRef::merge_dict(&self_cp.entries, other, vm)?;
            return Ok(self_cp);
        }
        Err(vm.new_type_error("__or__ not implemented for non-dict type".to_owned()))
    }

    #[pymethod]
    fn pop(
        self,
        key: PyObjectRef,
        default: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        match self.entries.pop(vm, &key)? {
            Some(value) => Ok(value),
            None => match default {
                OptionalArg::Present(default) => Ok(default),
                OptionalArg::Missing => Err(vm.new_key_error(key.clone())),
            },
        }
    }

    #[pymethod]
    fn popitem(self, vm: &VirtualMachine) -> PyResult {
        if let Some((key, value)) = self.entries.pop_front() {
            Ok(vm.ctx.new_tuple(vec![key, value]))
        } else {
            let err_msg = vm.new_str("popitem(): dictionary is empty".to_owned());
            Err(vm.new_key_error(err_msg))
        }
    }

    /// Take a python dictionary and convert it to attributes.
    pub fn to_attributes(self) -> PyAttributes {
        let mut attrs = PyAttributes::new();
        for (key, value) in self {
            let key = objstr::clone_value(&key);
            attrs.insert(key, value);
        }
        attrs
    }

    pub fn from_attributes(attrs: PyAttributes, vm: &VirtualMachine) -> PyResult<Self> {
        let dict = DictContentType::default();

        for (key, value) in attrs {
            dict.insert(vm, &vm.ctx.new_str(key), value)?;
        }

        Ok(PyDict { entries: dict }.into_ref(vm))
    }

    #[pymethod(magic)]
    fn hash(self, vm: &VirtualMachine) -> PyResult<()> {
        Err(vm.new_type_error("unhashable type".to_owned()))
    }

    pub fn contains_key<T: IntoPyObject>(&self, key: T, vm: &VirtualMachine) -> bool {
        let key = key.into_pyobject(vm).unwrap();
        self.entries.contains(vm, &key).unwrap()
    }

    pub fn size(&self) -> dictdatatype::DictSize {
        self.entries.size()
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
        match self.dict.entries.next_entry(&mut self.position) {
            Some((key, value)) => Some((key, value)),
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
            fn iter(&self) -> $iter_name {
                $iter_name::new(self.dict.clone())
            }

            #[pymethod(name = "__len__")]
            fn len(&self) -> usize {
                self.dict.clone().len()
            }

            #[pymethod(name = "__repr__")]
            #[allow(clippy::redundant_closure_call)]
            fn repr(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<String> {
                let s = if let Some(_guard) = ReprGuard::enter(zelf.as_object()) {
                    let mut str_parts = vec![];
                    for (key, value) in zelf.dict.clone() {
                        let s = vm.to_repr(&$result_fn(vm, key, value))?;
                        str_parts.push(s.as_str().to_owned());
                    }
                    format!("{}([{}])", $class_name, str_parts.join(", "))
                } else {
                    "{...}".to_owned()
                };
                Ok(s)
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
                if self.dict.entries.has_changed_size(&self.size) {
                    return Err(
                        vm.new_runtime_error("dictionary changed size during iteration".to_owned())
                    );
                }
                match self.dict.entries.next_entry(&mut position) {
                    Some((key, value)) => {
                        self.position.set(position);
                        Ok($result_fn(vm, key, value))
                    }
                    None => Err(objiter::new_stop_iteration(vm)),
                }
            }

            #[pymethod(name = "__iter__")]
            fn iter(zelf: PyRef<Self>) -> PyRef<Self> {
                zelf
            }

            #[pymethod(name = "__length_hint__")]
            fn length_hint(&self) -> usize {
                self.dict.entries.len_from_entry_index(self.position.get())
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
    "dict_keys",
    "dictkeyiterator",
    |_vm: &VirtualMachine, key: PyObjectRef, _value: PyObjectRef| key
}

dict_iterator! {
    PyDictValues,
    PyDictValueIterator,
    dictvalues_type,
    dictvalueiterator_type,
    "dict_values",
    "dictvalueiterator",
    |_vm: &VirtualMachine, _key: PyObjectRef, value: PyObjectRef| value
}

dict_iterator! {
    PyDictItems,
    PyDictItemIterator,
    dictitems_type,
    dictitemiterator_type,
    "dict_items",
    "dictitemiterator",
    |vm: &VirtualMachine, key: PyObjectRef, value: PyObjectRef|
        vm.ctx.new_tuple(vec![key, value])
}

pub(crate) fn init(context: &PyContext) {
    PyDictRef::extend_class(context, &context.types.dict_type);
    PyDictKeys::extend_class(context, &context.types.dictkeys_type);
    PyDictKeyIterator::extend_class(context, &context.types.dictkeyiterator_type);
    PyDictValues::extend_class(context, &context.types.dictvalues_type);
    PyDictValueIterator::extend_class(context, &context.types.dictvalueiterator_type);
    PyDictItems::extend_class(context, &context.types.dictitems_type);
    PyDictItemIterator::extend_class(context, &context.types.dictitemiterator_type);
}

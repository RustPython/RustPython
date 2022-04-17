use crate::{
    builtins::{PyList, PyListRef, PySlice, PyTuple, PyTupleRef},
    common::lock::OnceCell,
    function::{PyArithmeticValue, ToPyObject},
    protocol::PyMapping,
    AsPyObject, PyObject, PyObjectRef, PyResult, PyValue, VirtualMachine,
};
use itertools::Itertools;
use std::{
    borrow::{Borrow, Cow},
    fmt::Debug,
};

// Sequence Protocol
// https://docs.python.org/3/c-api/sequence.html

#[allow(clippy::type_complexity)]
#[derive(Default, Clone)]
pub struct PySequenceMethods {
    pub length: Option<fn(&PySequence, &VirtualMachine) -> PyResult<usize>>,
    pub concat: Option<fn(&PySequence, &PyObject, &VirtualMachine) -> PyResult>,
    pub repeat: Option<fn(&PySequence, usize, &VirtualMachine) -> PyResult>,
    pub item: Option<fn(&PySequence, isize, &VirtualMachine) -> PyResult>,
    pub ass_item:
        Option<fn(&PySequence, isize, Option<PyObjectRef>, &VirtualMachine) -> PyResult<()>>,
    pub contains: Option<fn(&PySequence, &PyObject, &VirtualMachine) -> PyResult<bool>>,
    pub inplace_concat: Option<fn(&PySequence, &PyObject, &VirtualMachine) -> PyResult>,
    pub inplace_repeat: Option<fn(&PySequence, usize, &VirtualMachine) -> PyResult>,
}

impl PySequenceMethods {
    pub const fn not_implemented() -> &'static Self {
        &NOT_IMPLEMENTED
    }
}

impl Debug for PySequenceMethods {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PySequenceMethods")
            .field("length", &self.length.map(|x| x as usize))
            .field("concat", &self.concat.map(|x| x as usize))
            .field("repeat", &self.repeat.map(|x| x as usize))
            .field("item", &self.item.map(|x| x as usize))
            .field("ass_item", &self.ass_item.map(|x| x as usize))
            .field("contains", &self.contains.map(|x| x as usize))
            .field("inplace_concat", &self.inplace_concat.map(|x| x as usize))
            .field("inplace_repeat", &self.inplace_repeat.map(|x| x as usize))
            .finish()
    }
}

pub struct PySequence<'a> {
    pub obj: &'a PyObject,
    // some function don't need it, so lazy initialize
    methods: OnceCell<Cow<'static, PySequenceMethods>>,
}

impl<'a> From<&'a PyObject> for PySequence<'a> {
    fn from(obj: &'a PyObject) -> Self {
        Self {
            obj,
            methods: OnceCell::new(),
        }
    }
}

impl<'a> PySequence<'a> {
    pub fn with_methods(obj: &'a PyObject, methods: Cow<'static, PySequenceMethods>) -> Self {
        Self {
            obj,
            methods: OnceCell::from(methods),
        }
    }

    pub fn try_protocol(obj: &'a PyObject, vm: &VirtualMachine) -> PyResult<Self> {
        let zelf = Self::from(obj);
        if zelf.check(vm) {
            Ok(zelf)
        } else {
            Err(vm.new_type_error(format!("'{}' is not a sequence", obj.class())))
        }
    }
}

impl PySequence<'_> {
    // PySequence_Check
    pub fn check(&self, vm: &VirtualMachine) -> bool {
        self.methods(vm).item.is_some()
    }

    pub fn methods(&self, vm: &VirtualMachine) -> &PySequenceMethods {
        self.methods_cow(vm).borrow()
    }

    pub fn methods_cow(&self, vm: &VirtualMachine) -> &Cow<'static, PySequenceMethods> {
        self.methods.get_or_init(|| {
            let cls = self.obj.class();
            if !cls.is(&vm.ctx.types.dict_type) {
                if let Some(f) = cls.mro_find_map(|x| x.slots.as_sequence.load()) {
                    return f(self.obj, vm);
                }
            }
            Cow::Borrowed(PySequenceMethods::not_implemented())
        })
    }

    pub fn length_opt(&self, vm: &VirtualMachine) -> Option<PyResult<usize>> {
        self.methods(vm).length.map(|f| f(self, vm))
    }

    pub fn length(&self, vm: &VirtualMachine) -> PyResult<usize> {
        self.length_opt(vm).ok_or_else(|| {
            vm.new_type_error(format!(
                "'{}' is not a sequence or has no len()",
                self.obj.class()
            ))
        })?
    }

    pub fn concat(&self, other: &PyObject, vm: &VirtualMachine) -> PyResult {
        if let Some(f) = self.methods(vm).concat {
            return f(self, other, vm);
        }

        // if both arguments apear to be sequences, try fallback to __add__
        if self.check(vm) && PySequence::from(other).check(vm) {
            let ret = vm._add(self.obj, other)?;
            if let PyArithmeticValue::Implemented(ret) = PyArithmeticValue::from_object(vm, ret) {
                return Ok(ret);
            }
        }
        Err(vm.new_type_error(format!(
            "'{}' object can't be concatenated",
            self.obj.class()
        )))
    }

    pub fn repeat(&self, n: usize, vm: &VirtualMachine) -> PyResult {
        if let Some(f) = self.methods(vm).repeat {
            return f(self, n, vm);
        }

        // try fallback to __mul__
        if self.check(vm) {
            let ret = vm._mul(self.obj, &n.to_pyobject(vm))?;
            if let PyArithmeticValue::Implemented(ret) = PyArithmeticValue::from_object(vm, ret) {
                return Ok(ret);
            }
        }
        Err(vm.new_type_error(format!("'{}' object can't be repeated", self.obj.class())))
    }

    pub fn inplace_concat(&self, other: &PyObject, vm: &VirtualMachine) -> PyResult {
        if let Some(f) = self.methods(vm).inplace_concat {
            return f(self, other, vm);
        }
        if let Some(f) = self.methods(vm).concat {
            return f(self, other, vm);
        }

        // if both arguments apear to be sequences, try fallback to __iadd__
        if self.check(vm) && PySequence::from(other).check(vm) {
            let ret = vm._iadd(self.obj, other)?;
            if let PyArithmeticValue::Implemented(ret) = PyArithmeticValue::from_object(vm, ret) {
                return Ok(ret);
            }
        }
        Err(vm.new_type_error(format!(
            "'{}' object can't be concatenated",
            self.obj.class()
        )))
    }

    pub fn inplace_repeat(&self, n: usize, vm: &VirtualMachine) -> PyResult {
        if let Some(f) = self.methods(vm).inplace_repeat {
            return f(self, n, vm);
        }
        if let Some(f) = self.methods(vm).repeat {
            return f(self, n, vm);
        }

        if self.check(vm) {
            let ret = vm._imul(self.obj, &n.to_pyobject(vm))?;
            if let PyArithmeticValue::Implemented(ret) = PyArithmeticValue::from_object(vm, ret) {
                return Ok(ret);
            }
        }
        Err(vm.new_type_error(format!("'{}' object can't be repeated", self.obj.class())))
    }

    pub fn get_item(&self, i: isize, vm: &VirtualMachine) -> PyResult {
        if let Some(f) = self.methods(vm).item {
            return f(self, i, vm);
        }
        Err(vm.new_type_error(format!(
            "'{}' is not a sequence or does not support indexing",
            self.obj.class()
        )))
    }

    fn _ass_item(&self, i: isize, value: Option<PyObjectRef>, vm: &VirtualMachine) -> PyResult<()> {
        if let Some(f) = self.methods(vm).ass_item {
            return f(self, i, value, vm);
        }
        Err(vm.new_type_error(format!(
            "'{}' is not a sequence or doesn't support item {}",
            self.obj.class(),
            if value.is_some() {
                "assignment"
            } else {
                "deletion"
            }
        )))
    }

    pub fn set_item(&self, i: isize, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self._ass_item(i, Some(value), vm)
    }

    pub fn del_item(&self, i: isize, vm: &VirtualMachine) -> PyResult<()> {
        self._ass_item(i, None, vm)
    }

    pub fn get_slice(&self, start: isize, stop: isize, vm: &VirtualMachine) -> PyResult {
        if let Ok(mapping) = PyMapping::try_protocol(self.obj, vm) {
            let slice = PySlice {
                start: Some(start.to_pyobject(vm)),
                stop: stop.to_pyobject(vm),
                step: None,
            };
            mapping.subscript(&slice.into_object(vm), vm)
        } else {
            Err(vm.new_type_error(format!("'{}' object is unsliceable", self.obj.class())))
        }
    }

    fn _ass_slice(
        &self,
        start: isize,
        stop: isize,
        value: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let mapping = PyMapping::from(self.obj);
        if let Some(f) = mapping.methods(vm).ass_subscript {
            let slice = PySlice {
                start: Some(start.to_pyobject(vm)),
                stop: stop.to_pyobject(vm),
                step: None,
            };
            f(&mapping, &slice.into_object(vm), value, vm)
        } else {
            Err(vm.new_type_error(format!(
                "'{}' object doesn't support slice {}",
                self.obj.class(),
                if value.is_some() {
                    "assignment"
                } else {
                    "deletion"
                }
            )))
        }
    }

    pub fn set_slice(
        &self,
        start: isize,
        stop: isize,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        self._ass_slice(start, stop, Some(value), vm)
    }

    pub fn del_slice(&self, start: isize, stop: isize, vm: &VirtualMachine) -> PyResult<()> {
        self._ass_slice(start, stop, None, vm)
    }

    pub fn tuple(&self, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
        if let Some(tuple) = self.obj.downcast_ref_if_exact::<PyTuple>(vm) {
            Ok(tuple.to_owned())
        } else if let Some(list) = self.obj.downcast_ref_if_exact::<PyList>(vm) {
            Ok(vm.ctx.new_tuple(list.borrow_vec().to_vec()))
        } else {
            let iter = self.obj.to_owned().get_iter(vm)?;
            let iter = iter.iter(vm)?;
            Ok(vm.ctx.new_tuple(iter.try_collect()?))
        }
    }

    pub fn list(&self, vm: &VirtualMachine) -> PyResult<PyListRef> {
        let list = vm.ctx.new_list(self.obj.try_to_value(vm)?);
        Ok(list)
    }

    pub fn contains(&self, target: &PyObject, vm: &VirtualMachine) -> PyResult<bool> {
        if let Some(f) = self.methods(vm).contains {
            return f(self, target, vm);
        }

        let iter = self.obj.to_owned().get_iter(vm)?;
        let iter = iter.iter::<PyObjectRef>(vm)?;

        for elem in iter {
            let elem = elem?;
            if vm.bool_eq(&elem, target)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn count(&self, target: &PyObject, vm: &VirtualMachine) -> PyResult<usize> {
        let mut n = 0;

        let iter = self.obj.to_owned().get_iter(vm)?;
        let iter = iter.iter::<PyObjectRef>(vm)?;

        for elem in iter {
            let elem = elem?;
            if vm.bool_eq(&elem, target)? {
                if n == isize::MAX as usize {
                    return Err(vm.new_overflow_error("index exceeds C integer size".to_string()));
                }
                n += 1;
            }
        }

        Ok(n)
    }

    pub fn index(&self, target: &PyObject, vm: &VirtualMachine) -> PyResult<usize> {
        let mut index: isize = -1;

        let iter = self.obj.to_owned().get_iter(vm)?;
        let iter = iter.iter::<PyObjectRef>(vm)?;

        for elem in iter {
            if index == isize::MAX {
                return Err(vm.new_overflow_error("index exceeds C integer size".to_string()));
            }
            index += 1;

            let elem = elem?;
            if vm.bool_eq(&elem, target)? {
                return Ok(index as usize);
            }
        }

        Err(vm.new_value_error("sequence.index(x): x not in sequence".to_string()))
    }

    pub fn extract<F, R>(&self, mut f: F, vm: &VirtualMachine) -> PyResult<Vec<R>>
    where
        F: FnMut(&PyObject) -> PyResult<R>,
    {
        if let Some(tuple) = self.obj.payload_if_exact::<PyTuple>(vm) {
            tuple.as_slice().iter().map(|x| f(x.as_ref())).collect()
        } else if let Some(list) = self.obj.payload_if_exact::<PyList>(vm) {
            list.borrow_vec().iter().map(|x| f(x.as_ref())).collect()
        } else {
            let iter = self.obj.to_owned().get_iter(vm)?;
            let iter = iter.iter::<PyObjectRef>(vm)?;
            let len = self.length(vm).unwrap_or(0);
            let mut v = Vec::with_capacity(len);
            for x in iter {
                v.push(f(x?.as_ref())?);
            }
            v.shrink_to_fit();
            Ok(v)
        }
    }

    pub fn extract_cloned<F, R>(&self, mut f: F, vm: &VirtualMachine) -> PyResult<Vec<R>>
    where
        F: FnMut(PyObjectRef) -> PyResult<R>,
    {
        if let Some(tuple) = self.obj.payload_if_exact::<PyTuple>(vm) {
            tuple.as_slice().iter().map(|x| f(x.clone())).collect()
        } else if let Some(list) = self.obj.payload_if_exact::<PyList>(vm) {
            list.borrow_vec().iter().map(|x| f(x.clone())).collect()
        } else {
            let iter = self.obj.to_owned().get_iter(vm)?;
            let iter = iter.iter::<PyObjectRef>(vm)?;
            let len = self.length(vm).unwrap_or(0);
            let mut v = Vec::with_capacity(len);
            for x in iter {
                v.push(f(x?)?);
            }
            v.shrink_to_fit();
            Ok(v)
        }
    }
}

const NOT_IMPLEMENTED: PySequenceMethods = PySequenceMethods {
    length: None,
    concat: None,
    repeat: None,
    item: None,
    ass_item: None,
    contains: None,
    inplace_concat: None,
    inplace_repeat: None,
};

use std::borrow::{Borrow, Cow};
use std::fmt::Debug;

use itertools::Itertools;

use crate::{
    builtins::{PyList, PySlice},
    function::IntoPyObject,
    IdProtocol, PyArithmeticValue, PyObject, PyObjectPayload, PyObjectRef, PyObjectView, PyResult,
    PyValue, TryFromObject, TypeProtocol, VirtualMachine,
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

#[derive(Debug)]
pub struct PySequence {
    pub obj: PyObjectRef,
    methods: Cow<'static, PySequenceMethods>,
}

impl PySequence {
    pub fn obj_as<T: PyObjectPayload>(&self) -> &PyObjectView<T> {
        unsafe { self.obj.downcast_unchecked_ref::<T>() }
    }

    pub fn check(obj: &PyObject, vm: &VirtualMachine) -> bool {
        let cls = obj.class();
        if cls.is(&vm.ctx.types.dict_type) {
            return false;
        }
        cls.mro_find_map(|x| x.slots.as_sequence.load())
            .map(|f| f(obj, vm).item.is_some())
            .unwrap_or(false)
    }

    pub fn from_object(vm: &VirtualMachine, obj: PyObjectRef) -> Option<Self> {
        let cls = obj.class();
        if cls.is(&vm.ctx.types.dict_type) {
            return None;
        }
        let f = cls.mro_find_map(|x| x.slots.as_sequence.load())?;
        drop(cls);
        let methods = f(&obj, vm);
        if methods.item.is_some() {
            Some(Self { obj, methods })
        } else {
            None
        }
    }

    pub fn methods(&self) -> &PySequenceMethods {
        self.methods.borrow()
    }

    pub fn length(&self, vm: &VirtualMachine) -> PyResult<usize> {
        if let Some(f) = self.methods().length {
            f(self, vm)
        } else {
            Err(vm.new_type_error(format!(
                "'{}' is not a sequence or has no len()",
                self.obj.class().name()
            )))
        }
    }

    pub fn concat(&self, other: &PyObject, vm: &VirtualMachine) -> PyResult {
        if let Some(f) = self.methods().concat {
            return f(self, other, vm);
        }
        try_add_for_concat(&self.obj, other, vm)
    }

    pub fn repeat(&self, n: usize, vm: &VirtualMachine) -> PyResult {
        if let Some(f) = self.methods().repeat {
            return f(self, n, vm);
        }
        try_mul_for_repeat(&self.obj, n, vm)
    }

    pub fn inplace_concat(&self, other: &PyObject, vm: &VirtualMachine) -> PyResult {
        if let Some(f) = self.methods().inplace_concat {
            return f(self, other, vm);
        }
        if let Some(f) = self.methods().concat {
            return f(self, other, vm);
        }
        try_iadd_for_inplace_concat(&self.obj, other, vm)
    }

    pub fn inplace_repeat(&self, n: usize, vm: &VirtualMachine) -> PyResult {
        if let Some(f) = self.methods().inplace_repeat {
            return f(self, n, vm);
        }
        if let Some(f) = self.methods().repeat {
            return f(self, n, vm);
        }
        try_imul_for_inplace_repeat(&self.obj, n, vm)
    }

    pub fn get_item(&self, i: isize, vm: &VirtualMachine) -> PyResult {
        if let Some(f) = self.methods().item {
            return f(self, i, vm);
        }
        Err(vm.new_type_error(format!(
            "'{}' is not a sequence or does not support indexing",
            self.obj.class().name()
        )))
    }

    fn _ass_item(&self, i: isize, value: Option<PyObjectRef>, vm: &VirtualMachine) -> PyResult<()> {
        if let Some(f) = self.methods().ass_item {
            return f(self, i, value, vm);
        }
        Err(vm.new_type_error(format!(
            "'{}' is not a sequence or doesn't support item {}",
            self.obj.class().name(),
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
        if let Some(f) = self.obj.class().mro_find_map(|x| x.slots.as_mapping.load()) {
            let mp = f(&self.obj, vm);
            if let Some(subscript) = mp.subscript {
                let slice = PySlice {
                    start: Some(start.into_pyobject(vm)),
                    stop: stop.into_pyobject(vm),
                    step: None,
                };

                return subscript(self.obj.clone(), slice.into_object(vm), vm);
            }
        }
        Err(vm.new_type_error(format!(
            "'{}' object is unsliceable",
            self.obj.class().name()
        )))
    }

    fn _ass_slice(
        &self,
        start: isize,
        stop: isize,
        value: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let cls = self.obj.class();
        if let Some(f) = cls.mro_find_map(|x| x.slots.as_mapping.load()) {
            drop(cls);
            let mp = f(&self.obj, vm);
            if let Some(ass_subscript) = mp.ass_subscript {
                let slice = PySlice {
                    start: Some(start.into_pyobject(vm)),
                    stop: stop.into_pyobject(vm),
                    step: None,
                };

                return ass_subscript(self.obj.clone(), slice.into_object(vm), value, vm);
            }
        }
        Err(vm.new_type_error(format!(
            "'{}' object doesn't support slice {}",
            self.obj.class().name(),
            if value.is_some() {
                "assignment"
            } else {
                "deletion"
            }
        )))
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

    pub fn tuple(&self, vm: &VirtualMachine) -> PyResult {
        if self.obj.class().is(&vm.ctx.types.tuple_type) {
            return Ok(self.obj.clone());
        }
        if self.obj.class().is(&vm.ctx.types.list_type) {
            let list = self.obj.payload::<PyList>().unwrap();
            return Ok(vm.ctx.new_tuple(list.borrow_vec().to_vec()).into());
        }

        let iter = self.obj.clone().get_iter(vm)?;
        let iter = iter.iter(vm)?;
        Ok(vm.ctx.new_tuple(iter.try_collect()?).into())
    }

    pub fn list(&self, vm: &VirtualMachine) -> PyResult {
        let list = vm.ctx.new_list(vec![]);
        list.extend(self.obj.clone(), vm)?;
        Ok(list.into())
    }

    pub fn contains(&self, target: &PyObject, vm: &VirtualMachine) -> PyResult<bool> {
        if let Some(f) = self.methods().contains {
            return f(self, target, vm);
        }

        let iter = self.obj.clone().get_iter(vm)?;
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

        let iter = self.obj.clone().get_iter(vm)?;
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

        let iter = self.obj.clone().get_iter(vm)?;
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
}

impl TryFromObject for PySequence {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        PySequence::from_object(vm, obj)
            .ok_or_else(|| vm.new_type_error("'{}' is not a sequence".to_string()))
    }
}

pub fn try_add_for_concat(a: &PyObject, b: &PyObject, vm: &VirtualMachine) -> PyResult {
    if PySequence::check(b, vm) {
        let ret = vm._add(a, b)?;
        if let PyArithmeticValue::Implemented(ret) = PyArithmeticValue::from_object(vm, ret) {
            return Ok(ret);
        }
    }
    Err(vm.new_type_error(format!(
        "'{}' object can't be concatenated",
        a.class().name()
    )))
}

pub fn try_mul_for_repeat(a: &PyObject, n: usize, vm: &VirtualMachine) -> PyResult {
    let ret = vm._mul(a, &n.into_pyobject(vm))?;
    if let PyArithmeticValue::Implemented(ret) = PyArithmeticValue::from_object(vm, ret) {
        return Ok(ret);
    }
    Err(vm.new_type_error(format!("'{}' object can't be repeated", a.class().name())))
}

pub fn try_iadd_for_inplace_concat(a: &PyObject, b: &PyObject, vm: &VirtualMachine) -> PyResult {
    if PySequence::check(b, vm) {
        let ret = vm._iadd(a, b)?;
        if let PyArithmeticValue::Implemented(ret) = PyArithmeticValue::from_object(vm, ret) {
            return Ok(ret);
        }
    }
    Err(vm.new_type_error(format!(
        "'{}' object can't be concatenated",
        a.class().name()
    )))
}

pub fn try_imul_for_inplace_repeat(a: &PyObject, n: usize, vm: &VirtualMachine) -> PyResult {
    let ret = vm._imul(a, &n.into_pyobject(vm))?;
    if let PyArithmeticValue::Implemented(ret) = PyArithmeticValue::from_object(vm, ret) {
        return Ok(ret);
    }
    Err(vm.new_type_error(format!("'{}' object can't be repeated", a.class().name())))
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

use crate::{
    builtins::{type_::PointerSlot, PyList, PyListRef, PySlice, PyTuple, PyTupleRef},
    convert::ToPyObject,
    function::PyArithmeticValue,
    protocol::PyMapping,
    AsObject, PyObject, PyObjectRef, PyPayload, PyResult, VirtualMachine,
};
use crossbeam_utils::atomic::AtomicCell;
use itertools::Itertools;
use std::fmt::Debug;

// Sequence Protocol
// https://docs.python.org/3/c-api/sequence.html

impl PyObject {
    #[inline]
    pub fn to_sequence(&self, vm: &VirtualMachine) -> PySequence<'_> {
        static GLOBAL_NOT_IMPLEMENTED: PySequenceMethods = PySequenceMethods::NOT_IMPLEMENTED;
        PySequence {
            obj: self,
            methods: PySequence::find_methods(self, vm)
                .map_or(&GLOBAL_NOT_IMPLEMENTED, |x| unsafe { x.borrow_static() }),
        }
    }
}

#[allow(clippy::type_complexity)]
#[derive(Default)]
pub struct PySequenceMethods {
    pub length: AtomicCell<Option<fn(PySequence, &VirtualMachine) -> PyResult<usize>>>,
    pub concat: AtomicCell<Option<fn(PySequence, &PyObject, &VirtualMachine) -> PyResult>>,
    pub repeat: AtomicCell<Option<fn(PySequence, usize, &VirtualMachine) -> PyResult>>,
    pub item: AtomicCell<Option<fn(PySequence, isize, &VirtualMachine) -> PyResult>>,
    pub ass_item: AtomicCell<
        Option<fn(PySequence, isize, Option<PyObjectRef>, &VirtualMachine) -> PyResult<()>>,
    >,
    pub contains: AtomicCell<Option<fn(PySequence, &PyObject, &VirtualMachine) -> PyResult<bool>>>,
    pub inplace_concat: AtomicCell<Option<fn(PySequence, &PyObject, &VirtualMachine) -> PyResult>>,
    pub inplace_repeat: AtomicCell<Option<fn(PySequence, usize, &VirtualMachine) -> PyResult>>,
}

impl Debug for PySequenceMethods {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Sequence Methods")
    }
}

impl PySequenceMethods {
    #[allow(clippy::declare_interior_mutable_const)]
    pub const NOT_IMPLEMENTED: PySequenceMethods = PySequenceMethods {
        length: AtomicCell::new(None),
        concat: AtomicCell::new(None),
        repeat: AtomicCell::new(None),
        item: AtomicCell::new(None),
        ass_item: AtomicCell::new(None),
        contains: AtomicCell::new(None),
        inplace_concat: AtomicCell::new(None),
        inplace_repeat: AtomicCell::new(None),
    };
}

#[derive(Copy, Clone)]
pub struct PySequence<'a> {
    pub obj: &'a PyObject,
    pub methods: &'static PySequenceMethods,
}

impl<'a> PySequence<'a> {
    #[inline]
    pub fn with_methods(obj: &'a PyObject, methods: &'static PySequenceMethods) -> Self {
        Self { obj, methods }
    }

    pub fn try_protocol(obj: &'a PyObject, vm: &VirtualMachine) -> PyResult<Self> {
        let seq = obj.to_sequence(vm);
        if seq.check() {
            Ok(seq)
        } else {
            Err(vm.new_type_error(format!("'{}' is not a sequence", obj.class())))
        }
    }
}

impl PySequence<'_> {
    pub fn check(&self) -> bool {
        self.methods.item.load().is_some()
    }

    pub fn find_methods(
        obj: &PyObject,
        vm: &VirtualMachine,
    ) -> Option<PointerSlot<PySequenceMethods>> {
        let cls = obj.class();
        // if cls.fast_issubclass(vm.ctx.types.dict_type) {
        if cls.is(vm.ctx.types.dict_type) {
            return None;
        }
        cls.mro_find_map(|x| x.slots.as_sequence.load())
    }

    pub fn length_opt(self, vm: &VirtualMachine) -> Option<PyResult<usize>> {
        self.methods.length.load().map(|f| f(self, vm))
    }

    pub fn length(self, vm: &VirtualMachine) -> PyResult<usize> {
        self.length_opt(vm).ok_or_else(|| {
            vm.new_type_error(format!(
                "'{}' is not a sequence or has no len()",
                self.obj.class()
            ))
        })?
    }

    pub fn concat(self, other: &PyObject, vm: &VirtualMachine) -> PyResult {
        if let Some(f) = self.methods.concat.load() {
            return f(self, other, vm);
        }

        // if both arguments apear to be sequences, try fallback to __add__
        if self.check() && other.to_sequence(vm).check() {
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

    pub fn repeat(self, n: usize, vm: &VirtualMachine) -> PyResult {
        if let Some(f) = self.methods.repeat.load() {
            return f(self, n, vm);
        }

        // fallback to __mul__
        if self.check() {
            let ret = vm._mul(self.obj, &n.to_pyobject(vm))?;
            if let PyArithmeticValue::Implemented(ret) = PyArithmeticValue::from_object(vm, ret) {
                return Ok(ret);
            }
        }

        Err(vm.new_type_error(format!("'{}' object can't be repeated", self.obj.class())))
    }

    pub fn inplace_concat(self, other: &PyObject, vm: &VirtualMachine) -> PyResult {
        if let Some(f) = self.methods.inplace_concat.load() {
            return f(self, other, vm);
        }
        if let Some(f) = self.methods.concat.load() {
            return f(self, other, vm);
        }

        // if both arguments apear to be sequences, try fallback to __iadd__
        if self.check() && other.to_sequence(vm).check() {
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

    pub fn inplace_repeat(self, n: usize, vm: &VirtualMachine) -> PyResult {
        if let Some(f) = self.methods.inplace_repeat.load() {
            return f(self, n, vm);
        }
        if let Some(f) = self.methods.repeat.load() {
            return f(self, n, vm);
        }

        if self.check() {
            let ret = vm._imul(self.obj, &n.to_pyobject(vm))?;
            if let PyArithmeticValue::Implemented(ret) = PyArithmeticValue::from_object(vm, ret) {
                return Ok(ret);
            }
        }

        Err(vm.new_type_error(format!("'{}' object can't be repeated", self.obj.class())))
    }

    pub fn get_item(self, i: isize, vm: &VirtualMachine) -> PyResult {
        if let Some(f) = self.methods.item.load() {
            return f(self, i, vm);
        }
        Err(vm.new_type_error(format!(
            "'{}' is not a sequence or does not support indexing",
            self.obj.class()
        )))
    }

    fn _ass_item(self, i: isize, value: Option<PyObjectRef>, vm: &VirtualMachine) -> PyResult<()> {
        if let Some(f) = self.methods.ass_item.load() {
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

    pub fn set_item(self, i: isize, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self._ass_item(i, Some(value), vm)
    }

    pub fn del_item(self, i: isize, vm: &VirtualMachine) -> PyResult<()> {
        self._ass_item(i, None, vm)
    }

    pub fn get_slice(&self, start: isize, stop: isize, vm: &VirtualMachine) -> PyResult {
        if let Ok(mapping) = PyMapping::try_protocol(self.obj, vm) {
            let slice = PySlice {
                start: Some(start.to_pyobject(vm)),
                stop: stop.to_pyobject(vm),
                step: None,
            };
            mapping.subscript(&slice.into_pyobject(vm), vm)
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
        let mapping = self.obj.to_mapping();
        if let Some(f) = mapping.methods.ass_subscript.load() {
            let slice = PySlice {
                start: Some(start.to_pyobject(vm)),
                stop: stop.to_pyobject(vm),
                step: None,
            };
            f(&mapping, &slice.into_pyobject(vm), value, vm)
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
            tuple.iter().map(|x| f(x.as_ref())).collect()
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

    pub fn contains(self, target: &PyObject, vm: &VirtualMachine) -> PyResult<bool> {
        if let Some(f) = self.methods.contains.load() {
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
}

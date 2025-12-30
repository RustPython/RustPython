use crate::{
    PyObject, PyObjectRef, PyPayload, PyResult, VirtualMachine,
    builtins::{PyList, PyListRef, PySlice, PyTuple, PyTupleRef},
    convert::ToPyObject,
    function::PyArithmeticValue,
    object::{Traverse, TraverseFn},
    protocol::PyNumberBinaryOp,
};
use crossbeam_utils::atomic::AtomicCell;
use itertools::Itertools;

// Sequence Protocol
// https://docs.python.org/3/c-api/sequence.html

#[allow(clippy::type_complexity)]
#[derive(Default)]
pub struct PySequenceSlots {
    pub length: AtomicCell<Option<fn(PySequence<'_>, &VirtualMachine) -> PyResult<usize>>>,
    pub concat: AtomicCell<Option<fn(PySequence<'_>, &PyObject, &VirtualMachine) -> PyResult>>,
    pub repeat: AtomicCell<Option<fn(PySequence<'_>, isize, &VirtualMachine) -> PyResult>>,
    pub item: AtomicCell<Option<fn(PySequence<'_>, isize, &VirtualMachine) -> PyResult>>,
    pub ass_item: AtomicCell<
        Option<fn(PySequence<'_>, isize, Option<PyObjectRef>, &VirtualMachine) -> PyResult<()>>,
    >,
    pub contains:
        AtomicCell<Option<fn(PySequence<'_>, &PyObject, &VirtualMachine) -> PyResult<bool>>>,
    pub inplace_concat:
        AtomicCell<Option<fn(PySequence<'_>, &PyObject, &VirtualMachine) -> PyResult>>,
    pub inplace_repeat: AtomicCell<Option<fn(PySequence<'_>, isize, &VirtualMachine) -> PyResult>>,
}

impl core::fmt::Debug for PySequenceSlots {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("PySequenceSlots")
    }
}

impl PySequenceSlots {
    pub fn has_item(&self) -> bool {
        self.item.load().is_some()
    }

    /// Copy from static PySequenceMethods
    pub fn copy_from(&self, methods: &PySequenceMethods) {
        if let Some(f) = methods.length {
            self.length.store(Some(f));
        }
        if let Some(f) = methods.concat {
            self.concat.store(Some(f));
        }
        if let Some(f) = methods.repeat {
            self.repeat.store(Some(f));
        }
        if let Some(f) = methods.item {
            self.item.store(Some(f));
        }
        if let Some(f) = methods.ass_item {
            self.ass_item.store(Some(f));
        }
        if let Some(f) = methods.contains {
            self.contains.store(Some(f));
        }
        if let Some(f) = methods.inplace_concat {
            self.inplace_concat.store(Some(f));
        }
        if let Some(f) = methods.inplace_repeat {
            self.inplace_repeat.store(Some(f));
        }
    }
}

#[allow(clippy::type_complexity)]
#[derive(Default)]
pub struct PySequenceMethods {
    pub length: Option<fn(PySequence<'_>, &VirtualMachine) -> PyResult<usize>>,
    pub concat: Option<fn(PySequence<'_>, &PyObject, &VirtualMachine) -> PyResult>,
    pub repeat: Option<fn(PySequence<'_>, isize, &VirtualMachine) -> PyResult>,
    pub item: Option<fn(PySequence<'_>, isize, &VirtualMachine) -> PyResult>,
    pub ass_item:
        Option<fn(PySequence<'_>, isize, Option<PyObjectRef>, &VirtualMachine) -> PyResult<()>>,
    pub contains: Option<fn(PySequence<'_>, &PyObject, &VirtualMachine) -> PyResult<bool>>,
    pub inplace_concat: Option<fn(PySequence<'_>, &PyObject, &VirtualMachine) -> PyResult>,
    pub inplace_repeat: Option<fn(PySequence<'_>, isize, &VirtualMachine) -> PyResult>,
}

impl core::fmt::Debug for PySequenceMethods {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("PySequenceMethods")
    }
}

impl PySequenceMethods {
    pub const NOT_IMPLEMENTED: Self = Self {
        length: None,
        concat: None,
        repeat: None,
        item: None,
        ass_item: None,
        contains: None,
        inplace_concat: None,
        inplace_repeat: None,
    };
}

impl PyObject {
    #[inline]
    pub fn sequence_unchecked(&self) -> PySequence<'_> {
        PySequence { obj: self }
    }

    pub fn try_sequence(&self, vm: &VirtualMachine) -> PyResult<PySequence<'_>> {
        let seq = self.sequence_unchecked();
        if seq.check() {
            Ok(seq)
        } else {
            Err(vm.new_type_error(format!("'{}' is not a sequence", self.class())))
        }
    }
}

#[derive(Copy, Clone)]
pub struct PySequence<'a> {
    pub obj: &'a PyObject,
}

unsafe impl Traverse for PySequence<'_> {
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        self.obj.traverse(tracer_fn)
    }
}

impl PySequence<'_> {
    #[inline]
    pub fn slots(&self) -> &PySequenceSlots {
        &self.obj.class().slots.as_sequence
    }

    pub fn check(&self) -> bool {
        self.slots().has_item()
    }

    pub fn length_opt(self, vm: &VirtualMachine) -> Option<PyResult<usize>> {
        self.slots().length.load().map(|f| f(self, vm))
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
        if let Some(f) = self.slots().concat.load() {
            return f(self, other, vm);
        }

        // if both arguments appear to be sequences, try fallback to __add__
        if self.check() && other.sequence_unchecked().check() {
            let ret = vm.binary_op1(self.obj, other, PyNumberBinaryOp::Add)?;
            if let PyArithmeticValue::Implemented(ret) = PyArithmeticValue::from_object(vm, ret) {
                return Ok(ret);
            }
        }

        Err(vm.new_type_error(format!(
            "'{}' object can't be concatenated",
            self.obj.class()
        )))
    }

    pub fn repeat(self, n: isize, vm: &VirtualMachine) -> PyResult {
        if let Some(f) = self.slots().repeat.load() {
            return f(self, n, vm);
        }

        // fallback to __mul__
        if self.check() {
            let ret = vm.binary_op1(self.obj, &n.to_pyobject(vm), PyNumberBinaryOp::Multiply)?;
            if let PyArithmeticValue::Implemented(ret) = PyArithmeticValue::from_object(vm, ret) {
                return Ok(ret);
            }
        }

        Err(vm.new_type_error(format!("'{}' object can't be repeated", self.obj.class())))
    }

    pub fn inplace_concat(self, other: &PyObject, vm: &VirtualMachine) -> PyResult {
        if let Some(f) = self.slots().inplace_concat.load() {
            return f(self, other, vm);
        }
        if let Some(f) = self.slots().concat.load() {
            return f(self, other, vm);
        }

        // if both arguments appear to be sequences, try fallback to __iadd__
        if self.check() && other.sequence_unchecked().check() {
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

    pub fn inplace_repeat(self, n: isize, vm: &VirtualMachine) -> PyResult {
        if let Some(f) = self.slots().inplace_repeat.load() {
            return f(self, n, vm);
        }
        if let Some(f) = self.slots().repeat.load() {
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
        if let Some(f) = self.slots().item.load() {
            return f(self, i, vm);
        }
        Err(vm.new_type_error(format!(
            "'{}' is not a sequence or does not support indexing",
            self.obj.class()
        )))
    }

    fn _ass_item(self, i: isize, value: Option<PyObjectRef>, vm: &VirtualMachine) -> PyResult<()> {
        if let Some(f) = self.slots().ass_item.load() {
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
        if let Ok(mapping) = self.obj.try_mapping(vm) {
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
        let mapping = self.obj.mapping_unchecked();
        if let Some(f) = mapping.slots().ass_subscript.load() {
            let slice = PySlice {
                start: Some(start.to_pyobject(vm)),
                stop: stop.to_pyobject(vm),
                step: None,
            };
            f(mapping, &slice.into_pyobject(vm), value, vm)
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
                    return Err(vm.new_overflow_error("index exceeds C integer size"));
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
                return Err(vm.new_overflow_error("index exceeds C integer size"));
            }
            index += 1;

            let elem = elem?;
            if vm.bool_eq(&elem, target)? {
                return Ok(index as usize);
            }
        }

        Err(vm.new_value_error("sequence.index(x): x not in sequence"))
    }

    pub fn extract<F, R>(&self, mut f: F, vm: &VirtualMachine) -> PyResult<Vec<R>>
    where
        F: FnMut(&PyObject) -> PyResult<R>,
    {
        if let Some(tuple) = self.obj.downcast_ref_if_exact::<PyTuple>(vm) {
            tuple.iter().map(|x| f(x.as_ref())).collect()
        } else if let Some(list) = self.obj.downcast_ref_if_exact::<PyList>(vm) {
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
        if let Some(f) = self.slots().contains.load() {
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

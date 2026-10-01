//! [Sequence Protocol](https://docs.python.org/3/c-api/sequence.html)

use crossbeam_utils::atomic::AtomicCell;
use itertools::Itertools;

use crate::{
    AsObject, PyObject, PyObjectRef, PyPayload, PyResult, VirtualMachine,
    builtins::{PyList, PyListRef, PySlice, PyTuple, PyTupleRef},
    convert::ToPyObject,
    function::PyArithmeticValue,
    object::{Traverse, TraverseFn},
    protocol::PyNumberBinaryOp,
};

#[expect(clippy::type_complexity)]
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

    /// Whether any slot is filled, which is what a non-null `tp_as_sequence`
    /// amounts to for a statically declared type.
    pub fn has_any(&self) -> bool {
        self.length.load().is_some()
            || self.concat.load().is_some()
            || self.repeat.load().is_some()
            || self.item.load().is_some()
            || self.ass_item.load().is_some()
            || self.contains.load().is_some()
            || self.inplace_concat.load().is_some()
            || self.inplace_repeat.load().is_some()
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

#[expect(clippy::type_complexity)]
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
    pub const fn sequence_unchecked(&self) -> PySequence<'_> {
        PySequence { obj: self }
    }

    pub fn try_sequence(&self, vm: &VirtualMachine) -> PyResult<PySequence<'_>> {
        let seq = self.sequence_unchecked();
        if seq.check() {
            Ok(seq)
        } else {
            Err(vm.new_type_error(format!("{} is not a sequence", self.class().slot_name())))
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
    #[must_use]
    pub fn slots(&self) -> &PySequenceSlots {
        &self.obj.class().slots().as_sequence
    }

    #[must_use]
    pub fn check(&self) -> bool {
        self.slots().has_item()
    }

    pub fn length_opt(self, vm: &VirtualMachine) -> Option<PyResult<usize>> {
        self.slots().length.load().map(|f| f(self, vm))
    }

    // Py_ssize_t PySequence_Size(PyObject *s)
    pub fn length(self, vm: &VirtualMachine) -> PyResult<usize> {
        self.length_opt(vm).ok_or_else(|| {
            let name = self.obj.class().slot_name();
            // Something that measures itself as a mapping is no sequence at all.
            let msg = if self.obj.mapping_unchecked().slots().length.load().is_some() {
                format!("{name} is not a sequence")
            } else {
                format!("object of type '{name}' has no len()")
            };
            vm.new_type_error(msg)
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
            self.obj.class().slot_name()
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

        Err(vm.new_type_error(format!(
            "'{}' object can't be repeated",
            self.obj.class().slot_name()
        )))
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
            let ret = vm.binary_iop1(
                self.obj,
                other,
                PyNumberBinaryOp::InplaceAdd,
                PyNumberBinaryOp::Add,
            )?;
            if let PyArithmeticValue::Implemented(ret) = PyArithmeticValue::from_object(vm, ret) {
                return Ok(ret);
            }
        }

        Err(vm.new_type_error(format!(
            "'{}' object can't be concatenated",
            self.obj.class().slot_name()
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
            let ret = vm.binary_iop1(
                self.obj,
                &n.to_pyobject(vm),
                PyNumberBinaryOp::InplaceMultiply,
                PyNumberBinaryOp::Multiply,
            )?;
            if let PyArithmeticValue::Implemented(ret) = PyArithmeticValue::from_object(vm, ret) {
                return Ok(ret);
            }
        }

        Err(vm.new_type_error(format!(
            "'{}' object can't be repeated",
            self.obj.class().slot_name()
        )))
    }

    pub fn get_item(self, i: isize, vm: &VirtualMachine) -> PyResult {
        if let Some(f) = self.slots().item.load() {
            return f(self, i, vm);
        }

        let name = self.obj.class().slot_name();
        // Something that subscripts itself as a mapping is no sequence at all.
        let msg = if self
            .obj
            .mapping_unchecked()
            .slots()
            .subscript
            .load()
            .is_some()
        {
            format!("{name} is not a sequence")
        } else {
            format!("'{name}' object does not support indexing")
        };
        Err(vm.new_type_error(msg))
    }

    fn _ass_item(self, i: isize, value: Option<PyObjectRef>, vm: &VirtualMachine) -> PyResult<()> {
        if let Some(f) = self.slots().ass_item.load() {
            return f(self, i, value, vm);
        }

        let name = self.obj.class().slot_name();
        let msg = if self
            .obj
            .mapping_unchecked()
            .slots()
            .ass_subscript
            .load()
            .is_some()
        {
            format!("{name} is not a sequence")
        } else if value.is_some() {
            format!("'{name}' object does not support item assignment")
        } else {
            format!("'{name}' object doesn't support item deletion")
        };
        Err(vm.new_type_error(msg))
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
            Err(vm.new_type_error(format!(
                "'{}' object is unsliceable",
                self.obj.class().slot_name()
            )))
        }
    }

    fn _ass_slice(
        self,
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
                self.obj.class().slot_name(),
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
        Ok(vm.ctx.new_list(self.obj.try_to_value(vm)?))
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
        let iter = self.obj.to_owned().get_iter(vm)?;
        let iter = iter.iter::<PyObjectRef>(vm)?;

        for (index, elem) in iter.enumerate() {
            if isize::try_from(index).is_err() {
                return Err(vm.new_overflow_error("index exceeds C integer size"));
            }

            let elem = elem?;
            if vm.bool_eq(&elem, target)? {
                return Ok(index);
            }
        }

        Err(vm.new_value_error("sequence.index(x): x not in sequence"))
    }

    pub fn extract<F, R>(&self, mut f: F, vm: &VirtualMachine) -> PyResult<Vec<R>>
    where
        F: FnMut(&PyObject) -> PyResult<R>,
    {
        let mut v = Vec::new();
        if let Some(tuple) = self.obj.downcast_ref_if_exact::<PyTuple>(vm) {
            v.try_reserve_exact(tuple.len())
                .map_err(|_| vm.no_memory_error())?;
            for x in tuple.as_slice() {
                v.push(f(x.as_ref())?);
            }
        } else if let Some(list) = self.obj.downcast_ref_if_exact::<PyList>(vm) {
            let elements = list.borrow_vec();
            v.try_reserve_exact(elements.len())
                .map_err(|_| vm.no_memory_error())?;
            for x in elements.iter() {
                v.push(f(x.as_ref())?);
            }
        } else {
            let iter = self.obj.to_owned().get_iter(vm)?;
            let iter = iter.iter::<PyObjectRef>(vm)?;
            let len = self.length(vm).unwrap_or(0);
            v.try_reserve_exact(len).map_err(|_| vm.no_memory_error())?;
            for x in iter {
                let item = f(x?.as_ref())?;
                if v.len() == v.capacity() {
                    v.try_reserve(1).map_err(|_| vm.no_memory_error())?;
                }
                v.push(item);
            }
        }
        Ok(v)
    }

    pub fn contains(self, target: &PyObject, vm: &VirtualMachine) -> PyResult<bool> {
        if let Some(f) = self.slots().contains.load() {
            return f(self, target, vm);
        }

        // CPython parity: when neither __contains__ nor __iter__ is available,
        // `PySequence_Contains` rewords the get_iter TypeError into the
        // membership-test wording. Other exception types propagate unchanged.
        let iter = self.obj.to_owned().get_iter(vm).map_err(|e| {
            if e.fast_isinstance(vm.ctx.exceptions.type_error) {
                vm.new_type_error(format!(
                    "argument of type '{}' is not a container or iterable",
                    self.obj.class().slot_name()
                ))
            } else {
                e
            }
        })?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Interpreter, builtins::PyRange};
    use core::cell::Cell;

    #[derive(Debug)]
    struct Converted<'a>(&'a Cell<usize>);

    impl Drop for Converted<'_> {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }

    #[test]
    fn unsupported_inplace_operations_keep_sequence_errors() {
        Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let range = PyRange {
                start: vm.ctx.new_int(0),
                stop: vm.ctx.new_int(3),
                step: vm.ctx.new_int(1),
            }
            .into_ref(&vm.ctx);
            let sequence = range.as_object().sequence_unchecked();
            for (result, message) in [
                (
                    sequence.inplace_repeat(2, vm),
                    "'range' object can't be repeated",
                ),
                (
                    sequence.inplace_concat(range.as_object(), vm),
                    "'range' object can't be concatenated",
                ),
            ] {
                let error = result.unwrap_err();
                assert!(error.fast_isinstance(vm.ctx.exceptions.type_error));
                let actual: String = error.args().as_slice()[0].try_to_value(vm).unwrap();
                assert_eq!(actual, message);
            }
        });
    }

    #[test]
    fn conversion_and_partial_result_cleanup() {
        Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let elements: Vec<PyObjectRef> =
                (0..20).map(|value| vm.ctx.new_int(value).into()).collect();
            let list = vm.ctx.new_list(elements.clone());
            let iterator = list.as_object().get_iter(vm).unwrap();
            assert!(iterator.sequence_unchecked().length_opt(vm).is_none());
            let sequences: [PyObjectRef; 3] = [
                vm.ctx.new_tuple(elements).into(),
                list.clone().into(),
                iterator.into(),
            ];
            for sequence in sequences {
                let values: Vec<i32> = sequence
                    .sequence_unchecked()
                    .extract(|item| item.try_to_value(vm), vm)
                    .unwrap();
                assert_eq!(values, (0..20).collect::<Vec<_>>());
            }

            let iterator = list.as_object().get_iter(vm).unwrap();
            let error = vm.new_value_error("conversion failed");
            let dropped = Cell::new(0);
            let mut calls = 0;
            let raised = iterator
                .sequence_unchecked()
                .extract(
                    |_| {
                        calls += 1;
                        if calls == 3 {
                            Err(error.clone())
                        } else {
                            Ok(Converted(&dropped))
                        }
                    },
                    vm,
                )
                .unwrap_err();
            assert!(raised.is(&error));
            assert_eq!(calls, 3);
            assert_eq!(dropped.get(), 2);
        });
    }

    #[test]
    fn capacity_overflow_precedes_conversion() {
        Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let range = PyRange {
                start: vm.ctx.new_int(0),
                stop: vm.ctx.new_int(isize::MAX),
                step: vm.ctx.new_int(1),
            }
            .into_ref(&vm.ctx);
            let called = Cell::new(false);
            // The byte capacity exceeds isize::MAX; no large allocation is attempted.
            let error = range
                .as_object()
                .sequence_unchecked()
                .extract(
                    |_| {
                        called.set(true);
                        Ok(0u64)
                    },
                    vm,
                )
                .unwrap_err();
            assert!(error.fast_isinstance(vm.ctx.exceptions.memory_error));
            assert!(!called.get());
        });
    }
}

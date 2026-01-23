use crate::{
    AsObject, PyObject, PyObjectRef, PyResult, VirtualMachine,
    builtins::{
        PyDict, PyStrInterned,
        dict::{PyDictItems, PyDictKeys, PyDictValues},
    },
    convert::ToPyResult,
    object::{Traverse, TraverseFn},
};
use crossbeam_utils::atomic::AtomicCell;

// Mapping protocol
// https://docs.python.org/3/c-api/mapping.html

#[allow(clippy::type_complexity)]
#[derive(Default)]
pub struct PyMappingSlots {
    pub length: AtomicCell<Option<fn(PyMapping<'_>, &VirtualMachine) -> PyResult<usize>>>,
    pub subscript: AtomicCell<Option<fn(PyMapping<'_>, &PyObject, &VirtualMachine) -> PyResult>>,
    pub ass_subscript: AtomicCell<
        Option<fn(PyMapping<'_>, &PyObject, Option<PyObjectRef>, &VirtualMachine) -> PyResult<()>>,
    >,
}

impl core::fmt::Debug for PyMappingSlots {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("PyMappingSlots")
    }
}

impl PyMappingSlots {
    pub fn has_subscript(&self) -> bool {
        self.subscript.load().is_some()
    }

    /// Copy from static PyMappingMethods
    pub fn copy_from(&self, methods: &PyMappingMethods) {
        if let Some(f) = methods.length {
            self.length.store(Some(f));
        }
        if let Some(f) = methods.subscript {
            self.subscript.store(Some(f));
        }
        if let Some(f) = methods.ass_subscript {
            self.ass_subscript.store(Some(f));
        }
    }
}

#[allow(clippy::type_complexity)]
#[derive(Clone, Copy, Default)]
pub struct PyMappingMethods {
    pub length: Option<fn(PyMapping<'_>, &VirtualMachine) -> PyResult<usize>>,
    pub subscript: Option<fn(PyMapping<'_>, &PyObject, &VirtualMachine) -> PyResult>,
    pub ass_subscript:
        Option<fn(PyMapping<'_>, &PyObject, Option<PyObjectRef>, &VirtualMachine) -> PyResult<()>>,
}

impl core::fmt::Debug for PyMappingMethods {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("PyMappingMethods")
    }
}

impl PyMappingMethods {
    pub const NOT_IMPLEMENTED: Self = Self {
        length: None,
        subscript: None,
        ass_subscript: None,
    };
}

impl PyObject {
    pub fn mapping_unchecked(&self) -> PyMapping<'_> {
        PyMapping { obj: self }
    }

    pub fn try_mapping(&self, vm: &VirtualMachine) -> PyResult<PyMapping<'_>> {
        let mapping = self.mapping_unchecked();
        if mapping.check() {
            Ok(mapping)
        } else {
            Err(vm.new_type_error(format!("{} is not a mapping object", self.class())))
        }
    }
}

#[derive(Copy, Clone)]
pub struct PyMapping<'a> {
    pub obj: &'a PyObject,
}

unsafe impl Traverse for PyMapping<'_> {
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        self.obj.traverse(tracer_fn)
    }
}

impl AsRef<PyObject> for PyMapping<'_> {
    #[inline(always)]
    fn as_ref(&self) -> &PyObject {
        self.obj
    }
}

impl PyMapping<'_> {
    #[inline]
    pub fn slots(&self) -> &PyMappingSlots {
        &self.obj.class().slots.as_mapping
    }

    #[inline]
    pub fn check(&self) -> bool {
        self.slots().has_subscript()
    }

    pub fn length_opt(self, vm: &VirtualMachine) -> Option<PyResult<usize>> {
        self.slots().length.load().map(|f| f(self, vm))
    }

    pub fn length(self, vm: &VirtualMachine) -> PyResult<usize> {
        self.length_opt(vm).ok_or_else(|| {
            vm.new_type_error(format!(
                "object of type '{}' has no len() or not a mapping",
                self.obj.class()
            ))
        })?
    }

    pub fn subscript(self, needle: &impl AsObject, vm: &VirtualMachine) -> PyResult {
        self._subscript(needle.as_object(), vm)
    }

    pub fn ass_subscript(
        self,
        needle: &impl AsObject,
        value: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        self._ass_subscript(needle.as_object(), value, vm)
    }

    fn _subscript(self, needle: &PyObject, vm: &VirtualMachine) -> PyResult {
        let f =
            self.slots().subscript.load().ok_or_else(|| {
                vm.new_type_error(format!("{} is not a mapping", self.obj.class()))
            })?;
        f(self, needle, vm)
    }

    fn _ass_subscript(
        self,
        needle: &PyObject,
        value: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let f = self.slots().ass_subscript.load().ok_or_else(|| {
            vm.new_type_error(format!(
                "'{}' object does not support item assignment",
                self.obj.class()
            ))
        })?;
        f(self, needle, value, vm)
    }

    pub fn keys(self, vm: &VirtualMachine) -> PyResult {
        if let Some(dict) = self.obj.downcast_ref_if_exact::<PyDict>(vm) {
            PyDictKeys::new(dict.to_owned()).to_pyresult(vm)
        } else {
            self.method_output_as_list(identifier!(vm, keys), vm)
        }
    }

    pub fn values(self, vm: &VirtualMachine) -> PyResult {
        if let Some(dict) = self.obj.downcast_ref_if_exact::<PyDict>(vm) {
            PyDictValues::new(dict.to_owned()).to_pyresult(vm)
        } else {
            self.method_output_as_list(identifier!(vm, values), vm)
        }
    }

    pub fn items(self, vm: &VirtualMachine) -> PyResult {
        if let Some(dict) = self.obj.downcast_ref_if_exact::<PyDict>(vm) {
            PyDictItems::new(dict.to_owned()).to_pyresult(vm)
        } else {
            self.method_output_as_list(identifier!(vm, items), vm)
        }
    }

    fn method_output_as_list(
        self,
        method_name: &'static PyStrInterned,
        vm: &VirtualMachine,
    ) -> PyResult {
        let meth_output = vm.call_method(self.obj, method_name.as_str(), ())?;
        if meth_output.is(vm.ctx.types.list_type) {
            return Ok(meth_output);
        }

        let iter = meth_output.get_iter(vm).map_err(|_| {
            vm.new_type_error(format!(
                "{}.{}() returned a non-iterable (type {})",
                self.obj.class(),
                method_name.as_str(),
                meth_output.class()
            ))
        })?;

        // TODO
        // PySequence::from(&iter).list(vm).map(|x| x.into())
        vm.ctx.new_list(iter.try_to_value(vm)?).to_pyresult(vm)
    }
}

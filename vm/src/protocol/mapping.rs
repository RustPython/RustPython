use std::marker::PhantomData;

use crate::{
    builtins::{
        dict::{PyDictItems, PyDictKeys, PyDictValues},
        type_::PointerSlot,
        PyDict, PyStrInterned,
    },
    convert::ToPyResult,
    AsObject, PyObject, PyObjectRef, PyResult, VirtualMachine,
};
use rustpython_common::atomic::{Ordering, PyAtomicFn};

// Mapping protocol
// https://docs.python.org/3/c-api/mapping.html

impl PyObject {
    pub fn to_mapping(&self) -> PyMapping<'_> {
        PyMapping::from(self)
    }
}

pub type MappingLengthFn = PyAtomicFn<Option<fn(PyMapping<'a>, &'vm VirtualMachine) -> PyResult<usize>>>;
pub type MappingSubscriptFn =
    PyAtomicFn<Option<fn(PyMapping, &PyObject, &VirtualMachine) -> PyResult>>;
pub type MappingAssSubscriptFn = PyAtomicFn<
    Option<fn(PyMapping, &PyObject, Option<PyObjectRef>, &VirtualMachine) -> PyResult<()>>,
>;

#[allow(clippy::type_complexity)]
#[derive(Default)]
pub struct PyMappingMethods {
    pub length: MappingLengthFn,
    pub subscript: MappingSubscriptFn,
    pub ass_subscript: MappingAssSubscriptFn,
}

impl std::fmt::Debug for PyMappingMethods {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "mapping methods")
    }
}

impl PyMappingMethods {
    fn check(&self) -> bool {
        self.subscript.load(Ordering::Relaxed).is_some()
    }

    #[allow(clippy::declare_interior_mutable_const)]
    pub const NOT_IMPLEMENTED: PyMappingMethods = PyMappingMethods::default();
}

impl<'a> From<&'a PyObject> for PyMapping<'a> {
    fn from(obj: &'a PyObject) -> Self {
        static GLOBAL_NOT_IMPLEMENTED: PyMappingMethods = PyMappingMethods::NOT_IMPLEMENTED;
        let methods = Self::find_methods(obj)
            .map_or(&GLOBAL_NOT_IMPLEMENTED, |x| unsafe { x.borrow_static() });
        Self { obj, methods }
    }
}

#[derive(Copy, Clone)]
pub struct PyMapping<'a> {
    pub obj: &'a PyObject,
    pub methods: &'static PyMappingMethods,
}

impl AsRef<PyObject> for PyMapping<'_> {
    #[inline(always)]
    fn as_ref(&self) -> &PyObject {
        self.obj
    }
}

impl<'a> PyMapping<'a> {
    pub fn try_protocol(obj: &'a PyObject, vm: &VirtualMachine) -> PyResult<Self> {
        if let Some(methods) = Self::find_methods(obj) {
            if methods.as_ref().check() {
                return Ok(Self {
                    obj,
                    methods: unsafe { methods.borrow_static() },
                });
            }
        }

        Err(vm.new_type_error(format!("{} is not a mapping object", obj.class())))
    }
}

impl PyMapping<'_> {
    // PyMapping::Check
    #[inline]
    pub fn check(obj: &PyObject) -> bool {
        Self::find_methods(obj).map_or(false, |x| x.as_ref().check())
    }

    pub fn find_methods(obj: &PyObject) -> Option<PointerSlot<PyMappingMethods>> {
        obj.class().mro_find_map(|cls| cls.slots.as_mapping.load())
    }

    pub fn length_opt(self, vm: &VirtualMachine) -> Option<PyResult<usize>> {
        self.methods
            .length
            .load(Ordering::Relaxed)
            .map(|f| f(self, vm))
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
        let f = self
            .methods
            .subscript
            .load(Ordering::Relaxed)
            .ok_or_else(|| vm.new_type_error(format!("{} is not a mapping", self.obj.class())))?;
        f(self, needle, vm)
    }

    fn _ass_subscript(
        self,
        needle: &PyObject,
        value: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let f = self
            .methods
            .ass_subscript
            .load(Ordering::Relaxed)
            .ok_or_else(|| {
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

        let iter = meth_output.clone().get_iter(vm).map_err(|_| {
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

use crate::{
    builtins::{
        dict::{PyDictItems, PyDictKeys, PyDictValues},
        PyDict, PyStrInterned,
    },
    convert::ToPyResult,
    AsObject, PyObject, PyObjectRef, PyResult, VirtualMachine,
};

// Mapping protocol
// https://docs.python.org/3/c-api/mapping.html
#[allow(clippy::type_complexity)]
pub struct PyMappingMethods {
    pub length: Option<fn(&PyMapping, &VirtualMachine) -> PyResult<usize>>,
    pub subscript: Option<fn(&PyMapping, &PyObject, &VirtualMachine) -> PyResult>,
    pub ass_subscript:
        Option<fn(&PyMapping, &PyObject, Option<PyObjectRef>, &VirtualMachine) -> PyResult<()>>,
}

impl std::fmt::Debug for PyMappingMethods {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "mapping methods")
    }
}

impl PyMappingMethods {
    fn check(&self) -> bool {
        self.subscript.is_some()
    }

    pub(crate) fn generic(
        has_length: bool,
        has_subscript: bool,
        has_ass_subscript: bool,
    ) -> &'static Self {
        static METHODS: &[PyMappingMethods] = &[
            new_generic(false, false, false),
            new_generic(true, false, false),
            new_generic(false, true, false),
            new_generic(true, true, false),
            new_generic(false, false, true),
            new_generic(true, false, true),
            new_generic(false, true, true),
            new_generic(true, true, true),
        ];

        fn length(mapping: &PyMapping, vm: &VirtualMachine) -> PyResult<usize> {
            crate::types::slot_length(mapping.obj, vm)
        }
        fn subscript(mapping: &PyMapping, needle: &PyObject, vm: &VirtualMachine) -> PyResult {
            vm.call_special_method(
                mapping.obj.to_owned(),
                identifier!(vm, __getitem__),
                (needle.to_owned(),),
            )
        }
        fn ass_subscript(
            mapping: &PyMapping,
            needle: &PyObject,
            value: Option<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            match value {
                Some(value) => vm
                    .call_special_method(
                        mapping.obj.to_owned(),
                        identifier!(vm, __setitem__),
                        (needle.to_owned(), value),
                    )
                    .map(|_| Ok(()))?,
                None => vm
                    .call_special_method(
                        mapping.obj.to_owned(),
                        identifier!(vm, __delitem__),
                        (needle.to_owned(),),
                    )
                    .map(|_| Ok(()))?,
            }
        }

        const fn new_generic(
            has_length: bool,
            has_subscript: bool,
            has_ass_subscript: bool,
        ) -> PyMappingMethods {
            PyMappingMethods {
                length: if has_length { Some(length) } else { None },
                subscript: if has_subscript { Some(subscript) } else { None },
                ass_subscript: if has_ass_subscript {
                    Some(ass_subscript)
                } else {
                    None
                },
            }
        }

        let key = (has_length as usize)
            | ((has_subscript as usize) << 1)
            | ((has_ass_subscript as usize) << 2);

        &METHODS[key]
    }
}

#[derive(Clone)]
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
    #[inline]
    pub fn new(obj: &'a PyObject, vm: &VirtualMachine) -> Option<Self> {
        let methods = Self::find_methods(obj, vm)?;
        Some(Self { obj, methods })
    }

    #[inline(always)]
    pub fn with_methods(obj: &'a PyObject, methods: &'static PyMappingMethods) -> Self {
        Self { obj, methods }
    }

    pub fn try_protocol(obj: &'a PyObject, vm: &VirtualMachine) -> PyResult<Self> {
        if let Some(methods) = Self::find_methods(obj, vm) {
            if methods.check() {
                return Ok(Self::with_methods(obj, methods));
            }
        }

        Err(vm.new_type_error(format!("{} is not a mapping object", obj.class())))
    }
}

impl PyMapping<'_> {
    // PyMapping::Check
    #[inline]
    pub fn check(obj: &PyObject, vm: &VirtualMachine) -> bool {
        Self::find_methods(obj, vm).map_or(false, PyMappingMethods::check)
    }

    pub fn find_methods(obj: &PyObject, vm: &VirtualMachine) -> Option<&'static PyMappingMethods> {
        let as_mapping = obj.class().mro_find_map(|cls| cls.slots.as_mapping.load());
        as_mapping.map(|f| f(obj, vm))
    }

    pub fn length_opt(&self, vm: &VirtualMachine) -> Option<PyResult<usize>> {
        self.methods.length.map(|f| f(self, vm))
    }

    pub fn length(&self, vm: &VirtualMachine) -> PyResult<usize> {
        self.length_opt(vm).ok_or_else(|| {
            vm.new_type_error(format!(
                "object of type '{}' has no len() or not a mapping",
                self.obj.class()
            ))
        })?
    }

    pub fn subscript(&self, needle: &impl AsObject, vm: &VirtualMachine) -> PyResult {
        self._subscript(needle.as_object(), vm)
    }

    pub fn ass_subscript(
        &self,
        needle: &impl AsObject,
        value: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        self._ass_subscript(needle.as_object(), value, vm)
    }

    fn _subscript(&self, needle: &PyObject, vm: &VirtualMachine) -> PyResult {
        let f = self
            .methods
            .subscript
            .ok_or_else(|| vm.new_type_error(format!("{} is not a mapping", self.obj.class())))?;
        f(self, needle, vm)
    }

    fn _ass_subscript(
        &self,
        needle: &PyObject,
        value: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let f = self.methods.ass_subscript.ok_or_else(|| {
            vm.new_type_error(format!(
                "'{}' object does not support item assignment",
                self.obj.class()
            ))
        })?;
        f(self, needle, value, vm)
    }

    pub fn keys(&self, vm: &VirtualMachine) -> PyResult {
        if let Some(dict) = self.obj.downcast_ref_if_exact::<PyDict>(vm) {
            PyDictKeys::new(dict.to_owned()).to_pyresult(vm)
        } else {
            self.method_output_as_list(identifier!(vm, keys), vm)
        }
    }

    pub fn values(&self, vm: &VirtualMachine) -> PyResult {
        if let Some(dict) = self.obj.downcast_ref_if_exact::<PyDict>(vm) {
            PyDictValues::new(dict.to_owned()).to_pyresult(vm)
        } else {
            self.method_output_as_list(identifier!(vm, values), vm)
        }
    }

    pub fn items(&self, vm: &VirtualMachine) -> PyResult {
        if let Some(dict) = self.obj.downcast_ref_if_exact::<PyDict>(vm) {
            PyDictItems::new(dict.to_owned()).to_pyresult(vm)
        } else {
            self.method_output_as_list(identifier!(vm, items), vm)
        }
    }

    fn method_output_as_list(
        &self,
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

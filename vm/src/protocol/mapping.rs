use crate::{
    builtins::{
        dict::{PyDictKeys, PyDictValues},
        PyDictRef, PyList,
    },
    function::IntoPyObject,
    IdProtocol, PyObjectRef, PyResult, TryFromObject, TypeProtocol, VirtualMachine,
};
use std::borrow::Borrow;
use std::ops::Deref;

// Mapping protocol
// https://docs.python.org/3/c-api/mapping.html
#[allow(clippy::type_complexity)]
#[derive(Default)]
pub struct PyMappingMethods {
    pub length: Option<fn(PyObjectRef, &VirtualMachine) -> PyResult<usize>>,
    pub subscript: Option<fn(PyObjectRef, PyObjectRef, &VirtualMachine) -> PyResult>,
    pub ass_subscript:
        Option<fn(PyObjectRef, PyObjectRef, Option<PyObjectRef>, &VirtualMachine) -> PyResult<()>>,
}

#[derive(Debug, Clone)]
#[repr(transparent)]
pub struct PyMapping<T = PyObjectRef>(T)
where
    T: Borrow<PyObjectRef>;

impl PyMapping<PyObjectRef> {
    pub fn into_object(self) -> PyObjectRef {
        self.0
    }

    pub fn check(obj: &PyObjectRef) -> bool {
        obj.class()
            .mro_find_map(|x| x.slots.as_mapping.load())
            .is_some()
    }

    pub fn methods(&self, vm: &VirtualMachine) -> PyMappingMethods {
        let obj_cls = self.0.class();
        for cls in obj_cls.iter_mro() {
            if let Some(f) = cls.slots.as_mapping.load() {
                return f(&self.0, vm).unwrap();
            }
        }
        PyMappingMethods::default()
    }
}

impl<T> PyMapping<T>
where
    T: Borrow<PyObjectRef>,
{
    pub fn new(obj: T) -> Self {
        Self(obj)
    }

    pub fn keys(&self, vm: &VirtualMachine) -> PyResult {
        if self.0.borrow().is(&vm.ctx.types.dict_type) {
            Ok(
                PyDictKeys::new(PyDictRef::try_from_object(vm, self.0.borrow().clone())?)
                    .into_pyobject(vm),
            )
        } else {
            Self::method_output_as_list(self.0.borrow(), "keys", vm)
        }
    }

    pub fn values(&self, vm: &VirtualMachine) -> PyResult {
        if self.0.borrow().is(&vm.ctx.types.dict_type) {
            Ok(
                PyDictValues::new(PyDictRef::try_from_object(vm, self.0.borrow().clone())?)
                    .into_pyobject(vm),
            )
        } else {
            Self::method_output_as_list(self.0.borrow(), "values", vm)
        }
    }

    fn method_output_as_list(
        obj: &PyObjectRef,
        method_name: &str,
        vm: &VirtualMachine,
    ) -> PyResult {
        let meth_output = vm.call_method(obj, method_name, ())?;
        if meth_output.is(&vm.ctx.types.list_type) {
            return Ok(meth_output);
        }

        let iter = meth_output.clone().get_iter(vm).map_err(|_| {
            vm.new_type_error(format!(
                "{}.{}() returned a non-iterable (type {})",
                obj.class(),
                method_name,
                meth_output.class()
            ))
        })?;

        Ok(PyList::from(vm.extract_elements(&iter)?).into_pyobject(vm))
    }
}

impl<T> Borrow<PyObjectRef> for PyMapping<T>
where
    T: Borrow<PyObjectRef>,
{
    fn borrow(&self) -> &PyObjectRef {
        self.0.borrow()
    }
}

impl<T> Deref for PyMapping<T>
where
    T: Borrow<PyObjectRef>,
{
    type Target = PyObjectRef;
    fn deref(&self) -> &Self::Target {
        self.0.borrow()
    }
}

impl IntoPyObject for PyMapping<PyObjectRef> {
    fn into_pyobject(self, _vm: &VirtualMachine) -> PyObjectRef {
        self.into_object()
    }
}

impl TryFromObject for PyMapping<PyObjectRef> {
    fn try_from_object(vm: &VirtualMachine, mapping: PyObjectRef) -> PyResult<Self> {
        if Self::check(&mapping) {
            Ok(Self::new(mapping))
        } else {
            Err(vm.new_type_error(format!("{} is not a mapping object", mapping.class())))
        }
    }
}

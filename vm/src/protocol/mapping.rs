use crate::{
    builtins::{
        dict::{PyDictKeys, PyDictValues},
        PyDictRef, PyList,
    },
    function::IntoPyObject,
    IdProtocol, PyObject, PyObjectRef, PyObjectWrap, PyResult, TryFromObject, TypeProtocol,
    VirtualMachine,
};
use std::borrow::Borrow;

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
    T: Borrow<PyObject>;

impl PyMapping<PyObjectRef> {
    pub fn check(obj: &PyObject, vm: &VirtualMachine) -> bool {
        obj.class()
            .mro_find_map(|x| x.slots.as_mapping.load())
            .map(|f| f(obj, vm).subscript.is_some())
            .unwrap_or(false)
    }

    pub fn methods(&self, vm: &VirtualMachine) -> PyMappingMethods {
        let obj_cls = self.0.class();
        for cls in obj_cls.iter_mro() {
            if let Some(f) = cls.slots.as_mapping.load() {
                return f(&self.0, vm);
            }
        }
        PyMappingMethods::default()
    }
}

impl<T> PyMapping<T>
where
    T: Borrow<PyObject>,
{
    pub fn new(obj: T) -> Self {
        Self(obj)
    }

    pub fn keys(&self, vm: &VirtualMachine) -> PyResult {
        if self.0.borrow().is(&vm.ctx.types.dict_type) {
            Ok(
                PyDictKeys::new(PyDictRef::try_from_object(vm, self.0.borrow().to_owned())?)
                    .into_pyobject(vm),
            )
        } else {
            Self::method_output_as_list(self.0.borrow(), "keys", vm)
        }
    }

    pub fn values(&self, vm: &VirtualMachine) -> PyResult {
        if self.0.borrow().is(&vm.ctx.types.dict_type) {
            Ok(
                PyDictValues::new(PyDictRef::try_from_object(vm, self.0.borrow().to_owned())?)
                    .into_pyobject(vm),
            )
        } else {
            Self::method_output_as_list(self.0.borrow(), "values", vm)
        }
    }

    fn method_output_as_list(obj: &PyObject, method_name: &str, vm: &VirtualMachine) -> PyResult {
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

impl PyObjectWrap for PyMapping<PyObjectRef> {
    fn into_object(self) -> PyObjectRef {
        self.0
    }
}

impl<O> AsRef<PyObject> for PyMapping<O>
where
    O: Borrow<PyObject>,
{
    fn as_ref(&self) -> &PyObject {
        self.0.borrow()
    }
}

impl IntoPyObject for PyMapping<PyObjectRef> {
    fn into_pyobject(self, _vm: &VirtualMachine) -> PyObjectRef {
        self.into()
    }
}

impl TryFromObject for PyMapping<PyObjectRef> {
    fn try_from_object(vm: &VirtualMachine, mapping: PyObjectRef) -> PyResult<Self> {
        if Self::check(&mapping, vm) {
            Ok(Self::new(mapping))
        } else {
            Err(vm.new_type_error(format!("{} is not a mapping object", mapping.class())))
        }
    }
}

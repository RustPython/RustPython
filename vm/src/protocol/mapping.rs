use crate::{
    builtins::{
        dict::{PyDictItems, PyDictKeys, PyDictValues},
        PyDict,
    },
    common::lock::OnceCell,
    IdProtocol, PyObject, PyObjectPayload, PyObjectRef, PyObjectView, PyResult, TypeProtocol,
    VirtualMachine,
};

// Mapping protocol
// https://docs.python.org/3/c-api/mapping.html
#[allow(clippy::type_complexity)]
#[derive(Default, Copy, Clone)]
pub struct PyMappingMethods {
    pub length: Option<fn(&PyMapping, &VirtualMachine) -> PyResult<usize>>,
    pub subscript: Option<fn(&PyMapping, &PyObject, &VirtualMachine) -> PyResult>,
    pub ass_subscript:
        Option<fn(&PyMapping, &PyObject, Option<PyObjectRef>, &VirtualMachine) -> PyResult<()>>,
}

#[derive(Debug, Clone)]
pub struct PyMapping<'a> {
    pub obj: &'a PyObject,
    methods: OnceCell<PyMappingMethods>,
}

impl From<&PyObject> for PyMapping<'_> {
    fn from(obj: &PyObject) -> Self {
        Self {
            obj,
            methods: OnceCell::new(),
        }
    }
}

impl AsRef<PyObject> for PyMapping<'_> {
    fn as_ref(&self) -> &PyObject {
        self.obj
    }
}

impl PyMapping<'_> {
    pub fn obj_as<T: PyObjectPayload>(&self) -> &PyObjectView<T> {
        unsafe { self.obj.downcast_unchecked_ref::<T>() }
    }

    pub fn methods(&self, vm: &VirtualMachine) -> &PyMappingMethods {
        self.methods.get_or_init(|| {
            if let Some(f) = self
                .obj
                .class()
                .mro_find_map(|cls| cls.slots.as_mapping.load())
            {
                f(self.obj, vm)
            } else {
                PyMappingMethods::default()
            }
        })
    }

    // PyMapping::Check
    pub fn has_protocol(&self, vm: &VirtualMachine) -> bool {
        self.methods(vm).subscript.is_some()
    }

    pub fn try_protocol(&self, vm: &VirtualMachine) -> PyResult<()> {
        if self.has_protocol(vm) {
            Ok(())
        } else {
            Err(vm.new_type_error(format!("{} is not a mapping object", self.obj.class())))
        }
    }

    pub fn length_opt(&self, vm: &VirtualMachine) -> Option<PyResult<usize>> {
        self.methods(vm).length.map(|f| f(self, vm))
    }

    pub fn length(&self, vm: &VirtualMachine) -> PyResult<usize> {
        self.length_opt(vm).ok_or_else(|| {
            vm.new_type_error(format!(
                "object of type '{}' has no len() or not a mapping",
                self.0.class().name()
            ))
        })?
    }

    pub fn keys(&self, vm: &VirtualMachine) -> PyResult {
        if let Some(dict) = self.obj.downcast_ref_if_exact::<PyDict>(vm) {
            PyDictKeys::new(dict.to_owned()).into_pyresult(vm)
        } else {
            self.method_output_as_list("keys", vm)
        }
    }

    pub fn values(&self, vm: &VirtualMachine) -> PyResult {
        if let Some(dict) = self.obj.downcast_ref_if_exact::<PyDict>(vm) {
            PyDictValues::new(dict.to_owned()).into_pyresult(vm)
        } else {
            self.method_output_as_list("values", vm)
        }
    }

    pub fn items(&self, vm: &VirtualMachine) -> PyResult {
        if let Some(dict) = self.obj.downcast_ref_if_exact::<PyDict>(vm) {
            PyDictItems::new(dict.to_owned()).into_pyresult(vm)
        } else {
            self.method_output_as_list("items", vm)
        }
    }

    fn method_output_as_list(&self, method_name: &str, vm: &VirtualMachine) -> PyResult {
        let meth_output = vm.call_method(self.obj, method_name, ())?;
        if meth_output.is(&vm.ctx.types.list_type) {
            return Ok(meth_output);
        }

        let iter = meth_output.clone().get_iter(vm).map_err(|_| {
            vm.new_type_error(format!(
                "{}.{}() returned a non-iterable (type {})",
                self.obj.class(),
                method_name,
                meth_output.class()
            ))
        })?;

        // TODO
        // PySequence::from(&iter).list(vm).map(|x| x.into())
        vm.ctx
            .new_list(vm.extract_elements(&iter)?)
            .into_pyresult(vm)
    }
}

use crate::builtins::{PyList, PyStr, PyTuple, PyTypeRef};
use crate::function::FuncArgs;
use crate::types::GetAttr;
use crate::{AsObject, Py, PyObjectRef, PyPayload, PyResult, VirtualMachine};
use rustpython_common::lock::PyRwLock;
use rustpython_vm::types::Constructor;
use std::collections::HashMap;
use std::fmt::Debug;

#[pyclass(name = "Structure", module = "_ctypes")]
#[derive(PyPayload, Debug)]
pub struct PyCStructure {
    #[allow(dead_code)]
    field_data: PyRwLock<HashMap<String, PyObjectRef>>,
    data: PyRwLock<HashMap<String, PyObjectRef>>,
}

impl Constructor for PyCStructure {
    type Args = FuncArgs;

    fn py_new(cls: PyTypeRef, _args: Self::Args, vm: &VirtualMachine) -> PyResult {
        let fields_attr = cls
            .get_class_attr(vm.ctx.interned_str("_fields_").unwrap())
            .ok_or_else(|| {
                vm.new_attribute_error("Structure must have a _fields_ attribute".to_string())
            })?;
        // downcast into list
        let fields = fields_attr.downcast_ref::<PyList>().ok_or_else(|| {
            vm.new_type_error("Structure _fields_ attribute must be a list".to_string())
        })?;
        let fields = fields.borrow_vec();
        let mut field_data = HashMap::new();
        for field in fields.iter() {
            let field = field
                .downcast_ref::<PyTuple>()
                .ok_or_else(|| vm.new_type_error("Field must be a tuple".to_string()))?;
            let name = field
                .first()
                .unwrap()
                .downcast_ref::<PyStr>()
                .ok_or_else(|| vm.new_type_error("Field name must be a string".to_string()))?;
            let typ = field.get(1).unwrap().clone();
            field_data.insert(name.as_str().to_string(), typ);
        }
        todo!("Implement PyCStructure::py_new")
    }
}

impl GetAttr for PyCStructure {
    fn getattro(zelf: &Py<Self>, name: &Py<PyStr>, vm: &VirtualMachine) -> PyResult {
        let name = name.to_string();
        let data = zelf.data.read();
        match data.get(&name) {
            Some(value) => Ok(value.clone()),
            None => Err(vm.new_attribute_error(format!("No attribute named {}", name))),
        }
    }
}

#[pyclass(flags(BASETYPE, IMMUTABLETYPE))]
impl PyCStructure {}

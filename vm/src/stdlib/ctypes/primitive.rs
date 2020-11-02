use crate::builtins::PyTypeRef;
use crate::builtins::pystr::PyStrRef;
use crate::pyobject::{PyValue, StaticType, PyResult};
use crate::VirtualMachine;

use crate::stdlib::ctypes::common::CDataObject;

const SIMPLE_TYPE_CHARS: &'static str = "cbBhHiIlLdfuzZqQP?g";


#[pyclass(module = "_ctypes", name = "_SimpleCData", base = "CDataObject")]
#[derive(Debug)]
pub struct PySimpleType {
    _type_: PyStrRef,
}

impl PyValue for PySimpleType {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        Self::static_type()
    }
}

#[pyimpl]
impl PySimpleType {
    #[inline]
    pub fn new(_type: PyStrRef,vm: &VirtualMachine) -> PyResult<PySimpleType> {
        // Needs to force the existence of _type_
        // Does it need to be here?
        // Err(vm.new_attribute_error("class must define a '_type_' attribute".to_string()))
        
        let s_type = _type.to_string();

        if s_type.len() != 1{
            Err(vm.new_attribute_error("class must define a '_type_' attribute which must be a string of length 1".to_string()))
        }

        else {
            
            if SIMPLE_TYPE_CHARS.contains(s_type.as_str()){
                Ok(PySimpleType {_type_ : _type})
            }

            else {
                Err(vm.new_attribute_error(format!("class must define a '_type_' attribute which must be\na single character string containing one of '{}'.",SIMPLE_TYPE_CHARS)))
            }            
        }
    }
}
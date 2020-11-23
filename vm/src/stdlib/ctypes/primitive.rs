use std::fmt;
use crossbeam_utils::atomic::AtomicCell;

use crate::builtins::pystr::PyStr;
use crate::builtins::PyTypeRef;
use crate::pyobject::{PyObjectRc, PyRef, PyResult, PyValue, StaticType};
use crate::VirtualMachine;

use crate::stdlib::ctypes::basics::PyCData;

pub const SIMPLE_TYPE_CHARS: &str = "cbBhHiIlLdfuzZqQP?g";

#[pyclass(module = "_ctypes", name = "_SimpleCData", base = "PyCData")]
pub struct PySimpleType {
    _type_: String,
    value: AtomicCell<PyObjectRc>,
}

impl fmt::Debug for PySimpleType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let value = unsafe { (*self.value.as_ptr()).to_string() };

        write!(
            f,
            "PySimpleType {{
            _type_: {},
            value: {},
        }}",
            self._type_.as_str(),
            value
        )
    }
}

impl PyValue for PySimpleType {
    fn class(_vm: &VirtualMachine) -> &PyTypeRef {
        Self::static_metaclass()
    }
}

#[pyimpl]
impl PySimpleType {
    #[pyslot]
    fn tp_new(cls: PyTypeRef, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        match vm.get_attribute(cls.as_object().to_owned(), "_type_") {
            Ok(_type_) => {
                if vm.isinstance(&_type_, &vm.ctx.types.str_type)? {
                    if _type_.to_string().len() != 1 {
                        Err(vm.new_value_error("class must define a '_type_' attribute which must be a string of length 1".to_string()))
                    } else if !SIMPLE_TYPE_CHARS.contains(_type_.to_string().as_str()) {
                        Err(vm.new_attribute_error(format!("class must define a '_type_' attribute which must be\na single character string containing one of {}.",SIMPLE_TYPE_CHARS)))
                    } else {
                        PySimpleType {
                            _type_: _type_.downcast_exact::<PyStr>(vm).unwrap().to_string(),
                            value: AtomicCell::new(vm.ctx.none()),
                        }
                        .into_ref_with_type(vm, cls)
                    }
                } else {
                    Err(vm.new_type_error(
                        "class must define a '_type_' string attribute".to_string(),
                    ))
                }
            }
            Err(_) => {
                Err(vm.new_attribute_error("class must define a '_type_' attribute".to_string()))
            }
        }
    }

    #[pymethod(name = "__init__")]
    pub fn init(&self, value: Option<PyObjectRc>, vm: &VirtualMachine) -> PyResult<()> {
        let content = if let Some(ref v) = value {
            // @TODO: Needs to check if value has a simple (rust native) payload
            // and convert into the type on _type_
            v.clone()
        } else {
            match self._type_.as_str() {
                "c" | "u" => vm.ctx.new_bytes(vec![0]),
                "b" | "B" | "h" | "H" | "i" | "I" | "l" | "q" | "L" | "Q" => vm.ctx.new_int(0),
                "f" | "d" | "g" => vm.ctx.new_float(0.0),
                "?" => vm.ctx.new_bool(false),
                "z" | "Z" | "P" | _ => vm.ctx.none(), // @TODO: What should we do here? Throw an error?
            }
        };

        self.value.store(content);
        Ok(())
    }

    // From Simple_Type Simple_methods
    #[pymethod(name = "__ctypes_from_outparam__")]
    pub fn ctypes_from_outparam(&self) {}

    // From PyCSimpleType_Type PyCSimpleType_methods
    #[pyclassmethod]
    pub fn from_param(cls: PyTypeRef, vm: &VirtualMachine) {}

    // #[pymethod(name = "__repr__")]
    // fn repr(&self) -> String {
    //     format!("{}({})",self.class().name, self.value.to_string())
    // }

    // #[pymethod(name = "__bool__")]
    // fn bool(&self) -> bool {
    //
    // }
}

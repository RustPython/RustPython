use crate::builtins::{PyBytes, PyFloat, PyInt, PyNone, PyStr, PyTypeRef};
use crate::{AsObject, Py, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject, VirtualMachine};
use crossbeam_utils::atomic::AtomicCell;
use num_traits::ToPrimitive;
use rustpython_common::lock::PyRwLock;
use std::fmt::Debug;
use crate::function::{Either, OptionalArg};
use crate::stdlib::ctypes::_ctypes::new_simple_type;
use crate::builtins::PyType;

#[allow(dead_code)]
fn set_primitive(_type_: &str, value: &PyObjectRef, vm: &VirtualMachine) -> PyResult {
    match _type_ {
        "c" => {
            if value
                .clone()
                .downcast_exact::<PyBytes>(vm)
                .is_ok_and(|v| v.len() == 1)
                || value
                    .clone()
                    .downcast_exact::<PyBytes>(vm)
                    .is_ok_and(|v| v.len() == 1)
                || value
                    .clone()
                    .downcast_exact::<PyInt>(vm)
                    .map_or(Ok(false), |v| {
                        let n = v.as_bigint().to_i64();
                        if let Some(n) = n {
                            Ok((0..=255).contains(&n))
                        } else {
                            Ok(false)
                        }
                    })?
            {
                Ok(value.clone())
            } else {
                Err(vm.new_type_error(
                    "one character bytes, bytearray or integer expected".to_string(),
                ))
            }
        }
        "u" => {
            if let Ok(b) = value.str(vm).map(|v| v.to_string().chars().count() == 1) {
                if b {
                    Ok(value.clone())
                } else {
                    Err(vm.new_type_error("one character unicode string expected".to_string()))
                }
            } else {
                Err(vm.new_type_error(format!(
                    "unicode string expected instead of {} instance",
                    value.class().name()
                )))
            }
        }
        "b" | "h" | "H" | "i" | "I" | "l" | "q" | "L" | "Q" => {
            if value.clone().downcast_exact::<PyInt>(vm).is_ok() {
                Ok(value.clone())
            } else {
                Err(vm.new_type_error(format!(
                    "an integer is required (got type {})",
                    value.class().name()
                )))
            }
        }
        "f" | "d" | "g" => {
            if value.clone().downcast_exact::<PyFloat>(vm).is_ok() {
                Ok(value.clone())
            } else {
                Err(vm.new_type_error(format!("must be real number, not {}", value.class().name())))
            }
        }
        "?" => Ok(PyObjectRef::from(
            vm.ctx.new_bool(value.clone().try_to_bool(vm)?),
        )),
        "B" => {
            if value.clone().downcast_exact::<PyInt>(vm).is_ok() {
                Ok(vm.new_pyobj(u8::try_from_object(vm, value.clone())?))
            } else {
                Err(vm.new_type_error(format!("int expected instead of {}", value.class().name())))
            }
        }
        "z" => {
            if value.clone().downcast_exact::<PyInt>(vm).is_ok()
                || value.clone().downcast_exact::<PyBytes>(vm).is_ok()
            {
                Ok(value.clone())
            } else {
                Err(vm.new_type_error(format!(
                    "bytes or integer address expected instead of {} instance",
                    value.class().name()
                )))
            }
        }
        "Z" => {
            if value.clone().downcast_exact::<PyStr>(vm).is_ok() {
                Ok(value.clone())
            } else {
                Err(vm.new_type_error(format!(
                    "unicode string or integer address expected instead of {} instance",
                    value.class().name()
                )))
            }
        }
        _ => {
            // "P"
            if value.clone().downcast_exact::<PyInt>(vm).is_ok()
                || value.clone().downcast_exact::<PyNone>(vm).is_ok()
            {
                Ok(value.clone())
            } else {
                Err(vm.new_type_error("cannot be converted to pointer".to_string()))
            }
        }
    }
}

pub struct RawBuffer {
    #[allow(dead_code)]
    pub inner: Box<[u8]>,
    #[allow(dead_code)]
    pub size: usize,
}

#[pyclass(name = "_CData", module = "_ctypes")]
pub struct PyCData {
    _objects: AtomicCell<Vec<PyObjectRef>>,
    _buffer: PyRwLock<RawBuffer>,
}

#[pyclass]
impl PyCData {}

#[pyclass(module = "_ctypes", name = "PyCSimpleType", base = "PyType")]
pub struct PySimpleMeta {}

#[pyclass(flags(BASETYPE))]
impl PySimpleMeta {
    #[pymethod]
    fn new(cls: PyTypeRef, _: OptionalArg, vm: &VirtualMachine) -> PyResult {
        Ok(PyObjectRef::from(new_simple_type(Either::B(&cls), vm)?
            .into_ref_with_type(vm, cls)?
            .clone()))
    }
}

#[pyclass(
    name = "_SimpleCData",
    base = "PyCData",
    module = "_ctypes",
    metaclass = "PySimpleMeta"
)]
#[derive(PyPayload)]
pub struct PyCSimple {
    pub _type_: String,
    pub value: AtomicCell<PyObjectRef>,
}

impl Debug for PyCSimple {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PyCSimple")
            .field("_type_", &self._type_)
            .finish()
    }
}

#[pyclass(flags(BASETYPE))]
impl PyCSimple {
    #[pymethod(magic)]
    pub fn __init__(&self, value: OptionalArg, vm: &VirtualMachine) -> PyResult<()> {
        if let Some(ref v) = value.into_option() {
            let content = set_primitive(self._type_.as_str(), v, vm)?;
            self.value.store(content);
        } else {
            self.value.store(match self._type_.as_str() {
                "c" | "u" => PyObjectRef::from(vm.ctx.new_bytes(vec![0])),
                "b" | "B" | "h" | "H" | "i" | "I" | "l" | "q" | "L" | "Q" => PyObjectRef::from(vm.ctx.new_int(0)),
                "f" | "d" | "g" => PyObjectRef::from(vm.ctx.new_float(0.0)),
                "?" => PyObjectRef::from(vm.ctx.new_bool(false)),
                _ => vm.ctx.none(), // "z" | "Z" | "P"
            });
        }
        Ok(())
    }

    #[pygetset(name = "value")]
    pub fn value(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let cls = instance.class();
        let subcls_vec = cls.subclasses.read();
        for subcls in subcls_vec.iter() {
            println!("subcls {}", subcls);
        }
        println!("value {}", cls.name().to_string());
        let zelf: &Py<Self> = instance.downcast_ref().ok_or_else(|| {
            vm.new_type_error("cannot get value of instance".to_string())
        })?;
        Ok(unsafe { (*zelf.value.as_ptr()).clone() })
    }

    #[pygetset(name = "value", setter)]
    fn set_value(instance: PyObjectRef, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let zelf: PyRef<Self> = instance.downcast().map_err(|_| {
            vm.new_type_error("cannot set value of instance".to_string())
        })?;
        let content = set_primitive(zelf._type_.as_str(), &value, vm)?;
        zelf.value.store(content);
        Ok(())
    }
}

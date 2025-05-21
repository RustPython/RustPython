use super::array::{PyCArray, PyCArrayType};
use crate::builtins::PyType;
use crate::builtins::{PyBytes, PyFloat, PyInt, PyNone, PyStr, PyTypeRef};
use crate::convert::ToPyObject;
use crate::function::{Either, OptionalArg};
use crate::stdlib::ctypes::_ctypes::new_simple_type;
use crate::types::Constructor;
use crate::{AsObject, Py, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject, VirtualMachine};
use crossbeam_utils::atomic::AtomicCell;
use num_traits::ToPrimitive;
use rustpython_common::lock::PyRwLock;
use std::fmt::Debug;

pub fn ffi_type_from_str(_type_: &str) -> Option<libffi::middle::Type> {
    match _type_ {
        "c" => Some(libffi::middle::Type::u8()),
        "u" => Some(libffi::middle::Type::u32()),
        "b" => Some(libffi::middle::Type::i8()),
        "B" => Some(libffi::middle::Type::u8()),
        "h" => Some(libffi::middle::Type::i16()),
        "H" => Some(libffi::middle::Type::u16()),
        "i" => Some(libffi::middle::Type::i32()),
        "I" => Some(libffi::middle::Type::u32()),
        "l" => Some(libffi::middle::Type::i32()),
        "L" => Some(libffi::middle::Type::u32()),
        "q" => Some(libffi::middle::Type::i64()),
        "Q" => Some(libffi::middle::Type::u64()),
        "f" => Some(libffi::middle::Type::f32()),
        "d" => Some(libffi::middle::Type::f64()),
        "g" => Some(libffi::middle::Type::f64()),
        "?" => Some(libffi::middle::Type::u8()),
        "z" => Some(libffi::middle::Type::u64()),
        "Z" => Some(libffi::middle::Type::u64()),
        "P" => Some(libffi::middle::Type::u64()),
        _ => None,
    }
}

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
pub struct PyCSimpleType {}

#[pyclass(flags(BASETYPE))]
impl PyCSimpleType {
    #[allow(clippy::new_ret_no_self)]
    #[pymethod]
    fn new(cls: PyTypeRef, _: OptionalArg, vm: &VirtualMachine) -> PyResult {
        Ok(PyObjectRef::from(
            new_simple_type(Either::B(&cls), vm)?
                .into_ref_with_type(vm, cls)?
                .clone(),
        ))
    }
}

#[pyclass(
    module = "_ctypes",
    name = "_SimpleCData",
    base = "PyCData",
    metaclass = "PyCSimpleType"
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

impl Constructor for PyCSimple {
    type Args = (OptionalArg,);

    fn py_new(cls: PyTypeRef, args: Self::Args, vm: &VirtualMachine) -> PyResult {
        let attributes = cls.get_attributes();
        let _type_ = attributes
            .iter()
            .find(|(k, _)| k.to_object().str(vm).unwrap().to_string() == *"_type_")
            .unwrap()
            .1
            .str(vm)?
            .to_string();
        let value = if let Some(ref v) = args.0.into_option() {
            set_primitive(_type_.as_str(), v, vm)?
        } else {
            match _type_.as_str() {
                "c" | "u" => PyObjectRef::from(vm.ctx.new_bytes(vec![0])),
                "b" | "B" | "h" | "H" | "i" | "I" | "l" | "q" | "L" | "Q" => {
                    PyObjectRef::from(vm.ctx.new_int(0))
                }
                "f" | "d" | "g" => PyObjectRef::from(vm.ctx.new_float(0.0)),
                "?" => PyObjectRef::from(vm.ctx.new_bool(false)),
                _ => vm.ctx.none(), // "z" | "Z" | "P"
            }
        };
        Ok(PyCSimple {
            _type_,
            value: AtomicCell::new(value),
        }
        .to_pyobject(vm))
    }
}

#[pyclass(flags(BASETYPE), with(Constructor))]
impl PyCSimple {
    #[pygetset(name = "value")]
    pub fn value(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let zelf: &Py<Self> = instance
            .downcast_ref()
            .ok_or_else(|| vm.new_type_error("cannot get value of instance".to_string()))?;
        Ok(unsafe { (*zelf.value.as_ptr()).clone() })
    }

    #[pygetset(name = "value", setter)]
    fn set_value(instance: PyObjectRef, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let zelf: PyRef<Self> = instance
            .downcast()
            .map_err(|_| vm.new_type_error("cannot set value of instance".to_string()))?;
        let content = set_primitive(zelf._type_.as_str(), &value, vm)?;
        zelf.value.store(content);
        Ok(())
    }

    #[pyclassmethod]
    fn repeat(cls: PyTypeRef, n: isize, vm: &VirtualMachine) -> PyResult {
        if n < 0 {
            return Err(vm.new_value_error(format!("Array length must be >= 0, not {}", n)));
        }
        Ok(PyCArrayType {
            inner: PyCArray {
                typ: PyRwLock::new(cls),
                length: AtomicCell::new(n as usize),
                value: PyRwLock::new(vm.ctx.none()),
            },
        }
        .to_pyobject(vm))
    }

    #[pyclassmethod(magic)]
    fn mul(cls: PyTypeRef, n: isize, vm: &VirtualMachine) -> PyResult {
        PyCSimple::repeat(cls, n, vm)
    }
}

impl PyCSimple {
    pub fn to_arg(
        &self,
        ty: libffi::middle::Type,
        vm: &VirtualMachine,
    ) -> Option<libffi::middle::Arg> {
        let value = unsafe { (*self.value.as_ptr()).clone() };
        if let Ok(i) = value.try_int(vm) {
            let i = i.as_bigint();
            return if std::ptr::eq(ty.as_raw_ptr(), libffi::middle::Type::u8().as_raw_ptr()) {
                i.to_u8().map(|r: u8| libffi::middle::Arg::new(&r))
            } else if std::ptr::eq(ty.as_raw_ptr(), libffi::middle::Type::i8().as_raw_ptr()) {
                i.to_i8().map(|r: i8| libffi::middle::Arg::new(&r))
            } else if std::ptr::eq(ty.as_raw_ptr(), libffi::middle::Type::u16().as_raw_ptr()) {
                i.to_u16().map(|r: u16| libffi::middle::Arg::new(&r))
            } else if std::ptr::eq(ty.as_raw_ptr(), libffi::middle::Type::i16().as_raw_ptr()) {
                i.to_i16().map(|r: i16| libffi::middle::Arg::new(&r))
            } else if std::ptr::eq(ty.as_raw_ptr(), libffi::middle::Type::u32().as_raw_ptr()) {
                i.to_u32().map(|r: u32| libffi::middle::Arg::new(&r))
            } else if std::ptr::eq(ty.as_raw_ptr(), libffi::middle::Type::i32().as_raw_ptr()) {
                i.to_i32().map(|r: i32| libffi::middle::Arg::new(&r))
            } else if std::ptr::eq(ty.as_raw_ptr(), libffi::middle::Type::u64().as_raw_ptr()) {
                i.to_u64().map(|r: u64| libffi::middle::Arg::new(&r))
            } else if std::ptr::eq(ty.as_raw_ptr(), libffi::middle::Type::i64().as_raw_ptr()) {
                i.to_i64().map(|r: i64| libffi::middle::Arg::new(&r))
            } else {
                None
            }
        }
        if let Ok(_f) = value.try_float(vm) {
            todo!();
        }
        if let Ok(_b) = value.try_to_bool(vm) {
            todo!();
        }
        None
    }
}

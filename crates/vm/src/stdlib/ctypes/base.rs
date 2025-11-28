use super::_ctypes::bytes_to_pyobject;
use super::array::{PyCArray, PyCArrayType};
use crate::builtins::PyType;
use crate::builtins::{PyBytes, PyFloat, PyInt, PyNone, PyStr, PyTypeRef};
use crate::convert::ToPyObject;
use crate::function::{ArgBytesLike, Either, OptionalArg};
use crate::protocol::{PyBuffer, PyNumberMethods};
use crate::stdlib::ctypes::_ctypes::new_simple_type;
use crate::types::{AsNumber, Constructor};
use crate::{AsObject, Py, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject, VirtualMachine};
use crossbeam_utils::atomic::AtomicCell;
use num_traits::ToPrimitive;
use rustpython_common::lock::PyRwLock;
use std::ffi::{c_uint, c_ulong, c_ulonglong, c_ushort};
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
                Err(vm.new_type_error("one character bytes, bytearray or integer expected"))
            }
        }
        "u" => {
            if let Ok(b) = value.str(vm).map(|v| v.to_string().chars().count() == 1) {
                if b {
                    Ok(value.clone())
                } else {
                    Err(vm.new_type_error("one character unicode string expected"))
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
                // Store as-is, conversion to unsigned happens in the getter
                Ok(value.clone())
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
                Err(vm.new_type_error("cannot be converted to pointer"))
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

#[pyclass(module = "_ctypes", name = "PyCSimpleType", base = PyType)]
#[derive(Debug, PyPayload)]
pub struct PyCSimpleType {}

#[pyclass(flags(BASETYPE), with(AsNumber))]
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

    #[pyclassmethod]
    fn from_param(cls: PyTypeRef, value: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        // If the value is already an instance of the requested type, return it
        if value.fast_isinstance(&cls) {
            return Ok(value);
        }

        // Check for _as_parameter_ attribute
        let Ok(as_parameter) = value.get_attr("_as_parameter_", vm) else {
            return Err(vm.new_type_error("wrong type"));
        };

        PyCSimpleType::from_param(cls, as_parameter, vm)
    }

    #[pymethod]
    fn __mul__(cls: PyTypeRef, n: isize, vm: &VirtualMachine) -> PyResult {
        PyCSimple::repeat(cls, n, vm)
    }
}

impl AsNumber for PyCSimpleType {
    fn as_number() -> &'static PyNumberMethods {
        static AS_NUMBER: PyNumberMethods = PyNumberMethods {
            multiply: Some(|a, b, vm| {
                // a is a PyCSimpleType instance (type object like c_char)
                // b is int (array size)
                let cls = a
                    .downcast_ref::<PyType>()
                    .ok_or_else(|| vm.new_type_error("expected type".to_owned()))?;
                let n = b
                    .try_index(vm)?
                    .as_bigint()
                    .to_isize()
                    .ok_or_else(|| vm.new_overflow_error("array size too large".to_owned()))?;
                PyCSimple::repeat(cls.to_owned(), n, vm)
            }),
            ..PyNumberMethods::NOT_IMPLEMENTED
        };
        &AS_NUMBER
    }
}

#[pyclass(
    module = "_ctypes",
    name = "_SimpleCData",
    base = PyCData,
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
            .find(|(k, _)| {
                k.to_object()
                    .str(vm)
                    .map(|s| s.to_string() == "_type_")
                    .unwrap_or(false)
            })
            .ok_or_else(|| {
                vm.new_type_error(format!(
                    "cannot create '{}' instances: no _type_ attribute",
                    cls.name()
                ))
            })?
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
        PyCSimple {
            _type_,
            value: AtomicCell::new(value),
        }
        .into_ref_with_type(vm, cls)
        .map(Into::into)
    }
}

#[pyclass(flags(BASETYPE), with(Constructor))]
impl PyCSimple {
    #[pygetset(name = "value")]
    pub fn value(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let zelf: &Py<Self> = instance
            .downcast_ref()
            .ok_or_else(|| vm.new_type_error("cannot get value of instance"))?;
        let raw_value = unsafe { (*zelf.value.as_ptr()).clone() };

        // Convert to unsigned if needed for unsigned types
        match zelf._type_.as_str() {
            "B" | "H" | "I" | "L" | "Q" => {
                if let Ok(int_val) = raw_value.try_int(vm) {
                    let n = int_val.as_bigint();
                    // Use platform-specific C types for correct unsigned conversion
                    match zelf._type_.as_str() {
                        "B" => {
                            if let Some(v) = n.to_i64() {
                                return Ok(vm.ctx.new_int((v as u8) as u64).into());
                            }
                        }
                        "H" => {
                            if let Some(v) = n.to_i64() {
                                return Ok(vm.ctx.new_int((v as c_ushort) as u64).into());
                            }
                        }
                        "I" => {
                            if let Some(v) = n.to_i64() {
                                return Ok(vm.ctx.new_int((v as c_uint) as u64).into());
                            }
                        }
                        "L" => {
                            if let Some(v) = n.to_i128() {
                                return Ok(vm.ctx.new_int(v as c_ulong).into());
                            }
                        }
                        "Q" => {
                            if let Some(v) = n.to_i128() {
                                return Ok(vm.ctx.new_int(v as c_ulonglong).into());
                            }
                        }
                        _ => {}
                    };
                }
                Ok(raw_value)
            }
            _ => Ok(raw_value),
        }
    }

    #[pygetset(name = "value", setter)]
    fn set_value(instance: PyObjectRef, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let zelf: PyRef<Self> = instance
            .downcast()
            .map_err(|_| vm.new_type_error("cannot set value of instance"))?;
        let content = set_primitive(zelf._type_.as_str(), &value, vm)?;
        zelf.value.store(content);
        Ok(())
    }

    #[pyclassmethod]
    fn repeat(cls: PyTypeRef, n: isize, vm: &VirtualMachine) -> PyResult {
        use super::_ctypes::get_size;
        if n < 0 {
            return Err(vm.new_value_error(format!("Array length must be >= 0, not {n}")));
        }
        // Get element size from cls
        let element_size = if let Ok(type_attr) = cls.as_object().get_attr("_type_", vm) {
            if let Ok(s) = type_attr.str(vm) {
                let s = s.to_string();
                if s.len() == 1 {
                    get_size(&s)
                } else {
                    std::mem::size_of::<usize>()
                }
            } else {
                std::mem::size_of::<usize>()
            }
        } else {
            std::mem::size_of::<usize>()
        };
        Ok(PyCArrayType {
            inner: PyCArray {
                typ: PyRwLock::new(cls.clone().into()),
                length: AtomicCell::new(n as usize),
                element_size: AtomicCell::new(element_size),
                buffer: PyRwLock::new(vec![]),
            },
        }
        .to_pyobject(vm))
    }

    #[pyclassmethod]
    fn from_address(cls: PyTypeRef, address: isize, vm: &VirtualMachine) -> PyResult {
        use super::_ctypes::get_size;
        // Get _type_ attribute directly
        let type_attr = cls
            .as_object()
            .get_attr("_type_", vm)
            .map_err(|_| vm.new_type_error(format!("'{}' has no _type_ attribute", cls.name())))?;
        let type_str = type_attr.str(vm)?.to_string();
        let size = get_size(&type_str);

        // Create instance with value read from address
        let value = if address != 0 && size > 0 {
            // Safety: This is inherently unsafe - reading from arbitrary memory address
            // CPython does the same thing without safety checks
            unsafe {
                let ptr = address as *const u8;
                let bytes = std::slice::from_raw_parts(ptr, size);
                // Convert bytes to appropriate Python value based on type
                bytes_to_pyobject(&cls, bytes, vm)?
            }
        } else {
            vm.ctx.none()
        };

        // Create instance using the type's constructor
        let instance = PyCSimple::py_new(cls.clone(), (OptionalArg::Present(value),), vm)?;
        Ok(instance)
    }

    #[pyclassmethod]
    fn from_buffer(
        cls: PyTypeRef,
        source: PyObjectRef,
        offset: OptionalArg<isize>,
        vm: &VirtualMachine,
    ) -> PyResult {
        use super::_ctypes::get_size;
        let offset = offset.unwrap_or(0);
        if offset < 0 {
            return Err(vm.new_value_error("offset cannot be negative".to_owned()));
        }
        let offset = offset as usize;

        // Get buffer from source
        let buffer = PyBuffer::try_from_object(vm, source.clone())?;

        // Check if buffer is writable
        if buffer.desc.readonly {
            return Err(vm.new_type_error("underlying buffer is not writable".to_owned()));
        }

        // Get _type_ attribute directly
        let type_attr = cls
            .as_object()
            .get_attr("_type_", vm)
            .map_err(|_| vm.new_type_error(format!("'{}' has no _type_ attribute", cls.name())))?;
        let type_str = type_attr.str(vm)?.to_string();
        let size = get_size(&type_str);

        // Check if buffer is large enough
        let buffer_len = buffer.desc.len;
        if offset + size > buffer_len {
            return Err(vm.new_value_error(format!(
                "Buffer size too small ({} instead of at least {} bytes)",
                buffer_len,
                offset + size
            )));
        }

        // Read bytes from buffer at offset
        let bytes = buffer.obj_bytes();
        let data = &bytes[offset..offset + size];
        let value = bytes_to_pyobject(&cls, data, vm)?;

        // Create instance
        let instance = PyCSimple::py_new(cls.clone(), (OptionalArg::Present(value),), vm)?;

        // TODO: Store reference to source in _objects to keep buffer alive
        Ok(instance)
    }

    #[pyclassmethod]
    fn from_buffer_copy(
        cls: PyTypeRef,
        source: ArgBytesLike,
        offset: OptionalArg<isize>,
        vm: &VirtualMachine,
    ) -> PyResult {
        use super::_ctypes::get_size;
        let offset = offset.unwrap_or(0);
        if offset < 0 {
            return Err(vm.new_value_error("offset cannot be negative".to_owned()));
        }
        let offset = offset as usize;

        // Get _type_ attribute directly for simple types
        let type_attr = cls
            .as_object()
            .get_attr("_type_", vm)
            .map_err(|_| vm.new_type_error(format!("'{}' has no _type_ attribute", cls.name())))?;
        let type_str = type_attr.str(vm)?.to_string();
        let size = get_size(&type_str);

        // Borrow bytes from source
        let source_bytes = source.borrow_buf();
        let buffer_len = source_bytes.len();

        // Check if buffer is large enough
        if offset + size > buffer_len {
            return Err(vm.new_value_error(format!(
                "Buffer size too small ({} instead of at least {} bytes)",
                buffer_len,
                offset + size
            )));
        }

        // Copy bytes from buffer at offset
        let data = &source_bytes[offset..offset + size];
        let value = bytes_to_pyobject(&cls, data, vm)?;

        // Create instance (independent copy, no reference tracking)
        PyCSimple::py_new(cls.clone(), (OptionalArg::Present(value),), vm)
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
            };
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

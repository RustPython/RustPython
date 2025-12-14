use super::_ctypes::bytes_to_pyobject;
use super::util::StgInfo;
use crate::builtins::{PyBytes, PyFloat, PyInt, PyNone, PyStr, PyStrRef, PyType, PyTypeRef};
use crate::function::{ArgBytesLike, Either, FuncArgs, KwArgs, OptionalArg};
use crate::protocol::{BufferDescriptor, BufferMethods, PyBuffer, PyNumberMethods};
use crate::stdlib::ctypes::_ctypes::new_simple_type;
use crate::types::{AsBuffer, AsNumber, Constructor};
use crate::{AsObject, Py, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject, VirtualMachine};
use crossbeam_utils::atomic::AtomicCell;
use num_traits::ToPrimitive;
use rustpython_common::lock::PyRwLock;
use std::ffi::{c_uint, c_ulong, c_ulonglong, c_ushort};
use std::fmt::Debug;

/// Get the type code string from a ctypes type (e.g., "i" for c_int)
pub fn get_type_code(cls: &PyTypeRef, vm: &VirtualMachine) -> Option<String> {
    cls.as_object()
        .get_attr("_type_", vm)
        .ok()
        .and_then(|t| t.downcast_ref::<PyStr>().map(|s| s.to_string()))
}

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
            // float allows int
            if value.clone().downcast_exact::<PyFloat>(vm).is_ok()
                || value.clone().downcast_exact::<PyInt>(vm).is_ok()
            {
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

/// Common data object for all ctypes types
#[derive(Debug, Clone)]
pub struct CDataObject {
    /// pointer to memory block (b_ptr + b_size)
    pub buffer: Vec<u8>,
    /// pointer to base object or None (b_base)
    #[allow(dead_code)]
    pub base: Option<PyObjectRef>,
    /// index into base's b_objects list (b_index)
    #[allow(dead_code)]
    pub index: usize,
    /// dictionary of references we need to keep (b_objects)
    pub objects: Option<PyObjectRef>,
}

impl CDataObject {
    /// Create from StgInfo (PyCData_MallocBuffer pattern)
    pub fn from_stg_info(stg_info: &StgInfo) -> Self {
        CDataObject {
            buffer: vec![0u8; stg_info.size],
            base: None,
            index: 0,
            objects: None,
        }
    }

    /// Create from existing bytes (copies data)
    pub fn from_bytes(data: Vec<u8>, objects: Option<PyObjectRef>) -> Self {
        CDataObject {
            buffer: data,
            base: None,
            index: 0,
            objects,
        }
    }

    /// Create from base object (copies data from base's buffer at offset)
    #[allow(dead_code)]
    pub fn from_base(
        base: PyObjectRef,
        _offset: usize,
        size: usize,
        index: usize,
        objects: Option<PyObjectRef>,
    ) -> Self {
        CDataObject {
            buffer: vec![0u8; size],
            base: Some(base),
            index,
            objects,
        }
    }

    #[inline]
    pub fn size(&self) -> usize {
        self.buffer.len()
    }
}

#[pyclass(name = "_CData", module = "_ctypes")]
#[derive(Debug, PyPayload)]
pub struct PyCData {
    pub cdata: PyRwLock<CDataObject>,
}

#[pyclass(flags(BASETYPE))]
impl PyCData {
    #[pygetset]
    fn _objects(&self) -> Option<PyObjectRef> {
        self.cdata.read().objects.clone()
    }
}

#[pyclass(module = "_ctypes", name = "PyCSimpleType", base = PyType)]
#[derive(Debug, Default)]
pub struct PyCSimpleType {}

#[pyclass(flags(BASETYPE), with(AsNumber))]
impl PyCSimpleType {
    /// Get stg_info for a simple type by reading _type_ attribute
    pub fn get_stg_info(cls: &PyTypeRef, vm: &VirtualMachine) -> StgInfo {
        if let Ok(type_attr) = cls.as_object().get_attr("_type_", vm)
            && let Ok(type_str) = type_attr.str(vm)
        {
            let tp_str = type_str.to_string();
            if tp_str.len() == 1 {
                let size = super::_ctypes::get_size(&tp_str);
                let align = super::_ctypes::get_align(&tp_str);
                return StgInfo::new(size, align);
            }
        }
        StgInfo::default()
    }
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
        // 1. If the value is already an instance of the requested type, return it
        if value.fast_isinstance(&cls) {
            return Ok(value);
        }

        // 2. Get the type code to determine conversion rules
        let type_code = get_type_code(&cls, vm);

        // 3. Handle None for pointer types (c_char_p, c_wchar_p, c_void_p)
        if vm.is_none(&value) && matches!(type_code.as_deref(), Some("z") | Some("Z") | Some("P")) {
            return Ok(value);
        }

        // 4. Try to convert value based on type code
        match type_code.as_deref() {
            // Integer types: accept integers
            Some("b" | "B" | "h" | "H" | "i" | "I" | "l" | "L" | "q" | "Q") => {
                if value.try_int(vm).is_ok() {
                    let simple = new_simple_type(Either::B(&cls), vm)?;
                    simple.value.store(value.clone());
                    return simple.into_ref_with_type(vm, cls.clone()).map(Into::into);
                }
            }
            // Float types: accept numbers
            Some("f" | "d" | "g") => {
                if value.try_float(vm).is_ok() || value.try_int(vm).is_ok() {
                    let simple = new_simple_type(Either::B(&cls), vm)?;
                    simple.value.store(value.clone());
                    return simple.into_ref_with_type(vm, cls.clone()).map(Into::into);
                }
            }
            // c_char: 1 byte character
            Some("c") => {
                if let Some(bytes) = value.downcast_ref::<PyBytes>()
                    && bytes.len() == 1
                {
                    let simple = new_simple_type(Either::B(&cls), vm)?;
                    simple.value.store(value.clone());
                    return simple.into_ref_with_type(vm, cls.clone()).map(Into::into);
                }
                if let Ok(int_val) = value.try_int(vm)
                    && int_val.as_bigint().to_u8().is_some()
                {
                    let simple = new_simple_type(Either::B(&cls), vm)?;
                    simple.value.store(value.clone());
                    return simple.into_ref_with_type(vm, cls.clone()).map(Into::into);
                }
                return Err(vm.new_type_error(
                    "one character bytes, bytearray or integer expected".to_string(),
                ));
            }
            // c_wchar: 1 unicode character
            Some("u") => {
                if let Some(s) = value.downcast_ref::<PyStr>()
                    && s.as_str().chars().count() == 1
                {
                    let simple = new_simple_type(Either::B(&cls), vm)?;
                    simple.value.store(value.clone());
                    return simple.into_ref_with_type(vm, cls.clone()).map(Into::into);
                }
                return Err(vm.new_type_error("one character unicode string expected".to_string()));
            }
            // c_char_p: bytes pointer
            Some("z") => {
                if value.downcast_ref::<PyBytes>().is_some() {
                    let simple = new_simple_type(Either::B(&cls), vm)?;
                    simple.value.store(value.clone());
                    return simple.into_ref_with_type(vm, cls.clone()).map(Into::into);
                }
            }
            // c_wchar_p: unicode pointer
            Some("Z") => {
                if value.downcast_ref::<PyStr>().is_some() {
                    let simple = new_simple_type(Either::B(&cls), vm)?;
                    simple.value.store(value.clone());
                    return simple.into_ref_with_type(vm, cls.clone()).map(Into::into);
                }
            }
            // c_void_p: most flexible - accepts int, bytes, str
            Some("P") => {
                if value.try_int(vm).is_ok()
                    || value.downcast_ref::<PyBytes>().is_some()
                    || value.downcast_ref::<PyStr>().is_some()
                {
                    let simple = new_simple_type(Either::B(&cls), vm)?;
                    simple.value.store(value.clone());
                    return simple.into_ref_with_type(vm, cls.clone()).map(Into::into);
                }
            }
            // c_bool
            Some("?") => {
                let bool_val = value.is_true(vm)?;
                let simple = new_simple_type(Either::B(&cls), vm)?;
                simple.value.store(vm.ctx.new_bool(bool_val).into());
                return simple.into_ref_with_type(vm, cls.clone()).map(Into::into);
            }
            _ => {}
        }

        // 5. Check for _as_parameter_ attribute
        if let Ok(as_parameter) = value.get_attr("_as_parameter_", vm) {
            return PyCSimpleType::from_param(cls, as_parameter, vm);
        }

        // 6. Type-specific error messages
        match type_code.as_deref() {
            Some("z") => Err(vm.new_type_error(format!(
                "'{}' object cannot be interpreted as ctypes.c_char_p",
                value.class().name()
            ))),
            Some("Z") => Err(vm.new_type_error(format!(
                "'{}' object cannot be interpreted as ctypes.c_wchar_p",
                value.class().name()
            ))),
            _ => Err(vm.new_type_error("wrong type".to_string())),
        }
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
pub struct PyCSimple {
    pub _type_: String,
    pub value: AtomicCell<PyObjectRef>,
    pub cdata: PyRwLock<CDataObject>,
}

impl Debug for PyCSimple {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PyCSimple")
            .field("_type_", &self._type_)
            .finish()
    }
}

fn value_to_bytes_endian(
    _type_: &str,
    value: &PyObjectRef,
    swapped: bool,
    vm: &VirtualMachine,
) -> Vec<u8> {
    // Helper macro for endian conversion
    macro_rules! to_bytes {
        ($val:expr) => {
            if swapped {
                // Use opposite endianness
                #[cfg(target_endian = "little")]
                {
                    $val.to_be_bytes().to_vec()
                }
                #[cfg(target_endian = "big")]
                {
                    $val.to_le_bytes().to_vec()
                }
            } else {
                $val.to_ne_bytes().to_vec()
            }
        };
    }

    match _type_ {
        "c" => {
            // c_char - single byte
            if let Some(bytes) = value.downcast_ref::<PyBytes>()
                && !bytes.is_empty()
            {
                return vec![bytes.as_bytes()[0]];
            }
            if let Ok(int_val) = value.try_int(vm)
                && let Some(v) = int_val.as_bigint().to_u8()
            {
                return vec![v];
            }
            vec![0]
        }
        "u" => {
            // c_wchar - 4 bytes (wchar_t on most platforms)
            if let Ok(s) = value.str(vm)
                && let Some(c) = s.as_str().chars().next()
            {
                return to_bytes!(c as u32);
            }
            vec![0; 4]
        }
        "b" => {
            // c_byte - signed char (1 byte)
            if let Ok(int_val) = value.try_int(vm)
                && let Some(v) = int_val.as_bigint().to_i8()
            {
                return vec![v as u8];
            }
            vec![0]
        }
        "B" => {
            // c_ubyte - unsigned char (1 byte)
            if let Ok(int_val) = value.try_int(vm)
                && let Some(v) = int_val.as_bigint().to_u8()
            {
                return vec![v];
            }
            vec![0]
        }
        "h" => {
            // c_short (2 bytes)
            if let Ok(int_val) = value.try_int(vm)
                && let Some(v) = int_val.as_bigint().to_i16()
            {
                return to_bytes!(v);
            }
            vec![0; 2]
        }
        "H" => {
            // c_ushort (2 bytes)
            if let Ok(int_val) = value.try_int(vm)
                && let Some(v) = int_val.as_bigint().to_u16()
            {
                return to_bytes!(v);
            }
            vec![0; 2]
        }
        "i" => {
            // c_int (4 bytes)
            if let Ok(int_val) = value.try_int(vm)
                && let Some(v) = int_val.as_bigint().to_i32()
            {
                return to_bytes!(v);
            }
            vec![0; 4]
        }
        "I" => {
            // c_uint (4 bytes)
            if let Ok(int_val) = value.try_int(vm)
                && let Some(v) = int_val.as_bigint().to_u32()
            {
                return to_bytes!(v);
            }
            vec![0; 4]
        }
        "l" => {
            // c_long (platform dependent)
            if let Ok(int_val) = value.try_to_value::<libc::c_long>(vm) {
                return to_bytes!(int_val);
            }
            const SIZE: usize = std::mem::size_of::<libc::c_long>();
            vec![0; SIZE]
        }
        "L" => {
            // c_ulong (platform dependent)
            if let Ok(int_val) = value.try_to_value::<libc::c_ulong>(vm) {
                return to_bytes!(int_val);
            }
            const SIZE: usize = std::mem::size_of::<libc::c_ulong>();
            vec![0; SIZE]
        }
        "q" => {
            // c_longlong (8 bytes)
            if let Ok(int_val) = value.try_int(vm)
                && let Some(v) = int_val.as_bigint().to_i64()
            {
                return to_bytes!(v);
            }
            vec![0; 8]
        }
        "Q" => {
            // c_ulonglong (8 bytes)
            if let Ok(int_val) = value.try_int(vm)
                && let Some(v) = int_val.as_bigint().to_u64()
            {
                return to_bytes!(v);
            }
            vec![0; 8]
        }
        "f" => {
            // c_float (4 bytes) - int도 허용
            if let Ok(float_val) = value.try_float(vm) {
                return to_bytes!(float_val.to_f64() as f32);
            }
            if let Ok(int_val) = value.try_int(vm)
                && let Some(v) = int_val.as_bigint().to_f64()
            {
                return to_bytes!(v as f32);
            }
            vec![0; 4]
        }
        "d" | "g" => {
            // c_double (8 bytes) - int도 허용
            if let Ok(float_val) = value.try_float(vm) {
                return to_bytes!(float_val.to_f64());
            }
            if let Ok(int_val) = value.try_int(vm)
                && let Some(v) = int_val.as_bigint().to_f64()
            {
                return to_bytes!(v);
            }
            vec![0; 8]
        }
        "?" => {
            // c_bool (1 byte)
            if let Ok(b) = value.clone().try_to_bool(vm) {
                return vec![if b { 1 } else { 0 }];
            }
            vec![0]
        }
        "P" | "z" | "Z" => {
            // Pointer types (platform pointer size)
            vec![0; std::mem::size_of::<usize>()]
        }
        _ => vec![0],
    }
}

impl Constructor for PyCSimple {
    type Args = (OptionalArg,);

    fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        let args: Self::Args = args.bind(vm)?;
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

        // Check if this is a swapped endian type
        let swapped = cls
            .as_object()
            .get_attr("_swappedbytes_", vm)
            .map(|v| v.is_true(vm).unwrap_or(false))
            .unwrap_or(false);

        let buffer = value_to_bytes_endian(&_type_, &value, swapped, vm);
        PyCSimple {
            _type_,
            value: AtomicCell::new(value),
            cdata: PyRwLock::new(CDataObject::from_bytes(buffer, None)),
        }
        .into_ref_with_type(vm, cls)
        .map(Into::into)
    }

    fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
        unimplemented!("use slot_new")
    }
}

#[pyclass(flags(BASETYPE), with(Constructor, AsBuffer))]
impl PyCSimple {
    #[pygetset]
    fn _objects(&self) -> Option<PyObjectRef> {
        self.cdata.read().objects.clone()
    }

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
            .clone()
            .downcast()
            .map_err(|_| vm.new_type_error("cannot set value of instance"))?;
        let content = set_primitive(zelf._type_.as_str(), &value, vm)?;

        // Check if this is a swapped endian type
        let swapped = instance
            .class()
            .as_object()
            .get_attr("_swappedbytes_", vm)
            .map(|v| v.is_true(vm).unwrap_or(false))
            .unwrap_or(false);

        // Update buffer when value changes
        let buffer_bytes = value_to_bytes_endian(&zelf._type_, &content, swapped, vm);
        zelf.cdata.write().buffer = buffer_bytes;
        zelf.value.store(content);
        Ok(())
    }

    #[pyclassmethod]
    fn repeat(cls: PyTypeRef, n: isize, vm: &VirtualMachine) -> PyResult {
        use super::_ctypes::get_size;
        use super::array::create_array_type_with_stg_info;

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
        let total_size = element_size * (n as usize);
        let stg_info = super::util::StgInfo::new_array(
            total_size,
            element_size,
            n as usize,
            cls.clone().into(),
            element_size,
        );
        create_array_type_with_stg_info(stg_info, vm)
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
        let args = FuncArgs::new(vec![value], KwArgs::default());
        PyCSimple::slot_new(cls.clone(), args, vm)
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
        let args = FuncArgs::new(vec![value], KwArgs::default());
        let instance = PyCSimple::slot_new(cls.clone(), args, vm)?;

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
        let args = FuncArgs::new(vec![value], KwArgs::default());
        PyCSimple::slot_new(cls.clone(), args, vm)
    }

    #[pyclassmethod]
    fn in_dll(cls: PyTypeRef, dll: PyObjectRef, name: PyStrRef, vm: &VirtualMachine) -> PyResult {
        use super::_ctypes::get_size;
        use libloading::Symbol;

        // Get the library handle from dll object
        let handle = if let Ok(int_handle) = dll.try_int(vm) {
            // dll is an integer handle
            int_handle
                .as_bigint()
                .to_usize()
                .ok_or_else(|| vm.new_value_error("Invalid library handle".to_owned()))?
        } else {
            // dll is a CDLL/PyDLL/WinDLL object with _handle attribute
            dll.get_attr("_handle", vm)?
                .try_int(vm)?
                .as_bigint()
                .to_usize()
                .ok_or_else(|| vm.new_value_error("Invalid library handle".to_owned()))?
        };

        // Get the library from cache
        let library_cache = crate::stdlib::ctypes::library::libcache().read();
        let library = library_cache
            .get_lib(handle)
            .ok_or_else(|| vm.new_attribute_error("Library not found".to_owned()))?;

        // Get symbol address from library
        let symbol_name = format!("{}\0", name.as_str());
        let inner_lib = library.lib.lock();

        let symbol_address = if let Some(lib) = &*inner_lib {
            unsafe {
                // Try to get the symbol from the library
                let symbol: Symbol<'_, *mut u8> = lib.get(symbol_name.as_bytes()).map_err(|e| {
                    vm.new_attribute_error(format!("{}: symbol '{}' not found", e, name.as_str()))
                })?;
                *symbol as usize
            }
        } else {
            return Err(vm.new_attribute_error("Library is closed".to_owned()));
        };

        // Get _type_ attribute and size
        let type_attr = cls
            .as_object()
            .get_attr("_type_", vm)
            .map_err(|_| vm.new_type_error(format!("'{}' has no _type_ attribute", cls.name())))?;
        let type_str = type_attr.str(vm)?.to_string();
        let size = get_size(&type_str);

        // Read value from symbol address
        let value = if symbol_address != 0 && size > 0 {
            // Safety: Reading from a symbol address provided by dlsym
            unsafe {
                let ptr = symbol_address as *const u8;
                let bytes = std::slice::from_raw_parts(ptr, size);
                bytes_to_pyobject(&cls, bytes, vm)?
            }
        } else {
            vm.ctx.none()
        };

        // Create instance
        let args = FuncArgs::new(vec![value], KwArgs::default());
        let instance = PyCSimple::slot_new(cls.clone(), args, vm)?;

        // Store base reference to keep dll alive
        if let Ok(simple_ref) = instance.clone().downcast::<PyCSimple>() {
            simple_ref.cdata.write().base = Some(dll);
        }

        Ok(instance)
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

static SIMPLE_BUFFER_METHODS: BufferMethods = BufferMethods {
    obj_bytes: |buffer| {
        rustpython_common::lock::PyMappedRwLockReadGuard::map(
            rustpython_common::lock::PyRwLockReadGuard::map(
                buffer.obj_as::<PyCSimple>().cdata.read(),
                |x: &CDataObject| x,
            ),
            |x: &CDataObject| x.buffer.as_slice(),
        )
        .into()
    },
    obj_bytes_mut: |buffer| {
        rustpython_common::lock::PyMappedRwLockWriteGuard::map(
            rustpython_common::lock::PyRwLockWriteGuard::map(
                buffer.obj_as::<PyCSimple>().cdata.write(),
                |x: &mut CDataObject| x,
            ),
            |x: &mut CDataObject| x.buffer.as_mut_slice(),
        )
        .into()
    },
    release: |_| {},
    retain: |_| {},
};

impl AsBuffer for PyCSimple {
    fn as_buffer(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<PyBuffer> {
        let buffer_len = zelf.cdata.read().buffer.len();
        let buf = PyBuffer::new(
            zelf.to_owned().into(),
            BufferDescriptor::simple(buffer_len, false), // readonly=false for ctypes
            &SIMPLE_BUFFER_METHODS,
        );
        Ok(buf)
    }
}

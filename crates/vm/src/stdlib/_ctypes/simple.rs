use super::_ctypes::CArgObject;
use super::array::PyCArray;
use super::base::{
    CArgValue, CDATA_BUFFER_METHODS, CDataBytes, ParamFunc, PyCData, StgInfo, StgInfoFlags,
    bytes_to_pyobject,
};
use super::function::PyCFuncPtr;
use super::pointer::PyCPointer;
use crate::builtins::{PyByteArray, PyBytes, PyInt, PyStr, PyType, PyTypeRef};
use crate::class::StaticType;
use crate::convert::{ToPyObject, TryFromObject};
use crate::function::{ArgIntoFloat, Either, FuncArgs, KwArgs, OptionalArg};
use crate::protocol::{BufferDescriptor, PyBuffer, PyNumberMethods, PySequenceMethods};
use crate::types::{AsBuffer, AsNumber, AsSequence, Constructor, Initializer, Representable};
use crate::{
    AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine, set_attrs,
};
use alloc::borrow::Cow;
use core::fmt::Debug;
use num_traits::ToPrimitive;
use rustpython_host_env::ctypes::{
    SimpleStorageValue, simple_storage_value_to_bytes_endian, simple_type_align,
    simple_type_pep3118_code, simple_type_size, zeroed_bytes,
};

/// Valid type codes for ctypes simple types
pub(super) const SIMPLE_TYPE_CHARS: &str = cfg_select! {
    // spell-checker: disable-next-line
    windows => "cbBhHiIlLdfuzZqQPXOv?g",
    // spell-checker: disable-next-line
    _ => "cbBhHiIlLdfuzZqQPOv?g",
};

/// _ctypes_alloc_format_string_for_type
fn alloc_format_string_for_type(code: char, big_endian: bool) -> String {
    let prefix = if big_endian { ">" } else { "<" };
    let pep_code = simple_type_pep3118_code(code);
    format!("{prefix}{pep_code}")
}

/// Create a new simple type instance from a class
fn new_simple_type(
    cls: Either<&PyObject, &Py<PyType>>,
    vm: &VirtualMachine,
) -> PyResult<PyCSimple> {
    let cls = match cls {
        Either::A(obj) => obj,
        Either::B(typ) => typ.as_object(),
    };

    if let Some(typ) = cls.downcast_ref::<PyType>()
        && let Some(info) = typ.stg_info_opt()
    {
        return Ok(PyCSimple(PyCData::from_bytes(
            zeroed_bytes(info.size),
            None,
        )));
    }

    let _type_ = cls
        .get_attr("_type_", vm)
        .map_err(|_| vm.new_attribute_error("class must define a '_type_' attribute"))?;

    if !_type_.is_instance((&vm.ctx.types.str_type).as_ref(), vm)? {
        return Err(vm.new_type_error("class must define a '_type_' string attribute"));
    }

    let tp_str = _type_.str(vm)?.to_string();

    // Complex ctypes storage and FFI are not implemented. Preserve the
    // optional-type AttributeError contract for the PEP 3118 codes.
    if matches!(tp_str.as_str(), "Zd" | "Zf" | "Zg") {
        return Err(vm.new_attribute_error("complex ctypes types are not supported"));
    }

    if tp_str.len() != 1 {
        return Err(vm.new_value_error(format!(
            "class must define a '_type_' attribute which must be a string of length 1, str: {tp_str}"
        )));
    }

    if !SIMPLE_TYPE_CHARS.contains(tp_str.as_str()) {
        return Err(vm.new_attribute_error(format!(
            "class must define a '_type_' attribute which must be\n a single character string containing one of {SIMPLE_TYPE_CHARS}, currently it is {tp_str}."
        )));
    }

    let size = simple_type_size(&tp_str).expect("invalid ctypes simple type");
    Ok(PyCSimple(PyCData::from_bytes(zeroed_bytes(size), None)))
}

pub(super) fn bigint_to_i128_wrapping(value: &malachite_bigint::BigInt) -> i128 {
    if let Some(value) = value.to_i128() {
        return value;
    }
    let bytes = value.to_signed_bytes_le();
    let fill = bytes
        .last()
        .map_or(0, |byte| if *byte & 0x80 == 0 { 0 } else { u8::MAX });
    let mut wrapped = [fill; 16];
    let len = bytes.len().min(wrapped.len());
    wrapped[..len].copy_from_slice(&bytes[..len]);
    i128::from_le_bytes(wrapped)
}

#[pyclass(module = "_ctypes", name = "PyCSimpleType", base = PyType)]
#[derive(Debug)]
#[repr(transparent)]
pub(crate) struct PyCSimpleType(PyType);

#[pyclass(flags(BASETYPE), with(AsSequence, Initializer))]
impl PyCSimpleType {
    #[pygetset(name = "__pointer_type__")]
    fn pointer_type(zelf: PyTypeRef, vm: &VirtualMachine) -> PyResult {
        super::base::pointer_type_get(&zelf, vm)
    }

    #[pygetset(name = "__pointer_type__", setter)]
    fn set_pointer_type(zelf: PyTypeRef, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        super::base::pointer_type_set(&zelf, value, vm)
    }

    #[allow(clippy::new_ret_no_self)]
    #[pymethod]
    fn new(cls: PyTypeRef, _: OptionalArg, vm: &VirtualMachine) -> PyResult {
        Ok(PyObjectRef::from(
            new_simple_type(Either::B(&cls), vm)?.into_ref_with_type(vm, cls)?,
        ))
    }

    #[pymethod]
    fn from_param(zelf: PyObjectRef, value: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        // zelf is the class (e.g., c_int) that from_param was called on
        let cls = zelf
            .downcast::<PyType>()
            .map_err(|_| vm.new_type_error("from_param: expected a type"))?;

        // 1. If the value is already an instance of the requested type, return it
        if value.is_instance(cls.as_object(), vm)? {
            return Ok(value);
        }

        // 2. Get the type code to determine conversion rules
        let code = cls
            .stg_info(vm)?
            .scalar_code
            .ok_or_else(|| vm.new_type_error("abstract class"))?;
        let type_code = code.to_string();

        // 3. Handle None for pointer types (c_char_p, c_wchar_p, c_void_p)
        if vm.is_none(&value) && matches!(code, 'z' | 'Z' | 'P') {
            return Ok(value);
        }

        // Foreign-call values are always native endian, even for swapped types.
        let create_argument = || -> PyResult {
            let (mut bytes, keep, hidden) = assignment_value(&type_code, &value, false, vm)?;
            bytes.commit();
            let tag = code as u8;
            Ok(CArgObject {
                tag,
                value: CArgValue::typed(code, &bytes),
                obj: keep.unwrap_or_else(|| vm.ctx.none()),
                keep: hidden,
                size: 0,
                offset: 0,
            }
            .to_pyobject(vm))
        };

        // 4. Try to convert value based on type code
        let error = match code {
            // c_char_p: bytes pointer
            'z' => {
                // 1. bytes → create CArgObject with null-terminated buffer
                if let Some(bytes) = value.downcast_ref::<PyBytes>() {
                    let (kept_alive, ptr) = super::base::ensure_z_null_terminated(bytes, vm);
                    return Ok(CArgObject {
                        tag: b'z',
                        value: CArgValue::pointer(ptr),
                        obj: value.clone(),
                        keep: Some(kept_alive),
                        size: 0,
                        offset: 0,
                    }
                    .to_pyobject(vm));
                }
                // 2. Array/Pointer with c_char element type
                if is_cchar_array_or_pointer(&value, vm) {
                    return Ok(value);
                }
                // 3. CArgObject (byref(c_char(...)))
                if let Some(carg) = value.downcast_ref::<CArgObject>()
                    && carg.tag == b'c'
                {
                    return Ok(value.clone());
                }
                vm.new_type_error(format!(
                    "'{}' object cannot be interpreted as ctypes.c_char_p",
                    value.class().name()
                ))
            }
            // c_wchar_p: unicode pointer
            'Z' => {
                // 1. str → create CArgObject with null-terminated wchar buffer
                if let Some(s) = value.downcast_ref::<PyStr>() {
                    let (holder, ptr) = super::base::str_to_wchar_bytes(s.as_wtf8(), vm);
                    return Ok(CArgObject {
                        tag: b'Z',
                        value: CArgValue::pointer(ptr),
                        obj: value.clone(),
                        keep: Some(holder),
                        size: 0,
                        offset: 0,
                    }
                    .to_pyobject(vm));
                }
                // 2. Array/Pointer with c_wchar element type
                if is_cwchar_array_or_pointer(&value, vm)? {
                    return Ok(value);
                }
                // 3. CArgObject (byref(c_wchar(...)))
                if let Some(carg) = value.downcast_ref::<CArgObject>()
                    && carg.tag == b'u'
                {
                    return Ok(value.clone());
                }
                vm.new_type_error(format!(
                    "'{}' object cannot be interpreted as ctypes.c_wchar_p",
                    value.class().name()
                ))
            }
            // c_void_p: most flexible - accepts int, bytes, str, any array/pointer, funcptr
            'P' => {
                // 1. int → create c_void_p with that address
                if value.downcast_ref::<PyInt>().is_some() {
                    return create_argument();
                }
                // 2. bytes → create CArgObject with null-terminated buffer
                if let Some(bytes) = value.downcast_ref::<PyBytes>() {
                    let (kept_alive, ptr) = super::base::ensure_z_null_terminated(bytes, vm);
                    return Ok(CArgObject {
                        tag: b'z',
                        value: CArgValue::pointer(ptr),
                        obj: value.clone(),
                        keep: Some(kept_alive),
                        size: 0,
                        offset: 0,
                    }
                    .to_pyobject(vm));
                }
                // 3. str → create CArgObject with null-terminated wchar buffer
                if let Some(s) = value.downcast_ref::<PyStr>() {
                    let (holder, ptr) = super::base::str_to_wchar_bytes(s.as_wtf8(), vm);
                    return Ok(CArgObject {
                        tag: b'Z',
                        value: CArgValue::pointer(ptr),
                        obj: value.clone(),
                        keep: Some(holder),
                        size: 0,
                        offset: 0,
                    }
                    .to_pyobject(vm));
                }
                // 4. Any Array or Pointer → accept directly
                if value.downcast_ref::<PyCArray>().is_some()
                    || value.downcast_ref::<PyCPointer>().is_some()
                {
                    return Ok(value);
                }
                // 5. CArgObject with 'P' tag (byref(c_void_p(...)))
                if let Some(carg) = value.downcast_ref::<CArgObject>()
                    && carg.tag == b'P'
                {
                    return Ok(value.clone());
                }
                // 6. PyCFuncPtr → extract function pointer address
                if let Some(funcptr) = value.downcast_ref::<PyCFuncPtr>() {
                    let ptr_val = {
                        let buffer = funcptr._base.buffer.read();
                        rustpython_host_env::ctypes::read_pointer_from_buffer(&buffer)
                    };
                    return Ok(CArgObject {
                        tag: b'P',
                        value: CArgValue::pointer(ptr_val),
                        obj: value.clone(),
                        keep: None,
                        size: 0,
                        offset: 0,
                    }
                    .to_pyobject(vm));
                }
                // 7. c_char_p or c_wchar_p instance → extract pointer value
                if let Some(simple) = value.downcast_ref::<PyCSimple>() {
                    let value_type_code = value.class().type_code(vm);
                    if matches!(value_type_code.as_deref(), Some("z" | "Z")) {
                        let ptr_val = {
                            let buffer = simple.0.buffer.read();
                            rustpython_host_env::ctypes::read_pointer_from_buffer(&buffer)
                        };
                        return Ok(CArgObject {
                            tag: b'Z',
                            value: CArgValue::pointer(ptr_val),
                            obj: value.clone(),
                            keep: None,
                            size: 0,
                            offset: 0,
                        }
                        .to_pyobject(vm));
                    }
                }
                vm.new_type_error("wrong type")
            }
            _ => match create_argument() {
                Ok(arg) => return Ok(arg),
                Err(error) => error,
            },
        };

        // A successful _as_parameter_ fallback replaces the conversion error;
        // an absent attribute preserves it, and a failing lookup propagates.
        if let Some(as_parameter) = vm.get_attribute_opt(&value, "_as_parameter_")? {
            return vm.with_recursion(" while processing _as_parameter_", || {
                Self::from_param(cls.as_object().to_owned(), as_parameter, vm)
            });
        }
        Err(error)
    }
}

impl AsSequence for PyCSimpleType {
    fn as_sequence() -> &'static PySequenceMethods {
        &super::array::CTYPE_SEQUENCE_METHODS
    }
}

impl Initializer for PyCSimpleType {
    type Args = FuncArgs;

    fn init(zelf: &Py<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
        // type_init requires exactly 3 positional arguments: name, bases, dict
        if args.args.len() != 3 {
            return Err(vm.new_type_error(format!(
                "type.__init__() takes 3 positional arguments but {} were given",
                args.args.len()
            )));
        }

        // Get the type from the metatype instance
        let type_ref: PyTypeRef = zelf
            .as_object()
            .to_owned()
            .downcast()
            .map_err(|_| vm.new_type_error("expected type"))?;

        type_ref.check_not_initialized(vm)?;

        // Get _type_ attribute
        let type_attr = match type_ref.as_object().get_attr("_type_", vm) {
            Ok(attr) => attr,
            Err(_) => {
                return Err(vm.new_attribute_error("class must define a '_type_' attribute"));
            }
        };

        // Validate _type_ is a string
        let type_str = type_attr.str(vm)?.to_string();

        // Validate _type_ is a single character
        // Complex ctypes storage and FFI are not implemented. Preserve the
        // optional-type AttributeError contract for the PEP 3118 codes.
        if matches!(type_str.as_str(), "Zd" | "Zf" | "Zg") {
            return Err(vm.new_attribute_error("complex ctypes types are not supported"));
        }

        if type_str.len() != 1 {
            return Err(vm.new_value_error(
                "class must define a '_type_' attribute which must be a string of length 1",
            ));
        }

        // Validate _type_ is a valid type character
        if !SIMPLE_TYPE_CHARS.contains(type_str.as_str()) {
            return Err(vm.new_attribute_error(format!(
                "class must define a '_type_' attribute which must be a single character string containing one of '{SIMPLE_TYPE_CHARS}', currently it is '{type_str}'."
            )));
        }

        // Initialize StgInfo
        let size = simple_type_size(&type_str).expect("invalid ctypes simple type");
        let align = simple_type_align(&type_str).expect("invalid ctypes simple type");
        let mut stg_info = StgInfo::new(size, align);
        stg_info.scalar_code = type_str.chars().next();

        // Set format for PEP 3118 buffer protocol
        stg_info.format = Some(alloc_format_string_for_type(
            type_str.chars().next().unwrap_or('?'),
            cfg!(target_endian = "big"),
        ));
        stg_info.paramfunc = super::base::ParamFunc::Simple;

        // Set TYPEFLAG_ISPOINTER for pointer types: z (c_char_p), Z (c_wchar_p),
        // P (c_void_p), s (char array), X (BSTR), O (py_object)
        if matches!(type_str.as_str(), "z" | "Z" | "P" | "s" | "X" | "O") {
            stg_info.flags |= StgInfoFlags::TYPEFLAG_ISPOINTER;
        }

        super::base::set_or_init_stginfo(&type_ref, stg_info);

        // Create __ctype_le__ and __ctype_be__ swapped types
        if type_ref.class().is(Self::static_type()) {
            create_swapped_types(&type_ref, &type_str, args, vm)?;
        }

        Ok(())
    }
}

/// Create __ctype_le__ and __ctype_be__ swapped byte order types
/// On little-endian systems: __ctype_le__ = self, __ctype_be__ = swapped type
/// On big-endian systems: __ctype_be__ = self, __ctype_le__ = swapped type
///
/// - Single-byte types (c, b, B): __ctype_le__ = __ctype_be__ = self
/// - Pointer/unsupported types (z, Z, P, u, O): NO __ctype_le__/__ctype_be__ attributes
/// - Multi-byte numeric types (h, H, i, I, l, L, q, Q, f, d, g, ?): create swapped types
fn create_swapped_types(
    type_ref: &Py<PyType>,
    type_str: &str,
    mut args: FuncArgs,
    vm: &VirtualMachine,
) -> PyResult<()> {
    // Types that don't support byte order swapping - no __ctype_le__/__ctype_be__
    // c_void_p (P), c_char_p (z), c_wchar_p (Z), c_wchar (u), py_object (O)
    let unsupported_types = ["P", "z", "Z", "X", "u", "O"];
    if unsupported_types.contains(&type_str) {
        return Ok(());
    }

    // Single-byte types - __ctype_le__ = __ctype_be__ = self (no swapping needed)
    // c_char (c), c_byte (b), c_ubyte (B)
    let single_byte_types = ["c", "b", "B"];
    if single_byte_types.contains(&type_str) {
        set_attrs!(
            type_ref.as_object(), vm,
            "__ctype_le__" => type_ref.as_object().to_owned(),
            "__ctype_be__" => type_ref.as_object().to_owned(),
        );
        return Ok(());
    }

    // Multi-byte types - create swapped type
    // Check system byte order at compile time
    let is_little_endian = cfg!(target_endian = "little");

    // Create only the non-native endian type
    let suffix = if is_little_endian { "_be" } else { "_le" };
    args.args[0] = vm
        .ctx
        .new_str(format!("{}{}", type_ref.name(), suffix))
        .into();
    // Endian variants share bases and class attributes; neither subclasses
    // the other. Bypass the initializer to avoid generating another pair.
    let swapped_type = PyType::slot_new(type_ref.class().to_owned(), args, vm)?;

    // Set _swappedbytes_ on the swapped type to indicate byte swapping is needed
    set_attrs!(
        swapped_type, vm,
        "_swappedbytes_" => vm.ctx.none(),
    );

    // Update swapped type's StgInfo format to use opposite endian prefix
    if let Ok(swapped_type_ref) = swapped_type.clone().downcast::<PyType>() {
        let mut sw_stg = type_ref.stg_info(vm)?.clone();
        // Swapped: little-endian system uses big-endian prefix and vice versa
        sw_stg.big_endian = is_little_endian;
        sw_stg.format = Some(alloc_format_string_for_type(
            type_str.chars().next().unwrap_or('?'),
            is_little_endian,
        ));
        super::base::set_or_init_stginfo(&swapped_type_ref, sw_stg);
    }

    // Set attributes based on system byte order
    // Native endian attribute points to self, non-native points to swapped type
    let type_obj = type_ref.as_object().to_owned();
    let swapped_obj = swapped_type.clone();
    let (ctype_le, ctype_be) = if is_little_endian {
        // Little-endian system: __ctype_le__ = self, __ctype_be__ = swapped
        (type_obj, swapped_obj)
    } else {
        // Big-endian system: __ctype_le__ = swapped, __ctype_be__ = self
        (swapped_obj, type_obj)
    };

    set_attrs!(
        type_ref.as_object(), vm,
        "__ctype_le__" => ctype_le.clone(),
        "__ctype_be__" => ctype_be.clone(),
    );

    set_attrs!(
       swapped_type, vm,
       "__ctype_le__" => ctype_le,
       "__ctype_be__" => ctype_be,
    );

    Ok(())
}

#[pyclass(
    module = "_ctypes",
    name = "_SimpleCData",
    base = PyCData,
    metaclass = "PyCSimpleType",
    traverse = "manual"
)]
#[repr(transparent)]
pub(crate) struct PyCSimple(pub PyCData);

// SAFETY: this transparent wrapper owns exactly the references in its base.
unsafe impl crate::object::Traverse for PyCSimple {
    fn traverse(&self, tracer: &mut crate::object::TraverseFn<'_>) {
        self.0.traverse(tracer);
    }
    fn clear_refs(&mut self, out: &mut Vec<PyObjectRef>) {
        self.0.clear_refs(out);
    }
}

impl Debug for PyCSimple {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PyCSimple")
            .field("size", &self.0.buffer.read().len())
            .finish()
    }
}

pub(super) fn is_swapped(cls: &Py<PyType>) -> bool {
    cls.stg_info_opt().is_some_and(|info| {
        info.paramfunc == ParamFunc::Simple && info.big_endian != cfg!(target_endian = "big")
    })
}

pub(super) fn returns_scalar(cls: &Py<PyType>) -> bool {
    cls.base
        .deref()
        .is_some_and(|base| base.is(PyCSimple::static_type()))
}

pub(super) fn assignment_value(
    type_code: &str,
    value: &PyObject,
    swapped: bool,
    vm: &VirtualMachine,
) -> PyResult<(CDataBytes, Option<PyObjectRef>, Option<PyObjectRef>)> {
    let mut keep = None;
    let mut hidden = None;
    // Convert once before writing: numeric conversion may reenter ctypes.
    let storage = match type_code {
        "c" => {
            let byte = if let Some(bytes) = value.downcast_ref::<PyBytes>() {
                match bytes.as_bytes() {
                    [byte] => *byte,
                    _ => return Err(vm.new_type_error("one character bytes expected")),
                }
            } else if let Some(bytes) = value.downcast_ref::<PyByteArray>() {
                match &*bytes.borrow_buf() {
                    [byte] => *byte,
                    _ => return Err(vm.new_type_error("one character bytearray expected")),
                }
            } else {
                value
                    .downcast_ref::<PyInt>()
                    .and_then(|integer| integer.as_bigint().to_u8())
                    .ok_or_else(|| {
                        vm.new_type_error("one character bytes, bytearray or integer expected")
                    })?
            };
            SimpleStorageValue::Byte(byte)
        }
        "u" => {
            let text = value
                .downcast_ref::<PyStr>()
                .ok_or_else(|| vm.new_type_error("unicode string expected"))?;
            let mut chars = text.as_wtf8().code_points();
            let character = chars
                .next()
                .filter(|_| chars.next().is_none())
                .ok_or_else(|| vm.new_type_error("one character unicode string expected"))?;
            if rustpython_host_env::ctypes::WCHAR_SIZE == 2 && character.to_u32() > 0xffff {
                return Err(vm.new_type_error("a single UTF-16 code unit is required"));
            }
            SimpleStorageValue::Wchar(character.to_u32())
        }
        "f" | "d" | "g" => SimpleStorageValue::Float(
            ArgIntoFloat::try_from_object(vm, value.to_owned())?.into_float(),
        ),
        "?" | "v" => SimpleStorageValue::Bool(value.to_owned().try_to_bool(vm)?),
        "O" => {
            keep = (!vm.is_none(value)).then(|| value.to_owned());
            SimpleStorageValue::ObjectId(value.get_id())
        }
        #[cfg(windows)]
        "X" => {
            let units = if vm.is_none(value) {
                None
            } else {
                let text = value.downcast_ref::<PyStr>().ok_or_else(|| {
                    vm.new_type_error(format!(
                        "unicode string expected instead of {} instance",
                        value.class().name()
                    ))
                })?;
                let units: Vec<u16> = text.as_wtf8().encode_wide().collect();
                if u32::try_from(units.len()).is_err() {
                    return Err(vm.new_value_error("String too long for BSTR"));
                }
                Some(units)
            };
            let bstr = rustpython_host_env::ctypes::BStr::new(units.as_deref())
                .ok_or_else(|| vm.new_memory_error("could not allocate BSTR"))?;
            return Ok((CDataBytes::BStr(bstr), None, None));
        }
        "z" | "Z" | "P" => {
            let address = if vm.is_none(value) {
                0
            } else if type_code == "z"
                && let Some(bytes) = value.downcast_ref::<PyBytes>()
            {
                let (holder, address) = super::base::ensure_z_null_terminated(bytes, vm);
                keep = Some(value.to_owned());
                hidden = Some(holder);
                address
            } else if type_code == "Z"
                && let Some(text) = value.downcast_ref::<PyStr>()
            {
                let (holder, address) = super::base::str_to_wchar_bytes(text.as_wtf8(), vm);
                keep = Some(holder);
                address
            } else if let Some(integer) = value.downcast_ref::<PyInt>() {
                bigint_to_i128_wrapping(integer.as_bigint()) as usize
            } else {
                return Err(vm.new_type_error("integer address expected"));
            };
            SimpleStorageValue::Pointer(address)
        }
        _ => {
            let index;
            let integer = if let Some(integer) = value.downcast_ref_if_exact::<PyInt>(vm) {
                integer
            } else {
                index = value.try_index(vm)?;
                &index
            };
            SimpleStorageValue::Signed(bigint_to_i128_wrapping(integer.as_bigint()))
        }
    };
    let bytes = match (type_code, storage) {
        (
            "b" | "B" | "h" | "H" | "i" | "I" | "l" | "L" | "q" | "Q",
            SimpleStorageValue::Signed(value),
        ) => {
            let size = simple_type_size(type_code).expect("integer ctypes size");
            let mut bytes = (value as u64).to_le_bytes();
            if super::base::is_big_endian(swapped) {
                bytes[..size].reverse();
            }
            CDataBytes::Integer { bytes, size }
        }
        (_, storage) => simple_storage_value_to_bytes_endian(type_code, storage, swapped).into(),
    };
    Ok((bytes, keep, hidden))
}

/// Check if value is a c_char array or pointer(c_char)
fn is_cchar_array_or_pointer(value: &PyObject, vm: &VirtualMachine) -> bool {
    // Check Array with c_char element type
    if let Some(arr) = value.downcast_ref::<PyCArray>()
        && let Some(info) = arr.class().stg_info_opt()
        && let Some(ref elem_type) = info.element_type
        && let Some(elem_code) = elem_type.type_code(vm)
    {
        return elem_code == "c";
    }
    // Check Pointer to c_char
    if let Some(ptr) = value.downcast_ref::<PyCPointer>()
        && let Some(info) = ptr.class().stg_info_opt()
        && let Some(ref proto) = info.proto
        && let Some(proto_code) = proto.type_code(vm)
    {
        return proto_code == "c";
    }
    false
}

/// Check if value is a c_wchar array or pointer(c_wchar)
fn is_cwchar_array_or_pointer(value: &PyObject, vm: &VirtualMachine) -> PyResult<bool> {
    // Check Array with c_wchar element type
    if let Some(arr) = value.downcast_ref::<PyCArray>() {
        let info = arr.class().stg_info(vm)?;
        let elem_type = info.element_type.as_ref().expect("array has element_type");
        if let Some(elem_code) = elem_type.type_code(vm) {
            return Ok(elem_code == "u");
        }
    }
    // Check Pointer to c_wchar
    if let Some(ptr) = value.downcast_ref::<PyCPointer>() {
        let info = ptr.class().stg_info(vm)?;
        if let Some(ref proto) = info.proto
            && let Some(proto_code) = proto.type_code(vm)
        {
            return Ok(proto_code == "u");
        }
    }
    Ok(false)
}

impl Constructor for PyCSimple {
    type Args = FuncArgs;

    fn slot_new(cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        // Allocation ignores arguments; only __init__ converts the value.
        // Do not let new_simple_type's _type_ fallback allocate abstract types.
        cls.stg_info(vm)?;
        new_simple_type(Either::B(&cls), vm)?
            .into_ref_with_type(vm, cls)
            .map(Into::into)
    }

    fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
        unimplemented!("use slot_new")
    }
}

impl Initializer for PyCSimple {
    // Simple_init accepts at most one positional value and ignores keywords.
    type Args = (OptionalArg, KwArgs);

    fn init(zelf: &Py<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
        // If an argument is provided, update the value
        if let Some(value) = args.0.into_option() {
            Py::<Self>::set_value(zelf.to_owned().into(), value, vm)?;
        }
        Ok(())
    }
}

// Simple_repr
impl Representable for PyCSimple {
    fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
        let cls = zelf.class();
        let type_name = cls.name();

        // Check if base is _SimpleCData (direct simple type like c_int, c_char)
        // vs subclass of simple type (like class X(c_int): pass)
        let is_direct_simple = returns_scalar(cls);

        if is_direct_simple {
            // Direct SimpleCData: "typename(repr(value))"
            let value = Py::<Self>::value(zelf.to_owned().into(), vm)?;
            let value_repr = value.repr(vm)?.to_string();
            Ok(format!("{type_name}({value_repr})"))
        } else {
            // Subclass: "<typename object at addr>"
            let addr = zelf.get_id();
            Ok(format!("<{type_name} object at {addr:#x}>"))
        }
    }
}

#[pyclass(
    flags(BASETYPE),
    with(Constructor, Initializer, AsBuffer, AsNumber, Representable)
)]
impl Py<PyCSimple> {
    #[pygetset]
    fn _b0_(&self) -> Option<PyObjectRef> {
        self.0.base.read().clone()
    }

    #[pygetset]
    pub(crate) fn value(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let zelf: &Self = instance
            .downcast_ref()
            .ok_or_else(|| vm.new_type_error("cannot get value of instance"))?;

        let cls = zelf.class();
        let type_code = cls
            .type_code(vm)
            .ok_or_else(|| vm.new_type_error("abstract class"))?;

        // Special handling for c_char_p (z) and c_wchar_p (Z)
        // z_get, Z_get - dereference pointer to get string
        if type_code == "z" {
            let buffer = zelf.0.buffer.read();
            return match rustpython_host_env::ctypes::decode_type_code(&type_code, &buffer) {
                rustpython_host_env::ctypes::DecodedValue::Bytes(value) => {
                    Ok(vm.ctx.new_bytes(value).into())
                }
                rustpython_host_env::ctypes::DecodedValue::None => Ok(vm.ctx.none()),
                _ => unreachable!("decode_type_code('z') only returns bytes or None"),
            };
        }
        if type_code == "Z" {
            let buffer = zelf.0.buffer.read();
            return match rustpython_host_env::ctypes::decode_type_code(&type_code, &buffer) {
                rustpython_host_env::ctypes::DecodedValue::String(value) => {
                    Ok(vm.ctx.new_str(value).into())
                }
                rustpython_host_env::ctypes::DecodedValue::None => Ok(vm.ctx.none()),
                _ => unreachable!("decode_type_code('Z') only returns string or None"),
            };
        }

        // O_get: py_object - read PyObject pointer from buffer
        if type_code == "O" {
            let buffer = zelf.0.buffer.read();
            let ptr = rustpython_host_env::ctypes::read_pointer_from_buffer(&buffer);
            let slot = buffer.as_ptr() as usize;
            drop(buffer);
            return zelf.0.object_at(ptr, slot, None, vm);
        }

        // Storage metadata belongs to this exact type, unlike inherited attributes.
        let swapped = is_swapped(cls);

        // Read value from buffer, swap bytes if needed
        let buffer = zelf.0.buffer.read();
        let buffer_data: alloc::borrow::Cow<'_, [u8]> = if swapped {
            // Reverse bytes for swapped endian types
            let mut swapped_bytes = buffer.to_vec();
            swapped_bytes.reverse();
            alloc::borrow::Cow::Owned(swapped_bytes)
        } else {
            alloc::borrow::Cow::Borrowed(&*buffer)
        };

        let cls_ref = cls.to_owned();
        Ok(bytes_to_pyobject(&cls_ref, &buffer_data, vm))
    }

    #[pygetset(setter)]
    fn set_value(instance: PyObjectRef, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let zelf: PyRef<PyCSimple> = instance
            .downcast()
            .map_err(|_| vm.new_type_error("cannot set value of instance"))?;

        let cls = zelf.class();
        let code = cls
            .stg_info(vm)?
            .scalar_code
            .ok_or_else(|| vm.new_type_error("abstract class"))?;
        let type_code = code.to_string();

        // Snapshot storage metadata before invoking any conversion callbacks.
        let swapped = is_swapped(cls);
        let (mut buffer_bytes, keep, hidden) = assignment_value(&type_code, &value, swapped, vm)?;

        let write = if type_code == "O" {
            let size = zelf.0.buffer.read().len();
            zelf.0.register_pyobject_view(0, 0, size, vm)?
        } else {
            super::base::CDataWrite::default()
        };

        write.publish(vm)?;

        // Write in place for borrowed storage and preserve any resized tail.
        zelf.0.write_value_at_offset(0, &mut buffer_bytes);

        // Raw z/Z addresses and None do not replace the old owner: the
        // address may still point into its buffer (CPython KeepRef(None)).
        if let Some(keep) = keep {
            zelf.0.keep_value(keep, hidden, vm)?;
        }
        Ok(())
    }

    #[pyclassmethod]
    fn repeat(cls: PyTypeRef, n: isize, vm: &VirtualMachine) -> PyResult {
        super::array::repeat_type(cls.as_object(), n, vm)
    }

    // Simple_from_outparm - convert output parameter back to Python value
    // For direct subclasses of _SimpleCData (e.g., c_int), returns the value.
    // For subclasses of those (e.g., class MyInt(c_int)), returns self.
    #[pymethod]
    fn __ctypes_from_outparam__(
        zelf: PyRef<PyCSimple>,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        // _ctypes_simple_instance: returns true if NOT a direct subclass of Simple_Type
        // i.e., c_int (direct) -> false, MyInt(c_int) (subclass) -> true
        let is_subclass_of_simple = !returns_scalar(zelf.class());

        if is_subclass_of_simple {
            // Subclass of simple type (e.g., MyInt(c_int)): return self
            Ok(zelf.into())
        } else {
            // Direct simple type (e.g., c_int): return value
            Self::value(zelf.into(), vm)
        }
    }
}

impl PyCSimple {
    /// Snapshot this object's buffer as a simple-typed foreign-call value.
    pub(crate) fn to_carg_value(&self, code: char) -> CArgValue {
        let buffer = self.0.buffer.read();
        CArgValue::typed(code, &buffer)
    }
}

impl AsBuffer for PyCSimple {
    fn as_buffer(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<PyBuffer> {
        let stg_info = zelf
            .class()
            .stg_info_opt()
            .expect("PyCSimple type must have StgInfo");
        let format = stg_info
            .format
            .clone()
            .map_or(Cow::Borrowed("B"), Cow::Owned);
        let itemsize = stg_info.size;
        // Simple types are scalars with ndim=0, shape=()
        let desc = BufferDescriptor::from_dim_desc(itemsize, 0, false, itemsize, format, vec![]);
        let buf = PyBuffer::new(zelf.to_owned().into(), desc, &CDATA_BUFFER_METHODS);
        Ok(buf)
    }
}

/// Simple_bool: return non-zero if any byte in buffer is non-zero
impl AsNumber for PyCSimple {
    fn as_number() -> &'static PyNumberMethods {
        static AS_NUMBER: PyNumberMethods = PyNumberMethods {
            boolean: Some(|obj, _vm| {
                let zelf = obj
                    .downcast_ref::<PyCSimple>()
                    .expect("PyCSimple::as_number called on non-PyCSimple");
                let buffer = zelf.0.buffer.read();
                // Simple_bool: memcmp(self->b_ptr, zeros, self->b_size)
                // Returns true if any byte is non-zero
                Ok(buffer.iter().any(|&b| b != 0))
            }),
            ..PyNumberMethods::NOT_IMPLEMENTED
        };
        &AS_NUMBER
    }
}

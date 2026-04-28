use super::_ctypes::CArgObject;
use super::array::PyCArray;
use super::base::{
    CDATA_BUFFER_METHODS, FfiArgValue, PyCData, StgInfo, StgInfoFlags, buffer_to_ffi_value,
    bytes_to_pyobject,
};
use super::function::PyCFuncPtr;
use super::pointer::PyCPointer;
use crate::builtins::{PyByteArray, PyBytes, PyInt, PyNone, PyStr, PyType, PyTypeRef};
use crate::convert::ToPyObject;
use crate::function::{Either, FuncArgs, OptionalArg};
use crate::protocol::{BufferDescriptor, PyBuffer, PyNumberMethods};
use crate::types::{AsBuffer, AsNumber, Constructor, Initializer, Representable};
use crate::{AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine};
use alloc::borrow::Cow;
use core::fmt::Debug;
use num_traits::ToPrimitive;
use rustpython_host_env::ctypes::{
    SimpleStorageValue, simple_storage_value_to_bytes_endian, simple_type_align,
    simple_type_pep3118_code, simple_type_size, write_simple_storage_buffer, zeroed_bytes,
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
    format!("{}{}", prefix, pep_code)
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

    let _type_ = cls
        .get_attr("_type_", vm)
        .map_err(|_| vm.new_attribute_error("class must define a '_type_' attribute"))?;

    if !_type_.is_instance((&vm.ctx.types.str_type).as_ref(), vm)? {
        return Err(vm.new_type_error("class must define a '_type_' string attribute"));
    }

    let tp_str = _type_.str(vm)?.to_string();

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

fn set_primitive(_type_: &str, value: &PyObject, vm: &VirtualMachine) -> PyResult {
    match _type_ {
        "c" => {
            // c_set: accepts bytes(len=1), bytearray(len=1), or int(0-255)
            if value
                .downcast_ref_if_exact::<PyBytes>(vm)
                .is_some_and(|v| v.len() == 1)
                || value
                    .downcast_ref_if_exact::<PyByteArray>(vm)
                    .is_some_and(|v| v.borrow_buf().len() == 1)
                || value.downcast_ref_if_exact::<PyInt>(vm).is_some_and(|v| {
                    v.as_bigint()
                        .to_i64()
                        .is_some_and(|n| (0..=255).contains(&n))
                })
            {
                Ok(value.to_owned())
            } else {
                Err(vm.new_type_error("one character bytes, bytearray or integer expected"))
            }
        }
        "u" => {
            if let Some(s) = value.downcast_ref::<PyStr>() {
                if s.as_wtf8().code_points().count() == 1 {
                    Ok(value.to_owned())
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
            // Support __index__ protocol
            if value.try_index(vm).is_ok() {
                Ok(value.to_owned())
            } else {
                Err(vm.new_type_error(format!(
                    "an integer is required (got type {})",
                    value.class().name()
                )))
            }
        }
        "f" | "d" | "g" => {
            // Handle int specially to check overflow
            if let Some(int_obj) = value.downcast_ref_if_exact::<PyInt>(vm) {
                // Check if int can fit in f64
                if let Some(f) = int_obj.as_bigint().to_f64()
                    && f.is_finite()
                {
                    return Ok(value.to_owned());
                }
                return Err(vm.new_overflow_error("int too large to convert to float"));
            }
            // __float__ protocol
            if value.try_float(vm).is_ok() {
                Ok(value.to_owned())
            } else {
                Err(vm.new_type_error(format!("must be real number, not {}", value.class().name())))
            }
        }
        "?" => Ok(PyObjectRef::from(
            vm.ctx.new_bool(value.to_owned().try_to_bool(vm)?),
        )),
        "v" => {
            // VARIANT_BOOL: any truthy → True
            Ok(PyObjectRef::from(
                vm.ctx.new_bool(value.to_owned().try_to_bool(vm)?),
            ))
        }
        "B" => {
            // Support __index__ protocol
            if value.try_index(vm).is_ok() {
                // Store as-is, conversion to unsigned happens in the getter
                Ok(value.to_owned())
            } else {
                Err(vm.new_type_error(format!("int expected instead of {}", value.class().name())))
            }
        }
        "z" => {
            if value.is(&vm.ctx.none)
                || value.downcast_ref_if_exact::<PyInt>(vm).is_some()
                || value.downcast_ref_if_exact::<PyBytes>(vm).is_some()
            {
                Ok(value.to_owned())
            } else {
                Err(vm.new_type_error(format!(
                    "bytes or integer address expected instead of {} instance",
                    value.class().name()
                )))
            }
        }
        "Z" => {
            if value.is(&vm.ctx.none)
                || value.downcast_ref_if_exact::<PyInt>(vm).is_some()
                || value.downcast_ref_if_exact::<PyStr>(vm).is_some()
            {
                Ok(value.to_owned())
            } else {
                Err(vm.new_type_error(format!(
                    "unicode string or integer address expected instead of {} instance",
                    value.class().name()
                )))
            }
        }
        // O_set: py_object accepts any Python object
        "O" => Ok(value.to_owned()),
        // X_set: BSTR - same as Z (c_wchar_p), accepts None, int, or str
        "X" => {
            if value.is(&vm.ctx.none)
                || value.downcast_ref_if_exact::<PyInt>(vm).is_some()
                || value.downcast_ref_if_exact::<PyStr>(vm).is_some()
            {
                Ok(value.to_owned())
            } else {
                Err(vm.new_type_error(format!(
                    "unicode string or integer address expected instead of {} instance",
                    value.class().name()
                )))
            }
        }
        _ => {
            // "P"
            if value.downcast_ref_if_exact::<PyInt>(vm).is_some()
                || value.downcast_ref_if_exact::<PyNone>(vm).is_some()
            {
                Ok(value.to_owned())
            } else {
                Err(vm.new_type_error("cannot be converted to pointer"))
            }
        }
    }
}

#[pyclass(module = "_ctypes", name = "PyCSimpleType", base = PyType)]
#[derive(Debug)]
#[repr(transparent)]
pub(crate) struct PyCSimpleType(PyType);

#[pyclass(flags(BASETYPE), with(AsNumber, Initializer))]
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
        let type_code = cls.type_code(vm);

        // 3. Handle None for pointer types (c_char_p, c_wchar_p, c_void_p)
        if vm.is_none(&value) && matches!(type_code.as_deref(), Some("z" | "Z" | "P")) {
            return Ok(value);
        }

        // Helper to create CArgObject wrapping a simple instance
        let create_simple_with_value = |type_str: &str, val: &PyObject| -> PyResult {
            let simple = new_simple_type(Either::B(&cls), vm)?;
            let buffer_bytes = value_to_bytes_endian(type_str, val, false, vm);
            *simple.0.buffer.write() = alloc::borrow::Cow::Owned(buffer_bytes.clone());
            let simple_obj: PyObjectRef = simple.into_ref_with_type(vm, cls.clone())?.into();
            // from_param returns CArgObject, not the simple type itself
            let tag = type_str.as_bytes().first().copied().unwrap_or(b'?');
            let ffi_value = buffer_to_ffi_value(type_str, &buffer_bytes);
            Ok(CArgObject {
                tag,
                value: ffi_value,
                obj: simple_obj,
                size: 0,
                offset: 0,
            }
            .to_pyobject(vm))
        };

        // 4. Try to convert value based on type code
        match type_code.as_deref() {
            // Integer types: accept integers
            Some(tc @ ("b" | "B" | "h" | "H" | "i" | "I" | "l" | "L" | "q" | "Q"))
                if value.try_int(vm).is_ok() =>
            {
                return create_simple_with_value(tc, &value);
            }

            // Float types: accept numbers
            Some(tc @ ("f" | "d" | "g"))
                if value.try_float(vm).is_ok() || value.try_int(vm).is_ok() =>
            {
                return create_simple_with_value(tc, &value);
            }
            // c_char: 1 byte character
            Some("c") => {
                if let Some(bytes) = value.downcast_ref::<PyBytes>()
                    && bytes.len() == 1
                {
                    return create_simple_with_value("c", &value);
                }
                if let Ok(int_val) = value.try_int(vm)
                    && int_val.as_bigint().to_u8().is_some()
                {
                    return create_simple_with_value("c", &value);
                }
                return Err(vm.new_type_error("one character bytes, bytearray or integer expected"));
            }
            // c_wchar: 1 unicode character
            Some("u") => {
                if let Some(s) = value.downcast_ref::<PyStr>()
                    && s.as_wtf8().code_points().count() == 1
                {
                    return create_simple_with_value("u", &value);
                }
                return Err(vm.new_type_error("one character unicode string expected"));
            }
            // c_char_p: bytes pointer
            Some("z") => {
                // 1. bytes → create CArgObject with null-terminated buffer
                if let Some(bytes) = value.downcast_ref::<PyBytes>() {
                    let (kept_alive, ptr) = super::base::ensure_z_null_terminated(bytes, vm);
                    return Ok(CArgObject {
                        tag: b'z',
                        value: FfiArgValue::OwnedPointer(ptr, kept_alive),
                        obj: value.clone(),
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
            }
            // c_wchar_p: unicode pointer
            Some("Z") => {
                // 1. str → create CArgObject with null-terminated wchar buffer
                if let Some(s) = value.downcast_ref::<PyStr>() {
                    let (holder, ptr) = super::base::str_to_wchar_bytes(s.as_wtf8(), vm);
                    return Ok(CArgObject {
                        tag: b'Z',
                        value: FfiArgValue::OwnedPointer(ptr, holder),
                        obj: value.clone(),
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
            }
            // c_void_p: most flexible - accepts int, bytes, str, any array/pointer, funcptr
            Some("P") => {
                // 1. int → create c_void_p with that address
                if value.try_int(vm).is_ok() {
                    return create_simple_with_value("P", &value);
                }
                // 2. bytes → create CArgObject with null-terminated buffer
                if let Some(bytes) = value.downcast_ref::<PyBytes>() {
                    let (kept_alive, ptr) = super::base::ensure_z_null_terminated(bytes, vm);
                    return Ok(CArgObject {
                        tag: b'z',
                        value: FfiArgValue::OwnedPointer(ptr, kept_alive),
                        obj: value.clone(),
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
                        value: FfiArgValue::OwnedPointer(ptr, holder),
                        obj: value.clone(),
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
                        value: FfiArgValue::pointer(ptr_val),
                        obj: value.clone(),
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
                            value: FfiArgValue::pointer(ptr_val),
                            obj: value.clone(),
                            size: 0,
                            offset: 0,
                        }
                        .to_pyobject(vm));
                    }
                }
            }
            // c_bool
            Some("?") => {
                let bool_val = value.is_true(vm)?;
                let bool_obj: PyObjectRef = vm.ctx.new_bool(bool_val).into();
                return create_simple_with_value("?", &bool_obj);
            }
            _ => {}
        }

        // 5. Check for _as_parameter_ attribute
        if let Ok(as_parameter) = value.get_attr("_as_parameter_", vm) {
            return PyCSimpleType::from_param(cls.as_object().to_owned(), as_parameter, vm);
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
            _ => Err(vm.new_type_error("wrong type")),
        }
    }

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
                    .ok_or_else(|| vm.new_type_error("expected type"))?;
                let n = b
                    .try_index(vm)?
                    .as_bigint()
                    .to_isize()
                    .ok_or_else(|| vm.new_overflow_error("array size too large"))?;
                PyCSimple::repeat(cls.to_owned(), n, vm)
            }),
            ..PyNumberMethods::NOT_IMPLEMENTED
        };
        &AS_NUMBER
    }
}

impl Initializer for PyCSimpleType {
    type Args = FuncArgs;

    fn init(zelf: PyRef<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
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
        if type_str.len() != 1 {
            return Err(vm.new_value_error(
                "class must define a '_type_' attribute which must be a string of length 1",
            ));
        }

        // Validate _type_ is a valid type character
        if !SIMPLE_TYPE_CHARS.contains(type_str.as_str()) {
            return Err(vm.new_attribute_error(format!(
                "class must define a '_type_' attribute which must be a single character string containing one of '{}', currently it is '{}'.",
                SIMPLE_TYPE_CHARS, type_str
            )));
        }

        // Initialize StgInfo
        let size = simple_type_size(&type_str).expect("invalid ctypes simple type");
        let align = simple_type_align(&type_str).expect("invalid ctypes simple type");
        let mut stg_info = StgInfo::new(size, align);

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
        create_swapped_types(&type_ref, &type_str, vm)?;

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
    vm: &VirtualMachine,
) -> PyResult<()> {
    use crate::builtins::PyDict;

    // Avoid infinite recursion - if __ctype_le__ already exists, skip
    if type_ref.as_object().get_attr("__ctype_le__", vm).is_ok() {
        return Ok(());
    }

    // Types that don't support byte order swapping - no __ctype_le__/__ctype_be__
    // c_void_p (P), c_char_p (z), c_wchar_p (Z), c_wchar (u), py_object (O)
    let unsupported_types = ["P", "z", "Z", "u", "O"];
    if unsupported_types.contains(&type_str) {
        return Ok(());
    }

    // Single-byte types - __ctype_le__ = __ctype_be__ = self (no swapping needed)
    // c_char (c), c_byte (b), c_ubyte (B)
    let single_byte_types = ["c", "b", "B"];
    if single_byte_types.contains(&type_str) {
        type_ref
            .as_object()
            .set_attr("__ctype_le__", type_ref.as_object().to_owned(), vm)?;
        type_ref
            .as_object()
            .set_attr("__ctype_be__", type_ref.as_object().to_owned(), vm)?;
        return Ok(());
    }

    // Multi-byte types - create swapped type
    // Check system byte order at compile time
    let is_little_endian = cfg!(target_endian = "little");

    // Create dict for the swapped (non-native) type
    let swapped_dict: crate::PyRef<crate::builtins::PyDict> = PyDict::default().into_ref(&vm.ctx);
    swapped_dict.set_item("_type_", vm.ctx.new_str(type_str).into(), vm)?;

    // Create the swapped type using the same metaclass
    let metaclass = type_ref.class();
    let bases = vm.ctx.new_tuple(vec![type_ref.as_object().to_owned()]);

    // Set placeholder first to prevent recursion
    type_ref
        .as_object()
        .set_attr("__ctype_le__", vm.ctx.none(), vm)?;
    type_ref
        .as_object()
        .set_attr("__ctype_be__", vm.ctx.none(), vm)?;

    // Create only the non-native endian type
    let suffix = if is_little_endian { "_be" } else { "_le" };
    let swapped_type = metaclass.as_object().call(
        (
            vm.ctx.new_str(format!("{}{}", type_ref.name(), suffix)),
            bases,
            swapped_dict.as_object().to_owned(),
        ),
        vm,
    )?;

    // Set _swappedbytes_ on the swapped type to indicate byte swapping is needed
    swapped_type.set_attr("_swappedbytes_", vm.ctx.none(), vm)?;

    // Update swapped type's StgInfo format to use opposite endian prefix
    if let Ok(swapped_type_ref) = swapped_type.clone().downcast::<PyType>()
        && let Some(mut sw_stg) = swapped_type_ref.get_type_data_mut::<StgInfo>()
    {
        // Swapped: little-endian system uses big-endian prefix and vice versa
        sw_stg.format = Some(alloc_format_string_for_type(
            type_str.chars().next().unwrap_or('?'),
            is_little_endian,
        ));
    }

    // Set attributes based on system byte order
    // Native endian attribute points to self, non-native points to swapped type
    if is_little_endian {
        // Little-endian system: __ctype_le__ = self, __ctype_be__ = swapped
        type_ref
            .as_object()
            .set_attr("__ctype_le__", type_ref.as_object().to_owned(), vm)?;
        type_ref
            .as_object()
            .set_attr("__ctype_be__", swapped_type.clone(), vm)?;
        swapped_type.set_attr("__ctype_le__", type_ref.as_object().to_owned(), vm)?;
        swapped_type.set_attr("__ctype_be__", swapped_type.clone(), vm)?;
    } else {
        // Big-endian system: __ctype_be__ = self, __ctype_le__ = swapped
        type_ref
            .as_object()
            .set_attr("__ctype_be__", type_ref.as_object().to_owned(), vm)?;
        type_ref
            .as_object()
            .set_attr("__ctype_le__", swapped_type.clone(), vm)?;
        swapped_type.set_attr("__ctype_be__", type_ref.as_object().to_owned(), vm)?;
        swapped_type.set_attr("__ctype_le__", swapped_type.clone(), vm)?;
    }

    Ok(())
}

#[pyclass(
    module = "_ctypes",
    name = "_SimpleCData",
    base = PyCData,
    metaclass = "PyCSimpleType"
)]
#[repr(transparent)]
pub(crate) struct PyCSimple(pub PyCData);

impl Debug for PyCSimple {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PyCSimple")
            .field("size", &self.0.buffer.read().len())
            .finish()
    }
}

fn value_to_bytes_endian(
    _type_: &str,
    value: &PyObject,
    swapped: bool,
    vm: &VirtualMachine,
) -> Vec<u8> {
    let storage_value = match _type_ {
        "c" => {
            // c_char - single byte (bytes, bytearray, or int 0-255)
            if let Some(bytes) = value.downcast_ref::<PyBytes>()
                && !bytes.is_empty()
            {
                SimpleStorageValue::Byte(bytes.as_bytes()[0])
            } else if let Some(bytearray) = value.downcast_ref::<PyByteArray>() {
                let buf = bytearray.borrow_buf();
                if !buf.is_empty() {
                    SimpleStorageValue::Byte(buf[0])
                } else {
                    SimpleStorageValue::Zero
                }
            } else if let Ok(int_val) = value.try_int(vm)
                && let Some(v) = int_val.as_bigint().to_u8()
            {
                SimpleStorageValue::Byte(v)
            } else {
                SimpleStorageValue::Zero
            }
        }
        "u" => {
            // c_wchar - platform-dependent size (2 on Windows, 4 on Unix)
            if let Some(s) = value.downcast_ref::<PyStr>() {
                let mut cps = s.as_wtf8().code_points();
                if let (Some(c), None) = (cps.next(), cps.next()) {
                    SimpleStorageValue::Wchar(c.to_u32())
                } else {
                    SimpleStorageValue::Zero
                }
            } else {
                SimpleStorageValue::Zero
            }
        }
        "b" => {
            // c_byte - signed char (1 byte)
            if let Ok(int_val) = value.try_index(vm) {
                SimpleStorageValue::Signed(int_val.as_bigint().to_i128().expect("int too large"))
            } else {
                SimpleStorageValue::Zero
            }
        }
        "B" => {
            // c_ubyte - unsigned char (1 byte)
            if let Ok(int_val) = value.try_index(vm) {
                SimpleStorageValue::Signed(int_val.as_bigint().to_i128().expect("int too large"))
            } else {
                SimpleStorageValue::Zero
            }
        }
        "h" => {
            // c_short (2 bytes)
            if let Ok(int_val) = value.try_index(vm) {
                SimpleStorageValue::Signed(int_val.as_bigint().to_i128().expect("int too large"))
            } else {
                SimpleStorageValue::Zero
            }
        }
        "H" => {
            // c_ushort (2 bytes)
            if let Ok(int_val) = value.try_index(vm) {
                SimpleStorageValue::Signed(int_val.as_bigint().to_i128().expect("int too large"))
            } else {
                SimpleStorageValue::Zero
            }
        }
        "i" => {
            // c_int (4 bytes)
            if let Ok(int_val) = value.try_index(vm) {
                SimpleStorageValue::Signed(int_val.as_bigint().to_i128().expect("int too large"))
            } else {
                SimpleStorageValue::Zero
            }
        }
        "I" => {
            // c_uint (4 bytes)
            if let Ok(int_val) = value.try_index(vm) {
                SimpleStorageValue::Signed(int_val.as_bigint().to_i128().expect("int too large"))
            } else {
                SimpleStorageValue::Zero
            }
        }
        "l" => {
            // c_long (platform dependent)
            if let Ok(int_val) = value.try_index(vm) {
                SimpleStorageValue::Signed(int_val.as_bigint().to_i128().expect("int too large"))
            } else {
                SimpleStorageValue::Zero
            }
        }
        "L" => {
            // c_ulong (platform dependent)
            if let Ok(int_val) = value.try_index(vm) {
                SimpleStorageValue::Signed(int_val.as_bigint().to_i128().expect("int too large"))
            } else {
                SimpleStorageValue::Zero
            }
        }
        "q" => {
            // c_longlong (8 bytes)
            if let Ok(int_val) = value.try_index(vm) {
                SimpleStorageValue::Signed(int_val.as_bigint().to_i128().expect("int too large"))
            } else {
                SimpleStorageValue::Zero
            }
        }
        "Q" => {
            // c_ulonglong (8 bytes)
            if let Ok(int_val) = value.try_index(vm) {
                SimpleStorageValue::Signed(int_val.as_bigint().to_i128().expect("int too large"))
            } else {
                SimpleStorageValue::Zero
            }
        }
        "f" => {
            // c_float (4 bytes) - also accepts int
            if let Ok(float_val) = value.try_float(vm) {
                SimpleStorageValue::Float(float_val.to_f64())
            } else if let Ok(int_val) = value.try_int(vm)
                && let Some(v) = int_val.as_bigint().to_f64()
            {
                SimpleStorageValue::Float(v)
            } else {
                SimpleStorageValue::Zero
            }
        }
        "d" => {
            // c_double (8 bytes) - also accepts int
            if let Ok(float_val) = value.try_float(vm) {
                SimpleStorageValue::Float(float_val.to_f64())
            } else if let Ok(int_val) = value.try_int(vm)
                && let Some(v) = int_val.as_bigint().to_f64()
            {
                SimpleStorageValue::Float(v)
            } else {
                SimpleStorageValue::Zero
            }
        }
        "g" => {
            // long double - platform dependent size
            // Store as f64, zero-pad to platform long double size
            // Note: This may lose precision on platforms where long double > 64 bits
            let value = if let Ok(float_val) = value.try_float(vm) {
                float_val.to_f64()
            } else if let Ok(int_val) = value.try_int(vm) {
                int_val.as_bigint().to_f64().unwrap_or(0.0)
            } else {
                0.0
            };
            SimpleStorageValue::Float(value)
        }
        "?" => {
            // c_bool (1 byte)
            if let Ok(b) = value.to_owned().try_to_bool(vm) {
                SimpleStorageValue::Bool(b)
            } else {
                SimpleStorageValue::Zero
            }
        }
        "v" => {
            // VARIANT_BOOL: True = 0xFFFF (-1 as i16), False = 0x0000
            if let Ok(b) = value.to_owned().try_to_bool(vm) {
                SimpleStorageValue::Bool(b)
            } else {
                SimpleStorageValue::Zero
            }
        }
        "P" => {
            // c_void_p - pointer type (platform pointer size)
            if let Ok(int_val) = value.try_index(vm) {
                let v = int_val
                    .as_bigint()
                    .to_usize()
                    .expect("int too large for pointer");
                SimpleStorageValue::Pointer(v)
            } else {
                SimpleStorageValue::Zero
            }
        }
        "z" => {
            // c_char_p - pointer to char (stores pointer value from int)
            // PyBytes case is handled in slot_new/set_value with make_z_buffer()
            if let Ok(int_val) = value.try_index(vm) {
                let v = int_val
                    .as_bigint()
                    .to_usize()
                    .expect("int too large for pointer");
                SimpleStorageValue::Pointer(v)
            } else {
                SimpleStorageValue::Zero
            }
        }
        "Z" => {
            // c_wchar_p - pointer to wchar_t (stores pointer value from int)
            // PyStr case is handled in slot_new/set_value with make_wchar_buffer()
            if let Ok(int_val) = value.try_index(vm) {
                let v = int_val
                    .as_bigint()
                    .to_usize()
                    .expect("int too large for pointer");
                SimpleStorageValue::Pointer(v)
            } else {
                SimpleStorageValue::Zero
            }
        }
        "O" => {
            // py_object - store object id as non-zero marker
            // The actual object is stored in _objects
            // Use object's id as a non-zero placeholder (indicates non-NULL)
            SimpleStorageValue::ObjectId(value.get_id())
        }
        _ => SimpleStorageValue::Zero,
    };
    simple_storage_value_to_bytes_endian(_type_, storage_value, swapped)
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
    type Args = (OptionalArg,);

    fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        let args: Self::Args = args.bind(vm)?;
        let _type_ = cls
            .type_code(vm)
            .ok_or_else(|| vm.new_type_error("abstract class"))?;
        // Save the initial argument for c_char_p/c_wchar_p _objects
        let init_arg = args.0.into_option();

        // Handle z/Z types with PyBytes/PyStr separately to avoid memory leak
        if let Some(ref v) = init_arg {
            if _type_ == "z" {
                if let Some(bytes) = v.downcast_ref::<PyBytes>() {
                    let (kept_alive, ptr) = super::base::ensure_z_null_terminated(bytes, vm);
                    let buffer = rustpython_host_env::ctypes::pointer_bytes(ptr);
                    let cdata = PyCData::from_bytes(buffer, Some(v.clone()));
                    *cdata.base.write() = Some(kept_alive);
                    return PyCSimple(cdata).into_ref_with_type(vm, cls).map(Into::into);
                }
            } else if _type_ == "Z"
                && let Some(s) = v.downcast_ref::<PyStr>()
            {
                let (holder, ptr) = super::base::str_to_wchar_bytes(s.as_wtf8(), vm);
                let buffer = rustpython_host_env::ctypes::pointer_bytes(ptr);
                let cdata = PyCData::from_bytes(buffer, Some(holder));
                return PyCSimple(cdata).into_ref_with_type(vm, cls).map(Into::into);
            }
        }

        let value = if let Some(ref v) = init_arg {
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

        // Check if this is a swapped endian type (presence of attribute indicates swapping)
        let swapped = cls.as_object().get_attr("_swappedbytes_", vm).is_ok();

        let buffer = value_to_bytes_endian(&_type_, &value, swapped, vm);

        // For c_char_p (type "z"), c_wchar_p (type "Z"), and py_object (type "O"),
        // store the initial value in _objects
        let objects = if (_type_ == "z" || _type_ == "Z" || _type_ == "O") && init_arg.is_some() {
            init_arg
        } else {
            None
        };

        PyCSimple(PyCData::from_bytes(buffer, objects))
            .into_ref_with_type(vm, cls)
            .map(Into::into)
    }

    fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
        unimplemented!("use slot_new")
    }
}

impl Initializer for PyCSimple {
    type Args = (OptionalArg,);

    fn init(zelf: PyRef<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
        // If an argument is provided, update the value
        if let Some(value) = args.0.into_option() {
            PyCSimple::set_value(zelf.into(), value, vm)?;
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
        let bases = cls.bases.read();
        let is_direct_simple = bases
            .iter()
            .any(|base| base.name().to_string() == "_SimpleCData");

        if is_direct_simple {
            // Direct SimpleCData: "typename(repr(value))"
            let value = PyCSimple::value(zelf.to_owned().into(), vm)?;
            let value_repr = value.repr(vm)?.to_string();
            Ok(format!("{}({})", type_name, value_repr))
        } else {
            // Subclass: "<typename object at addr>"
            let addr = zelf.get_id();
            Ok(format!("<{} object at {:#x}>", type_name, addr))
        }
    }
}

#[pyclass(
    flags(BASETYPE),
    with(Constructor, Initializer, AsBuffer, AsNumber, Representable)
)]
impl PyCSimple {
    #[pygetset]
    fn _b0_(&self) -> Option<PyObjectRef> {
        self.0.base.read().clone()
    }

    #[pygetset]
    pub(crate) fn value(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let zelf: &Py<Self> = instance
            .downcast_ref()
            .ok_or_else(|| vm.new_type_error("cannot get value of instance"))?;

        // Get _type_ from class
        let cls = zelf.class();
        let type_attr = cls
            .as_object()
            .get_attr("_type_", vm)
            .map_err(|_| vm.new_type_error("no _type_ attribute"))?;
        let type_code = type_attr.str(vm)?.to_string();

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
            if ptr == 0 {
                return Err(vm.new_value_error("PyObject is NULL"));
            }
            // Non-NULL: return stored object from _objects if available
            if let Some(obj) = zelf.0.objects.read().as_ref() {
                return Ok(obj.clone());
            }
            return Err(vm.new_value_error("PyObject is NULL"));
        }

        // Check if this is a swapped endian type (presence of attribute indicates swapping)
        let swapped = cls.as_object().get_attr("_swappedbytes_", vm).is_ok();

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
        bytes_to_pyobject(&cls_ref, &buffer_data, vm).or_else(|_| {
            // Fallback: return bytes as integer based on type
            match type_code.as_str() {
                "c" => {
                    if !buffer.is_empty() {
                        Ok(vm.ctx.new_bytes(vec![buffer[0]]).into())
                    } else {
                        Ok(vm.ctx.new_bytes(vec![0]).into())
                    }
                }
                "?" => {
                    let val = buffer.first().copied().unwrap_or(0);
                    Ok(vm.ctx.new_bool(val != 0).into())
                }
                _ => Ok(vm.ctx.new_int(0).into()),
            }
        })
    }

    #[pygetset(setter)]
    fn set_value(instance: PyObjectRef, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let zelf: PyRef<Self> = instance
            .clone()
            .downcast()
            .map_err(|_| vm.new_type_error("cannot set value of instance"))?;

        // Get _type_ from class
        let cls = zelf.class();
        let type_attr = cls
            .as_object()
            .get_attr("_type_", vm)
            .map_err(|_| vm.new_type_error("no _type_ attribute"))?;
        let type_code = type_attr.str(vm)?.to_string();

        // Handle z/Z types with PyBytes/PyStr separately to avoid memory leak
        if type_code == "z" {
            if let Some(bytes) = value.downcast_ref::<PyBytes>() {
                let (kept_alive, ptr) = super::base::ensure_z_null_terminated(bytes, vm);
                *zelf.0.buffer.write() =
                    alloc::borrow::Cow::Owned(rustpython_host_env::ctypes::pointer_bytes(ptr));
                *zelf.0.objects.write() = Some(value);
                *zelf.0.base.write() = Some(kept_alive);
                return Ok(());
            }
        } else if type_code == "Z"
            && let Some(s) = value.downcast_ref::<PyStr>()
        {
            let (holder, ptr) = super::base::str_to_wchar_bytes(s.as_wtf8(), vm);
            *zelf.0.buffer.write() =
                alloc::borrow::Cow::Owned(rustpython_host_env::ctypes::pointer_bytes(ptr));
            *zelf.0.objects.write() = Some(holder);
            return Ok(());
        }

        let content = set_primitive(&type_code, &value, vm)?;

        // Check if this is a swapped endian type (presence of attribute indicates swapping)
        let swapped = instance
            .class()
            .as_object()
            .get_attr("_swappedbytes_", vm)
            .is_ok();

        // Update buffer when value changes
        let buffer_bytes = value_to_bytes_endian(&type_code, &content, swapped, vm);

        // If the buffer is borrowed (from shared memory), write in-place
        // Otherwise replace with new owned buffer
        let mut buffer = zelf.0.buffer.write();
        write_simple_storage_buffer(&mut buffer, &buffer_bytes);

        // For c_char_p (type "z"), c_wchar_p (type "Z"), and py_object (type "O"),
        // keep the reference in _objects
        if type_code == "z" || type_code == "Z" || type_code == "O" {
            *zelf.0.objects.write() = Some(value);
        }
        Ok(())
    }

    #[pyclassmethod]
    fn repeat(cls: PyTypeRef, n: isize, vm: &VirtualMachine) -> PyResult {
        use super::array::array_type_from_ctype;

        if n < 0 {
            return Err(vm.new_value_error(format!("Array length must be >= 0, not {n}")));
        }
        // Use cached array type creation
        array_type_from_ctype(cls.into(), n as usize, vm)
    }

    /// Simple_from_outparm - convert output parameter back to Python value
    /// For direct subclasses of _SimpleCData (e.g., c_int), returns the value.
    /// For subclasses of those (e.g., class MyInt(c_int)), returns self.
    #[pymethod]
    fn __ctypes_from_outparam__(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        // _ctypes_simple_instance: returns true if NOT a direct subclass of Simple_Type
        // i.e., c_int (direct) -> false, MyInt(c_int) (subclass) -> true
        let is_subclass_of_simple = {
            let cls = zelf.class();
            let bases = cls.bases.read();
            // If base is NOT _SimpleCData, then it's a subclass of a subclass
            !bases
                .iter()
                .any(|base| base.name().to_string() == "_SimpleCData")
        };

        if is_subclass_of_simple {
            // Subclass of simple type (e.g., MyInt(c_int)): return self
            Ok(zelf.into())
        } else {
            // Direct simple type (e.g., c_int): return value
            PyCSimple::value(zelf.into(), vm)
        }
    }
}

impl PyCSimple {
    /// Extract the value from this ctypes object as an owned FfiArgValue.
    /// The value must be kept alive until after the FFI call completes.
    pub(crate) fn to_ffi_value(
        &self,
        ty: rustpython_host_env::ctypes::FfiType,
        _vm: &VirtualMachine,
    ) -> Option<FfiArgValue> {
        let buffer = self.0.buffer.read();
        Some(FfiArgValue::Scalar(
            rustpython_host_env::ctypes::ffi_value_from_type(&buffer, ty)?,
        ))
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
            .map(Cow::Owned)
            .unwrap_or(Cow::Borrowed("B"));
        let itemsize = stg_info.size;
        // Simple types are scalars with ndim=0, shape=()
        let desc = BufferDescriptor {
            len: itemsize,
            readonly: false,
            itemsize,
            format,
            dim_desc: vec![],
        };
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

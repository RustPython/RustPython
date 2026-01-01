use super::StgInfo;
use super::base::{CDATA_BUFFER_METHODS, PyCData};
use super::type_info;
use crate::{
    AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject, VirtualMachine,
    atomic_func,
    builtins::{
        PyBytes, PyInt, PyList, PySlice, PyStr, PyType, PyTypeRef, genericalias::PyGenericAlias,
    },
    class::StaticType,
    function::{ArgBytesLike, FuncArgs, PySetterValue},
    protocol::{BufferDescriptor, PyBuffer, PyNumberMethods, PySequenceMethods},
    types::{AsBuffer, AsNumber, AsSequence, Constructor, Initializer},
};
use alloc::borrow::Cow;
use num_traits::{Signed, ToPrimitive};

/// Get itemsize from a PEP 3118 format string
/// Extracts the type code (last char after endianness prefix) and returns its size
fn get_size_from_format(fmt: &str) -> usize {
    // Format is like "<f", ">q", etc. - strip endianness prefix and get type code
    let code = fmt
        .trim_start_matches(['<', '>', '@', '=', '!', '&'])
        .chars()
        .next()
        .map(|c| c.to_string());
    code.map(|c| type_info(&c).map(|t| t.size).unwrap_or(1))
        .unwrap_or(1)
}

/// Creates array type for (element_type, length)
/// Uses _array_type_cache to ensure identical calls return the same type object
pub(super) fn array_type_from_ctype(
    itemtype: PyObjectRef,
    length: usize,
    vm: &VirtualMachine,
) -> PyResult {
    // PyCArrayType_from_ctype

    // Get the _array_type_cache from _ctypes module
    let ctypes_module = vm.import("_ctypes", 0)?;
    let cache = ctypes_module.get_attr("_array_type_cache", vm)?;

    // Create cache key: (itemtype, length) tuple
    let length_obj: PyObjectRef = vm.ctx.new_int(length).into();
    let cache_key = vm.ctx.new_tuple(vec![itemtype.clone(), length_obj]);

    // Check if already in cache
    if let Ok(cached) = vm.call_method(&cache, "__getitem__", (cache_key.clone(),))
        && !vm.is_none(&cached)
    {
        return Ok(cached);
    }

    // Cache miss - create new array type
    let itemtype_ref = itemtype
        .clone()
        .downcast::<PyType>()
        .map_err(|_| vm.new_type_error("Expected a type object"))?;

    let item_stg = itemtype_ref
        .stg_info_opt()
        .ok_or_else(|| vm.new_type_error("_type_ must have storage info"))?;

    let element_size = item_stg.size;
    let element_align = item_stg.align;
    let item_format = item_stg.format.clone();
    let item_shape = item_stg.shape.clone();
    let item_flags = item_stg.flags;

    // Check overflow before multiplication
    let total_size = element_size
        .checked_mul(length)
        .ok_or_else(|| vm.new_overflow_error("array too large"))?;

    // format name: "c_int_Array_5"
    let type_name = format!("{}_Array_{}", itemtype_ref.name(), length);

    // Get item type code before moving itemtype
    let item_type_code = itemtype_ref
        .as_object()
        .get_attr("_type_", vm)
        .ok()
        .and_then(|t| t.downcast_ref::<PyStr>().map(|s| s.to_string()));

    let stg_info = StgInfo::new_array(
        total_size,
        element_align,
        length,
        itemtype_ref.clone(),
        element_size,
        item_format.as_deref(),
        &item_shape,
        item_flags,
    );

    let new_type = create_array_type_with_name(stg_info, &type_name, vm)?;

    // Special case for character arrays - add value/raw attributes
    let new_type_ref: PyTypeRef = new_type
        .clone()
        .downcast()
        .map_err(|_| vm.new_type_error("expected type"))?;

    match item_type_code.as_deref() {
        Some("c") => add_char_array_getsets(&new_type_ref, vm),
        Some("u") => add_wchar_array_getsets(&new_type_ref, vm),
        _ => {}
    }

    // Store in cache
    vm.call_method(&cache, "__setitem__", (cache_key, new_type.clone()))?;

    Ok(new_type)
}

/// create_array_type_with_name - create array type with specified name
fn create_array_type_with_name(
    stg_info: StgInfo,
    type_name: &str,
    vm: &VirtualMachine,
) -> PyResult {
    let metaclass = PyCArrayType::static_type().to_owned();
    let name = vm.ctx.new_str(type_name);
    let bases = vm
        .ctx
        .new_tuple(vec![PyCArray::static_type().to_owned().into()]);
    let dict = vm.ctx.new_dict();

    let args = FuncArgs::new(
        vec![name.into(), bases.into(), dict.into()],
        crate::function::KwArgs::default(),
    );

    let new_type = crate::builtins::type_::PyType::slot_new(metaclass, args, vm)?;

    let type_ref: PyTypeRef = new_type
        .clone()
        .downcast()
        .map_err(|_| vm.new_type_error("Failed to create array type"))?;

    // Set class attributes for _type_ and _length_
    if let Some(element_type) = stg_info.element_type.clone() {
        new_type.set_attr("_type_", element_type, vm)?;
    }
    new_type.set_attr("_length_", vm.ctx.new_int(stg_info.length), vm)?;

    super::base::set_or_init_stginfo(&type_ref, stg_info);

    Ok(new_type)
}

/// PyCArrayType - metatype for Array types
#[pyclass(name = "PyCArrayType", base = PyType, module = "_ctypes")]
#[derive(Debug)]
#[repr(transparent)]
pub(super) struct PyCArrayType(PyType);

// PyCArrayType implements Initializer for slots.init (PyCArrayType_init)
impl Initializer for PyCArrayType {
    type Args = FuncArgs;

    fn init(zelf: PyRef<Self>, _args: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
        // zelf is the newly created array type (e.g., T in "class T(Array)")
        let new_type: &PyType = &zelf.0;

        new_type.check_not_initialized(vm)?;

        // 1. Get _length_ from class dict first
        let direct_length = new_type
            .attributes
            .read()
            .get(vm.ctx.intern_str("_length_"))
            .cloned();

        // 2. Get _type_ from class dict first
        let direct_type = new_type
            .attributes
            .read()
            .get(vm.ctx.intern_str("_type_"))
            .cloned();

        // 3. Find parent StgInfo from MRO (for inheritance)
        // Note: PyType.mro does NOT include self, so no skip needed
        let parent_stg_info = new_type
            .mro
            .read()
            .iter()
            .find_map(|base| base.stg_info_opt().map(|s| s.clone()));

        // 4. Resolve _length_ (direct or inherited)
        let length = if let Some(length_attr) = direct_length {
            // Direct _length_ defined - validate it (PyLong_Check)
            let length_int = length_attr
                .downcast_ref::<PyInt>()
                .ok_or_else(|| vm.new_type_error("The '_length_' attribute must be an integer"))?;
            let bigint = length_int.as_bigint();
            // Check sign first - negative values are ValueError
            if bigint.is_negative() {
                return Err(vm.new_value_error("The '_length_' attribute must not be negative"));
            }
            // Positive values that don't fit in usize are OverflowError
            bigint
                .to_usize()
                .ok_or_else(|| vm.new_overflow_error("The '_length_' attribute is too large"))?
        } else if let Some(ref parent_info) = parent_stg_info {
            // Inherit from parent
            parent_info.length
        } else {
            return Err(vm.new_attribute_error("class must define a '_length_' attribute"));
        };

        // 5. Resolve _type_ and get item_info (direct or inherited)
        let (element_type, item_size, item_align, item_format, item_shape, item_flags) =
            if let Some(type_attr) = direct_type {
                // Direct _type_ defined - validate it (PyStgInfo_FromType)
                let type_ref = type_attr
                    .clone()
                    .downcast::<PyType>()
                    .map_err(|_| vm.new_type_error("_type_ must be a type"))?;
                let (size, align, format, shape, flags) = {
                    let item_info = type_ref
                        .stg_info_opt()
                        .ok_or_else(|| vm.new_type_error("_type_ must have storage info"))?;
                    (
                        item_info.size,
                        item_info.align,
                        item_info.format.clone(),
                        item_info.shape.clone(),
                        item_info.flags,
                    )
                };
                (type_ref, size, align, format, shape, flags)
            } else if let Some(ref parent_info) = parent_stg_info {
                // Inherit from parent
                let parent_type = parent_info
                    .element_type
                    .clone()
                    .ok_or_else(|| vm.new_type_error("_type_ must have storage info"))?;
                (
                    parent_type,
                    parent_info.element_size,
                    parent_info.align,
                    parent_info.format.clone(),
                    parent_info.shape.clone(),
                    parent_info.flags,
                )
            } else {
                return Err(vm.new_attribute_error("class must define a '_type_' attribute"));
            };

        // 6. Check overflow (item_size != 0 && length > MAX / item_size)
        if item_size != 0 && length > usize::MAX / item_size {
            return Err(vm.new_overflow_error("array too large"));
        }

        // 7. Initialize StgInfo (PyStgInfo_Init + field assignment)
        let stg_info = StgInfo::new_array(
            item_size * length, // size = item_size * length
            item_align,         // align = item_info->align
            length,             // length
            element_type.clone(),
            item_size, // element_size
            item_format.as_deref(),
            &item_shape,
            item_flags,
        );

        // 8. Store StgInfo in type_data
        super::base::set_or_init_stginfo(new_type, stg_info);

        // 9. Get type code before moving element_type
        let item_type_code = element_type
            .as_object()
            .get_attr("_type_", vm)
            .ok()
            .and_then(|t| t.downcast_ref::<PyStr>().map(|s| s.to_string()));

        // 10. Set class attributes for _type_ and _length_
        zelf.as_object().set_attr("_type_", element_type, vm)?;
        zelf.as_object()
            .set_attr("_length_", vm.ctx.new_int(length), vm)?;

        // 11. Special case for character arrays - add value/raw attributes
        // if (iteminfo->getfunc == _ctypes_get_fielddesc("c")->getfunc)
        //              add_getset((PyTypeObject*)self, CharArray_getsets);
        //          else if (iteminfo->getfunc == _ctypes_get_fielddesc("u")->getfunc)
        //              add_getset((PyTypeObject*)self, WCharArray_getsets);

        // Get type ref for add_getset
        let type_ref: PyTypeRef = zelf.as_object().to_owned().downcast().unwrap();
        match item_type_code.as_deref() {
            Some("c") => add_char_array_getsets(&type_ref, vm),
            Some("u") => add_wchar_array_getsets(&type_ref, vm),
            _ => {}
        }

        Ok(())
    }
}

#[pyclass(flags(IMMUTABLETYPE), with(Initializer, AsNumber))]
impl PyCArrayType {
    #[pymethod]
    fn from_param(zelf: PyObjectRef, value: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        // zelf is the array type class that from_param was called on
        let cls = zelf
            .downcast::<PyType>()
            .map_err(|_| vm.new_type_error("from_param: expected a type"))?;

        // 1. If already an instance of the requested type, return it
        if value.is_instance(cls.as_object(), vm)? {
            return Ok(value);
        }

        // 2. Check for CArgObject (PyCArg_CheckExact)
        if let Some(carg) = value.downcast_ref::<super::_ctypes::CArgObject>() {
            // Check if the wrapped object is an instance of the requested type
            if carg.obj.is_instance(cls.as_object(), vm)? {
                return Ok(value); // Return the CArgObject as-is
            }
        }

        // 3. Check for _as_parameter_ attribute
        if let Ok(as_parameter) = value.get_attr("_as_parameter_", vm) {
            return PyCArrayType::from_param(cls.as_object().to_owned(), as_parameter, vm);
        }

        Err(vm.new_type_error(format!(
            "expected {} instance instead of {}",
            cls.name(),
            value.class().name()
        )))
    }
}

impl AsNumber for PyCArrayType {
    fn as_number() -> &'static PyNumberMethods {
        static AS_NUMBER: PyNumberMethods = PyNumberMethods {
            multiply: Some(|a, b, vm| {
                // a is a type object whose metaclass is PyCArrayType (e.g., Array_5)
                let n = b
                    .try_index(vm)?
                    .as_bigint()
                    .to_isize()
                    .ok_or_else(|| vm.new_overflow_error("array size too large"))?;

                if n < 0 {
                    return Err(vm.new_value_error(format!("Array length must be >= 0, not {n}")));
                }

                // Check for overflow before creating the new array type
                let zelf_type = a
                    .downcast_ref::<PyType>()
                    .ok_or_else(|| vm.new_type_error("Expected type"))?;

                if let Some(stg_info) = zelf_type.stg_info_opt() {
                    let current_size = stg_info.size;
                    // Check if current_size * n would overflow
                    if current_size != 0 && (n as usize) > isize::MAX as usize / current_size {
                        return Err(vm.new_overflow_error("array too large"));
                    }
                }

                // Use cached array type creation
                // The element type of the new array is the current array type itself
                array_type_from_ctype(a.to_owned(), n as usize, vm)
            }),
            ..PyNumberMethods::NOT_IMPLEMENTED
        };
        &AS_NUMBER
    }
}

/// PyCArray - Array instance
/// All array metadata (element_type, length, element_size) is stored in the type's StgInfo
#[pyclass(
    name = "Array",
    base = PyCData,
    metaclass = "PyCArrayType",
    module = "_ctypes"
)]
#[derive(Debug)]
#[repr(transparent)]
pub struct PyCArray(pub PyCData);

impl PyCArray {
    /// Get the type code of array element type (e.g., "c" for c_char, "u" for c_wchar)
    fn get_element_type_code(zelf: &Py<Self>, vm: &VirtualMachine) -> Option<String> {
        zelf.class()
            .stg_info_opt()
            .and_then(|info| info.element_type.clone())?
            .as_object()
            .get_attr("_type_", vm)
            .ok()
            .and_then(|t| t.downcast_ref::<PyStr>().map(|s| s.to_string()))
    }
}

impl Constructor for PyCArray {
    type Args = FuncArgs;

    fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        // Check for abstract class - StgInfo must exist and be initialized
        // Extract values in a block to drop the borrow before using cls
        let (length, total_size) = {
            let stg = cls.stg_info(vm)?;
            (stg.length, stg.size)
        };

        // Check for too many initializers
        if args.args.len() > length {
            return Err(vm.new_index_error("too many initializers"));
        }

        // Create array with zero-initialized buffer
        let buffer = vec![0u8; total_size];
        let instance = PyCArray(PyCData::from_bytes_with_length(buffer, None, length))
            .into_ref_with_type(vm, cls)?;

        // Initialize elements using setitem_by_index (Array_init pattern)
        for (i, value) in args.args.iter().enumerate() {
            PyCArray::setitem_by_index(&instance, i as isize, value.clone(), vm)?;
        }

        Ok(instance.into())
    }

    fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
        unimplemented!("use slot_new")
    }
}

impl Initializer for PyCArray {
    type Args = FuncArgs;

    fn init(zelf: PyRef<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
        // Re-initialize array elements when __init__ is called
        for (i, value) in args.args.iter().enumerate() {
            PyCArray::setitem_by_index(&zelf, i as isize, value.clone(), vm)?;
        }
        Ok(())
    }
}

impl AsSequence for PyCArray {
    fn as_sequence() -> &'static PySequenceMethods {
        use std::sync::LazyLock;
        static AS_SEQUENCE: LazyLock<PySequenceMethods> = LazyLock::new(|| PySequenceMethods {
            length: atomic_func!(|seq, _vm| {
                let zelf = PyCArray::sequence_downcast(seq);
                Ok(zelf.class().stg_info_opt().map_or(0, |i| i.length))
            }),
            item: atomic_func!(|seq, i, vm| {
                let zelf = PyCArray::sequence_downcast(seq);
                PyCArray::getitem_by_index(zelf, i, vm)
            }),
            ass_item: atomic_func!(|seq, i, value, vm| {
                let zelf = PyCArray::sequence_downcast(seq);
                match value {
                    Some(v) => PyCArray::setitem_by_index(zelf, i, v, vm),
                    None => Err(vm.new_type_error("cannot delete array elements")),
                }
            }),
            ..PySequenceMethods::NOT_IMPLEMENTED
        });
        &AS_SEQUENCE
    }
}

#[pyclass(
    flags(BASETYPE, IMMUTABLETYPE),
    with(Constructor, Initializer, AsSequence, AsBuffer)
)]
impl PyCArray {
    #[pyclassmethod]
    fn __class_getitem__(cls: PyTypeRef, args: PyObjectRef, vm: &VirtualMachine) -> PyGenericAlias {
        PyGenericAlias::from_args(cls, args, vm)
    }

    fn int_to_bytes(i: &malachite_bigint::BigInt, size: usize) -> Vec<u8> {
        // Try unsigned first (handles values like 0xFFFFFFFF that overflow signed)
        // then fall back to signed (handles negative values)
        match size {
            1 => {
                if let Some(v) = i.to_u8() {
                    vec![v]
                } else {
                    vec![i.to_i8().unwrap_or(0) as u8]
                }
            }
            2 => {
                if let Some(v) = i.to_u16() {
                    v.to_ne_bytes().to_vec()
                } else {
                    i.to_i16().unwrap_or(0).to_ne_bytes().to_vec()
                }
            }
            4 => {
                if let Some(v) = i.to_u32() {
                    v.to_ne_bytes().to_vec()
                } else {
                    i.to_i32().unwrap_or(0).to_ne_bytes().to_vec()
                }
            }
            8 => {
                if let Some(v) = i.to_u64() {
                    v.to_ne_bytes().to_vec()
                } else {
                    i.to_i64().unwrap_or(0).to_ne_bytes().to_vec()
                }
            }
            _ => vec![0u8; size],
        }
    }

    fn bytes_to_int(
        bytes: &[u8],
        size: usize,
        type_code: Option<&str>,
        vm: &VirtualMachine,
    ) -> PyObjectRef {
        // Unsigned type codes: B (uchar), H (ushort), I (uint), L (ulong), Q (ulonglong)
        let is_unsigned = matches!(
            type_code,
            Some("B") | Some("H") | Some("I") | Some("L") | Some("Q")
        );

        match (size, is_unsigned) {
            (1, false) => vm.ctx.new_int(bytes[0] as i8).into(),
            (1, true) => vm.ctx.new_int(bytes[0]).into(),
            (2, false) => {
                let val = i16::from_ne_bytes([bytes[0], bytes[1]]);
                vm.ctx.new_int(val).into()
            }
            (2, true) => {
                let val = u16::from_ne_bytes([bytes[0], bytes[1]]);
                vm.ctx.new_int(val).into()
            }
            (4, false) => {
                let val = i32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                vm.ctx.new_int(val).into()
            }
            (4, true) => {
                let val = u32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                vm.ctx.new_int(val).into()
            }
            (8, false) => {
                let val = i64::from_ne_bytes([
                    bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
                ]);
                vm.ctx.new_int(val).into()
            }
            (8, true) => {
                let val = u64::from_ne_bytes([
                    bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
                ]);
                vm.ctx.new_int(val).into()
            }
            _ => vm.ctx.new_int(0).into(),
        }
    }

    fn getitem_by_index(zelf: &Py<PyCArray>, i: isize, vm: &VirtualMachine) -> PyResult {
        let stg = zelf.class().stg_info_opt();
        let length = stg.as_ref().map_or(0, |i| i.length) as isize;
        let index = if i < 0 { length + i } else { i };
        if index < 0 || index >= length {
            return Err(vm.new_index_error("invalid index"));
        }
        let index = index as usize;
        let element_size = stg.as_ref().map_or(0, |i| i.element_size);
        let offset = index * element_size;
        let type_code = Self::get_element_type_code(zelf, vm);

        // Get target buffer and offset (base's buffer if available, otherwise own)
        let base_obj = zelf.0.base.read().clone();
        let (buffer_lock, final_offset) = if let Some(cdata) = base_obj
            .as_ref()
            .and_then(|b| b.downcast_ref::<super::PyCData>())
        {
            (&cdata.buffer, zelf.0.base_offset.load() + offset)
        } else {
            (&zelf.0.buffer, offset)
        };

        let buffer = buffer_lock.read();
        Self::read_element_from_buffer(
            &buffer,
            final_offset,
            element_size,
            type_code.as_deref(),
            vm,
        )
    }

    /// Helper to read an element value from a buffer at given offset
    fn read_element_from_buffer(
        buffer: &[u8],
        offset: usize,
        element_size: usize,
        type_code: Option<&str>,
        vm: &VirtualMachine,
    ) -> PyResult {
        match type_code {
            Some("c") => {
                // Return single byte as bytes
                if offset < buffer.len() {
                    Ok(vm.ctx.new_bytes(vec![buffer[offset]]).into())
                } else {
                    Ok(vm.ctx.new_bytes(vec![0]).into())
                }
            }
            Some("u") => {
                // Return single wchar as str
                if let Some(code) = wchar_from_bytes(&buffer[offset..]) {
                    let s = char::from_u32(code)
                        .map(|c| c.to_string())
                        .unwrap_or_default();
                    Ok(vm.ctx.new_str(s).into())
                } else {
                    Ok(vm.ctx.new_str("").into())
                }
            }
            Some("z") => {
                // c_char_p: pointer to bytes - dereference to get string
                if offset + element_size > buffer.len() {
                    return Ok(vm.ctx.none());
                }
                let ptr_bytes = &buffer[offset..offset + element_size];
                let ptr_val = usize::from_ne_bytes(
                    ptr_bytes
                        .try_into()
                        .unwrap_or([0; core::mem::size_of::<usize>()]),
                );
                if ptr_val == 0 {
                    return Ok(vm.ctx.none());
                }
                // Read null-terminated string from pointer address
                unsafe {
                    let ptr = ptr_val as *const u8;
                    let mut len = 0;
                    while *ptr.add(len) != 0 {
                        len += 1;
                    }
                    let bytes = core::slice::from_raw_parts(ptr, len);
                    Ok(vm.ctx.new_bytes(bytes.to_vec()).into())
                }
            }
            Some("Z") => {
                // c_wchar_p: pointer to wchar_t - dereference to get string
                if offset + element_size > buffer.len() {
                    return Ok(vm.ctx.none());
                }
                let ptr_bytes = &buffer[offset..offset + element_size];
                let ptr_val = usize::from_ne_bytes(
                    ptr_bytes
                        .try_into()
                        .unwrap_or([0; core::mem::size_of::<usize>()]),
                );
                if ptr_val == 0 {
                    return Ok(vm.ctx.none());
                }
                // Read null-terminated wide string using WCHAR_SIZE
                unsafe {
                    let ptr = ptr_val as *const u8;
                    let mut chars = Vec::new();
                    let mut pos = 0usize;
                    loop {
                        let code = if WCHAR_SIZE == 2 {
                            let bytes = core::slice::from_raw_parts(ptr.add(pos), 2);
                            u16::from_ne_bytes([bytes[0], bytes[1]]) as u32
                        } else {
                            let bytes = core::slice::from_raw_parts(ptr.add(pos), 4);
                            u32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
                        };
                        if code == 0 {
                            break;
                        }
                        if let Some(ch) = char::from_u32(code) {
                            chars.push(ch);
                        }
                        pos += WCHAR_SIZE;
                    }
                    let s: String = chars.into_iter().collect();
                    Ok(vm.ctx.new_str(s).into())
                }
            }
            Some("f") => {
                // c_float
                if offset + 4 <= buffer.len() {
                    let bytes: [u8; 4] = buffer[offset..offset + 4].try_into().unwrap();
                    let val = f32::from_ne_bytes(bytes);
                    Ok(vm.ctx.new_float(val as f64).into())
                } else {
                    Ok(vm.ctx.new_float(0.0).into())
                }
            }
            Some("d") | Some("g") => {
                // c_double / c_longdouble - read f64 from first 8 bytes
                if offset + 8 <= buffer.len() {
                    let bytes: [u8; 8] = buffer[offset..offset + 8].try_into().unwrap();
                    let val = f64::from_ne_bytes(bytes);
                    Ok(vm.ctx.new_float(val).into())
                } else {
                    Ok(vm.ctx.new_float(0.0).into())
                }
            }
            _ => {
                if offset + element_size <= buffer.len() {
                    let bytes = &buffer[offset..offset + element_size];
                    Ok(Self::bytes_to_int(bytes, element_size, type_code, vm))
                } else {
                    Ok(vm.ctx.new_int(0).into())
                }
            }
        }
    }

    /// Helper to write an element value to a buffer at given offset
    /// This is extracted to share code between direct write and base-buffer write
    #[allow(clippy::too_many_arguments)]
    fn write_element_to_buffer(
        buffer: &mut [u8],
        offset: usize,
        element_size: usize,
        type_code: Option<&str>,
        value: &PyObject,
        zelf: &Py<PyCArray>,
        index: usize,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        match type_code {
            Some("c") => {
                if let Some(b) = value.downcast_ref::<PyBytes>() {
                    if offset < buffer.len() {
                        buffer[offset] = b.as_bytes().first().copied().unwrap_or(0);
                    }
                } else if let Ok(int_val) = value.try_int(vm) {
                    if offset < buffer.len() {
                        buffer[offset] = int_val.as_bigint().to_u8().unwrap_or(0);
                    }
                } else {
                    return Err(vm.new_type_error("an integer or bytes of length 1 is required"));
                }
            }
            Some("u") => {
                if let Some(s) = value.downcast_ref::<PyStr>() {
                    let code = s.as_str().chars().next().map(|c| c as u32).unwrap_or(0);
                    if offset + WCHAR_SIZE <= buffer.len() {
                        wchar_to_bytes(code, &mut buffer[offset..]);
                    }
                } else {
                    return Err(vm.new_type_error("unicode string expected"));
                }
            }
            Some("z") => {
                let (ptr_val, converted) = if value.is(&vm.ctx.none) {
                    (0usize, None)
                } else if let Some(bytes) = value.downcast_ref::<PyBytes>() {
                    let (c, ptr) = super::base::ensure_z_null_terminated(bytes, vm);
                    (ptr, Some(c))
                } else if let Ok(int_val) = value.try_index(vm) {
                    (int_val.as_bigint().to_usize().unwrap_or(0), None)
                } else {
                    return Err(vm.new_type_error(
                        "bytes or integer address expected instead of {}".to_owned(),
                    ));
                };
                if offset + element_size <= buffer.len() {
                    buffer[offset..offset + element_size].copy_from_slice(&ptr_val.to_ne_bytes());
                }
                if let Some(c) = converted {
                    return zelf.0.keep_ref(index, c, vm);
                }
            }
            Some("Z") => {
                let (ptr_val, converted) = if value.is(&vm.ctx.none) {
                    (0usize, None)
                } else if let Some(s) = value.downcast_ref::<PyStr>() {
                    let (holder, ptr) = super::base::str_to_wchar_bytes(s.as_str(), vm);
                    (ptr, Some(holder))
                } else if let Ok(int_val) = value.try_index(vm) {
                    (int_val.as_bigint().to_usize().unwrap_or(0), None)
                } else {
                    return Err(vm.new_type_error("unicode string or integer address expected"));
                };
                if offset + element_size <= buffer.len() {
                    buffer[offset..offset + element_size].copy_from_slice(&ptr_val.to_ne_bytes());
                }
                if let Some(c) = converted {
                    return zelf.0.keep_ref(index, c, vm);
                }
            }
            Some("f") => {
                // c_float: convert int/float to f32 bytes
                let f32_val = if let Ok(float_val) = value.try_float(vm) {
                    float_val.to_f64() as f32
                } else if let Ok(int_val) = value.try_int(vm) {
                    int_val.as_bigint().to_f64().unwrap_or(0.0) as f32
                } else {
                    return Err(vm.new_type_error("a float is required"));
                };
                if offset + 4 <= buffer.len() {
                    buffer[offset..offset + 4].copy_from_slice(&f32_val.to_ne_bytes());
                }
            }
            Some("d") | Some("g") => {
                // c_double / c_longdouble: convert int/float to f64 bytes
                let f64_val = if let Ok(float_val) = value.try_float(vm) {
                    float_val.to_f64()
                } else if let Ok(int_val) = value.try_int(vm) {
                    int_val.as_bigint().to_f64().unwrap_or(0.0)
                } else {
                    return Err(vm.new_type_error("a float is required"));
                };
                if offset + 8 <= buffer.len() {
                    buffer[offset..offset + 8].copy_from_slice(&f64_val.to_ne_bytes());
                }
                // For "g" type, remaining bytes stay zero
            }
            _ => {
                // Handle ctypes instances (copy their buffer)
                if let Some(cdata) = value.downcast_ref::<PyCData>() {
                    let src_buffer = cdata.buffer.read();
                    let copy_len = src_buffer.len().min(element_size);
                    if offset + copy_len <= buffer.len() {
                        buffer[offset..offset + copy_len].copy_from_slice(&src_buffer[..copy_len]);
                    }
                // Other types: use int_to_bytes
                } else if let Ok(int_val) = value.try_int(vm) {
                    let bytes = Self::int_to_bytes(int_val.as_bigint(), element_size);
                    if offset + element_size <= buffer.len() {
                        buffer[offset..offset + element_size].copy_from_slice(&bytes);
                    }
                } else {
                    return Err(vm.new_type_error(format!(
                        "expected {} instance, not {}",
                        type_code.unwrap_or("value"),
                        value.class().name()
                    )));
                }
            }
        }

        // KeepRef
        if super::base::PyCData::should_keep_ref(value) {
            let to_keep = super::base::PyCData::get_kept_objects(value, vm);
            zelf.0.keep_ref(index, to_keep, vm)?;
        }

        Ok(())
    }

    fn setitem_by_index(
        zelf: &Py<PyCArray>,
        i: isize,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let stg = zelf.class().stg_info_opt();
        let length = stg.as_ref().map_or(0, |i| i.length) as isize;
        let index = if i < 0 { length + i } else { i };
        if index < 0 || index >= length {
            return Err(vm.new_index_error("invalid index"));
        }
        let index = index as usize;
        let element_size = stg.as_ref().map_or(0, |i| i.element_size);
        let offset = index * element_size;
        let type_code = Self::get_element_type_code(zelf, vm);

        // Get target buffer and offset (base's buffer if available, otherwise own)
        let base_obj = zelf.0.base.read().clone();
        let (buffer_lock, final_offset) = if let Some(cdata) = base_obj
            .as_ref()
            .and_then(|b| b.downcast_ref::<super::PyCData>())
        {
            (&cdata.buffer, zelf.0.base_offset.load() + offset)
        } else {
            (&zelf.0.buffer, offset)
        };

        let mut buffer = buffer_lock.write();

        // For shared memory (Cow::Borrowed), we need to write directly to the memory
        // For owned memory (Cow::Owned), we can write to the owned buffer
        match &mut *buffer {
            Cow::Borrowed(slice) => {
                // SAFETY: For from_buffer, the slice points to writable shared memory.
                // Python's from_buffer requires writable buffer, so this is safe.
                let ptr = slice.as_ptr() as *mut u8;
                let len = slice.len();
                let owned_slice = unsafe { core::slice::from_raw_parts_mut(ptr, len) };
                Self::write_element_to_buffer(
                    owned_slice,
                    final_offset,
                    element_size,
                    type_code.as_deref(),
                    &value,
                    zelf,
                    index,
                    vm,
                )
            }
            Cow::Owned(vec) => Self::write_element_to_buffer(
                vec,
                final_offset,
                element_size,
                type_code.as_deref(),
                &value,
                zelf,
                index,
                vm,
            ),
        }
    }

    // Array_subscript
    #[pymethod]
    fn __getitem__(zelf: &Py<Self>, item: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        // PyIndex_Check
        if let Some(i) = item.downcast_ref::<PyInt>() {
            let i = i.as_bigint().to_isize().ok_or_else(|| {
                vm.new_index_error("cannot fit index into an index-sized integer")
            })?;
            // getitem_by_index handles negative index normalization
            Self::getitem_by_index(zelf, i, vm)
        }
        // PySlice_Check
        else if let Some(slice) = item.downcast_ref::<PySlice>() {
            Self::getitem_by_slice(zelf, slice, vm)
        } else {
            Err(vm.new_type_error("indices must be integers"))
        }
    }

    // Array_subscript slice handling
    fn getitem_by_slice(zelf: &Py<Self>, slice: &PySlice, vm: &VirtualMachine) -> PyResult {
        use crate::sliceable::SaturatedSliceIter;

        let stg = zelf.class().stg_info_opt();
        let length = stg.as_ref().map_or(0, |i| i.length);

        // PySlice_Unpack + PySlice_AdjustIndices
        let sat_slice = slice.to_saturated(vm)?;
        let (range, step, slice_len) = sat_slice.adjust_indices(length);

        let type_code = Self::get_element_type_code(zelf, vm);
        let element_size = stg.as_ref().map_or(0, |i| i.element_size);
        let start = range.start;

        match type_code.as_deref() {
            // c_char → bytes (item_info->getfunc == "c")
            Some("c") => {
                if slice_len == 0 {
                    return Ok(vm.ctx.new_bytes(vec![]).into());
                }
                let buffer = zelf.0.buffer.read();
                // step == 1 optimization: direct memcpy
                if step == 1 {
                    let start_offset = start * element_size;
                    let end_offset = start_offset + slice_len;
                    if end_offset <= buffer.len() {
                        return Ok(vm
                            .ctx
                            .new_bytes(buffer[start_offset..end_offset].to_vec())
                            .into());
                    }
                }
                // Non-contiguous: iterate
                let iter = SaturatedSliceIter::from_adjust_indices(range, step, slice_len);
                let mut result = Vec::with_capacity(slice_len);
                for idx in iter {
                    let offset = idx * element_size;
                    if offset < buffer.len() {
                        result.push(buffer[offset]);
                    }
                }
                Ok(vm.ctx.new_bytes(result).into())
            }
            // c_wchar → str (item_info->getfunc == "u")
            Some("u") => {
                if slice_len == 0 {
                    return Ok(vm.ctx.new_str("").into());
                }
                let buffer = zelf.0.buffer.read();
                // step == 1 optimization: direct conversion
                if step == 1 {
                    let start_offset = start * WCHAR_SIZE;
                    let end_offset = start_offset + slice_len * WCHAR_SIZE;
                    if end_offset <= buffer.len() {
                        let wchar_bytes = &buffer[start_offset..end_offset];
                        let result: String = wchar_bytes
                            .chunks(WCHAR_SIZE)
                            .filter_map(|chunk| wchar_from_bytes(chunk).and_then(char::from_u32))
                            .collect();
                        return Ok(vm.ctx.new_str(result).into());
                    }
                }
                // Non-contiguous: iterate
                let iter = SaturatedSliceIter::from_adjust_indices(range, step, slice_len);
                let mut result = String::with_capacity(slice_len);
                for idx in iter {
                    let offset = idx * WCHAR_SIZE;
                    if let Some(code_point) = wchar_from_bytes(&buffer[offset..])
                        && let Some(c) = char::from_u32(code_point)
                    {
                        result.push(c);
                    }
                }
                Ok(vm.ctx.new_str(result).into())
            }
            // Other types → list (PyList_New + Array_item for each)
            _ => {
                let iter = SaturatedSliceIter::from_adjust_indices(range, step, slice_len);
                let mut result = Vec::with_capacity(slice_len);
                for idx in iter {
                    result.push(Self::getitem_by_index(zelf, idx as isize, vm)?);
                }
                Ok(PyList::from(result).into_ref(&vm.ctx).into())
            }
        }
    }

    // Array_ass_subscript
    #[pymethod]
    fn __setitem__(
        zelf: &Py<Self>,
        item: PyObjectRef,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        // Array does not support item deletion
        // (handled implicitly - value is always provided in __setitem__)

        // PyIndex_Check
        if let Some(i) = item.downcast_ref::<PyInt>() {
            let i = i.as_bigint().to_isize().ok_or_else(|| {
                vm.new_index_error("cannot fit index into an index-sized integer")
            })?;
            // setitem_by_index handles negative index normalization
            Self::setitem_by_index(zelf, i, value, vm)
        }
        // PySlice_Check
        else if let Some(slice) = item.downcast_ref::<PySlice>() {
            Self::setitem_by_slice(zelf, slice, value, vm)
        } else {
            Err(vm.new_type_error("indices must be integer"))
        }
    }

    // Array does not support item deletion
    #[pymethod]
    fn __delitem__(&self, _item: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        Err(vm.new_type_error("Array does not support item deletion"))
    }

    // Array_ass_subscript slice handling
    fn setitem_by_slice(
        zelf: &Py<Self>,
        slice: &PySlice,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        use crate::sliceable::SaturatedSliceIter;

        let length = zelf.class().stg_info_opt().map_or(0, |i| i.length);

        // PySlice_Unpack + PySlice_AdjustIndices
        let sat_slice = slice.to_saturated(vm)?;
        let (range, step, slice_len) = sat_slice.adjust_indices(length);

        // other_len = PySequence_Length(value);
        let items: Vec<PyObjectRef> = vm.extract_elements_with(&value, Ok)?;
        let other_len = items.len();

        if other_len != slice_len {
            return Err(vm.new_value_error("Can only assign sequence of same size"));
        }

        // Use SaturatedSliceIter for correct index iteration (handles negative step)
        let iter = SaturatedSliceIter::from_adjust_indices(range, step, slice_len);

        for (idx, item) in iter.zip(items) {
            Self::setitem_by_index(zelf, idx as isize, item, vm)?;
        }
        Ok(())
    }

    fn __len__(zelf: &Py<Self>, _vm: &VirtualMachine) -> usize {
        zelf.class().stg_info_opt().map_or(0, |i| i.length)
    }
}

impl AsBuffer for PyCArray {
    fn as_buffer(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<PyBuffer> {
        let buffer_len = zelf.0.buffer.read().len();

        // Get format and shape from type's StgInfo
        let stg_info = zelf
            .class()
            .stg_info_opt()
            .expect("PyCArray type must have StgInfo");
        let format = stg_info.format.clone();
        let shape = stg_info.shape.clone();

        let desc = if let Some(fmt) = format
            && !shape.is_empty()
        {
            // itemsize is the size of the base element type (item_info->size)
            // For empty arrays, we still need the element size, not 0
            let total_elements: usize = shape.iter().product();
            let has_zero_dim = shape.contains(&0);
            let itemsize = if total_elements > 0 && buffer_len > 0 {
                buffer_len / total_elements
            } else {
                // For empty arrays, get itemsize from format type code
                get_size_from_format(&fmt)
            };

            // Build dim_desc from shape (C-contiguous: row-major order)
            // stride[i] = product(shape[i+1:]) * itemsize
            // For empty arrays (any dimension is 0), all strides are 0
            let mut dim_desc = Vec::with_capacity(shape.len());
            let mut stride = itemsize as isize;

            for &dim_size in shape.iter().rev() {
                let current_stride = if has_zero_dim { 0 } else { stride };
                dim_desc.push((dim_size, current_stride, 0));
                stride *= dim_size as isize;
            }
            dim_desc.reverse();

            BufferDescriptor {
                len: buffer_len,
                readonly: false,
                itemsize,
                format: alloc::borrow::Cow::Owned(fmt),
                dim_desc,
            }
        } else {
            // Fallback to simple buffer if no format/shape info
            BufferDescriptor::simple(buffer_len, false)
        };

        let buf = PyBuffer::new(zelf.to_owned().into(), desc, &CDATA_BUFFER_METHODS);
        Ok(buf)
    }
}

// CharArray and WCharArray getsets - added dynamically via add_getset

// CharArray_get_value
fn char_array_get_value(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    let zelf = obj.downcast_ref::<PyCArray>().unwrap();
    let buffer = zelf.0.buffer.read();
    let len = buffer.iter().position(|&b| b == 0).unwrap_or(buffer.len());
    Ok(vm.ctx.new_bytes(buffer[..len].to_vec()).into())
}

// CharArray_set_value
fn char_array_set_value(obj: PyObjectRef, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
    let zelf = obj.downcast_ref::<PyCArray>().unwrap();
    let bytes = value
        .downcast_ref::<PyBytes>()
        .ok_or_else(|| vm.new_type_error("bytes expected"))?;
    let mut buffer = zelf.0.buffer.write();
    let src = bytes.as_bytes();

    if src.len() > buffer.len() {
        return Err(vm.new_value_error("byte string too long"));
    }

    buffer.to_mut()[..src.len()].copy_from_slice(src);
    if src.len() < buffer.len() {
        buffer.to_mut()[src.len()] = 0;
    }
    Ok(())
}

// CharArray_get_raw
fn char_array_get_raw(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    let zelf = obj.downcast_ref::<PyCArray>().unwrap();
    let buffer = zelf.0.buffer.read();
    Ok(vm.ctx.new_bytes(buffer.to_vec()).into())
}

// CharArray_set_raw
fn char_array_set_raw(
    obj: PyObjectRef,
    value: PySetterValue<PyObjectRef>,
    vm: &VirtualMachine,
) -> PyResult<()> {
    let value = value.ok_or_else(|| vm.new_attribute_error("cannot delete attribute"))?;
    let zelf = obj.downcast_ref::<PyCArray>().unwrap();
    let bytes_like = ArgBytesLike::try_from_object(vm, value)?;
    let mut buffer = zelf.0.buffer.write();
    let src = bytes_like.borrow_buf();
    if src.len() > buffer.len() {
        return Err(vm.new_value_error("byte string too long"));
    }
    buffer.to_mut()[..src.len()].copy_from_slice(&src);
    Ok(())
}

// WCharArray_get_value
fn wchar_array_get_value(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    let zelf = obj.downcast_ref::<PyCArray>().unwrap();
    let buffer = zelf.0.buffer.read();
    Ok(vm.ctx.new_str(wstring_from_bytes(&buffer)).into())
}

// WCharArray_set_value
fn wchar_array_set_value(
    obj: PyObjectRef,
    value: PyObjectRef,
    vm: &VirtualMachine,
) -> PyResult<()> {
    let zelf = obj.downcast_ref::<PyCArray>().unwrap();
    let s = value
        .downcast_ref::<PyStr>()
        .ok_or_else(|| vm.new_type_error("unicode string expected"))?;
    let mut buffer = zelf.0.buffer.write();
    let wchar_count = buffer.len() / WCHAR_SIZE;
    let char_count = s.as_str().chars().count();

    if char_count > wchar_count {
        return Err(vm.new_value_error("string too long"));
    }

    for (i, ch) in s.as_str().chars().enumerate() {
        let offset = i * WCHAR_SIZE;
        wchar_to_bytes(ch as u32, &mut buffer.to_mut()[offset..]);
    }

    let terminator_offset = char_count * WCHAR_SIZE;
    if terminator_offset + WCHAR_SIZE <= buffer.len() {
        wchar_to_bytes(0, &mut buffer.to_mut()[terminator_offset..]);
    }
    Ok(())
}

/// add_getset for c_char arrays - adds 'value' and 'raw' attributes
/// add_getset((PyTypeObject*)self, CharArray_getsets)
fn add_char_array_getsets(array_type: &Py<PyType>, vm: &VirtualMachine) {
    // SAFETY: getset is owned by array_type which outlives the getset
    let value_getset = unsafe {
        vm.ctx.new_getset(
            "value",
            array_type,
            char_array_get_value,
            char_array_set_value,
        )
    };
    let raw_getset = unsafe {
        vm.ctx
            .new_getset("raw", array_type, char_array_get_raw, char_array_set_raw)
    };

    array_type
        .attributes
        .write()
        .insert(vm.ctx.intern_str("value"), value_getset.into());
    array_type
        .attributes
        .write()
        .insert(vm.ctx.intern_str("raw"), raw_getset.into());
}

/// add_getset for c_wchar arrays - adds only 'value' attribute (no 'raw')
fn add_wchar_array_getsets(array_type: &Py<PyType>, vm: &VirtualMachine) {
    // SAFETY: getset is owned by array_type which outlives the getset
    let value_getset = unsafe {
        vm.ctx.new_getset(
            "value",
            array_type,
            wchar_array_get_value,
            wchar_array_set_value,
        )
    };

    array_type
        .attributes
        .write()
        .insert(vm.ctx.intern_str("value"), value_getset.into());
}

// wchar_t helpers - Platform-independent wide character handling
// Windows: sizeof(wchar_t) == 2 (UTF-16)
// Linux/macOS: sizeof(wchar_t) == 4 (UTF-32)

/// Size of wchar_t on this platform
pub(super) const WCHAR_SIZE: usize = core::mem::size_of::<libc::wchar_t>();

/// Read a single wchar_t from bytes (platform-endian)
#[inline]
pub(super) fn wchar_from_bytes(bytes: &[u8]) -> Option<u32> {
    if bytes.len() < WCHAR_SIZE {
        return None;
    }
    Some(if WCHAR_SIZE == 2 {
        u16::from_ne_bytes([bytes[0], bytes[1]]) as u32
    } else {
        u32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
    })
}

/// Write a single wchar_t to bytes (platform-endian)
#[inline]
pub(super) fn wchar_to_bytes(ch: u32, buffer: &mut [u8]) {
    if WCHAR_SIZE == 2 {
        if buffer.len() >= 2 {
            buffer[..2].copy_from_slice(&(ch as u16).to_ne_bytes());
        }
    } else if buffer.len() >= 4 {
        buffer[..4].copy_from_slice(&ch.to_ne_bytes());
    }
}

/// Read a null-terminated wchar_t string from bytes, returns String
fn wstring_from_bytes(buffer: &[u8]) -> String {
    let mut chars = Vec::new();
    for chunk in buffer.chunks(WCHAR_SIZE) {
        if chunk.len() < WCHAR_SIZE {
            break;
        }
        let code = if WCHAR_SIZE == 2 {
            u16::from_ne_bytes([chunk[0], chunk[1]]) as u32
        } else {
            u32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])
        };
        if code == 0 {
            break; // null terminator
        }
        if let Some(ch) = char::from_u32(code) {
            chars.push(ch);
        }
    }
    chars.into_iter().collect()
}

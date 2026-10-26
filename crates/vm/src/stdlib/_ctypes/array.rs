use super::StgInfo;
use super::base::{CDATA_BUFFER_METHODS, PyCData};
use crate::common::lock::LazyLock;
use crate::sliceable::SaturatedSliceIter;
use crate::{
    AsObject, Py, PyObject, PyObjectRef, PyPayload, PyResult, TryFromObject, VirtualMachine,
    atomic_func,
    builtins::{
        PyBytes, PyInt, PyList, PySlice, PyStr, PyType, PyTypeRef, genericalias::PyGenericAlias,
    },
    class::StaticType,
    function::{ArgBytesLike, FuncArgs, PySetterValue},
    protocol::{BufferDescriptor, PyBuffer, PyMappingMethods, PySequenceMethods},
    types::{AsBuffer, AsMapping, AsSequence, Constructor, Initializer},
};
use alloc::borrow::Cow;
use num_traits::{Signed, ToPrimitive};

use rustpython_host_env::ctypes::{
    DecodedValue, WCHAR_SIZE, WCharArrayWriteError, char_array_field_value, read_array_element,
    simple_type_size, wchar_from_bytes, write_char_array_raw, write_char_array_value,
    write_wchar_array_value, wstring_from_bytes,
};

fn cow_bytes_as_mut<'buf>(buffer: &'buf mut Cow<'_, [u8]>) -> &'buf mut [u8] {
    match buffer {
        Cow::Borrowed(slice) => unsafe {
            rustpython_host_env::ctypes::borrowed_slice_as_mut(slice)
        },
        Cow::Owned(vec) => vec.as_mut_slice(),
    }
}

/// Get itemsize from a PEP 3118 format string
/// Extracts the type code (last char after endianness prefix) and returns its size
fn get_size_from_format(fmt: &str) -> usize {
    // Format is like "<f", ">q", etc. - strip endianness prefix and get type code
    let code = fmt
        .trim_start_matches(['<', '>', '@', '=', '!', '&'])
        .chars()
        .next()
        .map(|c| c.to_string());
    code.map_or(1, |c| simple_type_size(&c).unwrap_or(1))
}

pub(super) static CTYPE_SEQUENCE_METHODS: PySequenceMethods = PySequenceMethods {
    repeat: Some(|seq, length, vm| repeat_type(seq.obj, length, vm)),
    ..PySequenceMethods::NOT_IMPLEMENTED
};

pub(super) fn repeat_type(ctype: &PyObject, length: isize, vm: &VirtualMachine) -> PyResult {
    if length < 0 {
        return Err(vm.new_value_error(format!("Array length must be >= 0, not {length}")));
    }
    let cls = ctype
        .downcast_ref::<PyType>()
        .ok_or_else(|| vm.new_type_error("expected type"))?;
    if let Some(info) = cls.stg_info_opt()
        && info.size != 0
        && length as usize > isize::MAX as usize / info.size
    {
        return Err(vm.new_overflow_error("array too large"));
    }
    array_type_from_ctype(ctype.to_owned(), length as usize, vm)
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

// PyCArrayType - metatype for Array types
#[pyclass(name = "PyCArrayType", base = PyType, module = "_ctypes")]
#[derive(Debug)]
#[repr(transparent)]
pub(super) struct PyCArrayType(PyType);

// PyCArrayType implements Initializer for slots.init (PyCArrayType_init)
impl Initializer for PyCArrayType {
    type Args = FuncArgs;

    fn init(zelf: &Py<Self>, _args: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
        // zelf is the newly created array type (e.g., T in "class T(Array)")
        let new_type: &PyType = &zelf.0;

        new_type.check_not_initialized(vm)?;

        // 1. Get _length_ from class dict first
        let direct_length = new_type.attributes.get(vm.ctx.intern_str("_length_"));

        // 2. Get _type_ from class dict first
        let direct_type = new_type.attributes.get(vm.ctx.intern_str("_type_"));

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

#[pyclass(flags(IMMUTABLETYPE), with(Initializer, AsSequence))]
impl PyCArrayType {
    #[pygetset(name = "__pointer_type__")]
    fn pointer_type(zelf: PyTypeRef, vm: &VirtualMachine) -> PyResult {
        super::base::pointer_type_get(&zelf, vm)
    }

    #[pygetset(name = "__pointer_type__", setter)]
    fn set_pointer_type(zelf: PyTypeRef, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        super::base::pointer_type_set(&zelf, value, vm)
    }

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
            return Self::from_param(cls.as_object().to_owned(), as_parameter, vm);
        }

        Err(vm.new_type_error(format!(
            "expected {} instance instead of {}",
            cls.name(),
            value.class().name()
        )))
    }
}

impl AsSequence for PyCArrayType {
    fn as_sequence() -> &'static PySequenceMethods {
        &super::array::CTYPE_SEQUENCE_METHODS
    }
}

// All array metadata (element_type, length, element_size) is stored in the type's StgInfo
#[pyclass(
    name = "Array",
    base = PyCData,
    metaclass = "PyCArrayType",
    module = "_ctypes"
, traverse = "manual")]
#[derive(Debug)]
#[repr(transparent)]
pub(crate) struct PyCArray(pub PyCData);

// SAFETY: this transparent wrapper owns exactly the references in its base.
unsafe impl crate::object::Traverse for PyCArray {
    fn traverse(&self, tracer: &mut crate::object::TraverseFn<'_>) {
        self.0.traverse(tracer);
    }
    fn clear(&mut self, out: &mut Vec<PyObjectRef>) {
        self.0.clear(out);
    }
}

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
        let buffer = vm.new_zeroed_bytes(total_size)?;
        let instance = Self(PyCData::from_bytes_with_length(buffer, None, length))
            .into_ref_with_type(vm, cls)?;

        // Initialize elements using setitem_by_index (Array_init pattern)
        for (i, value) in args.args.iter().enumerate() {
            Self::setitem_by_index(&instance, i as isize, value, vm)?;
        }

        Ok(instance.into())
    }

    fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
        unimplemented!("use slot_new")
    }
}

impl Initializer for PyCArray {
    type Args = FuncArgs;

    fn init(zelf: &Py<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
        // Re-initialize array elements when __init__ is called
        for (i, value) in args.args.iter().enumerate() {
            Self::setitem_by_index(zelf, i as isize, value, vm)?;
        }
        Ok(())
    }
}

impl AsSequence for PyCArray {
    fn as_sequence() -> &'static PySequenceMethods {
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
                    Some(v) => PyCArray::setitem_by_index(zelf, i, &v, vm),
                    None => Err(vm.new_type_error("cannot delete array elements")),
                }
            }),
            ..PySequenceMethods::NOT_IMPLEMENTED
        });
        &AS_SEQUENCE
    }
}

impl AsMapping for PyCArray {
    fn as_mapping() -> &'static PyMappingMethods {
        static AS_MAPPING: LazyLock<PyMappingMethods> = LazyLock::new(|| PyMappingMethods {
            length: atomic_func!(|mapping, _vm| {
                let zelf = PyCArray::mapping_downcast(mapping);
                Ok(zelf.class().stg_info_opt().map_or(0, |i| i.length))
            }),
            subscript: atomic_func!(|mapping, needle, vm| {
                let zelf = PyCArray::mapping_downcast(mapping);
                PyCArray::__getitem__(zelf, needle, vm)
            }),
            ass_subscript: atomic_func!(|mapping, needle, value, vm| {
                let zelf = PyCArray::mapping_downcast(mapping);
                match value {
                    Some(value) => PyCArray::__setitem__(zelf, needle, value, vm),
                    None => PyCArray::__delitem__(zelf, needle.to_owned(), vm),
                }
            }),
        });
        &AS_MAPPING
    }
}

#[pyclass(
    flags(BASETYPE, IMMUTABLETYPE),
    with(Constructor, Initializer, AsSequence, AsMapping, AsBuffer)
)]
impl PyCArray {
    #[pyclassmethod]
    fn __class_getitem__(
        cls: PyTypeRef,
        object: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyGenericAlias> {
        PyGenericAlias::from_args(cls, object, vm)
    }

    fn getitem_by_index(zelf: &Py<Self>, i: isize, vm: &VirtualMachine) -> PyResult {
        let stg = zelf.class().stg_info_opt();
        let length = stg.as_ref().map_or(0, |i| i.length) as isize;
        let index = if i < 0 { length + i } else { i };
        if index < 0 || index >= length {
            return Err(vm.new_index_error("invalid index"));
        }
        let index = index as usize;
        let element_size = stg.as_ref().map_or(0, |i| i.element_size);
        let offset = index * element_size;
        if let Some(element_type) = stg.as_ref().and_then(|info| info.element_type.as_ref())
            && !super::simple::returns_scalar(element_type)
        {
            let length = element_type.stg_info(vm)?.length;
            let buffer = zelf.0.buffer.read();
            let Some(data) = buffer.get(offset..offset + element_size) else {
                drop(buffer);
                return Err(vm.new_index_error("invalid index"));
            };
            // SAFETY: this element lies within the array's live buffer. The view
            // retains the array; ctypes users must still avoid reallocating or
            // concurrently accessing the same native memory through aliases.
            let view = unsafe {
                PyCData::from_base_obj(
                    data.as_ptr().cast_mut(),
                    element_size,
                    zelf.to_owned().into(),
                    index,
                )
            };
            view.base_offset.store(offset);
            view.length.store(length);
            drop(buffer);
            return view.into_view(element_type.clone(), vm);
        }
        let type_code = Self::get_element_type_code(zelf, vm);

        let buffer = zelf.0.buffer.read();
        if type_code.as_deref() == Some("O") {
            let address = rustpython_host_env::ctypes::read_pointer_from_buffer(
                &buffer[offset..offset + element_size],
            );
            drop(buffer);
            return zelf.0.object_at(address, Some(index), vm);
        }
        if let Some(element_type) = stg.as_ref().and_then(|info| info.element_type.as_ref())
            && super::simple::is_swapped(element_type)
        {
            let mut bytes = buffer[offset..offset + element_size].to_vec();
            bytes.reverse();
            return Ok(super::base::bytes_to_pyobject(element_type, &bytes, vm));
        }
        Ok(Self::read_element_from_buffer(
            &buffer,
            offset,
            element_size,
            type_code.as_deref(),
            vm,
        ))
    }

    /// Helper to read an element value from a buffer at given offset
    fn read_element_from_buffer(
        buffer: &[u8],
        offset: usize,
        element_size: usize,
        type_code: Option<&str>,
        vm: &VirtualMachine,
    ) -> PyObjectRef {
        match read_array_element(buffer, offset, element_size, type_code) {
            DecodedValue::Bytes(bytes) => vm.ctx.new_bytes(bytes).into(),
            DecodedValue::String(value) => vm.ctx.new_str(value).into(),
            DecodedValue::Float(value) => vm.ctx.new_float(value).into(),
            DecodedValue::Signed(value) => vm.ctx.new_int(value).into(),
            DecodedValue::Unsigned(value) => vm.ctx.new_int(value).into(),
            DecodedValue::None => vm.ctx.none(),
            DecodedValue::Pointer(value) => vm.ctx.new_int(value).into(),
            DecodedValue::Bool(value) => vm.ctx.new_bool(value).into(),
        }
    }

    fn setitem_by_index(
        zelf: &Py<Self>,
        i: isize,
        value: &PyObject,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let (length, element_size, element_type) = {
            let stg = zelf.class().stg_info(vm)?;
            (
                stg.length as isize,
                stg.element_size,
                stg.element_type
                    .clone()
                    .ok_or_else(|| vm.new_type_error("no element type"))?,
            )
        };
        let index = if i < 0 { length + i } else { i };
        if index < 0 || index >= length {
            return Err(vm.new_index_error("invalid index"));
        }
        let index = index as usize;
        let prepared = super::base::CDataValue::new(
            &element_type,
            value,
            element_size,
            super::simple::is_swapped(&element_type),
            vm,
        )?;
        // A view's buffer already points to its element. Its base can be a
        // pointer object, whose own buffer stores an address rather than data.
        zelf.0
            .write_bytes_at_offset(index * element_size, &prepared.bytes);
        prepared.keep(&zelf.0, index, vm)
    }

    // Array_subscript
    fn __getitem__(zelf: &Py<Self>, item: &PyObject, vm: &VirtualMachine) -> PyResult {
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
    fn getitem_by_slice(zelf: &Py<Self>, slice: &Py<PySlice>, vm: &VirtualMachine) -> PyResult {
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
    fn __setitem__(
        zelf: &Py<Self>,
        item: &PyObject,
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
            Self::setitem_by_index(zelf, i, &value, vm)
        }
        // PySlice_Check
        else if let Some(slice) = item.downcast_ref::<PySlice>() {
            Self::setitem_by_slice(zelf, slice, &value, vm)
        } else {
            Err(vm.new_type_error("indices must be integer"))
        }
    }

    // Array does not support item deletion
    fn __delitem__(&self, _item: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        Err(vm.new_type_error("Array does not support item deletion"))
    }

    // Array_ass_subscript slice handling
    fn setitem_by_slice(
        zelf: &Py<Self>,
        slice: &Py<PySlice>,
        value: &PyObject,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let length = zelf.class().stg_info_opt().map_or(0, |i| i.length);

        // PySlice_Unpack + PySlice_AdjustIndices
        let sat_slice = slice.to_saturated(vm)?;
        let (range, step, slice_len) = sat_slice.adjust_indices(length);

        // other_len = PySequence_Length(value);
        // Size the operand before consuming it so an unbounded iterable is
        // rejected without being materialized.
        let other_len = value
            .sequence_unchecked()
            .length(vm)
            .map_err(|_| vm.new_value_error("Can only assign sequence of same size"))?;

        if other_len != slice_len {
            return Err(vm.new_value_error("Can only assign sequence of same size"));
        }

        let items: Vec<PyObjectRef> = vm.extract_elements_with(value, Ok)?;

        // Use SaturatedSliceIter for correct index iteration (handles negative step)
        let iter = SaturatedSliceIter::from_adjust_indices(range, step, slice_len);

        for (idx, item) in iter.zip(items) {
            Self::setitem_by_index(zelf, idx as isize, &item, vm)?;
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

            BufferDescriptor::from_dim_desc(
                buffer_len,
                0,
                false,
                itemsize,
                alloc::borrow::Cow::Owned(fmt),
                dim_desc,
            )
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
fn char_array_get_value(obj: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
    let zelf = obj.downcast_ref::<PyCArray>().unwrap();
    let buffer = zelf.0.buffer.read();
    vm.ctx
        .new_bytes(char_array_field_value(&buffer).to_vec())
        .into()
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

    write_char_array_value(cow_bytes_as_mut(&mut buffer), src);
    Ok(())
}

// CharArray_get_raw
fn char_array_get_raw(obj: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
    let zelf = obj.downcast_ref::<PyCArray>().unwrap();
    let buffer = zelf.0.buffer.read();
    vm.ctx.new_bytes(buffer.to_vec()).into()
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
    write_char_array_raw(cow_bytes_as_mut(&mut buffer), &src);
    Ok(())
}

// WCharArray_get_value
fn wchar_array_get_value(obj: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
    let zelf = obj.downcast_ref::<PyCArray>().unwrap();
    let buffer = zelf.0.buffer.read();
    vm.ctx.new_str(wstring_from_bytes(&buffer)).into()
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
    write_wchar_array_value(cow_bytes_as_mut(&mut buffer), s.as_wtf8()).map_err(
        |err| match err {
            WCharArrayWriteError::TooLong => vm.new_value_error("string too long"),
        },
    )?;
    Ok(())
}

/// add_getset for c_char arrays - adds 'value' and 'raw' attributes
/// add_getset((PyTypeObject*)self, CharArray_getsets)
fn add_char_array_getsets(array_type: &Py<PyType>, vm: &VirtualMachine) {
    let value_getset = vm.ctx.new_getset(
        "value",
        array_type,
        char_array_get_value,
        char_array_set_value,
    );
    let raw_getset = vm
        .ctx
        .new_getset("raw", array_type, char_array_get_raw, char_array_set_raw);

    array_type
        .attributes
        .insert(vm.ctx.intern_str("value"), value_getset.into());
    array_type
        .attributes
        .insert(vm.ctx.intern_str("raw"), raw_getset.into());
}

/// add_getset for c_wchar arrays - adds only 'value' attribute (no 'raw')
fn add_wchar_array_getsets(array_type: &Py<PyType>, vm: &VirtualMachine) {
    let value_getset = vm.ctx.new_getset(
        "value",
        array_type,
        wchar_array_get_value,
        wchar_array_set_value,
    );

    array_type
        .attributes
        .insert(vm.ctx.intern_str("value"), value_getset.into());
}

use super::base::CDATA_BUFFER_METHODS;
use super::{PyCArray, PyCData, PyCSimple, PyCStructure, StgInfo, StgInfoFlags};
use crate::protocol::{BufferDescriptor, PyBuffer, PyNumberMethods};
use crate::types::{AsBuffer, AsNumber, Constructor, Initializer};
use crate::{
    AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    builtins::{PyBytes, PyInt, PyList, PySlice, PyStr, PyType, PyTypeRef},
    class::StaticType,
    function::{FuncArgs, OptionalArg},
};
use num_traits::ToPrimitive;
use std::borrow::Cow;

#[pyclass(name = "PyCPointerType", base = PyType, module = "_ctypes")]
#[derive(Debug)]
#[repr(transparent)]
pub(super) struct PyCPointerType(PyType);

impl Initializer for PyCPointerType {
    type Args = FuncArgs;

    fn init(zelf: crate::PyRef<Self>, _args: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
        // Get the type as PyTypeRef
        let obj: PyObjectRef = zelf.clone().into();
        let new_type: PyTypeRef = obj
            .downcast()
            .map_err(|_| vm.new_type_error("expected type"))?;

        new_type.check_not_initialized(vm)?;

        // Get the _type_ attribute (element type)
        // PyCPointerType_init gets the element type from _type_ attribute
        let proto = new_type
            .as_object()
            .get_attr("_type_", vm)
            .ok()
            .and_then(|obj| obj.downcast::<PyType>().ok());

        // Initialize StgInfo for pointer type
        let pointer_size = std::mem::size_of::<usize>();
        let mut stg_info = StgInfo::new(pointer_size, pointer_size);
        stg_info.proto = proto;
        stg_info.paramfunc = super::base::ParamFunc::Pointer;
        stg_info.length = 1;
        stg_info.flags |= StgInfoFlags::TYPEFLAG_ISPOINTER;

        // Set format string: "&<element_format>" or "&(shape)<element_format>" for arrays
        if let Some(ref proto) = stg_info.proto
            && let Some(item_info) = proto.stg_info_opt()
        {
            let current_format = item_info.format.as_deref().unwrap_or("B");
            // Include shape for array types in the pointer format
            let shape_str = if !item_info.shape.is_empty() {
                let dims: Vec<String> = item_info.shape.iter().map(|d| d.to_string()).collect();
                format!("({})", dims.join(","))
            } else {
                String::new()
            };
            stg_info.format = Some(format!("&{}{}", shape_str, current_format));
        }

        let _ = new_type.init_type_data(stg_info);

        Ok(())
    }
}

#[pyclass(flags(IMMUTABLETYPE), with(AsNumber, Initializer))]
impl PyCPointerType {
    #[pymethod]
    fn from_param(zelf: PyObjectRef, value: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        // zelf is the pointer type class that from_param was called on
        let cls = zelf
            .downcast::<PyType>()
            .map_err(|_| vm.new_type_error("from_param: expected a type"))?;

        // 1. None is allowed for pointer types
        if vm.is_none(&value) {
            return Ok(value);
        }

        // 1.5 CArgObject (from byref()) - check if underlying obj is instance of _type_
        if let Some(carg) = value.downcast_ref::<super::_ctypes::CArgObject>()
            && let Ok(type_attr) = cls.as_object().get_attr("_type_", vm)
            && let Ok(type_ref) = type_attr.downcast::<PyType>()
            && carg.obj.is_instance(type_ref.as_object(), vm)?
        {
            return Ok(value);
        }

        // 2. If already an instance of the requested type, return it
        if value.is_instance(cls.as_object(), vm)? {
            return Ok(value);
        }

        // 3. If value is an instance of _type_ (the pointed-to type), wrap with byref
        if let Ok(type_attr) = cls.as_object().get_attr("_type_", vm)
            && let Ok(type_ref) = type_attr.downcast::<PyType>()
            && value.is_instance(type_ref.as_object(), vm)?
        {
            // Return byref(value)
            return super::_ctypes::byref(value, crate::function::OptionalArg::Missing, vm);
        }

        // 4. Array/Pointer instances with compatible proto
        // "Array instances are also pointers when the item types are the same."
        let is_pointer_or_array = value.downcast_ref::<PyCPointer>().is_some()
            || value.downcast_ref::<super::array::PyCArray>().is_some();

        if is_pointer_or_array {
            let is_compatible = {
                if let Some(value_stginfo) = value.class().stg_info_opt()
                    && let Some(ref value_proto) = value_stginfo.proto
                    && let Some(cls_stginfo) = cls.stg_info_opt()
                    && let Some(ref cls_proto) = cls_stginfo.proto
                {
                    // Check if value's proto is a subclass of target's proto
                    value_proto.fast_issubclass(cls_proto)
                } else {
                    false
                }
            };
            if is_compatible {
                return Ok(value);
            }
        }

        // 5. Check for _as_parameter_ attribute
        if let Ok(as_parameter) = value.get_attr("_as_parameter_", vm) {
            return PyCPointerType::from_param(cls.as_object().to_owned(), as_parameter, vm);
        }

        Err(vm.new_type_error(format!(
            "expected {} instance instead of {}",
            cls.name(),
            value.class().name()
        )))
    }

    #[pymethod]
    fn __mul__(cls: PyTypeRef, n: isize, vm: &VirtualMachine) -> PyResult {
        use super::array::array_type_from_ctype;

        if n < 0 {
            return Err(vm.new_value_error(format!("Array length must be >= 0, not {n}")));
        }
        // Use cached array type creation
        array_type_from_ctype(cls.into(), n as usize, vm)
    }

    // PyCPointerType_set_type: Complete an incomplete pointer type
    #[pymethod]
    fn set_type(zelf: PyTypeRef, typ: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        use crate::AsObject;

        // 1. Validate that typ is a type
        let typ_type = typ
            .clone()
            .downcast::<PyType>()
            .map_err(|_| vm.new_type_error("_type_ must be a type"))?;

        // 2. Validate that typ has storage info
        if typ_type.stg_info_opt().is_none() {
            return Err(vm.new_type_error("_type_ must have storage info"));
        }

        // 3. Update StgInfo.proto and format using mutable access
        if let Some(mut stg_info) = zelf.get_type_data_mut::<StgInfo>() {
            stg_info.proto = Some(typ_type.clone());

            // Update format string: "&<element_format>" or "&(shape)<element_format>" for arrays
            let item_info = typ_type.stg_info_opt().expect("proto has StgInfo");
            let current_format = item_info.format.as_deref().unwrap_or("B");
            // Include shape for array types in the pointer format
            let shape_str = if !item_info.shape.is_empty() {
                let dims: Vec<String> = item_info.shape.iter().map(|d| d.to_string()).collect();
                format!("({})", dims.join(","))
            } else {
                String::new()
            };
            stg_info.format = Some(format!("&{}{}", shape_str, current_format));
        }

        // 4. Set _type_ attribute on the pointer type
        zelf.as_object().set_attr("_type_", typ_type, vm)?;

        Ok(())
    }
}

impl AsNumber for PyCPointerType {
    fn as_number() -> &'static PyNumberMethods {
        static AS_NUMBER: PyNumberMethods = PyNumberMethods {
            multiply: Some(|a, b, vm| {
                let cls = a
                    .downcast_ref::<PyType>()
                    .ok_or_else(|| vm.new_type_error("expected type"))?;
                let n = b
                    .try_index(vm)?
                    .as_bigint()
                    .to_isize()
                    .ok_or_else(|| vm.new_overflow_error("array size too large"))?;
                PyCPointerType::__mul__(cls.to_owned(), n, vm)
            }),
            ..PyNumberMethods::NOT_IMPLEMENTED
        };
        &AS_NUMBER
    }
}

/// PyCPointer - Pointer instance
/// `contents` is a computed property, not a stored field.
#[pyclass(
    name = "_Pointer",
    base = PyCData,
    metaclass = "PyCPointerType",
    module = "_ctypes"
)]
#[derive(Debug)]
#[repr(transparent)]
pub struct PyCPointer(pub PyCData);

impl Constructor for PyCPointer {
    type Args = FuncArgs;

    fn slot_new(cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        // Pointer_new: Check if _type_ is defined
        let has_type = cls.stg_info_opt().is_some_and(|info| info.proto.is_some());
        if !has_type {
            return Err(vm.new_type_error("Cannot create instance: has no _type_"));
        }

        // Create a new PyCPointer instance with NULL pointer (all zeros)
        // Initial contents is set via __init__ if provided
        let cdata = PyCData::from_bytes(vec![0u8; std::mem::size_of::<usize>()], None);
        // pointer instance has b_length set to 2 (for index 0 and 1)
        cdata.length.store(2);
        PyCPointer(cdata)
            .into_ref_with_type(vm, cls)
            .map(Into::into)
    }

    fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
        unimplemented!("use slot_new")
    }
}

impl Initializer for PyCPointer {
    type Args = (OptionalArg<PyObjectRef>,);

    fn init(zelf: PyRef<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
        let (value,) = args;
        if let OptionalArg::Present(val) = value
            && !vm.is_none(&val)
        {
            Self::set_contents(&zelf, val, vm)?;
        }
        Ok(())
    }
}

#[pyclass(
    flags(BASETYPE, IMMUTABLETYPE),
    with(Constructor, Initializer, AsBuffer)
)]
impl PyCPointer {
    /// Get the pointer value stored in buffer as usize
    pub fn get_ptr_value(&self) -> usize {
        let buffer = self.0.buffer.read();
        super::base::read_ptr_from_buffer(&buffer)
    }

    /// Set the pointer value in buffer
    pub fn set_ptr_value(&self, value: usize) {
        let mut buffer = self.0.buffer.write();
        let bytes = value.to_ne_bytes();
        if buffer.len() >= bytes.len() {
            buffer.to_mut()[..bytes.len()].copy_from_slice(&bytes);
        }
    }

    /// Pointer_bool: returns True if pointer is not NULL
    #[pymethod]
    fn __bool__(&self) -> bool {
        self.get_ptr_value() != 0
    }

    /// contents getter - reads address from b_ptr and creates an instance of the pointed-to type
    #[pygetset]
    fn contents(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        // Pointer_get_contents
        let ptr_val = zelf.get_ptr_value();
        if ptr_val == 0 {
            return Err(vm.new_value_error("NULL pointer access"));
        }

        // Get element type from StgInfo.proto
        let stg_info = zelf.class().stg_info(vm)?;
        let proto_type = stg_info.proto();
        let element_size = proto_type
            .stg_info_opt()
            .map_or(std::mem::size_of::<usize>(), |info| info.size);

        // Create instance that references the memory directly
        // PyCData.into_ref_with_type works for all ctypes (simple, structure, union, array, pointer)
        let cdata = unsafe { super::base::PyCData::at_address(ptr_val as *const u8, element_size) };
        cdata
            .into_ref_with_type(vm, proto_type.to_owned())
            .map(Into::into)
    }

    /// contents setter - stores address in b_ptr and keeps reference
    /// Pointer_set_contents
    #[pygetset(setter)]
    fn set_contents(zelf: &Py<Self>, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        // Get stginfo and proto for type validation
        let stg_info = zelf.class().stg_info(vm)?;
        let proto = stg_info.proto();

        // Check if value is CData, or isinstance(value, proto)
        let cdata = if let Some(c) = value.downcast_ref::<PyCData>() {
            c
        } else if value.is_instance(proto.as_object(), vm)? {
            value
                .downcast_ref::<PyCData>()
                .ok_or_else(|| vm.new_type_error("expected ctypes instance"))?
        } else {
            return Err(vm.new_type_error(format!(
                "expected {} instead of {}",
                proto.name(),
                value.class().name()
            )));
        };

        // Set pointer value
        {
            let buffer = cdata.buffer.read();
            let addr = buffer.as_ptr() as usize;
            drop(buffer);
            zelf.set_ptr_value(addr);
        }

        // KeepRef: store the object directly with index 1
        zelf.0.keep_ref(1, value.clone(), vm)?;

        // KeepRef: store GetKeepedObjects(dst) at index 0
        if let Some(kept) = cdata.objects.read().clone() {
            zelf.0.keep_ref(0, kept, vm)?;
        }

        Ok(())
    }

    // Pointer_subscript
    #[pymethod]
    fn __getitem__(zelf: &Py<Self>, item: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        // PyIndex_Check
        if let Some(i) = item.downcast_ref::<PyInt>() {
            let i = i.as_bigint().to_isize().ok_or_else(|| {
                vm.new_index_error("cannot fit index into an index-sized integer")
            })?;
            // Note: Pointer does NOT adjust negative indices (no length)
            Self::getitem_by_index(zelf, i, vm)
        }
        // PySlice_Check
        else if let Some(slice) = item.downcast_ref::<PySlice>() {
            Self::getitem_by_slice(zelf, slice, vm)
        } else {
            Err(vm.new_type_error("Pointer indices must be integer"))
        }
    }

    // Pointer_item
    fn getitem_by_index(zelf: &Py<Self>, index: isize, vm: &VirtualMachine) -> PyResult {
        // if (*(void **)self->b_ptr == NULL) { PyErr_SetString(...); }
        let ptr_value = zelf.get_ptr_value();
        if ptr_value == 0 {
            return Err(vm.new_value_error("NULL pointer access"));
        }

        // Get element type and size from StgInfo.proto
        let stg_info = zelf.class().stg_info(vm)?;
        let proto_type = stg_info.proto();
        let element_size = proto_type
            .stg_info_opt()
            .map_or(std::mem::size_of::<usize>(), |info| info.size);

        // offset = index * iteminfo->size
        let offset = index * element_size as isize;
        let addr = (ptr_value as isize + offset) as usize;

        // Check if it's a simple type (has _type_ attribute)
        if let Ok(type_attr) = proto_type.as_object().get_attr("_type_", vm)
            && let Ok(type_str) = type_attr.str(vm)
        {
            let type_code = type_str.to_string();
            return Self::read_value_at_address(addr, element_size, Some(&type_code), vm);
        }

        // Complex type: create instance that references the memory directly (not a copy)
        // This allows p[i].val = x to modify the original memory
        // PyCData.into_ref_with_type works for all ctypes (array, structure, union, pointer)
        let cdata = unsafe { super::base::PyCData::at_address(addr as *const u8, element_size) };
        cdata
            .into_ref_with_type(vm, proto_type.to_owned())
            .map(Into::into)
    }

    // Pointer_subscript slice handling (manual parsing, not PySlice_Unpack)
    fn getitem_by_slice(zelf: &Py<Self>, slice: &PySlice, vm: &VirtualMachine) -> PyResult {
        // Since pointers have no length, we have to dissect the slice ourselves

        // step: defaults to 1, step == 0 is error
        let step: isize = if let Some(ref step_obj) = slice.step
            && !vm.is_none(step_obj)
        {
            let s = step_obj
                .try_int(vm)?
                .as_bigint()
                .to_isize()
                .ok_or_else(|| vm.new_value_error("slice step too large"))?;
            if s == 0 {
                return Err(vm.new_value_error("slice step cannot be zero"));
            }
            s
        } else {
            1
        };

        // start: defaults to 0, but required if step < 0
        let start: isize = if let Some(ref start_obj) = slice.start
            && !vm.is_none(start_obj)
        {
            start_obj
                .try_int(vm)?
                .as_bigint()
                .to_isize()
                .ok_or_else(|| vm.new_value_error("slice start too large"))?
        } else {
            if step < 0 {
                return Err(vm.new_value_error("slice start is required for step < 0"));
            }
            0
        };

        // stop: ALWAYS required for pointers
        if vm.is_none(&slice.stop) {
            return Err(vm.new_value_error("slice stop is required"));
        }
        let stop: isize = slice
            .stop
            .try_int(vm)?
            .as_bigint()
            .to_isize()
            .ok_or_else(|| vm.new_value_error("slice stop too large"))?;

        // calculate length
        let len: usize = if (step > 0 && start > stop) || (step < 0 && start < stop) {
            0
        } else if step > 0 {
            ((stop - start - 1) / step + 1) as usize
        } else {
            ((stop - start + 1) / step + 1) as usize
        };

        // Get element info
        let stg_info = zelf.class().stg_info(vm)?;
        let element_size = if let Some(ref proto_type) = stg_info.proto {
            proto_type.stg_info_opt().expect("proto has StgInfo").size
        } else {
            std::mem::size_of::<usize>()
        };
        let type_code = stg_info
            .proto
            .as_ref()
            .and_then(|p| p.as_object().get_attr("_type_", vm).ok())
            .and_then(|t| t.str(vm).ok())
            .map(|s| s.to_string());

        let ptr_value = zelf.get_ptr_value();

        // c_char → bytes
        if type_code.as_deref() == Some("c") {
            if len == 0 {
                return Ok(vm.ctx.new_bytes(vec![]).into());
            }
            let mut result = Vec::with_capacity(len);
            if step == 1 {
                // Optimized contiguous copy
                let start_addr = (ptr_value as isize + start * element_size as isize) as *const u8;
                unsafe {
                    result.extend_from_slice(std::slice::from_raw_parts(start_addr, len));
                }
            } else {
                let mut cur = start;
                for _ in 0..len {
                    let addr = (ptr_value as isize + cur * element_size as isize) as *const u8;
                    unsafe {
                        result.push(*addr);
                    }
                    cur += step;
                }
            }
            return Ok(vm.ctx.new_bytes(result).into());
        }

        // c_wchar → str
        if type_code.as_deref() == Some("u") {
            if len == 0 {
                return Ok(vm.ctx.new_str("").into());
            }
            let mut result = String::with_capacity(len);
            let wchar_size = std::mem::size_of::<libc::wchar_t>();
            let mut cur = start;
            for _ in 0..len {
                let addr = (ptr_value as isize + cur * wchar_size as isize) as *const libc::wchar_t;
                unsafe {
                    if let Some(c) = char::from_u32(*addr as u32) {
                        result.push(c);
                    }
                }
                cur += step;
            }
            return Ok(vm.ctx.new_str(result).into());
        }

        // other types → list with Pointer_item for each
        let mut items = Vec::with_capacity(len);
        let mut cur = start;
        for _ in 0..len {
            items.push(Self::getitem_by_index(zelf, cur, vm)?);
            cur += step;
        }
        Ok(PyList::from(items).into_ref(&vm.ctx).into())
    }

    // Pointer_ass_item
    #[pymethod]
    fn __setitem__(
        zelf: &Py<Self>,
        item: PyObjectRef,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        // Pointer does not support item deletion (value always provided)
        // only integer indices supported for setitem
        if let Some(i) = item.downcast_ref::<PyInt>() {
            let i = i.as_bigint().to_isize().ok_or_else(|| {
                vm.new_index_error("cannot fit index into an index-sized integer")
            })?;
            Self::setitem_by_index(zelf, i, value, vm)
        } else {
            Err(vm.new_type_error("Pointer indices must be integer"))
        }
    }

    fn setitem_by_index(
        zelf: &Py<Self>,
        index: isize,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let ptr_value = zelf.get_ptr_value();
        if ptr_value == 0 {
            return Err(vm.new_value_error("NULL pointer access"));
        }

        // Get element type, size and type_code from StgInfo.proto
        let stg_info = zelf.class().stg_info(vm)?;
        let proto_type = stg_info.proto();

        // Get type code from proto's _type_ attribute
        let type_code: Option<String> = proto_type
            .as_object()
            .get_attr("_type_", vm)
            .ok()
            .and_then(|t| t.downcast_ref::<PyStr>().map(|s| s.to_string()));

        let element_size = proto_type
            .stg_info_opt()
            .map_or(std::mem::size_of::<usize>(), |info| info.size);

        // Calculate address
        let offset = index * element_size as isize;
        let addr = (ptr_value as isize + offset) as usize;

        // Write value at address
        // Handle Structure/Array types by copying their buffer
        if let Some(cdata) = value.downcast_ref::<super::PyCData>()
            && (cdata.fast_isinstance(PyCStructure::static_type())
                || cdata.fast_isinstance(PyCArray::static_type())
                || cdata.fast_isinstance(PyCSimple::static_type()))
        {
            let src_buffer = cdata.buffer.read();
            let copy_len = src_buffer.len().min(element_size);
            unsafe {
                let dest_ptr = addr as *mut u8;
                std::ptr::copy_nonoverlapping(src_buffer.as_ptr(), dest_ptr, copy_len);
            }
        } else {
            // Handle z/Z specially to store converted value
            if type_code.as_deref() == Some("z")
                && let Some(bytes) = value.downcast_ref::<PyBytes>()
            {
                let (converted, ptr_val) = super::base::ensure_z_null_terminated(bytes, vm);
                unsafe {
                    *(addr as *mut usize) = ptr_val;
                }
                return zelf.0.keep_ref(index as usize, converted, vm);
            } else if type_code.as_deref() == Some("Z")
                && let Some(s) = value.downcast_ref::<PyStr>()
            {
                let (holder, ptr_val) = super::base::str_to_wchar_bytes(s.as_str(), vm);
                unsafe {
                    *(addr as *mut usize) = ptr_val;
                }
                return zelf.0.keep_ref(index as usize, holder, vm);
            } else {
                Self::write_value_at_address(addr, element_size, &value, type_code.as_deref(), vm)?;
            }
        }

        // KeepRef: store reference to keep value alive using actual index
        zelf.0.keep_ref(index as usize, value, vm)
    }

    /// Read a value from memory address
    fn read_value_at_address(
        addr: usize,
        size: usize,
        type_code: Option<&str>,
        vm: &VirtualMachine,
    ) -> PyResult {
        unsafe {
            let ptr = addr as *const u8;
            match type_code {
                // Single-byte types don't need read_unaligned
                Some("c") => Ok(vm.ctx.new_bytes(vec![*ptr]).into()),
                Some("b") => Ok(vm.ctx.new_int(*ptr as i8 as i32).into()),
                Some("B") => Ok(vm.ctx.new_int(*ptr as i32).into()),
                // Multi-byte types need read_unaligned for safety on strict-alignment architectures
                Some("h") => Ok(vm
                    .ctx
                    .new_int(std::ptr::read_unaligned(ptr as *const i16) as i32)
                    .into()),
                Some("H") => Ok(vm
                    .ctx
                    .new_int(std::ptr::read_unaligned(ptr as *const u16) as i32)
                    .into()),
                Some("i") | Some("l") => Ok(vm
                    .ctx
                    .new_int(std::ptr::read_unaligned(ptr as *const i32))
                    .into()),
                Some("I") | Some("L") => Ok(vm
                    .ctx
                    .new_int(std::ptr::read_unaligned(ptr as *const u32))
                    .into()),
                Some("q") => Ok(vm
                    .ctx
                    .new_int(std::ptr::read_unaligned(ptr as *const i64))
                    .into()),
                Some("Q") => Ok(vm
                    .ctx
                    .new_int(std::ptr::read_unaligned(ptr as *const u64))
                    .into()),
                Some("f") => Ok(vm
                    .ctx
                    .new_float(std::ptr::read_unaligned(ptr as *const f32) as f64)
                    .into()),
                Some("d") | Some("g") => Ok(vm
                    .ctx
                    .new_float(std::ptr::read_unaligned(ptr as *const f64))
                    .into()),
                Some("P") | Some("z") | Some("Z") => Ok(vm
                    .ctx
                    .new_int(std::ptr::read_unaligned(ptr as *const usize))
                    .into()),
                _ => {
                    // Default: read as bytes
                    let bytes = std::slice::from_raw_parts(ptr, size).to_vec();
                    Ok(vm.ctx.new_bytes(bytes).into())
                }
            }
        }
    }

    /// Write a value to memory address
    fn write_value_at_address(
        addr: usize,
        size: usize,
        value: &PyObject,
        type_code: Option<&str>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        unsafe {
            let ptr = addr as *mut u8;

            // Handle c_char_p (z) and c_wchar_p (Z) - store pointer address
            // Note: PyBytes/PyStr cases are handled by caller (setitem_by_index)
            match type_code {
                Some("z") | Some("Z") => {
                    let ptr_val = if vm.is_none(value) {
                        0usize
                    } else if let Ok(int_val) = value.try_index(vm) {
                        int_val.as_bigint().to_usize().unwrap_or(0)
                    } else {
                        return Err(vm.new_type_error(
                            "bytes/string or integer address expected".to_owned(),
                        ));
                    };
                    std::ptr::write_unaligned(ptr as *mut usize, ptr_val);
                    return Ok(());
                }
                _ => {}
            }

            // Try to get value as integer
            // Use write_unaligned for safety on strict-alignment architectures
            if let Ok(int_val) = value.try_int(vm) {
                let i = int_val.as_bigint();
                match size {
                    1 => {
                        *ptr = i.to_u8().expect("int too large");
                    }
                    2 => {
                        std::ptr::write_unaligned(
                            ptr as *mut i16,
                            i.to_i16().expect("int too large"),
                        );
                    }
                    4 => {
                        std::ptr::write_unaligned(
                            ptr as *mut i32,
                            i.to_i32().expect("int too large"),
                        );
                    }
                    8 => {
                        std::ptr::write_unaligned(
                            ptr as *mut i64,
                            i.to_i64().expect("int too large"),
                        );
                    }
                    _ => {
                        let bytes = i.to_signed_bytes_le();
                        let copy_len = bytes.len().min(size);
                        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, copy_len);
                    }
                }
                return Ok(());
            }

            // Try to get value as float
            if let Ok(float_val) = value.try_float(vm) {
                let f = float_val.to_f64();
                match size {
                    4 => {
                        std::ptr::write_unaligned(ptr as *mut f32, f as f32);
                    }
                    8 => {
                        std::ptr::write_unaligned(ptr as *mut f64, f);
                    }
                    _ => {}
                }
                return Ok(());
            }

            // Try bytes
            if let Ok(bytes) = value.try_bytes_like(vm, |b| b.to_vec()) {
                let copy_len = bytes.len().min(size);
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, copy_len);
                return Ok(());
            }

            Err(vm.new_type_error(format!(
                "cannot convert {} to ctypes data",
                value.class().name()
            )))
        }
    }
}

impl AsBuffer for PyCPointer {
    fn as_buffer(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<PyBuffer> {
        let stg_info = zelf
            .class()
            .stg_info_opt()
            .expect("PyCPointer type must have StgInfo");
        let format = stg_info
            .format
            .clone()
            .map(Cow::Owned)
            .unwrap_or(Cow::Borrowed("&B"));
        let itemsize = stg_info.size;
        // Pointer types are scalars with ndim=0, shape=()
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

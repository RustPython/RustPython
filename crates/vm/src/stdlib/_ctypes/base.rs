use crate::builtins::{
    PyBytes, PyDict, PyList, PyMemoryView, PyStr, PyTuple, PyType, PyTypeRef, PyUtf8Str,
};
use crate::class::StaticType;
use crate::convert::ToPyObject;
use crate::function::{ArgBytesLike, OptionalArg, PySetterValue};
use crate::protocol::{BufferMethods, PyBuffer};
use crate::types::{Constructor, GetDescriptor, Representable};
use crate::{
    AsObject, Py, PyObject, PyObjectRef, PyPayload, PyResult, TryFromObject, VirtualMachine,
};
use alloc::borrow::Cow;
use core::fmt::Debug;
use crossbeam_utils::atomic::AtomicCell;
use num_traits::{Signed, ToPrimitive};
use rustpython_common::lock::PyRwLock;
use rustpython_common::wtf8::Wtf8;
use rustpython_host_env::ctypes::{
    CTypeParamKind, FfiArg, FfiType, FfiValue, char_array_assignment_bytes, char_array_field_value,
    ffi_arg_from_value, ffi_type_for_layout, wchar_array_field_value, write_cow_bytes_at_offset,
};

// StgInfo - Storage information for ctypes types
// Stored in TypeDataSlot of heap types (PyType::init_type_data/get_type_data)

// Flag constants
bitflags::bitflags! {
    #[derive(Default, Copy, Clone, Debug, PartialEq, Eq)]
    pub struct StgInfoFlags: u32 {
        // Function calling convention flags
        /// Standard call convention (Windows)
        const FUNCFLAG_STDCALL = 0x0;
        /// C calling convention
        const FUNCFLAG_CDECL = 0x1;
        /// Function returns HRESULT
        const FUNCFLAG_HRESULT = 0x2;
        /// Use Python API calling convention
        const FUNCFLAG_PYTHONAPI = 0x4;
        /// Capture errno after call
        const FUNCFLAG_USE_ERRNO = 0x8;
        /// Capture last error after call (Windows)
        const FUNCFLAG_USE_LASTERROR = 0x10;

        // Type flags
        /// Type is a pointer type
        const TYPEFLAG_ISPOINTER = 0x100;
        /// Type contains pointer fields
        const TYPEFLAG_HASPOINTER = 0x200;
        /// Type is or contains a union
        const TYPEFLAG_HASUNION = 0x400;
        /// Type contains bitfield members
        const TYPEFLAG_HASBITFIELD = 0x800;

        // Dict flags
        /// Type is finalized (_fields_ has been set)
        const DICTFLAG_FINAL = 0x1000;
    }
}

/// ParamFunc - determines how a type is passed to foreign functions
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum ParamFunc {
    #[default]
    None,
    /// Array types are passed as pointers (tag = 'P')
    Array,
    /// Simple types use their specific conversion (tag = type code)
    Simple,
    /// Pointer types (tag = 'P')
    Pointer,
    /// Structure types (tag = 'V' for value)
    Structure,
    /// Union types (tag = 'V' for value)
    Union,
}

#[derive(Clone)]
pub struct StgInfo {
    pub initialized: bool,
    pub size: usize,              // number of bytes
    pub align: usize,             // alignment requirements
    pub length: usize,            // number of fields (for arrays/structures)
    pub proto: Option<PyTypeRef>, // Only for Pointer/ArrayObject
    pub flags: StgInfoFlags,      // type flags (TYPEFLAG_*, DICTFLAG_*)

    // Array-specific fields
    pub element_type: Option<PyTypeRef>, // _type_ for arrays
    pub element_size: usize,             // size of each element

    // PEP 3118 buffer protocol fields
    pub format: Option<String>, // struct format string (e.g., "i", "(5)i")
    pub shape: Vec<usize>,      // shape for multi-dimensional arrays

    // Function parameter conversion
    pub(super) paramfunc: ParamFunc, // how to pass to foreign functions

    // Byte order (for _swappedbytes_)
    pub big_endian: bool, // true if big endian, false if little endian

    // FFI field types for structure/union passing (inherited from base class)
    pub ffi_field_types: Vec<FfiType>,

    // Cached pointer type (non-inheritable via descriptor)
    pub pointer_type: Option<PyObjectRef>,
}

// StgInfo is stored in type_data which requires Send + Sync.
// The PyTypeRef in proto/element_type fields is protected by the type system's locking mechanism.
// ctypes objects are not thread-safe by design; users must synchronize access.
unsafe impl Send for StgInfo {}
unsafe impl Sync for StgInfo {}

impl core::fmt::Debug for StgInfo {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("StgInfo")
            .field("initialized", &self.initialized)
            .field("size", &self.size)
            .field("align", &self.align)
            .field("length", &self.length)
            .field("proto", &self.proto)
            .field("flags", &self.flags)
            .field("element_type", &self.element_type)
            .field("element_size", &self.element_size)
            .field("format", &self.format)
            .field("shape", &self.shape)
            .field("paramfunc", &self.paramfunc)
            .field("big_endian", &self.big_endian)
            .field("ffi_field_types", &self.ffi_field_types.len())
            .finish()
    }
}

impl Default for StgInfo {
    fn default() -> Self {
        StgInfo {
            initialized: false,
            size: 0,
            align: 1,
            length: 0,
            proto: None,
            flags: StgInfoFlags::empty(),
            element_type: None,
            element_size: 0,
            format: None,
            shape: Vec::new(),
            paramfunc: ParamFunc::None,
            big_endian: cfg!(target_endian = "big"), // native endian by default
            ffi_field_types: Vec::new(),
            pointer_type: None,
        }
    }
}

impl StgInfo {
    pub fn new(size: usize, align: usize) -> Self {
        StgInfo {
            initialized: true,
            size,
            align,
            length: 0,
            proto: None,
            flags: StgInfoFlags::empty(),
            element_type: None,
            element_size: 0,
            format: None,
            shape: Vec::new(),
            paramfunc: ParamFunc::None,
            big_endian: cfg!(target_endian = "big"), // native endian by default
            ffi_field_types: Vec::new(),
            pointer_type: None,
        }
    }

    /// Create StgInfo for an array type
    /// item_format: the innermost element's format string (kept as-is, e.g., "<i")
    /// item_shape: the element's shape (will be prepended with length)
    /// item_flags: the element type's flags (for HASPOINTER inheritance)
    #[allow(clippy::too_many_arguments)]
    pub fn new_array(
        size: usize,
        align: usize,
        length: usize,
        element_type: PyTypeRef,
        element_size: usize,
        item_format: Option<&str>,
        item_shape: &[usize],
        item_flags: StgInfoFlags,
    ) -> Self {
        // Format is kept from innermost element (e.g., "<i" for c_int arrays)
        // The array dimensions go into shape only, not format
        let format = item_format.map(|f| f.to_owned());

        // Build shape: [length, ...item_shape]
        let mut shape = vec![length];
        shape.extend_from_slice(item_shape);

        // Inherit HASPOINTER flag from element type
        // if (iteminfo->flags & (TYPEFLAG_ISPOINTER | TYPEFLAG_HASPOINTER))
        //     stginfo->flags |= TYPEFLAG_HASPOINTER;
        let flags = if item_flags
            .intersects(StgInfoFlags::TYPEFLAG_ISPOINTER | StgInfoFlags::TYPEFLAG_HASPOINTER)
        {
            StgInfoFlags::TYPEFLAG_HASPOINTER
        } else {
            StgInfoFlags::empty()
        };

        StgInfo {
            initialized: true,
            size,
            align,
            length,
            proto: None,
            flags,
            element_type: Some(element_type),
            element_size,
            format,
            shape,
            paramfunc: ParamFunc::Array,
            big_endian: cfg!(target_endian = "big"), // native endian by default
            ffi_field_types: Vec::new(),
            pointer_type: None,
        }
    }

    /// Get libffi type for this StgInfo
    /// Note: For very large types, returns pointer type to avoid overflow
    pub fn to_ffi_type(&self) -> FfiType {
        let kind = match self.paramfunc {
            ParamFunc::Structure => CTypeParamKind::Structure,
            ParamFunc::Union => CTypeParamKind::Union,
            ParamFunc::Array => CTypeParamKind::Array,
            ParamFunc::Pointer => CTypeParamKind::Pointer,
            _ => CTypeParamKind::Simple,
        };
        ffi_type_for_layout(
            kind,
            &self.ffi_field_types,
            self.size,
            self.length,
            self.format.as_deref(),
        )
    }

    /// Check if this type is finalized (cannot set _fields_ again)
    pub fn is_final(&self) -> bool {
        self.flags.contains(StgInfoFlags::DICTFLAG_FINAL)
    }

    /// Get proto type reference (for Pointer/Array types)
    pub fn proto(&self) -> &Py<PyType> {
        self.proto.as_deref().expect("type has proto")
    }
}

/// __pointer_type__ getter for ctypes metaclasses.
/// Reads from StgInfo.pointer_type (non-inheritable).
pub(super) fn pointer_type_get(zelf: &Py<PyType>, vm: &VirtualMachine) -> PyResult {
    zelf.stg_info_opt()
        .and_then(|info| info.pointer_type.clone())
        .ok_or_else(|| {
            vm.new_attribute_error(format!(
                "type {} has no attribute '__pointer_type__'",
                zelf.name()
            ))
        })
}

/// __pointer_type__ setter for ctypes metaclasses.
/// Writes to StgInfo.pointer_type (non-inheritable).
pub(super) fn pointer_type_set(
    zelf: &Py<PyType>,
    value: PyObjectRef,
    vm: &VirtualMachine,
) -> PyResult<()> {
    if let Some(mut info) = zelf.get_type_data_mut::<StgInfo>() {
        info.pointer_type = Some(value);
        Ok(())
    } else {
        Err(vm.new_attribute_error(format!("cannot set __pointer_type__ on {}", zelf.name())))
    }
}

/// Get PEP3118 format string for a field type
/// Returns the format string considering byte order
pub(super) fn get_field_format(
    field_type: &PyObject,
    big_endian: bool,
    vm: &VirtualMachine,
) -> String {
    let endian_prefix = if big_endian { ">" } else { "<" };

    // 1. Check StgInfo for format
    if let Some(type_obj) = field_type.downcast_ref::<PyType>()
        && let Some(stg_info) = type_obj.stg_info_opt()
        && let Some(fmt) = &stg_info.format
    {
        // For structures (T{...}), arrays ((n)...), and pointers (&...), return as-is
        // These complex types have their own endianness markers inside
        if fmt.starts_with('T')
            || fmt.starts_with('(')
            || fmt.starts_with('&')
            || fmt.starts_with("X{")
        {
            return fmt.clone();
        }

        // For simple types, replace existing endian prefix with the correct one
        let base_fmt = fmt.trim_start_matches(['<', '>', '@', '=', '!']);
        if !base_fmt.is_empty() {
            return format!("{}{}", endian_prefix, base_fmt);
        }
        return fmt.clone();
    }

    // 2. Try to get _type_ attribute for simple types
    if let Ok(type_attr) = field_type.get_attr("_type_", vm)
        && let Some(type_str) = type_attr.downcast_ref::<PyStr>()
    {
        let s = type_str
            .to_str()
            .expect("_type_ is validated as ASCII at type creation");
        return format!("{}{}", endian_prefix, s);
    }

    // Default: single byte
    "B".to_string()
}

/// Compute byte order based on swapped flag
#[inline]
pub(super) fn is_big_endian(is_swapped: bool) -> bool {
    if is_swapped {
        !cfg!(target_endian = "big")
    } else {
        cfg!(target_endian = "big")
    }
}

/// Shared BufferMethods for all ctypes types (PyCArray, PyCSimple, PyCStructure, PyCUnion)
/// All these types are #[repr(transparent)] wrappers around PyCData
pub(super) static CDATA_BUFFER_METHODS: BufferMethods = BufferMethods {
    obj_bytes: |buffer| {
        rustpython_common::lock::PyRwLockReadGuard::map(
            buffer.obj_as::<PyCData>().buffer.read(),
            |x| &**x,
        )
        .into()
    },
    obj_bytes_mut: |buffer| {
        rustpython_common::lock::PyRwLockWriteGuard::map(
            buffer.obj_as::<PyCData>().buffer.write(),
            |x| x.to_mut().as_mut_slice(),
        )
        .into()
    },
    release: |_| {},
    retain: |_| {},
};

/// Ensure PyBytes data is null-terminated. Returns (kept_alive_obj, pointer).
/// The caller must keep the returned object alive to keep the pointer valid.
pub(super) fn ensure_z_null_terminated(
    bytes: &PyBytes,
    vm: &VirtualMachine,
) -> (PyObjectRef, usize) {
    let buffer = rustpython_host_env::ctypes::null_terminated_bytes(bytes.as_bytes());
    let ptr = buffer.as_ptr() as usize;
    let kept_alive: PyObjectRef = vm.ctx.new_bytes(buffer).into();
    (kept_alive, ptr)
}

/// Convert str to null-terminated wchar_t buffer. Returns (PyBytes holder, pointer).
pub(super) fn str_to_wchar_bytes(s: &Wtf8, vm: &VirtualMachine) -> (PyObjectRef, usize) {
    let bytes = rustpython_host_env::ctypes::wchar_null_terminated_bytes(s);
    let ptr = bytes.as_ptr() as usize;
    let holder: PyObjectRef = vm.ctx.new_bytes(bytes).into();
    (holder, ptr)
}

/// PyCData - base type for all ctypes data types
#[pyclass(name = "_CData", module = "_ctypes")]
#[derive(Debug, PyPayload)]
pub struct PyCData {
    /// Memory buffer - Owned (self-owned) or Borrowed (external reference)
    ///
    /// SAFETY: Borrowed variant's 'static lifetime is not actually static.
    /// When created via from_address or from_base_obj, only valid for the lifetime of the source memory.
    /// Same behavior as CPython's b_ptr (user responsibility, kept alive via b_base).
    pub buffer: PyRwLock<Cow<'static, [u8]>>,
    /// pointer to base object or None (b_base)
    pub base: PyRwLock<Option<PyObjectRef>>,
    /// byte offset within base's buffer (for field access)
    pub base_offset: AtomicCell<usize>,
    /// index into base's b_objects list (b_index)
    pub index: AtomicCell<usize>,
    /// dictionary of references we need to keep (b_objects)
    pub objects: PyRwLock<Option<PyObjectRef>>,
    /// number of references we need (b_length)
    pub length: AtomicCell<usize>,
    /// References kept alive but not visible in _objects.
    /// Used for null-terminated c_char_p buffer copies, since
    /// RustPython's PyBytes lacks CPython's internal trailing null.
    /// Keyed by unique_key (hierarchical) so nested fields don't collide.
    pub(super) kept_refs: PyRwLock<std::collections::HashMap<String, PyObjectRef>>,
}

impl PyCData {
    /// Create from StgInfo (PyCData_MallocBuffer pattern)
    pub fn from_stg_info(stg_info: &StgInfo) -> Self {
        PyCData {
            buffer: PyRwLock::new(Cow::Owned(vec![0u8; stg_info.size])),
            base: PyRwLock::new(None),
            base_offset: AtomicCell::new(0),
            index: AtomicCell::new(0),
            objects: PyRwLock::new(None),
            length: AtomicCell::new(stg_info.length),
            kept_refs: PyRwLock::new(std::collections::HashMap::new()),
        }
    }

    /// Create from existing bytes (copies data)
    pub fn from_bytes(data: Vec<u8>, objects: Option<PyObjectRef>) -> Self {
        PyCData {
            buffer: PyRwLock::new(Cow::Owned(data)),
            base: PyRwLock::new(None),
            base_offset: AtomicCell::new(0),
            index: AtomicCell::new(0),
            objects: PyRwLock::new(objects),
            length: AtomicCell::new(0),
            kept_refs: PyRwLock::new(std::collections::HashMap::new()),
        }
    }

    /// Create from bytes with specified length (for arrays)
    pub fn from_bytes_with_length(
        data: Vec<u8>,
        objects: Option<PyObjectRef>,
        length: usize,
    ) -> Self {
        PyCData {
            buffer: PyRwLock::new(Cow::Owned(data)),
            base: PyRwLock::new(None),
            base_offset: AtomicCell::new(0),
            index: AtomicCell::new(0),
            objects: PyRwLock::new(objects),
            length: AtomicCell::new(length),
            kept_refs: PyRwLock::new(std::collections::HashMap::new()),
        }
    }

    /// Create from external memory address
    ///
    /// # Safety
    /// The returned slice's 'static lifetime is a lie.
    /// Actually only valid for the lifetime of the memory pointed to by ptr.
    /// PyCData_AtAddress
    pub unsafe fn at_address(ptr: *const u8, size: usize) -> Self {
        // = PyCData_AtAddress
        // SAFETY: Caller must ensure ptr is valid for the lifetime of returned PyCData
        let slice = unsafe { rustpython_host_env::ctypes::borrow_memory(ptr, size) };
        PyCData {
            buffer: PyRwLock::new(Cow::Borrowed(slice)),
            base: PyRwLock::new(None),
            base_offset: AtomicCell::new(0),
            index: AtomicCell::new(0),
            objects: PyRwLock::new(None),
            length: AtomicCell::new(0),
            kept_refs: PyRwLock::new(std::collections::HashMap::new()),
        }
    }

    /// Create from base object with offset and data copy
    ///
    /// Similar to from_base_with_offset, but also stores a copy of the data.
    /// This is used for arrays where we need our own buffer for the buffer protocol,
    /// but still maintain the base reference for KeepRef and tracking.
    pub fn from_base_with_data(
        base_obj: PyObjectRef,
        offset: usize,
        idx: usize,
        length: usize,
        data: Vec<u8>,
    ) -> Self {
        PyCData {
            buffer: PyRwLock::new(Cow::Owned(data)), // Has its own buffer copy
            base: PyRwLock::new(Some(base_obj)),     // But still tracks base
            base_offset: AtomicCell::new(offset),    // And offset for writes
            index: AtomicCell::new(idx),
            objects: PyRwLock::new(None),
            length: AtomicCell::new(length),
            kept_refs: PyRwLock::new(std::collections::HashMap::new()),
        }
    }

    /// Create from base object's buffer
    ///
    /// This creates a borrowed view into the base's buffer at the given address.
    /// The base object is stored in b_base to keep the memory alive.
    ///
    /// # Safety
    /// ptr must point into base_obj's buffer and remain valid as long as base_obj is alive.
    pub unsafe fn from_base_obj(
        ptr: *mut u8,
        size: usize,
        base_obj: PyObjectRef,
        idx: usize,
    ) -> Self {
        // = PyCData_FromBaseObj
        // SAFETY: ptr points into base_obj's buffer, kept alive via base reference
        let slice = unsafe { rustpython_host_env::ctypes::borrow_memory(ptr, size) };
        PyCData {
            buffer: PyRwLock::new(Cow::Borrowed(slice)),
            base: PyRwLock::new(Some(base_obj)),
            base_offset: AtomicCell::new(0),
            index: AtomicCell::new(idx),
            objects: PyRwLock::new(None),
            length: AtomicCell::new(0),
            kept_refs: PyRwLock::new(std::collections::HashMap::new()),
        }
    }

    /// Create from buffer protocol object (for from_buffer method)
    ///
    /// Unlike from_bytes, this shares memory with the source buffer.
    /// The source object is stored in objects dict to keep the buffer alive.
    /// Python stores with key -1 via KeepRef(result, -1, mv).
    ///
    /// # Safety
    /// ptr must point to valid memory that remains valid as long as source is alive.
    pub unsafe fn from_buffer_shared(
        ptr: *const u8,
        size: usize,
        length: usize,
        source: PyObjectRef,
        vm: &VirtualMachine,
    ) -> Self {
        // SAFETY: Caller must ensure ptr is valid for the lifetime of source
        let slice = unsafe { rustpython_host_env::ctypes::borrow_memory(ptr, size) };

        // Python stores the reference in a dict with key "-1" (unique_key pattern)
        let objects_dict = vm.ctx.new_dict();
        objects_dict
            .set_item("-1", source, vm)
            .expect("Failed to store buffer reference");

        PyCData {
            buffer: PyRwLock::new(Cow::Borrowed(slice)),
            base: PyRwLock::new(None),
            base_offset: AtomicCell::new(0),
            index: AtomicCell::new(0),
            objects: PyRwLock::new(Some(objects_dict.into())),
            length: AtomicCell::new(length),
            kept_refs: PyRwLock::new(std::collections::HashMap::new()),
        }
    }

    /// Common implementation for from_buffer class method.
    /// Validates buffer, creates memoryview, and returns PyCData sharing memory with source.
    ///
    /// CDataType_from_buffer_impl
    pub fn from_buffer_impl(
        cls: &Py<PyType>,
        source: PyObjectRef,
        offset: isize,
        vm: &VirtualMachine,
    ) -> PyResult<Self> {
        let (size, length) = {
            let stg_info = cls
                .stg_info_opt()
                .ok_or_else(|| vm.new_type_error("not a ctypes type"))?;
            (stg_info.size, stg_info.length)
        };

        if offset < 0 {
            return Err(vm.new_value_error("offset cannot be negative"));
        }
        let offset = offset as usize;

        // Get buffer from source (this exports the buffer)
        let buffer = PyBuffer::try_from_object(vm, source)?;

        // Check if buffer is writable
        if buffer.desc.readonly {
            return Err(vm.new_type_error("underlying buffer is not writable"));
        }

        // Check if buffer is C contiguous
        if !buffer.desc.is_contiguous() {
            return Err(vm.new_type_error("underlying buffer is not C contiguous"));
        }

        // Check if buffer is large enough
        let buffer_len = buffer.desc.len;
        if offset + size > buffer_len {
            return Err(vm.new_value_error(format!(
                "Buffer size too small ({} instead of at least {} bytes)",
                buffer_len,
                offset + size
            )));
        }

        // Get buffer pointer - the memory is owned by source
        let ptr = {
            let bytes = buffer.obj_bytes();
            bytes.as_ptr().wrapping_add(offset)
        };

        // Create memoryview to keep buffer exported (prevents source from being modified)
        // mv = PyMemoryView_FromObject(obj); KeepRef(result, -1, mv);
        let memoryview = PyMemoryView::from_buffer(buffer, vm)?;
        let mv_obj = memoryview.into_pyobject(vm);

        // Create CData that shares memory with the buffer
        Ok(unsafe { Self::from_buffer_shared(ptr, size, length, mv_obj, vm) })
    }

    /// Common implementation for from_buffer_copy class method.
    /// Copies data from buffer and creates new independent instance.
    ///
    /// CDataType_from_buffer_copy_impl
    pub fn from_buffer_copy_impl(
        cls: &Py<PyType>,
        source: &[u8],
        offset: isize,
        vm: &VirtualMachine,
    ) -> PyResult<Self> {
        let (size, length) = {
            let stg_info = cls
                .stg_info_opt()
                .ok_or_else(|| vm.new_type_error("not a ctypes type"))?;
            (stg_info.size, stg_info.length)
        };

        if offset < 0 {
            return Err(vm.new_value_error("offset cannot be negative"));
        }
        let offset = offset as usize;

        // Check if buffer is large enough
        if offset + size > source.len() {
            return Err(vm.new_value_error(format!(
                "Buffer size too small ({} instead of at least {} bytes)",
                source.len(),
                offset + size
            )));
        }

        // Copy bytes from buffer at offset
        let data = source[offset..offset + size].to_vec();

        Ok(Self::from_bytes_with_length(data, None, length))
    }

    #[inline]
    pub fn size(&self) -> usize {
        self.buffer.read().len()
    }

    /// Check if this buffer is borrowed (external memory reference)
    #[inline]
    pub fn is_borrowed(&self) -> bool {
        matches!(&*self.buffer.read(), Cow::Borrowed(_))
    }

    /// Write bytes at offset - handles both borrowed and owned buffers
    ///
    /// For borrowed buffers (from from_address), writes directly to external memory.
    /// For owned buffers, writes through to_mut() as normal.
    ///
    /// # Safety
    /// For borrowed buffers, caller must ensure the memory is writable.
    pub fn write_bytes_at_offset(&self, offset: usize, bytes: &[u8]) {
        let mut buffer = self.buffer.write();
        write_cow_bytes_at_offset(&mut buffer, offset, bytes);
    }

    /// Generate unique key for nested references (unique_key)
    /// Creates a hierarchical key by walking up the b_base chain.
    /// Format: "index:parent_index:grandparent_index:..."
    pub fn unique_key(&self, index: usize) -> String {
        let mut key = format!("{index:x}");
        // Walk up the base chain to build hierarchical key
        if self.base.read().is_some() {
            let parent_index = self.index.load();
            key.push_str(&format!(":{parent_index:x}"));
        }
        key
    }

    /// Keep a reference in the objects dictionary (KeepRef)
    ///
    /// Stores 'keep' in this object's b_objects dict at key 'index'.
    /// If keep is None, does nothing (optimization).
    /// This function stores the value directly - caller should use get_kept_objects()
    /// first if they want to store the _objects of a CData instead of the object itself.
    ///
    /// If this object has a base (is embedded in another structure/union/array),
    /// the reference is stored in the root object's b_objects with a hierarchical key.
    pub fn keep_ref(&self, index: usize, keep: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        // Optimization: no need to store None
        if vm.is_none(&keep) {
            return Ok(());
        }

        // Build hierarchical key
        let key = self.unique_key(index);

        // If we have a base object, find root and store there
        if let Some(base_obj) = self.base.read().clone() {
            // Find root by walking up the base chain
            let root_obj = Self::find_root_object(&base_obj);
            Self::store_in_object(&root_obj, &key, keep, vm)?;
            return Ok(());
        }

        // No base - store in own objects dict
        let mut objects = self.objects.write();

        // Initialize b_objects if needed
        if objects.is_none() {
            if self.length.load() > 0 {
                // Need to store multiple references - create a dict
                *objects = Some(vm.ctx.new_dict().into());
            } else {
                // Only one reference needed - store directly
                *objects = Some(keep);
                return Ok(());
            }
        }

        // If b_objects is not a dict, convert it to a dict first
        // This preserves the existing reference (e.g., from cast) when adding new references
        if let Some(obj) = objects.as_ref()
            && obj.downcast_ref::<PyDict>().is_none()
        {
            // Convert existing single reference to a dict
            let dict = vm.ctx.new_dict();
            // Store the original object with a special key (id-based)
            let id_key: PyObjectRef = vm.ctx.new_int(obj.get_id() as i64).into();
            dict.set_item(&*id_key, obj.clone(), vm)?;
            *objects = Some(dict.into());
        }

        // Store in dict with unique key
        if let Some(dict_obj) = objects.as_ref()
            && let Some(dict) = dict_obj.downcast_ref::<PyDict>()
        {
            let key_obj: PyObjectRef = vm.ctx.new_str(key).into();
            dict.set_item(&*key_obj, keep, vm)?;
        }

        Ok(())
    }

    /// Keep a reference alive without exposing it in _objects.
    /// Walks up to root object (same as keep_ref) so the reference
    /// lives as long as the owning ctypes object.
    /// Uses unique_key (hierarchical) so nested fields don't collide.
    pub fn keep_alive(&self, index: usize, obj: PyObjectRef) {
        let key = self.unique_key(index);
        if let Some(base_obj) = self.base.read().clone() {
            let root = Self::find_root_object(&base_obj);
            if let Some(cdata) = root.downcast_ref::<PyCData>() {
                cdata.kept_refs.write().insert(key, obj);
                return;
            }
        }
        self.kept_refs.write().insert(key, obj);
    }

    /// Find the root object (one without a base) by walking up the base chain
    fn find_root_object(obj: &PyObject) -> PyObjectRef {
        // Try to get base from different ctypes types
        let base = if let Some(cdata) = obj.downcast_ref::<PyCData>() {
            cdata.base.read().clone()
        } else {
            None
        };

        // Recurse if there's a base, otherwise this is the root
        if let Some(base_obj) = base {
            Self::find_root_object(&base_obj)
        } else {
            obj.to_owned()
        }
    }

    /// Store a value in an object's _objects dict with the given key
    fn store_in_object(
        obj: &PyObject,
        key: &str,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        // Get the objects dict from the object
        let objects_lock = if let Some(cdata) = obj.downcast_ref::<PyCData>() {
            &cdata.objects
        } else {
            return Ok(()); // Unknown type, skip
        };

        let mut objects = objects_lock.write();

        // Initialize if needed
        if objects.is_none() {
            *objects = Some(vm.ctx.new_dict().into());
        }

        // If not a dict, convert to dict
        if let Some(obj) = objects.as_ref()
            && obj.downcast_ref::<PyDict>().is_none()
        {
            let dict = vm.ctx.new_dict();
            let id_key: PyObjectRef = vm.ctx.new_int(obj.get_id() as i64).into();
            dict.set_item(&*id_key, obj.clone(), vm)?;
            *objects = Some(dict.into());
        }

        // Store in dict
        if let Some(dict_obj) = objects.as_ref()
            && let Some(dict) = dict_obj.downcast_ref::<PyDict>()
        {
            let key_obj: PyObjectRef = vm.ctx.new_str(key).into();
            dict.set_item(&*key_obj, value, vm)?;
        }

        Ok(())
    }

    /// Get kept objects from a CData instance
    /// Returns the _objects of the CData, or an empty dict if None.
    pub fn get_kept_objects(value: &PyObject, vm: &VirtualMachine) -> PyObjectRef {
        value
            .downcast_ref::<PyCData>()
            .and_then(|cdata| cdata.objects.read().clone())
            .unwrap_or_else(|| vm.ctx.new_dict().into())
    }

    /// Check if a value should be stored in _objects
    /// Returns true for ctypes objects and bytes (for c_char_p)
    pub fn should_keep_ref(value: &PyObject) -> bool {
        value.downcast_ref::<PyCData>().is_some() || value.downcast_ref::<PyBytes>().is_some()
    }

    /// PyCData_set
    /// Sets a field value at the given offset, handling type conversion and KeepRef
    #[allow(clippy::too_many_arguments)]
    pub fn set_field(
        &self,
        proto: &PyObject,
        value: PyObjectRef,
        index: usize,
        size: usize,
        offset: usize,
        needs_swap: bool,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        // Check if this is a c_char or c_wchar array field
        let is_char_array = PyCField::is_char_array(proto, vm);
        let is_wchar_array = PyCField::is_wchar_array(proto, vm);

        // For c_char arrays with bytes input, copy only up to first null
        if is_char_array {
            if let Some(bytes_val) = value.downcast_ref::<PyBytes>() {
                let src = bytes_val.as_bytes();
                let to_copy = char_array_assignment_bytes(src);
                let copy_len = core::cmp::min(to_copy.len(), size);
                self.write_bytes_at_offset(offset, &to_copy[..copy_len]);
                self.keep_ref(index, value, vm)?;
                return Ok(());
            } else {
                return Err(vm.new_type_error("bytes expected instead of str instance"));
            }
        }

        // For c_wchar arrays with str input, convert to wchar_t
        if is_wchar_array {
            if let Some(str_val) = value.downcast_ref::<PyStr>() {
                let wchar_bytes = rustpython_host_env::ctypes::encode_wtf8_to_wchar_padded(
                    str_val.as_wtf8(),
                    size,
                );
                self.write_bytes_at_offset(offset, &wchar_bytes);
                self.keep_ref(index, value, vm)?;
                return Ok(());
            } else if value.downcast_ref::<PyBytes>().is_some() {
                return Err(vm.new_type_error("str expected instead of bytes instance"));
            }
        }

        // Special handling for Pointer fields with Array values
        if let Some(proto_type) = proto.downcast_ref::<PyType>()
            && proto_type
                .class()
                .fast_issubclass(super::pointer::PyCPointerType::static_type())
            && let Some(array) = value.downcast_ref::<super::array::PyCArray>()
        {
            let buffer_addr = {
                let array_buffer = array.0.buffer.read();
                array_buffer.as_ptr() as usize
            };
            let addr_bytes = rustpython_host_env::ctypes::pointer_to_sized_bytes(buffer_addr, size);
            self.write_bytes_at_offset(offset, &addr_bytes);
            self.keep_ref(index, value, vm)?;
            return Ok(());
        }

        // For array fields with tuple/list input, instantiate the array type
        // and unpack elements as positional args (Array_init expects *args)
        if let Some(proto_type) = proto.downcast_ref::<PyType>()
            && let Some(stg) = proto_type.stg_info_opt()
            && stg.element_type.is_some()
        {
            let items: Option<Vec<PyObjectRef>> =
                if let Some(tuple) = value.downcast_ref::<PyTuple>() {
                    Some(tuple.to_vec())
                } else {
                    value
                        .downcast_ref::<crate::builtins::PyList>()
                        .map(|list| list.borrow_vec().to_vec())
                };
            if let Some(items) = items {
                let array_obj = proto_type.as_object().call(items, vm).map_err(|e| {
                    // Wrap errors in RuntimeError with type name prefix
                    let type_name = proto_type.name().to_string();
                    let exc_name = e.class().name().to_string();
                    let exc_args = e.args();
                    let exc_msg = exc_args
                        .first()
                        .and_then(|a| a.downcast_ref::<PyStr>().map(|s| s.to_string()))
                        .unwrap_or_default();
                    vm.new_runtime_error(format!("({type_name}) {exc_name}: {exc_msg}"))
                })?;
                if let Some(arr) = array_obj.downcast_ref::<super::array::PyCArray>() {
                    let arr_buffer = arr.0.buffer.read();
                    let len = core::cmp::min(arr_buffer.len(), size);
                    self.write_bytes_at_offset(offset, &arr_buffer[..len]);
                    drop(arr_buffer);
                    self.keep_ref(index, array_obj, vm)?;
                    return Ok(());
                }
            }
        }

        // Get field type code for special handling
        let field_type_code = proto
            .get_attr("_type_", vm)
            .ok()
            .and_then(|attr| attr.downcast_ref::<PyStr>().map(|s| s.to_string()));

        // c_char_p (z type) with bytes: store original in _objects, keep
        // null-terminated copy alive separately for the pointer.
        if field_type_code.as_deref() == Some("z")
            && let Some(bytes_val) = value.downcast_ref::<PyBytes>()
        {
            let (kept_alive, ptr) = ensure_z_null_terminated(bytes_val, vm);
            let result =
                rustpython_host_env::ctypes::pointer_to_sized_bytes_endian(ptr, size, needs_swap);
            self.write_bytes_at_offset(offset, &result);
            self.keep_ref(index, value, vm)?;
            self.keep_alive(index, kept_alive);
            return Ok(());
        }

        let (mut bytes, converted_value) = if let Some(type_code) = &field_type_code {
            PyCField::value_to_bytes_for_type(type_code, &value, size, vm)?
        } else {
            (PyCField::value_to_bytes(&value, size, vm)?, None)
        };

        // Swap bytes for opposite endianness
        if needs_swap {
            bytes.reverse();
        }

        self.write_bytes_at_offset(offset, &bytes);

        // KeepRef: for z/Z types use converted value, otherwise use original
        if let Some(converted) = converted_value {
            self.keep_ref(index, converted, vm)?;
        } else if Self::should_keep_ref(&value) {
            let to_keep = Self::get_kept_objects(&value, vm);
            self.keep_ref(index, to_keep, vm)?;
        }

        Ok(())
    }

    /// PyCData_get
    /// Gets a field value at the given offset
    pub fn get_field(
        &self,
        proto: &PyObject,
        index: usize,
        size: usize,
        offset: usize,
        base_obj: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult {
        // Get buffer data at offset
        let buffer = self.buffer.read();
        if offset + size > buffer.len() {
            return Ok(vm.ctx.new_int(0).into());
        }

        // Check if field type is an array type
        if let Some(type_ref) = proto.downcast_ref::<PyType>()
            && let Some(stg) = type_ref.stg_info_opt()
            && stg.element_type.is_some()
        {
            // c_char array → return bytes
            if PyCField::is_char_array(proto, vm) {
                let data = &buffer[offset..offset + size];
                return Ok(vm
                    .ctx
                    .new_bytes(char_array_field_value(data).to_vec())
                    .into());
            }

            // c_wchar array → return str
            if PyCField::is_wchar_array(proto, vm) {
                let data = &buffer[offset..offset + size];
                return Ok(vm.ctx.new_str(wchar_array_field_value(data)).into());
            }

            // Other array types - create array with a copy of data from the base's buffer
            // The array also keeps a reference to the base for keeping it alive and for writes
            let array_data = buffer[offset..offset + size].to_vec();
            drop(buffer);

            let cdata_obj =
                Self::from_base_with_data(base_obj, offset, index, stg.length, array_data);
            let array_type: PyTypeRef = proto
                .to_owned()
                .downcast()
                .map_err(|_| vm.new_type_error("expected array type"))?;

            return super::array::PyCArray(cdata_obj)
                .into_ref_with_type(vm, array_type)
                .map(Into::into);
        }

        let buffer_data = buffer[offset..offset + size].to_vec();
        drop(buffer);

        // Get proto as type
        let proto_type: PyTypeRef = proto
            .to_owned()
            .downcast()
            .map_err(|_| vm.new_type_error("field proto is not a type"))?;

        let proto_metaclass = proto_type.class();

        // Simple types: return primitive value
        if proto_metaclass.fast_issubclass(super::simple::PyCSimpleType::static_type()) {
            // Check for byte swapping
            let needs_swap = base_obj
                .class()
                .as_object()
                .get_attr("_swappedbytes_", vm)
                .is_ok()
                || proto_type
                    .as_object()
                    .get_attr("_swappedbytes_", vm)
                    .is_ok();

            let data = if needs_swap && size > 1 {
                let mut swapped = buffer_data.clone();
                swapped.reverse();
                swapped
            } else {
                buffer_data
            };

            return bytes_to_pyobject(&proto_type, &data, vm);
        }

        // Complex types: create ctypes instance via PyCData_FromBaseObj
        let ptr = self.buffer.read().as_ptr().wrapping_add(offset) as *mut u8;
        let cdata_obj = unsafe { Self::from_base_obj(ptr, size, base_obj.clone(), index) };

        if proto_metaclass.fast_issubclass(super::structure::PyCStructType::static_type())
            || proto_metaclass.fast_issubclass(super::union::PyCUnionType::static_type())
            || proto_metaclass.fast_issubclass(super::pointer::PyCPointerType::static_type())
        {
            cdata_obj.into_ref_with_type(vm, proto_type).map(Into::into)
        } else {
            // Fallback
            Ok(vm.ctx.new_int(0).into())
        }
    }
}

#[pyclass(flags(BASETYPE))]
impl PyCData {
    #[pygetset]
    fn _objects(&self) -> Option<PyObjectRef> {
        self.objects.read().clone()
    }

    #[pygetset]
    fn _b_base_(&self) -> Option<PyObjectRef> {
        self.base.read().clone()
    }

    #[pygetset]
    fn _b_needsfree_(&self) -> i32 {
        // Borrowed (from_address) or has base object → 0 (don't free)
        // Owned and no base → 1 (need to free)
        if self.is_borrowed() || self.base.read().is_some() {
            0
        } else {
            1
        }
    }

    // CDataType_methods - shared across all ctypes types

    #[pyclassmethod]
    pub(super) fn from_buffer(
        cls: PyTypeRef,
        source: PyObjectRef,
        offset: OptionalArg<isize>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let cdata = Self::from_buffer_impl(&cls, source, offset.unwrap_or(0), vm)?;
        cdata.into_ref_with_type(vm, cls).map(Into::into)
    }

    #[pyclassmethod]
    pub(super) fn from_buffer_copy(
        cls: PyTypeRef,
        source: ArgBytesLike,
        offset: OptionalArg<isize>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let cdata =
            Self::from_buffer_copy_impl(&cls, &source.borrow_buf(), offset.unwrap_or(0), vm)?;
        cdata.into_ref_with_type(vm, cls).map(Into::into)
    }

    #[pyclassmethod]
    pub(super) fn from_address(cls: PyTypeRef, address: isize, vm: &VirtualMachine) -> PyResult {
        let size = {
            let stg_info = cls.stg_info(vm)?;
            stg_info.size
        };

        if size == 0 {
            return Err(vm.new_type_error("abstract class"));
        }

        // PyCData_AtAddress
        let cdata = unsafe { Self::at_address(address as *const u8, size) };
        cdata.into_ref_with_type(vm, cls).map(Into::into)
    }

    #[pyclassmethod]
    pub(super) fn in_dll(
        cls: PyTypeRef,
        dll: PyObjectRef,
        name: crate::builtins::PyStrRef,
        vm: &VirtualMachine,
    ) -> PyResult {
        let size = {
            let stg_info = cls.stg_info(vm)?;
            stg_info.size
        };

        if size == 0 {
            return Err(vm.new_type_error("abstract class"));
        }

        // Get the library handle from dll object
        let handle = if let Ok(int_handle) = dll.try_int(vm) {
            int_handle
                .as_bigint()
                .to_usize()
                .ok_or_else(|| vm.new_value_error("Invalid library handle"))?
        } else {
            dll.get_attr("_handle", vm)?
                .try_int(vm)?
                .as_bigint()
                .to_usize()
                .ok_or_else(|| vm.new_value_error("Invalid library handle"))?
        };

        let symbol_name_with_nul = format!("{}\0", name.as_wtf8());
        let ptr = match rustpython_host_env::ctypes::lookup_data_symbol_addr(
            handle,
            symbol_name_with_nul.as_bytes(),
        ) {
            Ok(ptr) => ptr as *const u8,
            Err(rustpython_host_env::ctypes::LookupSymbolError::LibraryNotFound) => {
                return Err(vm.new_value_error("Library not found"));
            }
            Err(rustpython_host_env::ctypes::LookupSymbolError::LibraryClosed) => {
                return Err(vm.new_value_error("Library closed"));
            }
            Err(rustpython_host_env::ctypes::LookupSymbolError::Load(_)) => {
                return Err(vm.new_value_error(format!("symbol '{}' not found", name.as_wtf8())));
            }
        };

        // dlsym can return NULL for symbols that resolve to NULL (e.g., GNU IFUNC)
        // Treat NULL addresses as errors
        if ptr.is_null() {
            return Err(vm.new_value_error(format!("symbol '{}' not found", name.as_wtf8())));
        }

        // PyCData_AtAddress
        let cdata = unsafe { Self::at_address(ptr, size) };
        cdata.into_ref_with_type(vm, cls).map(Into::into)
    }
}

// PyCField - Field descriptor for Structure/Union types

/// CField descriptor for Structure/Union field access
#[pyclass(name = "CField", module = "_ctypes")]
#[derive(Debug, PyPayload)]
pub struct PyCField {
    /// Field name
    pub(crate) name: String,
    /// Byte offset of the field within the structure/union
    pub(crate) offset: isize,
    /// Byte size of the underlying type
    pub(crate) byte_size_val: isize,
    /// Index into PyCData's object array
    pub(crate) index: usize,
    /// The ctypes type for this field
    pub(crate) proto: PyTypeRef,
    /// Flag indicating if the field is anonymous (MakeAnonFields sets this)
    pub(crate) anonymous: bool,
    /// Bitfield size in bits (0 for non-bitfield)
    pub(crate) bitfield_size: u16,
    /// Bit offset within the storage unit (only meaningful for bitfields)
    pub(crate) bit_offset_val: u16,
}

impl PyCField {
    /// Create a new CField descriptor (non-bitfield)
    pub fn new(
        name: String,
        proto: PyTypeRef,
        offset: isize,
        byte_size: isize,
        index: usize,
    ) -> Self {
        Self {
            name,
            offset,
            byte_size_val: byte_size,
            index,
            proto,
            anonymous: false,
            bitfield_size: 0,
            bit_offset_val: 0,
        }
    }

    /// Create a new CField descriptor for a bitfield
    pub fn new_bitfield(
        name: String,
        proto: PyTypeRef,
        offset: isize,
        byte_size: isize,
        bitfield_size: u16,
        bit_offset: u16,
        index: usize,
    ) -> Self {
        Self {
            name,
            offset,
            byte_size_val: byte_size,
            index,
            proto,
            anonymous: false,
            bitfield_size,
            bit_offset_val: bit_offset,
        }
    }

    /// Get the byte size of the field's underlying type
    pub fn get_byte_size(&self) -> usize {
        self.byte_size_val as usize
    }

    /// Create a new CField from an existing field with adjusted offset and index
    /// Used by MakeFields to promote anonymous fields
    pub fn new_from_field(fdescr: &PyCField, index_offset: usize, offset_delta: isize) -> Self {
        Self {
            name: fdescr.name.clone(),
            offset: fdescr.offset + offset_delta,
            byte_size_val: fdescr.byte_size_val,
            index: fdescr.index + index_offset,
            proto: fdescr.proto.clone(),
            anonymous: false, // promoted fields are not anonymous themselves
            bitfield_size: fdescr.bitfield_size,
            bit_offset_val: fdescr.bit_offset_val,
        }
    }

    /// Set anonymous flag
    pub fn set_anonymous(&mut self, anonymous: bool) {
        self.anonymous = anonymous;
    }
}

impl Constructor for PyCField {
    type Args = crate::function::FuncArgs;

    fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
        // PyCField_new_impl: requires _internal_use=True
        let internal_use = if let Some(v) = args.kwargs.get("_internal_use") {
            v.clone().try_to_bool(vm)?
        } else {
            false
        };

        if !internal_use {
            return Err(vm.new_type_error(
                "CField is not intended to be used directly; use it via Structure or Union fields",
            ));
        }

        let name: String = args
            .kwargs
            .get("name")
            .ok_or_else(|| vm.new_type_error("missing required argument: 'name'"))?
            .try_to_value(vm)?;

        let field_type: PyTypeRef = args
            .kwargs
            .get("type")
            .ok_or_else(|| vm.new_type_error("missing required argument: 'type'"))?
            .clone()
            .downcast()
            .map_err(|_| vm.new_type_error("'type' must be a ctypes type"))?;

        let byte_size: isize = args
            .kwargs
            .get("byte_size")
            .ok_or_else(|| vm.new_type_error("missing required argument: 'byte_size'"))?
            .try_to_value(vm)?;

        let byte_offset: isize = args
            .kwargs
            .get("byte_offset")
            .ok_or_else(|| vm.new_type_error("missing required argument: 'byte_offset'"))?
            .try_to_value(vm)?;

        let index: usize = args
            .kwargs
            .get("index")
            .ok_or_else(|| vm.new_type_error("missing required argument: 'index'"))?
            .try_to_value(vm)?;

        // Validate byte_size matches the type
        let type_size = super::base::get_field_size(field_type.as_object(), vm)? as isize;
        if byte_size != type_size {
            return Err(vm.new_value_error(format!(
                "byte_size {} does not match type size {}",
                byte_size, type_size
            )));
        }

        let bit_size_val: Option<isize> = args
            .kwargs
            .get("bit_size")
            .map(|v| v.try_to_value(vm))
            .transpose()?;

        let bit_offset_val: Option<isize> = args
            .kwargs
            .get("bit_offset")
            .map(|v| v.try_to_value(vm))
            .transpose()?;

        if let Some(bs) = bit_size_val {
            if bs < 0 {
                return Err(vm.new_value_error("number of bits invalid for bit field"));
            }
            let bo = bit_offset_val.unwrap_or(0);
            if bo < 0 {
                return Err(vm.new_value_error("bit_offset must be >= 0"));
            }
            let type_bits = byte_size * 8;
            if bo + bs > type_bits {
                return Err(vm.new_value_error(format!(
                    "bit field '{}' overflows its type ({} + {} > {})",
                    name, bo, bs, type_bits
                )));
            }
            Ok(Self::new_bitfield(
                name,
                field_type,
                byte_offset,
                byte_size,
                bs as u16,
                bo as u16,
                index,
            ))
        } else {
            Ok(Self::new(name, field_type, byte_offset, byte_size, index))
        }
    }
}

impl Representable for PyCField {
    fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
        // Get type name from proto (which is always PyTypeRef)
        let tp_name = zelf.proto.name().to_string();

        // Bitfield: <Field type=TYPE, ofs=OFFSET:BIT_OFFSET, bits=NUM_BITS>
        // Regular:  <Field type=TYPE, ofs=OFFSET, size=SIZE>
        if zelf.bitfield_size > 0 {
            Ok(format!(
                "<Field type={}, ofs={}:{}, bits={}>",
                tp_name, zelf.offset, zelf.bit_offset_val, zelf.bitfield_size
            ))
        } else {
            Ok(format!(
                "<Field type={}, ofs={}, size={}>",
                tp_name, zelf.offset, zelf.byte_size_val
            ))
        }
    }
}

/// PyCField_get
impl GetDescriptor for PyCField {
    fn descr_get(
        zelf: PyObjectRef,
        obj: Option<PyObjectRef>,
        _cls: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let zelf = zelf
            .downcast::<PyCField>()
            .map_err(|_| vm.new_type_error("expected CField"))?;

        // If obj is None, return the descriptor itself (class attribute access)
        let obj = match obj {
            Some(obj) if !vm.is_none(&obj) => obj,
            _ => return Ok(zelf.into()),
        };

        let offset = zelf.offset as usize;
        let size = zelf.get_byte_size();

        // Get PyCData from obj (works for both Structure and Union)
        let cdata = PyCField::get_cdata_from_obj(&obj, vm)?;

        // PyCData_get
        cdata.get_field(
            zelf.proto.as_object(),
            zelf.index,
            size,
            offset,
            obj.clone(),
            vm,
        )
    }
}

impl PyCField {
    /// Convert a Python value to bytes
    fn value_to_bytes(value: &PyObject, size: usize, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        // 1. Handle bytes objects
        if let Some(bytes) = value.downcast_ref::<PyBytes>() {
            Ok(rustpython_host_env::ctypes::copy_to_sized_bytes(
                bytes.as_bytes(),
                size,
            ))
        }
        // 2. Handle ctypes array instances (copy their buffer)
        else if let Some(cdata) = value.downcast_ref::<super::PyCData>() {
            let buffer = cdata.buffer.read();
            Ok(rustpython_host_env::ctypes::copy_to_sized_bytes(
                &buffer, size,
            ))
        }
        // 4. Handle float values (check before int, since float.try_int would truncate)
        else if let Some(float_val) = value.downcast_ref::<crate::builtins::PyFloat>() {
            let f = float_val.to_f64();
            match size {
                4 | 8 => Ok(rustpython_host_env::ctypes::float_to_sized_bytes(f, size)
                    .expect("float size checked")),
                _ => unreachable!("wrong payload size"),
            }
        }
        // 4. Handle integer values
        else if let Ok(int_val) = value.try_int(vm) {
            let i = int_val.as_bigint();
            match size {
                1 => Ok(rustpython_host_env::ctypes::int_to_sized_bytes(
                    i.to_i8().unwrap_or(0).into(),
                    size,
                )),
                2 => Ok(rustpython_host_env::ctypes::int_to_sized_bytes(
                    i.to_i16().unwrap_or(0).into(),
                    size,
                )),
                4 => Ok(rustpython_host_env::ctypes::int_to_sized_bytes(
                    i.to_i32().unwrap_or(0).into(),
                    size,
                )),
                8 => Ok(rustpython_host_env::ctypes::int_to_sized_bytes(
                    i.to_i64().unwrap_or(0),
                    size,
                )),
                _ => Ok(rustpython_host_env::ctypes::zeroed_bytes(size)),
            }
        } else {
            Ok(rustpython_host_env::ctypes::zeroed_bytes(size))
        }
    }

    /// Convert a Python value to bytes with type-specific handling for pointer types.
    /// Returns (bytes, optional holder for wchar buffer).
    fn value_to_bytes_for_type(
        type_code: &str,
        value: &PyObject,
        size: usize,
        vm: &VirtualMachine,
    ) -> PyResult<(Vec<u8>, Option<PyObjectRef>)> {
        match type_code {
            // c_float: always convert to float first (f_set)
            "f" => {
                let f = if let Some(float_val) = value.downcast_ref::<crate::builtins::PyFloat>() {
                    float_val.to_f64()
                } else if let Ok(int_val) = value.try_int(vm) {
                    int_val.as_bigint().to_i64().unwrap_or(0) as f64
                } else {
                    return Err(vm.new_type_error(format!(
                        "float expected instead of {}",
                        value.class().name()
                    )));
                };
                Ok((
                    rustpython_host_env::ctypes::float_to_sized_bytes(f, 4)
                        .expect("c_float size is fixed"),
                    None,
                ))
            }
            // c_double: always convert to float first (d_set)
            "d" => {
                let f = if let Some(float_val) = value.downcast_ref::<crate::builtins::PyFloat>() {
                    float_val.to_f64()
                } else if let Ok(int_val) = value.try_int(vm) {
                    int_val.as_bigint().to_i64().unwrap_or(0) as f64
                } else {
                    return Err(vm.new_type_error(format!(
                        "float expected instead of {}",
                        value.class().name()
                    )));
                };
                Ok((
                    rustpython_host_env::ctypes::float_to_sized_bytes(f, 8)
                        .expect("c_double size is fixed"),
                    None,
                ))
            }
            // c_longdouble: convert to float (treated as f64 in RustPython)
            "g" => {
                let f = if let Some(float_val) = value.downcast_ref::<crate::builtins::PyFloat>() {
                    float_val.to_f64()
                } else if let Ok(int_val) = value.try_int(vm) {
                    int_val.as_bigint().to_i64().unwrap_or(0) as f64
                } else {
                    return Err(vm.new_type_error(format!(
                        "float expected instead of {}",
                        value.class().name()
                    )));
                };
                Ok((
                    rustpython_host_env::ctypes::float_to_sized_bytes(f, 8)
                        .expect("c_longdouble bytes are stored as f64"),
                    None,
                ))
            }
            "z" => {
                // c_char_p with bytes is handled in set_field before this call.
                // This handles integer address and None cases.
                // Integer address
                if let Ok(int_val) = value.try_index(vm) {
                    let v = int_val.as_bigint().to_usize().unwrap_or(0);
                    return Ok((
                        rustpython_host_env::ctypes::pointer_to_sized_bytes(v, size),
                        None,
                    ));
                }
                // None -> NULL pointer
                if vm.is_none(value) {
                    return Ok((rustpython_host_env::ctypes::zeroed_bytes(size), None));
                }
                Ok((PyCField::value_to_bytes(value, size, vm)?, None))
            }
            "Z" => {
                // c_wchar_p: store pointer to null-terminated wchar_t buffer
                if let Some(s) = value.downcast_ref::<PyStr>() {
                    let (holder, ptr) = str_to_wchar_bytes(s.as_wtf8(), vm);
                    return Ok((
                        rustpython_host_env::ctypes::pointer_to_sized_bytes(ptr, size),
                        Some(holder),
                    ));
                }
                // Integer address
                if let Ok(int_val) = value.try_index(vm) {
                    let v = int_val.as_bigint().to_usize().unwrap_or(0);
                    return Ok((
                        rustpython_host_env::ctypes::pointer_to_sized_bytes(v, size),
                        None,
                    ));
                }
                // None -> NULL pointer
                if vm.is_none(value) {
                    return Ok((rustpython_host_env::ctypes::zeroed_bytes(size), None));
                }
                Ok((PyCField::value_to_bytes(value, size, vm)?, None))
            }
            "P" => {
                // c_void_p: store integer as pointer
                if let Ok(int_val) = value.try_index(vm) {
                    let v = int_val.as_bigint().to_usize().unwrap_or(0);
                    return Ok((
                        rustpython_host_env::ctypes::pointer_to_sized_bytes(v, size),
                        None,
                    ));
                }
                // None -> NULL pointer
                if vm.is_none(value) {
                    return Ok((rustpython_host_env::ctypes::zeroed_bytes(size), None));
                }
                Ok((PyCField::value_to_bytes(value, size, vm)?, None))
            }
            _ => Ok((PyCField::value_to_bytes(value, size, vm)?, None)),
        }
    }

    /// Check if the field type is a c_char array (element type has _type_ == 'c')
    fn is_char_array(proto: &PyObject, vm: &VirtualMachine) -> bool {
        // Get element_type from StgInfo (for array types)
        if let Some(proto_type) = proto.downcast_ref::<PyType>()
            && let Some(stg) = proto_type.stg_info_opt()
            && let Some(element_type) = &stg.element_type
        {
            // Check if element type has _type_ == "c"
            if let Ok(type_code) = element_type.as_object().get_attr("_type_", vm)
                && let Some(s) = type_code.downcast_ref::<PyStr>()
            {
                return s.as_bytes() == b"c";
            }
        }
        false
    }

    /// Check if the field type is a c_wchar array (element type has _type_ == 'u')
    fn is_wchar_array(proto: &PyObject, vm: &VirtualMachine) -> bool {
        // Get element_type from StgInfo (for array types)
        if let Some(proto_type) = proto.downcast_ref::<PyType>()
            && let Some(stg) = proto_type.stg_info_opt()
            && let Some(element_type) = &stg.element_type
        {
            // Check if element type has _type_ == "u"
            if let Ok(type_code) = element_type.as_object().get_attr("_type_", vm)
                && let Some(s) = type_code.downcast_ref::<PyStr>()
            {
                return s.as_bytes() == b"u";
            }
        }
        false
    }
}

#[pyclass(flags(IMMUTABLETYPE), with(Representable, GetDescriptor, Constructor))]
impl PyCField {
    /// Get PyCData from object (works for both Structure and Union)
    fn get_cdata_from_obj<'a>(obj: &'a PyObjectRef, vm: &VirtualMachine) -> PyResult<&'a PyCData> {
        if let Some(s) = obj.downcast_ref::<super::structure::PyCStructure>() {
            Ok(&s.0)
        } else if let Some(u) = obj.downcast_ref::<super::union::PyCUnion>() {
            Ok(&u.0)
        } else {
            Err(vm.new_type_error(format!(
                "descriptor works only on Structure or Union instances, got {}",
                obj.class().name()
            )))
        }
    }

    /// PyCField_set
    #[pyslot]
    fn descr_set(
        zelf: &PyObject,
        obj: PyObjectRef,
        value: PySetterValue<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let zelf = zelf
            .downcast_ref::<PyCField>()
            .ok_or_else(|| vm.new_type_error("expected CField"))?;

        let offset = zelf.offset as usize;
        let size = zelf.get_byte_size();

        // Get PyCData from obj (works for both Structure and Union)
        let cdata = Self::get_cdata_from_obj(&obj, vm)?;

        match value {
            PySetterValue::Assign(value) => {
                // Check if needs byte swapping
                let needs_swap = (obj
                    .class()
                    .as_object()
                    .get_attr("_swappedbytes_", vm)
                    .is_ok()
                    || zelf
                        .proto
                        .as_object()
                        .get_attr("_swappedbytes_", vm)
                        .is_ok())
                    && size > 1;

                // PyCData_set
                cdata.set_field(
                    zelf.proto.as_object(),
                    value,
                    zelf.index,
                    size,
                    offset,
                    needs_swap,
                    vm,
                )
            }
            PySetterValue::Delete => Err(vm.new_type_error("cannot delete field")),
        }
    }

    #[pygetset]
    fn name(&self) -> String {
        self.name.clone()
    }

    #[pygetset(name = "type")]
    fn type_(&self) -> PyTypeRef {
        self.proto.clone()
    }

    #[pygetset]
    fn offset(&self) -> isize {
        self.offset
    }

    #[pygetset]
    fn byte_offset(&self) -> isize {
        self.offset
    }

    #[pygetset]
    fn size(&self) -> isize {
        // Legacy: encode as (bitfield_size << 16) | bit_offset for bitfields
        if self.bitfield_size > 0 {
            ((self.bitfield_size as isize) << 16) | (self.bit_offset_val as isize)
        } else {
            self.byte_size_val
        }
    }

    #[pygetset]
    fn byte_size(&self) -> isize {
        self.byte_size_val
    }

    #[pygetset]
    fn bit_offset(&self) -> isize {
        self.bit_offset_val as isize
    }

    #[pygetset]
    fn bit_size(&self, vm: &VirtualMachine) -> PyObjectRef {
        if self.bitfield_size > 0 {
            vm.ctx.new_int(self.bitfield_size).into()
        } else {
            // Non-bitfield: bit_size = byte_size * 8
            let byte_size = self.byte_size_val as i128;
            vm.ctx.new_int(byte_size * 8).into()
        }
    }

    #[pygetset]
    fn is_bitfield(&self) -> bool {
        self.bitfield_size > 0
    }

    #[pygetset]
    fn is_anonymous(&self) -> bool {
        self.anonymous
    }
}

// ParamFunc implementations (PyCArgObject creation)

use super::_ctypes::CArgObject;

/// Call the appropriate paramfunc based on StgInfo.paramfunc
/// info->paramfunc(st, obj)
pub(super) fn call_paramfunc(obj: &PyObject, vm: &VirtualMachine) -> PyResult<CArgObject> {
    let cls = obj.class();
    let stg_info = cls
        .stg_info_opt()
        .ok_or_else(|| vm.new_type_error("not a ctypes type"))?;

    match stg_info.paramfunc {
        ParamFunc::Simple => simple_paramfunc(obj, vm),
        ParamFunc::Array => array_paramfunc(obj, vm),
        ParamFunc::Pointer => pointer_paramfunc(obj, vm),
        ParamFunc::Structure | ParamFunc::Union => struct_union_paramfunc(obj, &stg_info, vm),
        ParamFunc::None => Err(vm.new_type_error("no paramfunc")),
    }
}

/// PyCSimpleType_paramfunc
fn simple_paramfunc(obj: &PyObject, vm: &VirtualMachine) -> PyResult<CArgObject> {
    use super::simple::PyCSimple;

    let simple = obj
        .downcast_ref::<PyCSimple>()
        .ok_or_else(|| vm.new_type_error("expected simple type"))?;

    // Get type code from _type_ attribute
    let cls = obj.class().to_owned();
    let type_code = cls
        .type_code(vm)
        .ok_or_else(|| vm.new_type_error("no _type_ attribute"))?;
    let tag = type_code.as_bytes().first().copied().unwrap_or(b'?');

    // Read value from buffer: memcpy(&parg->value, self->b_ptr, self->b_size)
    let buffer = simple.0.buffer.read();
    let ffi_value = buffer_to_ffi_value(&type_code, &buffer);

    Ok(CArgObject {
        tag,
        value: ffi_value,
        obj: obj.to_owned(),
        size: 0,
        offset: 0,
    })
}

/// PyCArrayType_paramfunc
fn array_paramfunc(obj: &PyObject, vm: &VirtualMachine) -> PyResult<CArgObject> {
    use super::array::PyCArray;

    let array = obj
        .downcast_ref::<PyCArray>()
        .ok_or_else(|| vm.new_type_error("expected array"))?;

    // p->value.p = (char *)self->b_ptr
    let buffer = array.0.buffer.read();
    let ptr_val = buffer.as_ptr() as usize;

    Ok(CArgObject {
        tag: b'P',
        value: FfiArgValue::pointer(ptr_val),
        obj: obj.to_owned(),
        size: 0,
        offset: 0,
    })
}

/// PyCPointerType_paramfunc
fn pointer_paramfunc(obj: &PyObject, vm: &VirtualMachine) -> PyResult<CArgObject> {
    use super::pointer::PyCPointer;

    let ptr = obj
        .downcast_ref::<PyCPointer>()
        .ok_or_else(|| vm.new_type_error("expected pointer"))?;

    // parg->value.p = *(void **)self->b_ptr
    let ptr_val = ptr.get_ptr_value();

    Ok(CArgObject {
        tag: b'P',
        value: FfiArgValue::pointer(ptr_val),
        obj: obj.to_owned(),
        size: 0,
        offset: 0,
    })
}

/// StructUnionType_paramfunc (for both Structure and Union)
fn struct_union_paramfunc(
    obj: &PyObject,
    stg_info: &StgInfo,
    _vm: &VirtualMachine,
) -> PyResult<CArgObject> {
    // Get buffer pointer
    // For large structs (> sizeof(void*)), we'd need to allocate and copy.
    // For now, just point to buffer directly and keep obj reference for memory safety.
    let buffer = if let Some(cdata) = obj.downcast_ref::<PyCData>() {
        cdata.buffer.read()
    } else {
        return Ok(CArgObject {
            tag: b'V',
            value: FfiArgValue::pointer(0),
            obj: obj.to_owned(),
            size: stg_info.size,
            offset: 0,
        });
    };

    let ptr_val = buffer.as_ptr() as usize;
    let size = buffer.len();

    Ok(CArgObject {
        tag: b'V',
        value: FfiArgValue::pointer(ptr_val),
        obj: obj.to_owned(),
        size,
        offset: 0,
    })
}

// FfiArgValue - Owned FFI argument value

/// Owned FFI argument value. Keeps the value alive for the duration of the FFI call.
#[derive(Debug, Clone)]
pub enum FfiArgValue {
    Scalar(FfiValue),
    /// Pointer with owned data. The PyObjectRef keeps the pointed data alive.
    OwnedPointer(usize, #[allow(dead_code)] PyObjectRef),
}

impl FfiArgValue {
    pub fn pointer(value: usize) -> Self {
        Self::Scalar(FfiValue::Pointer(value))
    }

    /// Create an Arg reference to this owned value
    pub fn as_arg(&self) -> FfiArg<'_> {
        match self {
            FfiArgValue::Scalar(value) => ffi_arg_from_value(value),
            FfiArgValue::OwnedPointer(v, _) => rustpython_host_env::ctypes::ffi_arg(
                rustpython_host_env::ctypes::FfiArgRef::Pointer(v),
            ),
        }
    }
}

/// Convert buffer bytes to FfiArgValue based on type code
pub(super) fn buffer_to_ffi_value(type_code: &str, buffer: &[u8]) -> FfiArgValue {
    FfiArgValue::Scalar(rustpython_host_env::ctypes::ffi_value_from_type_code(
        type_code, buffer,
    ))
}

/// Convert bytes to appropriate Python object based on ctypes type
pub(super) fn bytes_to_pyobject(
    cls: &Py<PyType>,
    bytes: &[u8],
    vm: &VirtualMachine,
) -> PyResult<PyObjectRef> {
    // Try to get _type_ attribute
    if let Ok(type_attr) = cls.as_object().get_attr("_type_", vm)
        && let Ok(s) = type_attr.str(vm)
    {
        let ty = s.to_string();
        return match rustpython_host_env::ctypes::decode_type_code(ty.as_str(), bytes) {
            rustpython_host_env::ctypes::DecodedValue::Bytes(value) => {
                Ok(vm.ctx.new_bytes(value).into())
            }
            rustpython_host_env::ctypes::DecodedValue::Signed(value) => {
                Ok(vm.ctx.new_int(value).into())
            }
            rustpython_host_env::ctypes::DecodedValue::Unsigned(value) => {
                Ok(vm.ctx.new_int(value).into())
            }
            rustpython_host_env::ctypes::DecodedValue::Float(value) => {
                Ok(vm.ctx.new_float(value).into())
            }
            rustpython_host_env::ctypes::DecodedValue::Bool(value) => {
                Ok(vm.ctx.new_bool(value).into())
            }
            rustpython_host_env::ctypes::DecodedValue::Pointer(value) => {
                if value == 0 {
                    Ok(vm.ctx.none())
                } else {
                    Ok(vm.ctx.new_int(value).into())
                }
            }
            rustpython_host_env::ctypes::DecodedValue::String(value) => {
                Ok(vm.ctx.new_str(value).into())
            }
            rustpython_host_env::ctypes::DecodedValue::None => Ok(vm.ctx.none()),
        };
    }
    // Default: return bytes as-is
    Ok(vm.ctx.new_bytes(bytes.to_vec()).into())
}

// Shared functions for Structure and Union types

/// Parse a non-negative integer attribute, returning default if not present
pub(super) fn get_usize_attr(
    obj: &PyObject,
    attr: &str,
    default: usize,
    vm: &VirtualMachine,
) -> PyResult<usize> {
    let Ok(attr_val) = obj.get_attr(vm.ctx.intern_str(attr), vm) else {
        return Ok(default);
    };
    let n = attr_val
        .try_int(vm)
        .map_err(|_| vm.new_value_error(format!("{attr} must be a non-negative integer")))?;
    let val = n.as_bigint();
    if val.is_negative() {
        return Err(vm.new_value_error(format!("{attr} must be a non-negative integer")));
    }
    Ok(val.to_usize().unwrap_or(default))
}

/// Check if a type is a "simple instance" (direct subclass of a simple type)
/// Returns TRUE for c_int, c_void_p, etc. (simple types with _type_ attribute)
/// Returns FALSE for Structure, Array, POINTER(T), etc.
pub(super) fn is_simple_instance(typ: &Py<PyType>) -> bool {
    // _ctypes_simple_instance
    // Check if the type's metaclass is PyCSimpleType
    let metaclass = typ.class();
    metaclass.fast_issubclass(super::simple::PyCSimpleType::static_type())
}

/// Set or initialize StgInfo on a type
pub(super) fn set_or_init_stginfo(type_ref: &PyType, stg_info: StgInfo) {
    if type_ref.init_type_data(stg_info.clone()).is_err()
        && let Some(mut existing) = type_ref.get_type_data_mut::<StgInfo>()
    {
        // Preserve pointer_type cache across StgInfo replacement
        let old_pointer_type = existing.pointer_type.take();
        *existing = stg_info;
        if existing.pointer_type.is_none() {
            existing.pointer_type = old_pointer_type;
        }
    }
}

/// Check if a field type supports byte order swapping
pub(super) fn check_other_endian_support(
    field_type: &PyObject,
    vm: &VirtualMachine,
) -> PyResult<()> {
    let other_endian_attr = if cfg!(target_endian = "little") {
        "__ctype_be__"
    } else {
        "__ctype_le__"
    };

    if field_type.get_attr(other_endian_attr, vm).is_ok() {
        return Ok(());
    }

    // Array type: recursively check element type
    if let Ok(elem_type) = field_type.get_attr("_type_", vm)
        && field_type.get_attr("_length_", vm).is_ok()
    {
        return check_other_endian_support(&elem_type, vm);
    }

    // Structure/Union: has StgInfo but no _type_ attribute
    if let Some(type_obj) = field_type.downcast_ref::<PyType>()
        && type_obj.stg_info_opt().is_some()
        && field_type.get_attr("_type_", vm).is_err()
    {
        return Ok(());
    }

    Err(vm.new_type_error(format!(
        "This type does not support other endian: {}",
        field_type.class().name()
    )))
}

/// Get the size of a ctypes field type
pub(super) fn get_field_size(field_type: &PyObject, vm: &VirtualMachine) -> PyResult<usize> {
    if let Some(type_obj) = field_type.downcast_ref::<PyType>()
        && let Some(stg_info) = type_obj.stg_info_opt()
    {
        return Ok(stg_info.size);
    }

    if let Some(size) = field_type
        .get_attr("_type_", vm)
        .ok()
        .and_then(|type_attr| type_attr.str(vm).ok())
        .and_then(|type_str| {
            let s = type_str.to_string();
            (s.len() == 1).then(|| rustpython_host_env::ctypes::simple_type_size(&s))
        })
        .flatten()
    {
        return Ok(size);
    }

    if let Some(s) = field_type
        .get_attr("size_of_instances", vm)
        .ok()
        .and_then(|size_method| size_method.call((), vm).ok())
        .and_then(|size| size.try_int(vm).ok())
        .and_then(|n| n.as_bigint().to_usize())
    {
        return Ok(s);
    }

    Ok(rustpython_host_env::ctypes::pointer_size())
}

/// Get the alignment of a ctypes field type
pub(super) fn get_field_align(field_type: &PyObject, vm: &VirtualMachine) -> usize {
    if let Some(type_obj) = field_type.downcast_ref::<PyType>()
        && let Some(stg_info) = type_obj.stg_info_opt()
        && stg_info.align > 0
    {
        return stg_info.align;
    }

    if let Some(align) = field_type
        .get_attr("_type_", vm)
        .ok()
        .and_then(|type_attr| type_attr.str(vm).ok())
        .and_then(|type_str| {
            let s = type_str.to_string();
            (s.len() == 1).then(|| rustpython_host_env::ctypes::simple_type_align(&s))
        })
        .flatten()
    {
        return align;
    }

    1
}

/// Promote fields from anonymous struct/union to parent type
fn make_fields(
    cls: &Py<PyType>,
    descr: &super::PyCField,
    index: usize,
    offset: isize,
    vm: &VirtualMachine,
) -> PyResult<()> {
    let fields = descr.proto.as_object().get_attr("_fields_", vm)?;
    let fieldlist: Vec<PyObjectRef> = if let Some(list) = fields.downcast_ref::<PyList>() {
        list.borrow_vec().to_vec()
    } else if let Some(tuple) = fields.downcast_ref::<PyTuple>() {
        tuple.to_vec()
    } else {
        return Err(vm.new_type_error("_fields_ must be a sequence"));
    };

    for pair in fieldlist.iter() {
        let field_tuple = pair
            .downcast_ref::<PyTuple>()
            .ok_or_else(|| vm.new_type_error("_fields_ must contain tuples"))?;

        if field_tuple.len() < 2 {
            continue;
        }

        let fname = field_tuple
            .first()
            .expect("len checked")
            .downcast_ref::<PyUtf8Str>()
            .ok_or_else(|| vm.new_type_error("field name must be a string"))?;

        let fdescr_obj = descr
            .proto
            .as_object()
            .get_attr(vm.ctx.intern_str(fname.as_str()), vm)?;
        let fdescr = fdescr_obj
            .downcast_ref::<super::PyCField>()
            .ok_or_else(|| vm.new_type_error("unexpected type"))?;

        if fdescr.anonymous {
            make_fields(
                cls,
                fdescr,
                index + fdescr.index,
                offset + fdescr.offset,
                vm,
            )?;
            continue;
        }

        let new_descr = super::PyCField::new_from_field(fdescr, index, offset);
        cls.set_attr(
            vm.ctx.intern_str(fname.as_wtf8()),
            new_descr.to_pyobject(vm),
        );
    }

    Ok(())
}

/// Process _anonymous_ attribute for struct/union
pub(super) fn make_anon_fields(cls: &Py<PyType>, vm: &VirtualMachine) -> PyResult<()> {
    let anon = match cls.as_object().get_attr("_anonymous_", vm) {
        Ok(anon) => anon,
        Err(_) => return Ok(()),
    };

    let anon_names: Vec<PyObjectRef> = if let Some(list) = anon.downcast_ref::<PyList>() {
        list.borrow_vec().to_vec()
    } else if let Some(tuple) = anon.downcast_ref::<PyTuple>() {
        tuple.to_vec()
    } else {
        return Err(vm.new_type_error("_anonymous_ must be a sequence"));
    };

    for fname_obj in anon_names.iter() {
        let fname = fname_obj
            .downcast_ref::<PyStr>()
            .ok_or_else(|| vm.new_type_error("_anonymous_ items must be strings"))?;

        let descr_obj = cls
            .as_object()
            .get_attr(vm.ctx.intern_str(fname.as_wtf8()), vm)?;

        let descr = descr_obj.downcast_ref::<super::PyCField>().ok_or_else(|| {
            vm.new_attribute_error(format!(
                "'{}' is specified in _anonymous_ but not in _fields_",
                fname.as_wtf8()
            ))
        })?;

        let mut new_descr = super::PyCField::new_from_field(descr, 0, 0);
        new_descr.set_anonymous(true);
        cls.set_attr(
            vm.ctx.intern_str(fname.as_wtf8()),
            new_descr.to_pyobject(vm),
        );

        make_fields(cls, descr, descr.index, descr.offset, vm)?;
    }

    Ok(())
}

use super::array::{WCHAR_SIZE, wchar_from_bytes, wchar_to_bytes};
use crate::builtins::{PyBytes, PyDict, PyMemoryView, PyStr, PyType, PyTypeRef};
use crate::class::StaticType;
use crate::function::{ArgBytesLike, OptionalArg, PySetterValue};
use crate::protocol::{BufferMethods, PyBuffer};
use crate::types::{GetDescriptor, Representable};
use crate::{
    AsObject, Py, PyObject, PyObjectRef, PyPayload, PyResult, TryFromObject, VirtualMachine,
};
use alloc::borrow::Cow;
use core::ffi::{
    c_double, c_float, c_int, c_long, c_longlong, c_short, c_uint, c_ulong, c_ulonglong, c_ushort,
};
use core::fmt::Debug;
use core::mem;
use crossbeam_utils::atomic::AtomicCell;
use num_traits::{Signed, ToPrimitive};
use rustpython_common::lock::PyRwLock;
use widestring::WideChar;

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
    pub ffi_field_types: Vec<libffi::middle::Type>,
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
        }
    }

    /// Get libffi type for this StgInfo
    /// Note: For very large types, returns pointer type to avoid overflow
    pub fn to_ffi_type(&self) -> libffi::middle::Type {
        // Limit to avoid overflow in libffi (MAX_STRUCT_SIZE is platform-dependent)
        const MAX_FFI_STRUCT_SIZE: usize = 1024 * 1024; // 1MB limit for safety

        match self.paramfunc {
            ParamFunc::Structure | ParamFunc::Union => {
                if !self.ffi_field_types.is_empty() {
                    libffi::middle::Type::structure(self.ffi_field_types.iter().cloned())
                } else if self.size <= MAX_FFI_STRUCT_SIZE {
                    // Small struct without field types: use bytes array
                    libffi::middle::Type::structure(core::iter::repeat_n(
                        libffi::middle::Type::u8(),
                        self.size,
                    ))
                } else {
                    // Large struct: treat as pointer (passed by reference)
                    libffi::middle::Type::pointer()
                }
            }
            ParamFunc::Array => {
                if self.size > MAX_FFI_STRUCT_SIZE || self.length > MAX_FFI_STRUCT_SIZE {
                    // Large array: treat as pointer
                    libffi::middle::Type::pointer()
                } else if let Some(ref fmt) = self.format {
                    let elem_type = Self::format_to_ffi_type(fmt);
                    libffi::middle::Type::structure(core::iter::repeat_n(elem_type, self.length))
                } else {
                    libffi::middle::Type::structure(core::iter::repeat_n(
                        libffi::middle::Type::u8(),
                        self.size,
                    ))
                }
            }
            ParamFunc::Pointer => libffi::middle::Type::pointer(),
            _ => {
                // Simple type: derive from format
                if let Some(ref fmt) = self.format {
                    Self::format_to_ffi_type(fmt)
                } else {
                    libffi::middle::Type::u8()
                }
            }
        }
    }

    /// Convert format string to libffi type
    fn format_to_ffi_type(fmt: &str) -> libffi::middle::Type {
        // Strip endian prefix if present
        let code = fmt.trim_start_matches(['<', '>', '!', '@', '=']);
        match code {
            "b" => libffi::middle::Type::i8(),
            "B" => libffi::middle::Type::u8(),
            "h" => libffi::middle::Type::i16(),
            "H" => libffi::middle::Type::u16(),
            "i" | "l" => libffi::middle::Type::i32(),
            "I" | "L" => libffi::middle::Type::u32(),
            "q" => libffi::middle::Type::i64(),
            "Q" => libffi::middle::Type::u64(),
            "f" => libffi::middle::Type::f32(),
            "d" => libffi::middle::Type::f64(),
            "P" | "z" | "Z" | "O" => libffi::middle::Type::pointer(),
            _ => libffi::middle::Type::u8(), // default
        }
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
        let s = type_str.as_str();
        if !s.is_empty() {
            return format!("{}{}", endian_prefix, s);
        }
        return s.to_string();
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

/// Convert Vec<T> to Vec<u8> by reinterpreting the memory (same allocation).
fn vec_to_bytes<T>(vec: Vec<T>) -> Vec<u8> {
    let len = vec.len() * core::mem::size_of::<T>();
    let cap = vec.capacity() * core::mem::size_of::<T>();
    let ptr = vec.as_ptr() as *mut u8;
    core::mem::forget(vec);
    unsafe { Vec::from_raw_parts(ptr, len, cap) }
}

/// Ensure PyBytes is null-terminated. Returns (PyBytes to keep, pointer).
/// If already contains null, returns original. Otherwise creates new with null appended.
pub(super) fn ensure_z_null_terminated(
    bytes: &PyBytes,
    vm: &VirtualMachine,
) -> (PyObjectRef, usize) {
    let data = bytes.as_bytes();
    if data.contains(&0) {
        // Already has null, use original
        let original: PyObjectRef = vm.ctx.new_bytes(data.to_vec()).into();
        (original, data.as_ptr() as usize)
    } else {
        // Create new with null appended
        let mut buffer = data.to_vec();
        buffer.push(0);
        let ptr = buffer.as_ptr() as usize;
        let new_bytes: PyObjectRef = vm.ctx.new_bytes(buffer).into();
        (new_bytes, ptr)
    }
}

/// Convert str to null-terminated wchar_t buffer. Returns (PyBytes holder, pointer).
pub(super) fn str_to_wchar_bytes(s: &str, vm: &VirtualMachine) -> (PyObjectRef, usize) {
    let wchars: Vec<libc::wchar_t> = s
        .chars()
        .map(|c| c as libc::wchar_t)
        .chain(core::iter::once(0))
        .collect();
    let ptr = wchars.as_ptr() as usize;
    let bytes = vec_to_bytes(wchars);
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
        let slice: &'static [u8] = unsafe { core::slice::from_raw_parts(ptr, size) };
        PyCData {
            buffer: PyRwLock::new(Cow::Borrowed(slice)),
            base: PyRwLock::new(None),
            base_offset: AtomicCell::new(0),
            index: AtomicCell::new(0),
            objects: PyRwLock::new(None),
            length: AtomicCell::new(0),
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
        let slice: &'static [u8] = unsafe { core::slice::from_raw_parts(ptr, size) };
        PyCData {
            buffer: PyRwLock::new(Cow::Borrowed(slice)),
            base: PyRwLock::new(Some(base_obj)),
            base_offset: AtomicCell::new(0),
            index: AtomicCell::new(idx),
            objects: PyRwLock::new(None),
            length: AtomicCell::new(0),
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
        let slice: &'static [u8] = unsafe { core::slice::from_raw_parts(ptr, size) };

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
        let buffer = self.buffer.read();
        if offset + bytes.len() > buffer.len() {
            return; // Out of bounds
        }

        match &*buffer {
            Cow::Borrowed(slice) => {
                // For borrowed memory, write directly
                // SAFETY: We assume the caller knows this memory is writable
                // (e.g., from from_address pointing to a ctypes buffer)
                unsafe {
                    let ptr = slice.as_ptr() as *mut u8;
                    core::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr.add(offset), bytes.len());
                }
            }
            Cow::Owned(_) => {
                // For owned memory, use to_mut() through write lock
                drop(buffer);
                let mut buffer = self.buffer.write();
                buffer.to_mut()[offset..offset + bytes.len()].copy_from_slice(bytes);
            }
        }
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
                let to_copy = PyCField::bytes_for_char_array(src);
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
                // Convert str to wchar_t bytes (platform-dependent size)
                let mut wchar_bytes = Vec::with_capacity(size);
                for ch in str_val.as_str().chars().take(size / WCHAR_SIZE) {
                    let mut bytes = [0u8; 4];
                    wchar_to_bytes(ch as u32, &mut bytes);
                    wchar_bytes.extend_from_slice(&bytes[..WCHAR_SIZE]);
                }
                // Pad with nulls to fill the array
                while wchar_bytes.len() < size {
                    wchar_bytes.push(0);
                }
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
            let addr_bytes = buffer_addr.to_ne_bytes();
            let len = core::cmp::min(addr_bytes.len(), size);
            self.write_bytes_at_offset(offset, &addr_bytes[..len]);
            self.keep_ref(index, value, vm)?;
            return Ok(());
        }

        // Get field type code for special handling
        let field_type_code = proto
            .get_attr("_type_", vm)
            .ok()
            .and_then(|attr| attr.downcast_ref::<PyStr>().map(|s| s.to_string()));

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
                // Find first null terminator (or use full length)
                let end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
                return Ok(vm.ctx.new_bytes(data[..end].to_vec()).into());
            }

            // c_wchar array → return str
            if PyCField::is_wchar_array(proto, vm) {
                let data = &buffer[offset..offset + size];
                // wchar_t → char conversion, skip null
                let chars: String = data
                    .chunks(WCHAR_SIZE)
                    .filter_map(|chunk| {
                        wchar_from_bytes(chunk)
                            .filter(|&wchar| wchar != 0)
                            .and_then(char::from_u32)
                    })
                    .collect();
                return Ok(vm.ctx.new_str(chars).into());
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
    fn from_buffer(
        cls: PyTypeRef,
        source: PyObjectRef,
        offset: OptionalArg<isize>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let cdata = Self::from_buffer_impl(&cls, source, offset.unwrap_or(0), vm)?;
        cdata.into_ref_with_type(vm, cls).map(Into::into)
    }

    #[pyclassmethod]
    fn from_buffer_copy(
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
    fn from_address(cls: PyTypeRef, address: isize, vm: &VirtualMachine) -> PyResult {
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
    fn in_dll(
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

        // Look up the library in the cache and use lib.get() for symbol lookup
        let library_cache = super::library::libcache().read();
        let library = library_cache
            .get_lib(handle)
            .ok_or_else(|| vm.new_value_error("Library not found"))?;
        let inner_lib = library.lib.lock();

        let symbol_name_with_nul = format!("{}\0", name.as_str());
        let ptr: *const u8 = if let Some(lib) = &*inner_lib {
            unsafe {
                lib.get::<*const u8>(symbol_name_with_nul.as_bytes())
                    .map(|sym| *sym)
                    .map_err(|_| {
                        vm.new_value_error(format!("symbol '{}' not found", name.as_str()))
                    })?
            }
        } else {
            return Err(vm.new_value_error("Library closed"));
        };

        // dlsym can return NULL for symbols that resolve to NULL (e.g., GNU IFUNC)
        // Treat NULL addresses as errors
        if ptr.is_null() {
            return Err(vm.new_value_error(format!("symbol '{}' not found", name.as_str())));
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
    /// Byte offset of the field within the structure/union
    pub(crate) offset: isize,
    /// Encoded size: for bitfields (bit_size << 16) | bit_offset, otherwise byte size
    pub(crate) size: isize,
    /// Index into PyCData's object array
    pub(crate) index: usize,
    /// The ctypes type for this field
    pub(crate) proto: PyTypeRef,
    /// Flag indicating if the field is anonymous (MakeAnonFields sets this)
    pub(crate) anonymous: bool,
}

#[inline(always)]
const fn num_bits(size: isize) -> isize {
    size >> 16
}

#[inline(always)]
const fn field_size(size: isize) -> isize {
    size & 0xFFFF
}

#[inline(always)]
const fn is_bitfield(size: isize) -> bool {
    (size >> 16) != 0
}

impl PyCField {
    /// Create a new CField descriptor (non-bitfield)
    pub fn new(proto: PyTypeRef, offset: isize, size: isize, index: usize) -> Self {
        Self {
            offset,
            size,
            index,
            proto,
            anonymous: false,
        }
    }

    /// Create a new CField descriptor for a bitfield
    #[allow(dead_code)]
    pub fn new_bitfield(
        proto: PyTypeRef,
        offset: isize,
        bit_size: u16,
        bit_offset: u16,
        index: usize,
    ) -> Self {
        let encoded_size = ((bit_size as isize) << 16) | (bit_offset as isize);
        Self {
            offset,
            size: encoded_size,
            index,
            proto,
            anonymous: false,
        }
    }

    /// Get the actual byte size (for non-bitfields) or bit storage size (for bitfields)
    pub fn byte_size(&self) -> usize {
        field_size(self.size) as usize
    }

    /// Create a new CField from an existing field with adjusted offset and index
    /// Used by MakeFields to promote anonymous fields
    pub fn new_from_field(fdescr: &PyCField, index_offset: usize, offset_delta: isize) -> Self {
        Self {
            offset: fdescr.offset + offset_delta,
            size: fdescr.size,
            index: fdescr.index + index_offset,
            proto: fdescr.proto.clone(),
            anonymous: false, // promoted fields are not anonymous themselves
        }
    }

    /// Set anonymous flag
    pub fn set_anonymous(&mut self, anonymous: bool) {
        self.anonymous = anonymous;
    }
}

impl Representable for PyCField {
    fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
        // Get type name from proto (which is always PyTypeRef)
        let tp_name = zelf.proto.name().to_string();

        // Bitfield: <Field type=TYPE, ofs=OFFSET:BIT_OFFSET, bits=NUM_BITS>
        // Regular:  <Field type=TYPE, ofs=OFFSET, size=SIZE>
        if is_bitfield(zelf.size) {
            let bit_offset = field_size(zelf.size);
            let bits = num_bits(zelf.size);
            Ok(format!(
                "<Field type={}, ofs={}:{}, bits={}>",
                tp_name, zelf.offset, bit_offset, bits
            ))
        } else {
            Ok(format!(
                "<Field type={}, ofs={}, size={}>",
                tp_name, zelf.offset, zelf.size
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
        let size = zelf.byte_size();

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
            let src = bytes.as_bytes();
            let mut result = vec![0u8; size];
            let len = core::cmp::min(src.len(), size);
            result[..len].copy_from_slice(&src[..len]);
            Ok(result)
        }
        // 2. Handle ctypes array instances (copy their buffer)
        else if let Some(cdata) = value.downcast_ref::<super::PyCData>() {
            let buffer = cdata.buffer.read();
            let mut result = vec![0u8; size];
            let len = core::cmp::min(buffer.len(), size);
            result[..len].copy_from_slice(&buffer[..len]);
            Ok(result)
        }
        // 4. Handle float values (check before int, since float.try_int would truncate)
        else if let Some(float_val) = value.downcast_ref::<crate::builtins::PyFloat>() {
            let f = float_val.to_f64();
            match size {
                4 => {
                    let val = f as f32;
                    Ok(val.to_ne_bytes().to_vec())
                }
                8 => Ok(f.to_ne_bytes().to_vec()),
                _ => unreachable!("wrong payload size"),
            }
        }
        // 4. Handle integer values
        else if let Ok(int_val) = value.try_int(vm) {
            let i = int_val.as_bigint();
            match size {
                1 => {
                    let val = i.to_i8().unwrap_or(0);
                    Ok(val.to_ne_bytes().to_vec())
                }
                2 => {
                    let val = i.to_i16().unwrap_or(0);
                    Ok(val.to_ne_bytes().to_vec())
                }
                4 => {
                    let val = i.to_i32().unwrap_or(0);
                    Ok(val.to_ne_bytes().to_vec())
                }
                8 => {
                    let val = i.to_i64().unwrap_or(0);
                    Ok(val.to_ne_bytes().to_vec())
                }
                _ => Ok(vec![0u8; size]),
            }
        } else {
            Ok(vec![0u8; size])
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
                let val = f as f32;
                Ok((val.to_ne_bytes().to_vec(), None))
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
                Ok((f.to_ne_bytes().to_vec(), None))
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
                Ok((f.to_ne_bytes().to_vec(), None))
            }
            "z" => {
                // c_char_p: store pointer to null-terminated bytes
                if let Some(bytes) = value.downcast_ref::<PyBytes>() {
                    let (converted, ptr) = ensure_z_null_terminated(bytes, vm);
                    let mut result = vec![0u8; size];
                    let addr_bytes = ptr.to_ne_bytes();
                    let len = core::cmp::min(addr_bytes.len(), size);
                    result[..len].copy_from_slice(&addr_bytes[..len]);
                    return Ok((result, Some(converted)));
                }
                // Integer address
                if let Ok(int_val) = value.try_index(vm) {
                    let v = int_val.as_bigint().to_usize().unwrap_or(0);
                    let mut result = vec![0u8; size];
                    let bytes = v.to_ne_bytes();
                    let len = core::cmp::min(bytes.len(), size);
                    result[..len].copy_from_slice(&bytes[..len]);
                    return Ok((result, None));
                }
                // None -> NULL pointer
                if vm.is_none(value) {
                    return Ok((vec![0u8; size], None));
                }
                Ok((PyCField::value_to_bytes(value, size, vm)?, None))
            }
            "Z" => {
                // c_wchar_p: store pointer to null-terminated wchar_t buffer
                if let Some(s) = value.downcast_ref::<PyStr>() {
                    let (holder, ptr) = str_to_wchar_bytes(s.as_str(), vm);
                    let mut result = vec![0u8; size];
                    let addr_bytes = ptr.to_ne_bytes();
                    let len = core::cmp::min(addr_bytes.len(), size);
                    result[..len].copy_from_slice(&addr_bytes[..len]);
                    return Ok((result, Some(holder)));
                }
                // Integer address
                if let Ok(int_val) = value.try_index(vm) {
                    let v = int_val.as_bigint().to_usize().unwrap_or(0);
                    let mut result = vec![0u8; size];
                    let bytes = v.to_ne_bytes();
                    let len = core::cmp::min(bytes.len(), size);
                    result[..len].copy_from_slice(&bytes[..len]);
                    return Ok((result, None));
                }
                // None -> NULL pointer
                if vm.is_none(value) {
                    return Ok((vec![0u8; size], None));
                }
                Ok((PyCField::value_to_bytes(value, size, vm)?, None))
            }
            "P" => {
                // c_void_p: store integer as pointer
                if let Ok(int_val) = value.try_index(vm) {
                    let v = int_val.as_bigint().to_usize().unwrap_or(0);
                    let mut result = vec![0u8; size];
                    let bytes = v.to_ne_bytes();
                    let len = core::cmp::min(bytes.len(), size);
                    result[..len].copy_from_slice(&bytes[..len]);
                    return Ok((result, None));
                }
                // None -> NULL pointer
                if vm.is_none(value) {
                    return Ok((vec![0u8; size], None));
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
                return s.as_str() == "c";
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
                return s.as_str() == "u";
            }
        }
        false
    }

    /// Convert bytes for c_char array assignment (stops at first null terminator)
    /// Returns (bytes_to_copy, copy_len)
    fn bytes_for_char_array(src: &[u8]) -> &[u8] {
        // Find first null terminator and include it
        if let Some(null_pos) = src.iter().position(|&b| b == 0) {
            &src[..=null_pos]
        } else {
            src
        }
    }
}

#[pyclass(
    flags(DISALLOW_INSTANTIATION, IMMUTABLETYPE),
    with(Representable, GetDescriptor)
)]
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
        let size = zelf.byte_size();

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
    fn offset(&self) -> isize {
        self.offset
    }

    #[pygetset]
    fn size(&self) -> isize {
        self.size
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
        value: FfiArgValue::Pointer(ptr_val),
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
        value: FfiArgValue::Pointer(ptr_val),
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
            value: FfiArgValue::Pointer(0),
            obj: obj.to_owned(),
            size: stg_info.size,
            offset: 0,
        });
    };

    let ptr_val = buffer.as_ptr() as usize;
    let size = buffer.len();

    Ok(CArgObject {
        tag: b'V',
        value: FfiArgValue::Pointer(ptr_val),
        obj: obj.to_owned(),
        size,
        offset: 0,
    })
}

// FfiArgValue - Owned FFI argument value

/// Owned FFI argument value. Keeps the value alive for the duration of the FFI call.
#[derive(Debug, Clone)]
pub enum FfiArgValue {
    U8(u8),
    I8(i8),
    U16(u16),
    I16(i16),
    U32(u32),
    I32(i32),
    U64(u64),
    I64(i64),
    F32(f32),
    F64(f64),
    Pointer(usize),
    /// Pointer with owned data. The PyObjectRef keeps the pointed data alive.
    OwnedPointer(usize, #[allow(dead_code)] PyObjectRef),
}

impl FfiArgValue {
    /// Create an Arg reference to this owned value
    pub fn as_arg(&self) -> libffi::middle::Arg<'_> {
        match self {
            FfiArgValue::U8(v) => libffi::middle::Arg::new(v),
            FfiArgValue::I8(v) => libffi::middle::Arg::new(v),
            FfiArgValue::U16(v) => libffi::middle::Arg::new(v),
            FfiArgValue::I16(v) => libffi::middle::Arg::new(v),
            FfiArgValue::U32(v) => libffi::middle::Arg::new(v),
            FfiArgValue::I32(v) => libffi::middle::Arg::new(v),
            FfiArgValue::U64(v) => libffi::middle::Arg::new(v),
            FfiArgValue::I64(v) => libffi::middle::Arg::new(v),
            FfiArgValue::F32(v) => libffi::middle::Arg::new(v),
            FfiArgValue::F64(v) => libffi::middle::Arg::new(v),
            FfiArgValue::Pointer(v) => libffi::middle::Arg::new(v),
            FfiArgValue::OwnedPointer(v, _) => libffi::middle::Arg::new(v),
        }
    }
}

/// Convert buffer bytes to FfiArgValue based on type code
pub(super) fn buffer_to_ffi_value(type_code: &str, buffer: &[u8]) -> FfiArgValue {
    match type_code {
        "c" | "b" => {
            let v = buffer.first().map(|&b| b as i8).unwrap_or(0);
            FfiArgValue::I8(v)
        }
        "B" => {
            let v = buffer.first().copied().unwrap_or(0);
            FfiArgValue::U8(v)
        }
        "h" => {
            let v = buffer.first_chunk().copied().map_or(0, i16::from_ne_bytes);
            FfiArgValue::I16(v)
        }
        "H" => {
            let v = buffer.first_chunk().copied().map_or(0, u16::from_ne_bytes);
            FfiArgValue::U16(v)
        }
        "i" => {
            let v = buffer.first_chunk().copied().map_or(0, i32::from_ne_bytes);
            FfiArgValue::I32(v)
        }
        "I" => {
            let v = buffer.first_chunk().copied().map_or(0, u32::from_ne_bytes);
            FfiArgValue::U32(v)
        }
        "l" | "q" => {
            let v = if let Some(&bytes) = buffer.first_chunk::<8>() {
                i64::from_ne_bytes(bytes)
            } else if let Some(&bytes) = buffer.first_chunk::<4>() {
                i32::from_ne_bytes(bytes).into()
            } else {
                0
            };
            FfiArgValue::I64(v)
        }
        "L" | "Q" => {
            let v = if let Some(&bytes) = buffer.first_chunk::<8>() {
                u64::from_ne_bytes(bytes)
            } else if let Some(&bytes) = buffer.first_chunk::<4>() {
                u32::from_ne_bytes(bytes).into()
            } else {
                0
            };
            FfiArgValue::U64(v)
        }
        "f" => {
            let v = buffer
                .first_chunk::<4>()
                .copied()
                .map_or(0.0, f32::from_ne_bytes);
            FfiArgValue::F32(v)
        }
        "d" | "g" => {
            let v = buffer
                .first_chunk::<8>()
                .copied()
                .map_or(0.0, f64::from_ne_bytes);
            FfiArgValue::F64(v)
        }
        "z" | "Z" | "P" | "O" => FfiArgValue::Pointer(read_ptr_from_buffer(buffer)),
        "?" => {
            let v = buffer.first().map(|&b| b != 0).unwrap_or(false);
            FfiArgValue::U8(if v { 1 } else { 0 })
        }
        "u" => {
            // wchar_t - 4 bytes on most platforms
            let v = buffer.first_chunk().copied().map_or(0, u32::from_ne_bytes);
            FfiArgValue::U32(v)
        }
        _ => FfiArgValue::Pointer(0),
    }
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
        return match ty.as_str() {
            "c" => Ok(vm.ctx.new_bytes(bytes.to_vec()).into()),
            "b" => {
                let val = if !bytes.is_empty() { bytes[0] as i8 } else { 0 };
                Ok(vm.ctx.new_int(val).into())
            }
            "B" => {
                let val = if !bytes.is_empty() { bytes[0] } else { 0 };
                Ok(vm.ctx.new_int(val).into())
            }
            "h" => {
                const SIZE: usize = mem::size_of::<c_short>();
                let val = if bytes.len() >= SIZE {
                    c_short::from_ne_bytes(bytes[..SIZE].try_into().expect("size checked"))
                } else {
                    0
                };
                Ok(vm.ctx.new_int(val).into())
            }
            "H" => {
                const SIZE: usize = mem::size_of::<c_ushort>();
                let val = if bytes.len() >= SIZE {
                    c_ushort::from_ne_bytes(bytes[..SIZE].try_into().expect("size checked"))
                } else {
                    0
                };
                Ok(vm.ctx.new_int(val).into())
            }
            "i" => {
                const SIZE: usize = mem::size_of::<c_int>();
                let val = if bytes.len() >= SIZE {
                    c_int::from_ne_bytes(bytes[..SIZE].try_into().expect("size checked"))
                } else {
                    0
                };
                Ok(vm.ctx.new_int(val).into())
            }
            "I" => {
                const SIZE: usize = mem::size_of::<c_uint>();
                let val = if bytes.len() >= SIZE {
                    c_uint::from_ne_bytes(bytes[..SIZE].try_into().expect("size checked"))
                } else {
                    0
                };
                Ok(vm.ctx.new_int(val).into())
            }
            "l" => {
                const SIZE: usize = mem::size_of::<c_long>();
                let val = if bytes.len() >= SIZE {
                    c_long::from_ne_bytes(bytes[..SIZE].try_into().expect("size checked"))
                } else {
                    0
                };
                Ok(vm.ctx.new_int(val).into())
            }
            "L" => {
                const SIZE: usize = mem::size_of::<c_ulong>();
                let val = if bytes.len() >= SIZE {
                    c_ulong::from_ne_bytes(bytes[..SIZE].try_into().expect("size checked"))
                } else {
                    0
                };
                Ok(vm.ctx.new_int(val).into())
            }
            "q" => {
                const SIZE: usize = mem::size_of::<c_longlong>();
                let val = if bytes.len() >= SIZE {
                    c_longlong::from_ne_bytes(bytes[..SIZE].try_into().expect("size checked"))
                } else {
                    0
                };
                Ok(vm.ctx.new_int(val).into())
            }
            "Q" => {
                const SIZE: usize = mem::size_of::<c_ulonglong>();
                let val = if bytes.len() >= SIZE {
                    c_ulonglong::from_ne_bytes(bytes[..SIZE].try_into().expect("size checked"))
                } else {
                    0
                };
                Ok(vm.ctx.new_int(val).into())
            }
            "f" => {
                const SIZE: usize = mem::size_of::<c_float>();
                let val = if bytes.len() >= SIZE {
                    c_float::from_ne_bytes(bytes[..SIZE].try_into().expect("size checked"))
                } else {
                    0.0
                };
                Ok(vm.ctx.new_float(val as f64).into())
            }
            "d" => {
                const SIZE: usize = mem::size_of::<c_double>();
                let val = if bytes.len() >= SIZE {
                    c_double::from_ne_bytes(bytes[..SIZE].try_into().expect("size checked"))
                } else {
                    0.0
                };
                Ok(vm.ctx.new_float(val).into())
            }
            "g" => {
                // long double - read as f64 for now since Rust doesn't have native long double
                // This may lose precision on platforms where long double > 64 bits
                const SIZE: usize = mem::size_of::<c_double>();
                let val = if bytes.len() >= SIZE {
                    c_double::from_ne_bytes(bytes[..SIZE].try_into().expect("size checked"))
                } else {
                    0.0
                };
                Ok(vm.ctx.new_float(val).into())
            }
            "?" => {
                let val = !bytes.is_empty() && bytes[0] != 0;
                Ok(vm.ctx.new_bool(val).into())
            }
            "v" => {
                // VARIANT_BOOL: non-zero = True, zero = False
                const SIZE: usize = mem::size_of::<c_short>();
                let val = if bytes.len() >= SIZE {
                    c_short::from_ne_bytes(bytes[..SIZE].try_into().expect("size checked"))
                } else {
                    0
                };
                Ok(vm.ctx.new_bool(val != 0).into())
            }
            "z" => {
                // c_char_p: read NULL-terminated string from pointer
                let ptr = read_ptr_from_buffer(bytes);
                if ptr == 0 {
                    return Ok(vm.ctx.none());
                }
                let c_str = unsafe { core::ffi::CStr::from_ptr(ptr as _) };
                Ok(vm.ctx.new_bytes(c_str.to_bytes().to_vec()).into())
            }
            "Z" => {
                // c_wchar_p: read NULL-terminated wide string from pointer
                let ptr = read_ptr_from_buffer(bytes);
                if ptr == 0 {
                    return Ok(vm.ctx.none());
                }
                let len = unsafe { libc::wcslen(ptr as *const libc::wchar_t) };
                let wchars =
                    unsafe { core::slice::from_raw_parts(ptr as *const libc::wchar_t, len) };
                let s: String = wchars
                    .iter()
                    .filter_map(|&c| char::from_u32(c as u32))
                    .collect();
                Ok(vm.ctx.new_str(s).into())
            }
            "P" => {
                // c_void_p: return pointer value as integer
                let val = read_ptr_from_buffer(bytes);
                if val == 0 {
                    return Ok(vm.ctx.none());
                }
                Ok(vm.ctx.new_int(val).into())
            }
            "u" => {
                let val = if bytes.len() >= mem::size_of::<WideChar>() {
                    let wc = if mem::size_of::<WideChar>() == 2 {
                        u16::from_ne_bytes([bytes[0], bytes[1]]) as u32
                    } else {
                        u32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
                    };
                    char::from_u32(wc).unwrap_or('\0')
                } else {
                    '\0'
                };
                Ok(vm.ctx.new_str(val).into())
            }
            _ => Ok(vm.ctx.none()),
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

/// Read a pointer value from buffer
#[inline]
pub(super) fn read_ptr_from_buffer(buffer: &[u8]) -> usize {
    const PTR_SIZE: usize = core::mem::size_of::<usize>();
    buffer
        .first_chunk::<PTR_SIZE>()
        .copied()
        .map_or(0, usize::from_ne_bytes)
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
        *existing = stg_info;
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
            (s.len() == 1).then(|| super::get_size(&s))
        })
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

    Ok(core::mem::size_of::<usize>())
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
            (s.len() == 1).then(|| super::get_size(&s))
        })
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
    use crate::builtins::{PyList, PyTuple};
    use crate::convert::ToPyObject;

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
            .downcast_ref::<PyStr>()
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
        cls.set_attr(vm.ctx.intern_str(fname.as_str()), new_descr.to_pyobject(vm));
    }

    Ok(())
}

/// Process _anonymous_ attribute for struct/union
pub(super) fn make_anon_fields(cls: &Py<PyType>, vm: &VirtualMachine) -> PyResult<()> {
    use crate::builtins::{PyList, PyTuple};
    use crate::convert::ToPyObject;

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
            .get_attr(vm.ctx.intern_str(fname.as_str()), vm)?;

        let descr = descr_obj.downcast_ref::<super::PyCField>().ok_or_else(|| {
            vm.new_attribute_error(format!(
                "'{}' is specified in _anonymous_ but not in _fields_",
                fname.as_str()
            ))
        })?;

        let mut new_descr = super::PyCField::new_from_field(descr, 0, 0);
        new_descr.set_anonymous(true);
        cls.set_attr(vm.ctx.intern_str(fname.as_str()), new_descr.to_pyobject(vm));

        make_fields(cls, descr, descr.index, descr.offset, vm)?;
    }

    Ok(())
}

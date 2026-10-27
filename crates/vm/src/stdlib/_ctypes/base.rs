#![allow(unreachable_pub)]

use crate::builtins::{
    PyBytes, PyDict, PyInt, PyList, PyMemoryView, PyStr, PyTuple, PyType, PyTypeRef, PyUtf8Str,
    memory::buffer_owner,
};
use crate::class::StaticType;
use crate::convert::ToPyObject;
use crate::dict_inner::DictKey;
use crate::function::{ArgBytesLike, ArgStrictInt, OptionalArg, PySetterValue};
use crate::object::{Traverse, TraverseFn};
use crate::protocol::{BufferMethods, PyBuffer};
use crate::types::{Constructor, GetDescriptor, Representable};
use crate::{
    AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject, VirtualMachine,
};
use alloc::borrow::Cow;
use core::fmt::Debug;
use crossbeam_utils::atomic::AtomicCell;
use num_traits::{Signed, ToPrimitive};
use rustpython_common::lock::PyRwLock;
use rustpython_common::wtf8::Wtf8;
use rustpython_host_env::ctypes::{
    CTypeLayout, char_array_assignment_bytes, char_array_field_value, clone_wchar_null_terminated,
    wchar_array_field_value, write_cow_bytes_at_offset,
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
    // Scalar assignment uses the converter selected at type initialization.
    pub(super) scalar_code: Option<char>,

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

    // Call layouts of the struct/union fields, in declaration order (inherited
    // from base class). Drives by-value aggregate passing.
    pub field_layouts: Vec<CTypeLayout>,

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
            .field("scalar_code", &self.scalar_code)
            .field("element_type", &self.element_type)
            .field("element_size", &self.element_size)
            .field("format", &self.format)
            .field("shape", &self.shape)
            .field("paramfunc", &self.paramfunc)
            .field("big_endian", &self.big_endian)
            .field("field_layouts", &self.field_layouts.len())
            .finish()
    }
}

impl Default for StgInfo {
    fn default() -> Self {
        Self {
            initialized: false,
            size: 0,
            align: 1,
            length: 0,
            proto: None,
            flags: StgInfoFlags::empty(),
            scalar_code: None,
            element_type: None,
            element_size: 0,
            format: None,
            shape: Vec::new(),
            paramfunc: ParamFunc::None,
            big_endian: cfg!(target_endian = "big"), // native endian by default
            field_layouts: Vec::new(),
            pointer_type: None,
        }
    }
}

// SAFETY: these are the only Python references in the type's storage metadata.
unsafe impl Traverse for StgInfo {
    fn traverse(&self, tracer: &mut TraverseFn<'_>) {
        self.proto.traverse(tracer);
        self.element_type.traverse(tracer);
        self.pointer_type.traverse(tracer);
    }

    fn clear(&mut self, out: &mut Vec<PyObjectRef>) {
        out.extend(self.proto.take().map(Into::into));
        out.extend(self.element_type.take().map(Into::into));
        out.extend(self.pointer_type.take());
    }
}

impl StgInfo {
    pub(crate) fn new(size: usize, align: usize) -> Self {
        Self {
            initialized: true,
            size,
            align,
            length: 0,
            proto: None,
            flags: StgInfoFlags::empty(),
            scalar_code: None,
            element_type: None,
            element_size: 0,
            format: None,
            shape: Vec::new(),
            paramfunc: ParamFunc::None,
            big_endian: cfg!(target_endian = "big"), // native endian by default
            field_layouts: Vec::new(),
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

        Self {
            initialized: true,
            size,
            align,
            length,
            proto: Some(element_type.clone()),
            flags,
            scalar_code: None,
            element_type: Some(element_type),
            element_size,
            format,
            shape,
            paramfunc: ParamFunc::Array,
            big_endian: cfg!(target_endian = "big"), // native endian by default
            field_layouts: Vec::new(),
            pointer_type: None,
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

/// Build the host_env call layout for a ctypes type from its already-borrowed
/// `StgInfo`. Aggregate layouts come straight from the type's `field_layouts`
/// (built incrementally from the base class, so struct inheritance is
/// reflected); array elements recurse into the element type; simple types use
/// their initialized scalar code. The caller passes the borrowed `stg` so this never
/// re-locks `ty`'s own type data.
pub(super) fn type_layout(stg: &StgInfo) -> CTypeLayout {
    match stg.paramfunc {
        ParamFunc::Structure => CTypeLayout::Struct {
            fields: stg.field_layouts.clone(),
            size: stg.size,
        },
        ParamFunc::Union => CTypeLayout::Union {
            fields: stg.field_layouts.clone(),
            size: stg.size,
        },
        ParamFunc::Array => {
            let element = stg
                .element_type
                .as_ref()
                .and_then(|et| et.stg_info_opt().map(|et_stg| type_layout(&et_stg)))
                .unwrap_or(CTypeLayout::Opaque {
                    size: stg.element_size,
                });
            CTypeLayout::Array {
                element: Box::new(element),
                length: stg.length,
                size: stg.size,
            }
        }
        ParamFunc::Pointer => CTypeLayout::Pointer,
        ParamFunc::Simple | ParamFunc::None => stg
            .scalar_code
            .map_or(CTypeLayout::Opaque { size: stg.size }, CTypeLayout::Simple),
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
        let old = info.pointer_type.replace(value);
        drop(info);
        drop(old);
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
            return format!("{endian_prefix}{base_fmt}");
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
        return format!("{endian_prefix}{s}");
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
            |x| match x {
                // ctypes buffers are writable, including views into their base object.
                // SAFETY: requires valid writable storage and exclusive access.
                // This guard only locks this view, not its owner or sibling views;
                // ctypes does not enforce these requirements across aliases.
                Cow::Borrowed(slice) => unsafe {
                    rustpython_host_env::ctypes::borrowed_slice_as_mut(slice)
                },
                Cow::Owned(vec) => vec.as_mut_slice(),
            },
        )
        .into()
    },
    release: |_| {},
    retain: |_| {},
};

/// Ensure PyBytes data is null-terminated. Returns (kept_alive_obj, pointer).
///
/// The caller must keep the returned object alive to keep the pointer valid.
pub(super) fn ensure_z_null_terminated(
    bytes: &Py<PyBytes>,
    vm: &VirtualMachine,
) -> (PyObjectRef, usize) {
    let buffer = rustpython_host_env::ctypes::clone_as_null_terminated(bytes.as_bytes());
    let ptr = buffer.as_ptr() as usize;
    let kept_alive: PyObjectRef = vm.ctx.new_bytes(buffer).into();
    (kept_alive, ptr)
}

/// Convert str to null-terminated wchar_t buffer. Returns (PyBytes holder, pointer).
pub(super) fn str_to_wchar_bytes(s: &Wtf8, vm: &VirtualMachine) -> (PyObjectRef, usize) {
    let bytes = clone_wchar_null_terminated(s);
    let ptr = bytes.as_ptr() as usize;
    let holder: PyObjectRef = vm.ctx.new_bytes(bytes).into();
    (holder, ptr)
}

// PyCData - base type for all ctypes data types
#[pyclass(name = "_CData", module = "_ctypes", traverse = "manual")]
#[derive(Debug, PyPayload)]
pub struct PyCData {
    /// Memory buffer - Owned (self-owned) or Borrowed (external reference)
    ///
    /// SAFETY: Borrowed variant's 'static lifetime is not actually static.
    /// When created via from_address or from_base_obj, only valid for the lifetime of the source memory.
    /// A base reference keeps the owner alive but does not pin its allocation.
    /// Resizing the owner may invalidate the slice; locks on distinct views
    /// do not synchronize access to their shared storage.
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
    kept_refs: PyRwLock<Option<Box<PrivateRefs>>>,
}

// SAFETY: visit each owned reference once; slot IDs are not additional owners.
unsafe impl Traverse for PyCData {
    fn traverse(&self, tracer: &mut TraverseFn<'_>) {
        self.base.traverse(tracer);
        self.objects.traverse(tracer);
        if let Some(refs) = self.kept_refs.try_read_recursive()
            && let Some(refs) = refs.as_ref()
        {
            refs.traverse(tracer);
        }
    }

    fn clear(&mut self, out: &mut Vec<PyObjectRef>) {
        // Disconnect a view before its backing owners can be released. Owned
        // storage keeps its allocation until deallocation.
        if let Some(mut buffer) = self.buffer.try_write() {
            match &mut *buffer {
                Cow::Owned(_) => {}
                Cow::Borrowed(_) => *buffer = Cow::Owned(Vec::new()),
            }
        } else {
            return;
        }
        if let Some(mut base) = self.base.try_write() {
            out.extend(base.take());
        }
        if let Some(mut objects) = self.objects.try_write() {
            out.extend(objects.take());
        }
        if let Some(mut refs) = self.kept_refs.try_write()
            && let Some(mut refs) = refs.take()
        {
            refs.clear(out);
        }
    }
}

#[derive(Debug, Default)]
struct PrivateRefs {
    writer_prune_remaining: usize,
    buffer_alias: Option<BufferAlias>,
    // Exact-key hints only: these IDs never own or reconstruct Python objects.
    object_index: Option<Box<ObjectIndex>>,
    // A registration cache, not a backing owner; do not copy it with values.
    writer_paths: Option<WriterPaths>,
    // Views are temporary; retain their caches on the same root as KeepRef.
    view_writer_paths: std::collections::HashMap<String, WriterPaths>,
    writes: std::collections::HashMap<WriteSlot, WriteRegistration>,
    slots: std::collections::HashMap<
        String,
        std::collections::HashSet<usize, rapidhash::quality::RandomState>,
        rapidhash::quality::RandomState,
    >,
    owners: std::collections::HashMap<usize, (PyObjectRef, usize), rapidhash::quality::RandomState>,
}

// SAFETY: the owner map stores one strong reference per identity; slots hold IDs.
// The registration cache owns weakref wrappers, not their referents.
unsafe impl Traverse for PrivateRefs {
    #[expect(clippy::iter_over_hash_type, reason = "GC edge order is irrelevant")]
    fn traverse(&self, tracer: &mut TraverseFn<'_>) {
        if let Some(alias) = &self.buffer_alias {
            alias.source.traverse(tracer);
        }
        for (owner, _) in self.owners.values() {
            owner.traverse(tracer);
        }
        for registration in self.writes.values() {
            registration.record.writer.traverse(tracer);
        }
        for paths in self
            .writer_paths
            .iter()
            .chain(self.view_writer_paths.values())
        {
            for path in paths.iter() {
                for link in path {
                    link.source.traverse(tracer);
                }
            }
        }
    }
    fn clear(&mut self, out: &mut Vec<PyObjectRef>) {
        if let Some(alias) = self.buffer_alias.take() {
            out.push(alias.source.clone());
        }
        self.object_index = None;
        out.extend(
            self.writes
                .drain()
                .map(|(_, registration)| registration.record.writer.clone().into()),
        );
        self.slots.clear();
        out.extend(self.owners.drain().map(|(_, (owner, _))| owner));
        for paths in self
            .writer_paths
            .take()
            .into_iter()
            .chain(self.view_writer_paths.drain().map(|(_, paths)| paths))
        {
            for path in paths.iter() {
                out.extend(path.iter().map(|link| link.source.clone().into()));
            }
        }
    }
}

impl PrivateRefs {
    fn first(&self, key: &str) -> Option<PyObjectRef> {
        self.slots
            .get(key)
            .and_then(|ids| ids.iter().next())
            .and_then(|id| self.owners.get(id))
            .map(|(obj, _)| obj.clone())
    }

    fn replace(&mut self, key: String, owners: Vec<PyObjectRef>) -> Vec<PyObjectRef> {
        let mut ids = std::collections::HashSet::default();
        // Count each slot once, even if a copied value repeats an owner.
        // Add first so replacing an owner with itself cannot release it.
        for obj in owners {
            if ids.insert(obj.get_id()) {
                self.owners.entry(obj.get_id()).or_insert((obj, 0)).1 += 1;
            }
        }
        let old = if ids.is_empty() {
            self.slots.remove(&key)
        } else {
            self.slots.insert(key, ids)
        };
        let mut displaced = Vec::new();
        for id in old.into_iter().flatten() {
            let owner = self.owners.get_mut(&id).expect("registered private owner");
            owner.1 -= 1;
            if owner.1 == 0 {
                displaced.push(self.owners.remove(&id).unwrap().0);
            }
        }
        // The caller releases these after unlocking and publishing _objects.
        displaced
    }

    fn snapshot(&self) -> Vec<PyObjectRef> {
        self.owners.values().map(|(obj, _)| obj.clone()).collect()
    }
}

// Per-interpreter reverse provenance for independent from_buffer views.
// Callback-free weakrefs do not retain writers or their py_object values.
#[derive(Debug, Default)]
pub(crate) struct BufferViewRegistry {
    views: PyRwLock<std::collections::HashMap<usize, RangeIndex<PyRef<crate::object::PyWeak>>>>,
    writes: PyRwLock<RangeIndex<alloc::sync::Weak<BufferWrite>>>,
}

#[derive(Debug)]
struct RangeEntry<T> {
    end: usize,
    value: T,
}

#[derive(Debug)]
struct RangeIndex<T> {
    // One entry per view. Size classes keep a large overlapping view from
    // widening the search window for every small view of the same exporter.
    ranges: alloc::collections::BTreeMap<
        usize,
        alloc::collections::BTreeMap<(usize, usize), RangeEntry<T>>,
    >,
}

impl<T> Default for RangeIndex<T> {
    fn default() -> Self {
        Self {
            ranges: Default::default(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct RangeKey {
    start: usize,
    span: usize,
    id: usize,
}

impl RangeKey {
    fn new(start: usize, size: usize, id: usize) -> Self {
        Self {
            start,
            span: size.checked_next_power_of_two().unwrap_or(usize::MAX),
            id,
        }
    }
}

impl<T: Clone> RangeIndex<T> {
    fn insert(&mut self, key: RangeKey, size: usize, value: T) -> Option<T> {
        self.ranges
            .entry(key.span)
            .or_default()
            .insert(
                (key.start, key.id),
                RangeEntry {
                    end: key.start.saturating_add(size),
                    value,
                },
            )
            .map(|entry| entry.value)
    }

    fn remove(&mut self, key: RangeKey) -> Option<T> {
        let ranges = self.ranges.get_mut(&key.span)?;
        let removed = ranges.remove(&(key.start, key.id));
        if ranges.is_empty() {
            self.ranges.remove(&key.span);
        }
        removed.map(|entry| entry.value)
    }

    fn matching(&self, slot: Option<usize>) -> Vec<T> {
        let mut found = Vec::new();
        for (&span, ranges) in &self.ranges {
            if let Some(slot) = slot {
                let first = slot.saturating_sub(span - 1);
                let end = slot.saturating_add(rustpython_host_env::ctypes::pointer_size());
                for (&(start, _), entry) in ranges.range((first, 0)..(end, 0)) {
                    if entry.end > slot && entry.end > start {
                        found.push(entry.value.clone());
                    }
                }
            } else {
                found.extend(ranges.values().map(|entry| entry.value.clone()));
            }
        }
        found
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum WriteSlot {
    Root(usize),
    View(String),
}

struct WriteRange {
    root: PyObjectRef,
    slot: WriteSlot,
    address: usize,
    size: usize,
}

// Pin the destination across user conversions, but publish the actual range
// afterwards: a nested conversion may write through the same retention slot.
#[derive(Default)]
pub(super) struct CDataWrite {
    _storage: Option<PyObjectRef>,
    range: Option<WriteRange>,
}

impl CDataWrite {
    fn new(storage: Option<PyObjectRef>, range: impl FnOnce() -> WriteRange) -> Self {
        // Owned CData storage is directly reachable through its buffer views.
        // Borrowed/foreign writes can extend beyond every retained source view.
        let range = storage
            .as_ref()
            .is_none_or(|storage| storage.downcast_ref::<PyCData>().unwrap().is_borrowed())
            .then(range);
        Self {
            _storage: storage,
            range,
        }
    }

    pub(super) fn publish(&self, vm: &VirtualMachine) -> PyResult<()> {
        if let Some(range) = &self.range {
            PyCData::index_write(&range.root, &range.slot, range.address, range.size, vm)?;
        }
        Ok(())
    }
}

#[derive(Debug)]
struct BufferWrite {
    writer: PyRef<crate::object::PyWeak>,
    address: usize,
    size: usize,
}

// The retention root owns this token. The registry holds only a Rust Weak,
// so the Python weakref wrapper has exactly the GC edge traversed above.
#[derive(Debug)]
struct WriteRegistration {
    registry: alloc::sync::Arc<BufferViewRegistry>,
    key: RangeKey,
    record: alloc::sync::Arc<BufferWrite>,
}

impl Drop for WriteRegistration {
    fn drop(&mut self) {
        let removed = self.registry.writes.write().remove(self.key);
        drop(removed);
    }
}

#[derive(Debug)]
struct BufferAlias {
    registry: alloc::sync::Arc<BufferViewRegistry>,
    exporter: usize,
    key: RangeKey,
    source: PyObjectRef,
}

impl Drop for BufferAlias {
    fn drop(&mut self) {
        let removed = {
            let mut views = self.registry.views.write();
            if let Some(aliases) = views.get_mut(&self.exporter) {
                let removed = aliases.remove(self.key);
                if aliases.ranges.is_empty() {
                    views.remove(&self.exporter);
                }
                removed
            } else {
                None
            }
        };
        drop(removed);
    }
}

impl BufferViewRegistry {
    fn owners(&self, exporter: &PyObject, slot: Option<usize>) -> Vec<PyObjectRef> {
        let exporter = buffer_owner(exporter);
        let aliases = {
            let views = self.views.read();
            let Some(aliases) = views.get(&exporter.get_id()) else {
                return Vec::new();
            };
            aliases.matching(slot)
        };
        let mut owners = Vec::new();
        for weak in aliases {
            // Do not upgrade or release writers under the registry lock.
            let Some(owner) = weak.upgrade() else {
                continue;
            };
            let source =
                owner.downcast_ref::<PyCData>().and_then(|cdata| {
                    cdata.kept_refs.read().as_ref().and_then(|refs| {
                        refs.buffer_alias.as_ref().map(|alias| alias.source.clone())
                    })
                });
            if source
                .as_ref()
                .is_some_and(|source| buffer_owner(source).is(exporter))
            {
                owners.push(owner);
            }
        }
        owners
    }

    fn writers(&self, slot: usize) -> Vec<PyObjectRef> {
        let records = self.writes.read().matching(Some(slot));
        // Upgrade and release Python references outside the range-index lock.
        records
            .into_iter()
            .filter_map(|record| record.upgrade()?.writer.upgrade())
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum WriterKey {
    String(String),
    Id(usize),
    Index(usize),
}

type WriterPaths = alloc::sync::Arc<[Vec<WriterLink>]>;
type WriterSource = (PyObjectRef, WriterOrigin);

#[derive(Clone, Debug)]
enum WriterOrigin {
    Native,
    Public(Vec<WriterKey>),
    Private { slot: String, owner: usize },
}

#[derive(Debug)]
struct WriterLink {
    source: PyRef<crate::object::PyWeak>,
    origin: WriterOrigin,
}

// Index direct retention entries, including duplicate values, without retaining
// their keys or values as Python objects. Native writes update a warmed index.
#[derive(Debug)]
struct ObjectIndex {
    dictionary: usize,
    slots: std::collections::HashMap<WriterKey, usize>,
    values: std::collections::HashMap<usize, ObjectKeys>,
}

impl ObjectIndex {
    fn set(&mut self, key: WriterKey, value: Option<usize>) {
        if let Some(old) = self.slots.remove(&key) {
            let keys = self.values.get_mut(&old).expect("indexed retention slot");
            if keys.remove(&key) {
                self.values.remove(&old);
            }
        }
        if let Some(value) = value {
            self.slots.insert(key.clone(), value);
            match self.values.entry(value) {
                std::collections::hash_map::Entry::Occupied(mut entry) => {
                    entry.get_mut().insert(key)
                }
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(ObjectKeys::One(key));
                }
            }
        }
    }
}

// Most referents occur once. Allocate a tree only for duplicate keys, whose
// removal must not require scanning all other references to the same object.
#[derive(Debug)]
enum ObjectKeys {
    One(WriterKey),
    Many(alloc::collections::BTreeSet<WriterKey>),
}

impl ObjectKeys {
    fn insert(&mut self, key: WriterKey) {
        match self {
            Self::One(old) if *old == key => {}
            Self::One(old) => *self = Self::Many([old.clone(), key].into()),
            Self::Many(keys) => {
                keys.insert(key);
            }
        }
    }

    // Return true when the last key was removed.
    fn remove(&mut self, key: &WriterKey) -> bool {
        match self {
            Self::One(old) => old == key,
            Self::Many(keys) => {
                keys.remove(key);
                if keys.len() == 1 {
                    *self = Self::One(keys.pop_first().unwrap());
                }
                false
            }
        }
    }

    fn first(&self) -> &WriterKey {
        match self {
            Self::One(key) => key,
            Self::Many(keys) => keys.first().expect("nonempty retention keys"),
        }
    }

    fn len(&self) -> usize {
        match self {
            Self::One(_) => 1,
            Self::Many(keys) => keys.len(),
        }
    }
}

// Retention metadata is user-mutable. Its fast lookup must not call Python
// equality on a colliding key; non-exact string keys use the graph fallback.
struct ObjectKey<'a>(&'a str);

impl DictKey for ObjectKey<'_> {
    type Owned = String;

    fn _to_owned(&self, _vm: &VirtualMachine) -> String {
        self.0.to_owned()
    }

    fn key_hash(&self, vm: &VirtualMachine) -> PyResult<crate::common::hash::PyHash> {
        self.0.key_hash(vm)
    }

    fn key_is(&self, _other: &PyObject) -> bool {
        false
    }

    fn key_eq(&self, vm: &VirtualMachine, other: &PyObject) -> PyResult<bool> {
        Ok(other
            .downcast_ref_if_exact::<PyStr>(vm)
            .is_some_and(|key| key.as_wtf8() == self.0))
    }

    fn key_as_isize(&self, vm: &VirtualMachine) -> PyResult<isize> {
        self.0.key_as_isize(vm)
    }
}

// cast() stores a strong source reference under id(source). Only exact int
// keys are eligible for the cache; lookup must not invoke user equality.
struct ObjectIdKey(usize);

impl DictKey for ObjectIdKey {
    type Owned = usize;

    fn _to_owned(&self, _vm: &VirtualMachine) -> usize {
        self.0
    }

    fn key_hash(&self, vm: &VirtualMachine) -> PyResult<crate::common::hash::PyHash> {
        self.0.key_hash(vm)
    }

    fn key_is(&self, _other: &PyObject) -> bool {
        false
    }

    fn key_eq(&self, vm: &VirtualMachine, other: &PyObject) -> PyResult<bool> {
        Ok(other
            .downcast_ref_if_exact::<PyInt>(vm)
            .and_then(|key| key.as_bigint().to_usize())
            == Some(self.0))
    }

    fn key_as_isize(&self, vm: &VirtualMachine) -> PyResult<isize> {
        self.0.key_as_isize(vm)
    }
}

// Prepare conversions and ownership before writing. Python conversion hooks
// and old-owner finalizers must never run with a storage or metadata lock held.
pub(super) enum CDataBytes {
    Integer { bytes: [u8; 8], size: usize },
    Owned(Vec<u8>),
}

impl From<Vec<u8>> for CDataBytes {
    fn from(bytes: Vec<u8>) -> Self {
        Self::Owned(bytes)
    }
}

impl core::ops::Deref for CDataBytes {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        match self {
            Self::Integer { bytes, size } => &bytes[..*size],
            Self::Owned(bytes) => bytes,
        }
    }
}

pub(super) struct CDataValue {
    pub bytes: CDataBytes,
    keep: Option<PyObjectRef>,
    hidden: Vec<PyObjectRef>,
    temporary: Option<PyObjectRef>,
}

impl CDataValue {
    pub(super) fn new(
        proto: &Py<PyType>,
        value: &PyObject,
        size: usize,
        swapped: bool,
        vm: &VirtualMachine,
    ) -> PyResult<Self> {
        if let Some(source) = value.downcast_ref::<PyCData>() {
            if value.fast_isinstance(proto) {
                // Snapshot before writing: source and destination may overlap.
                let bytes =
                    rustpython_host_env::ctypes::copy_to_sized_bytes(&source.buffer.read(), size);
                let hidden = PyCData::hidden_refs(value);
                let keep = PyCData::get_kept_objects(value, vm);
                return Ok(Self {
                    bytes: bytes.into(),
                    keep: (!vm.is_none(&keep)).then_some(keep),
                    hidden,
                    temporary: None,
                });
            }
            if proto
                .class()
                .fast_issubclass(super::pointer::PyCPointerType::static_type())
                && let Some(array) = value.downcast_ref::<super::array::PyCArray>()
                && {
                    let target = proto.stg_info(vm)?;
                    let array_info = array.class().stg_info(vm)?;
                    target
                        .proto
                        .as_deref()
                        .zip(array_info.element_type.as_deref())
                        .is_some_and(|(target, element)| target.is(element))
                }
            {
                let address = array.0.buffer.read().as_ptr() as usize;
                return Ok(Self {
                    bytes: rustpython_host_env::ctypes::pointer_to_sized_bytes(address, size)
                        .into(),
                    keep: Some(
                        vm.ctx
                            .new_tuple(vec![PyCData::get_kept_objects(value, vm), value.to_owned()])
                            .into(),
                    ),
                    hidden: Vec::new(),
                    temporary: None,
                });
            }
            return Err(vm.new_type_error(format!(
                "incompatible types, {} instance instead of {} instance",
                value.class().name(),
                proto.name()
            )));
        }
        let metaclass = proto.class();
        let simple_metaclass = super::simple::PyCSimpleType::static_type();
        if metaclass.is(simple_metaclass) || metaclass.fast_issubclass(simple_metaclass) {
            // Reading the mutable _type_ here could select a wider converter
            // than the allocated slot, or change conversion during callbacks.
            let code = proto
                .stg_info(vm)?
                .scalar_code
                .ok_or_else(|| vm.new_type_error("abstract class"))?;
            let mut encoded = [0; 4];
            let type_code = code.encode_utf8(&mut encoded);
            let (bytes, keep, hidden) =
                super::simple::assignment_value(type_code, value, swapped, vm)?;
            return Ok(Self {
                bytes,
                keep,
                hidden: hidden.into_iter().collect(),
                temporary: None,
            });
        }
        if let Some(tuple) = value.downcast_ref::<PyTuple>() {
            let instance = proto
                .as_object()
                .call(tuple.as_slice().to_vec(), vm)
                .map_err(|e| {
                    let args = e.args();
                    let message = args
                        .as_slice()
                        .first()
                        .and_then(|arg| arg.downcast_ref::<PyStr>())
                        .map(|s| s.to_string())
                        .unwrap_or_default();
                    vm.new_runtime_error(format!(
                        "({}) {}: {message}",
                        proto.name(),
                        e.class().name()
                    ))
                })?;
            let mut prepared = Self::new(proto, &instance, size, swapped, vm)?;
            prepared.temporary = Some(instance);
            return Ok(prepared);
        }
        if vm.is_none(value)
            && proto
                .class()
                .fast_issubclass(super::pointer::PyCPointerType::static_type())
        {
            return Ok(Self {
                bytes: vec![0; size].into(),
                keep: None,
                hidden: Vec::new(),
                temporary: None,
            });
        }
        Err(vm.new_type_error(format!(
            "expected {} instance, not {}",
            proto.name(),
            value.class().name()
        )))
    }

    #[inline]
    pub(super) fn keep(
        self,
        destination: &PyCData,
        index: usize,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        // Tuple conversion can create an instance with a finalizer. Keep it
        // until the bytes and all owners have been published.
        let _temporary = self.temporary;
        // Like KeepRef(None), preserve previous owners when assigning a raw
        // address: it can still point into their private backing allocation.
        if self.keep.is_none() && self.hidden.is_empty() {
            return Ok(());
        }
        // Publish every owner before dropping the displaced value, whose
        // finalizer may assign this same slot again.
        let _old_hidden = destination.replace_hidden(index, self.hidden);
        if let Some(keep) = self.keep {
            destination.keep_ref(index, keep, vm)?;
        }
        Ok(())
    }
}

impl PyCData {
    /// Create from StgInfo (PyCData_MallocBuffer pattern)
    pub(crate) fn from_stg_info(stg_info: &StgInfo) -> Self {
        Self {
            buffer: PyRwLock::new(Cow::Owned(vec![0u8; stg_info.size])),
            base: PyRwLock::new(None),
            base_offset: AtomicCell::new(0),
            index: AtomicCell::new(0),
            objects: PyRwLock::new(None),
            length: AtomicCell::new(stg_info.length),
            kept_refs: PyRwLock::new(None),
        }
    }

    /// Create from existing bytes (copies data)
    pub(crate) fn from_bytes(data: Vec<u8>, objects: Option<PyObjectRef>) -> Self {
        Self {
            buffer: PyRwLock::new(Cow::Owned(data)),
            base: PyRwLock::new(None),
            base_offset: AtomicCell::new(0),
            index: AtomicCell::new(0),
            objects: PyRwLock::new(objects),
            length: AtomicCell::new(0),
            kept_refs: PyRwLock::new(None),
        }
    }

    /// Create from bytes with specified length (for arrays)
    pub fn from_bytes_with_length(
        data: Vec<u8>,
        objects: Option<PyObjectRef>,
        length: usize,
    ) -> Self {
        Self {
            buffer: PyRwLock::new(Cow::Owned(data)),
            base: PyRwLock::new(None),
            base_offset: AtomicCell::new(0),
            index: AtomicCell::new(0),
            objects: PyRwLock::new(objects),
            length: AtomicCell::new(length),
            kept_refs: PyRwLock::new(None),
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
        Self {
            buffer: PyRwLock::new(Cow::Borrowed(slice)),
            base: PyRwLock::new(None),
            base_offset: AtomicCell::new(0),
            index: AtomicCell::new(0),
            objects: PyRwLock::new(None),
            length: AtomicCell::new(0),
            kept_refs: PyRwLock::new(None),
        }
    }

    /// Create from base object's buffer
    ///
    /// This creates a borrowed view into the base's buffer at the given address.
    /// The base object is stored in b_base to keep the owner alive, not to pin its allocation.
    ///
    /// # Safety
    /// `ptr` must cover `size` valid bytes for the lifetime of the returned view.
    /// Keeping `base_obj` alive does not prevent reallocation or incompatible
    /// access through another alias; the caller must account for both.
    pub unsafe fn from_base_obj(
        ptr: *mut u8,
        size: usize,
        base_obj: PyObjectRef,
        idx: usize,
    ) -> Self {
        // = PyCData_FromBaseObj
        // SAFETY: the caller must uphold the storage validity and aliasing requirements above.
        let slice = unsafe { rustpython_host_env::ctypes::borrow_memory(ptr, size) };
        Self {
            buffer: PyRwLock::new(Cow::Borrowed(slice)),
            base: PyRwLock::new(Some(base_obj)),
            base_offset: AtomicCell::new(0),
            index: AtomicCell::new(idx),
            objects: PyRwLock::new(None),
            length: AtomicCell::new(0),
            kept_refs: PyRwLock::new(None),
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

        Self {
            buffer: PyRwLock::new(Cow::Borrowed(slice)),
            base: PyRwLock::new(None),
            base_offset: AtomicCell::new(0),
            index: AtomicCell::new(0),
            objects: PyRwLock::new(Some(objects_dict.into())),
            length: AtomicCell::new(length),
            kept_refs: PyRwLock::new(None),
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
            // Contiguity is checked above, so this is the view's own bytes rather
            // than the whole exporter's.
            let bytes = unsafe { buffer.contiguous_unchecked() };
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
        let mut base = self.base.read().clone();
        if base.is_some() {
            key.push_str(&format!(":{:x}", self.index.load()));
        }
        while let Some(parent) = base.take() {
            let Some(cdata) = parent.downcast_ref::<Self>() else {
                break;
            };
            base.clone_from(&cdata.base.read());
            if base.is_some() {
                key.push_str(&format!(":{:x}", cdata.index.load()));
            }
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
        let base = self.base.read().clone();
        if let Some(base_obj) = base {
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

        // Replacing a reference can run a finalizer that accesses this object.
        let dict_obj = objects.clone();
        drop(objects);
        // Store in dict with unique key
        if let Some(dict_obj) = dict_obj
            && let Some(dict) = dict_obj.downcast_ref::<PyDict>()
        {
            let key_obj: PyObjectRef = vm.ctx.new_str(key.as_str()).into();
            dict.set_item(&*key_obj, keep, vm)?;
            self.update_object_index(dict, &key, vm)?;
        }

        Ok(())
    }

    /// Keep a reference alive without exposing it in _objects.
    /// Retains a backing allocation used by a scalar's pointer.
    pub fn keep_alive(&self, obj: PyObjectRef) {
        drop(self.replace_hidden(0, vec![obj]));
    }

    fn replace_hidden(&self, index: usize, owners: Vec<PyObjectRef>) -> Vec<PyObjectRef> {
        let base = self.base.read().clone();
        let root = base.as_ref().map(|base| Self::find_root_object(base));
        let cdata = root
            .as_ref()
            .and_then(|root| root.downcast_ref::<Self>())
            .map_or(self, |cdata| cdata);
        if owners.is_empty()
            && cdata
                .kept_refs
                .read()
                .as_ref()
                .is_none_or(|refs| refs.slots.is_empty())
        {
            return Vec::new();
        }
        let key = self.unique_key(index);
        let mut refs = cdata.kept_refs.write();
        if refs.is_none() && owners.is_empty() {
            return Vec::new();
        }
        refs.get_or_insert_with(Default::default)
            .replace(key, owners)
    }

    /// Find the root object (one without a base) by walking up the base chain
    fn find_root_object(obj: &PyObject) -> PyObjectRef {
        let mut root = obj.to_owned();
        while let Some(cdata) = root.downcast_ref::<Self>() {
            let base = cdata.base.read().clone();
            match base {
                Some(base) if base.downcast_ref::<Self>().is_some() => root = base,
                _ => break,
            }
        }
        root
    }

    /// Store a value in an object's _objects dict with the given key
    fn store_in_object(
        obj: &PyObject,
        key: &str,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        // Get the objects dict from the object
        let objects_lock = if let Some(cdata) = obj.downcast_ref::<Self>() {
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

        let dict_obj = objects.clone();
        drop(objects);
        // Store in dict
        if let Some(dict_obj) = dict_obj
            && let Some(dict) = dict_obj.downcast_ref::<PyDict>()
        {
            let key_obj: PyObjectRef = vm.ctx.new_str(key).into();
            dict.set_item(&*key_obj, value, vm)?;
            obj.downcast_ref::<Self>()
                .unwrap()
                .update_object_index(dict, key, vm)?;
        }

        Ok(())
    }

    /// Get kept objects from a CData instance, initializing its root container.
    pub fn get_kept_objects(value: &PyObject, vm: &VirtualMachine) -> PyObjectRef {
        let root = Self::find_root_object(value);
        let Some(cdata) = root.downcast_ref::<Self>() else {
            return vm.ctx.none();
        };
        cdata
            .objects
            .write()
            .get_or_insert_with(|| {
                if cdata.length.load() == 0 {
                    vm.ctx.none()
                } else {
                    vm.ctx.new_dict().into()
                }
            })
            .clone()
    }

    // Copying a view must retain its root's references, including private
    // backing allocations and aliases through other fields or pointers.
    // Keep the snapshot flat and deduplicate owners: self-copy must not nest
    // a new holder around all previous snapshots.
    fn hidden_refs(value: &PyObject) -> Vec<PyObjectRef> {
        let root = Self::find_root_object(value);
        root.downcast_ref::<Self>()
            .and_then(|cdata| cdata.kept_refs.read().as_ref().map(|refs| refs.snapshot()))
            .unwrap_or_default()
    }

    // A scalar owns one reference; a scalar view stores it in its root instead.
    pub(super) fn keep_value(
        &self,
        value: PyObjectRef,
        hidden: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        if vm.is_none(&value) {
            return Ok(());
        }
        let _old_hidden = self.replace_hidden(0, hidden.into_iter().collect());
        if self.base.read().is_some() {
            self.keep_ref(0, value, vm)
        } else {
            let old = self.objects.write().replace(value);
            drop(old);
            Ok(())
        }
    }

    pub(super) fn keep_cast_owner(&self, owner: PyObjectRef) {
        let old = self
            .kept_refs
            .write()
            .get_or_insert_with(Default::default)
            .replace("@cast".to_owned(), vec![owner]);
        drop(old);
    }

    // Follow provenance, including weak writer links. This is not GC traversal:
    // upgrading a weak link does not make it a stored strong ownership edge.
    fn provenance_children(obj: &PyObject, pending: &mut Vec<PyObjectRef>) {
        if let Some(cdata) = obj.downcast_ref::<Self>() {
            pending.extend(cdata.objects.read().iter().cloned());
            pending.extend(cdata.base.read().iter().cloned());
            if let Some(refs) = cdata.kept_refs.read().as_ref() {
                pending.extend(refs.snapshot());
                pending.extend(refs.buffer_alias.as_ref().map(|alias| alias.source.clone()));
            }
        } else if let Some(arg) = obj.downcast_ref::<super::_ctypes::CArgObject>() {
            pending.push(arg.obj.clone());
            pending.extend(arg.keep.iter().cloned());
        } else if let Some(dict) = obj.downcast_ref::<PyDict>() {
            pending.extend(dict.values_vec());
        } else if let Some(tuple) = obj.downcast_ref::<PyTuple>() {
            pending.extend(tuple.as_slice().iter().cloned());
        } else if obj.downcast_ref::<PyMemoryView>().is_some() {
            pending.push(buffer_owner(obj).to_owned());
        } else if let Some(weak) = obj.downcast_ref::<crate::object::PyWeak>() {
            pending.extend(weak.upgrade());
        }
    }

    // Validate a short chain of native ownership links. The public dictionary
    // is mutable: a live weak target and matching address alone prove nothing.
    // Exact-key lookups must not run user equality during pointer assignment.
    fn retains_writer_source(
        parent: &PyObject,
        source: &PyObject,
        vm: &VirtualMachine,
    ) -> PyResult<bool> {
        if parent.downcast_ref::<PyMemoryView>().is_some() {
            return Ok(buffer_owner(parent).is(source));
        }
        let Some(cdata) = parent.downcast_ref::<Self>() else {
            return Ok(false);
        };
        let base = cdata.base.read().clone();
        if base.is_some_and(|base| base.is(source)) {
            return Ok(true);
        }
        let objects = cdata.objects.read().clone();
        if let Some(dict) = objects
            .as_ref()
            .and_then(|obj| obj.downcast_ref::<PyDict>())
        {
            for found in [
                dict.inner_getitem_opt(&ObjectKey("1"), vm)?,
                dict.inner_getitem_opt(&ObjectIdKey(source.get_id()), vm)?,
            ] {
                if found.is_some_and(|obj| obj.is(source)) {
                    return Ok(true);
                }
            }
        }
        if cdata
            .kept_refs
            .read()
            .as_ref()
            .and_then(|refs| refs.buffer_alias.as_ref())
            .is_some_and(|alias| alias.source.is(source))
        {
            return Ok(true);
        }
        let cast = cdata
            .kept_refs
            .read()
            .as_ref()
            .and_then(|refs| refs.first("@cast"));
        Ok(cast
            .and_then(|arg| arg.downcast::<super::_ctypes::CArgObject>().ok())
            .is_some_and(|arg| arg.obj.is(source)))
    }

    fn writer_link_source(
        parent: &PyObject,
        link: &WriterLink,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>> {
        let Some(source) = link.source.upgrade() else {
            return Ok(None);
        };
        let keys = match &link.origin {
            WriterOrigin::Native => {
                return Ok(Self::retains_writer_source(parent, &source, vm)?.then_some(source));
            }
            WriterOrigin::Private { slot, owner } => {
                let retained = parent.downcast_ref::<Self>().and_then(|cdata| {
                    let refs = cdata.kept_refs.read();
                    let refs = refs.as_ref()?;
                    refs.slots.get(slot)?.contains(owner).then(|| {
                        refs.owners
                            .get(owner)
                            .expect("registered private owner")
                            .0
                            .clone()
                    })
                });
                let valid = retained.is_some_and(|retained| {
                    retained.is(&source)
                        || retained
                            .downcast_ref::<super::_ctypes::CArgObject>()
                            .is_some_and(|arg| {
                                arg.obj.is(&source)
                                    || arg.keep.as_ref().is_some_and(|keep| keep.is(&source))
                            })
                });
                return Ok(valid.then_some(source));
            }
            WriterOrigin::Public(keys) => keys,
        };
        let mut value = parent
            .downcast_ref::<Self>()
            .and_then(|cdata| cdata.objects.read().clone());
        for key in keys {
            let Some(current) = value.as_ref() else {
                return Ok(None);
            };
            value = match key {
                WriterKey::String(key) => match current.downcast_ref::<PyDict>() {
                    Some(dict) => dict.inner_getitem_opt(&ObjectKey(key), vm)?,
                    None => return Ok(None),
                },
                WriterKey::Id(key) => match current.downcast_ref::<PyDict>() {
                    Some(dict) => dict.inner_getitem_opt(&ObjectIdKey(*key), vm)?,
                    None => return Ok(None),
                },
                WriterKey::Index(index) => current
                    .downcast_ref::<PyTuple>()
                    .and_then(|tuple| tuple.as_slice().get(*index))
                    .cloned(),
            };
        }
        Ok(value.filter(|value| value.is(&source)))
    }

    // A copied aggregate nests dictionaries, whereas writes through its views
    // use flat index:parent keys. Follow both layouts without scanning siblings.
    fn view_writer_objects(
        cdata: &Self,
        key: &str,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<(PyObjectRef, Vec<WriterKey>)>> {
        let mut pending: Vec<_> = cdata
            .objects
            .read()
            .iter()
            .cloned()
            .map(|objects| (objects, key, Vec::new()))
            .collect();
        let mut entries = Vec::new();
        let mut visited = std::collections::HashSet::new();
        while let Some((objects, key, path)) = pending.pop() {
            if !visited.insert((objects.get_id(), key)) {
                continue;
            }
            let Some(dict) = objects.downcast_ref::<PyDict>() else {
                continue;
            };
            for internal in ["0", "1"] {
                let key = format!("{internal}:{key}");
                if let Some(value) = dict.inner_getitem_opt(&ObjectKey(&key), vm)? {
                    let mut path = path.clone();
                    path.push(WriterKey::String(key));
                    entries.push((value, path));
                }
            }
            for split in
                core::iter::once(0).chain(key.match_indices(':').map(|(index, _)| index + 1))
            {
                let suffix = &key[split..];
                if let Some(value) = dict.inner_getitem_opt(&ObjectKey(suffix), vm)? {
                    let mut path = path.clone();
                    path.push(WriterKey::String(suffix.to_owned()));
                    if split == 0 {
                        entries.push((value, path));
                    } else {
                        pending.push((value, &key[..split - 1], path));
                    }
                }
            }
        }
        Ok(entries)
    }

    // Copying an aggregate flattens its private owners into the aggregate's
    // slot. Follow suffixes as for public metadata, but cache the exact member
    // of that slot: the same owner can also survive in an unrelated field.
    fn private_writer_sources(cdata: &Self, key: &str) -> Vec<WriterSource> {
        let retained = {
            let refs = cdata.kept_refs.read();
            let Some(refs) = refs.as_ref() else {
                return Vec::new();
            };
            core::iter::once(0)
                .chain(key.match_indices(':').map(|(index, _)| index + 1))
                .flat_map(|split| {
                    let slot = &key[split..];
                    refs.slots.get(slot).into_iter().flatten().map(move |id| {
                        (
                            slot.to_owned(),
                            refs.owners
                                .get(id)
                                .expect("registered private owner")
                                .0
                                .clone(),
                        )
                    })
                })
                .collect::<Vec<_>>()
        };
        let mut sources = Vec::new();
        for (slot, owner) in retained {
            let origin = WriterOrigin::Private {
                slot,
                owner: owner.get_id(),
            };
            if let Some(arg) = owner.downcast_ref::<super::_ctypes::CArgObject>() {
                for source in core::iter::once(&arg.obj).chain(arg.keep.iter()) {
                    if source.downcast_ref::<Self>().is_some()
                        || source.downcast_ref::<PyMemoryView>().is_some()
                    {
                        sources.push((source.clone(), origin.clone()));
                    }
                }
            } else if owner.downcast_ref::<Self>().is_some()
                || owner.downcast_ref::<PyMemoryView>().is_some()
            {
                sources.push((owner, origin));
            }
        }
        sources
    }

    // A byref cast has separate metadata, unlike cast(array), which shares
    // its dictionary. Project field keys only when the native layout and
    // starting address match; arbitrary casts use the general search.
    fn projected_writer_key<'a>(
        pointer: &PyObject,
        source: &PyObject,
        key: &'a str,
    ) -> Option<&'a str> {
        let proto = {
            let info = pointer.class().stg_info_opt()?;
            if info.paramfunc != ParamFunc::Pointer {
                return None;
            }
            info.proto.as_ref()?.clone()
        };
        let pointer_data = pointer.downcast_ref::<Self>()?;
        let source_data = source.downcast_ref::<Self>()?;
        if source_data.base.read().is_some() {
            return None;
        }
        let buffer = pointer_data.buffer.read();
        if buffer.len() < rustpython_host_env::ctypes::pointer_size()
            || rustpython_host_env::ctypes::read_pointer_from_buffer(&buffer)
                != source_data.buffer.read().as_ptr() as usize
        {
            return None;
        }
        let source_info = source.class().stg_info_opt()?;
        if source_info.paramfunc == ParamFunc::Array
            && source_info
                .element_type
                .as_ref()
                .is_some_and(|element| element.is(&proto))
        {
            Some(key)
        } else if source.class().is(&proto) {
            let (field, index) = key.rsplit_once(':')?;
            (index == "0").then_some(field)
        } else {
            None
        }
    }

    // Discover once, then validate exact keys without copying growing _objects
    // dictionaries. Dictionary paths do not retain the dictionaries or values.
    fn writer_paths(
        pointer: &PyObject,
        address: usize,
        size: usize,
        vm: &VirtualMachine,
        view_key: Option<&str>,
    ) -> PyResult<Vec<(PyObjectRef, Vec<WriterSource>)>> {
        let mut pending = vec![(pointer.to_owned(), Vec::new())];
        let mut targets = Vec::new();
        let mut visited = std::collections::HashSet::new();
        let mut visited_containers = std::collections::HashSet::new();
        while let Some((obj, path)) = pending.pop() {
            if !visited.insert(obj.get_id()) {
                continue;
            }
            if !path.is_empty() && obj.downcast_ref::<Self>().is_some() {
                let root = Self::find_root_object(&obj);
                if let Some(storage) = root.downcast_ref::<Self>() {
                    // Keep smaller overlapping roots for later indexed writes.
                    // Borrowed roots also need their native exporters registered.
                    let owned_target =
                        storage.contains_storage(address, size) && !storage.is_borrowed();
                    targets.push((root, path.clone()));
                    if owned_target {
                        continue;
                    }
                }
            }
            let mut children = Vec::new();
            if let Some(cdata) = obj.downcast_ref::<Self>() {
                let base = cdata.base.read().clone();
                let cast = cdata
                    .kept_refs
                    .read()
                    .as_ref()
                    .and_then(|refs| refs.first("@cast"))
                    .and_then(|arg| arg.downcast::<super::_ctypes::CArgObject>().ok());
                let native_parent = base.is_some() || cast.is_some();
                let source_key = view_key.and_then(|key| {
                    if obj.is(pointer) {
                        Some(key)
                    } else {
                        Self::projected_writer_key(pointer, &obj, key)
                    }
                });
                let entries = if let Some(key) = source_key {
                    Self::view_writer_objects(cdata, key, vm)?
                } else {
                    cdata
                        .objects
                        .read()
                        .iter()
                        .cloned()
                        .map(|obj| (obj, Vec::new()))
                        .collect()
                };
                let mut containers = Vec::new();
                for (value, keys) in entries {
                    if value.downcast_ref::<Self>().is_some()
                        || value.downcast_ref::<PyMemoryView>().is_some()
                    {
                        children.push((value, WriterOrigin::Public(keys)));
                    } else {
                        containers.push((value, keys));
                    }
                }
                while let Some((container, keys)) = containers.pop() {
                    if !visited_containers.insert(container.get_id()) {
                        continue;
                    }
                    let entries: Vec<_> = if let Some(dict) = container.downcast_ref::<PyDict>() {
                        dict.items_vec()
                            .into_iter()
                            .filter_map(|(key, value)| {
                                let key = if let Some(key) = key
                                    .downcast_ref_if_exact::<PyStr>(vm)
                                    .and_then(|key| key.as_wtf8().as_str().ok())
                                {
                                    WriterKey::String(key.to_owned())
                                } else {
                                    let key = key
                                        .downcast_ref_if_exact::<PyInt>(vm)
                                        .and_then(|key| key.as_bigint().to_usize())
                                        .filter(|key| *key == value.get_id())?;
                                    WriterKey::Id(key)
                                };
                                Some((key, value))
                            })
                            .collect()
                    } else if let Some(tuple) = container.downcast_ref::<PyTuple>() {
                        tuple
                            .as_slice()
                            .iter()
                            .enumerate()
                            .map(|(index, value)| (WriterKey::Index(index), value.clone()))
                            .collect()
                    } else {
                        continue;
                    };
                    let before = children.len();
                    let mut nested = Vec::new();
                    for (key, value) in entries {
                        // Other slots may be arbitrary py_object payloads, not
                        // native source links. Leave those to the general fallback.
                        let native_source = match &key {
                            WriterKey::String(key) => {
                                key == "1" || key == "-1" || key.starts_with("1:")
                            }
                            WriterKey::Id(_) | WriterKey::Index(1) => true,
                            WriterKey::Index(_) => false,
                        };
                        let mut keys = keys.clone();
                        keys.push(key);
                        if value.downcast_ref::<Self>().is_some()
                            || value.downcast_ref::<PyMemoryView>().is_some()
                        {
                            if native_source {
                                children.push((value, WriterOrigin::Public(keys)));
                            }
                        } else if value.downcast_ref::<PyDict>().is_some()
                            || value.downcast_ref::<PyTuple>().is_some()
                        {
                            nested.push((value, keys));
                        }
                    }
                    // A native source already retains its own metadata. Do not
                    // mistake dictionaries stored as py_object values for more
                    // source links, making every later write scan all payloads.
                    if children.len() == before && !native_parent {
                        containers.extend(nested);
                    }
                }
                children.extend(
                    cdata
                        .kept_refs
                        .read()
                        .as_ref()
                        .and_then(|refs| refs.buffer_alias.as_ref())
                        .map(|alias| (alias.source.clone(), WriterOrigin::Native)),
                );
                if let Some(key) = source_key {
                    children.extend(Self::private_writer_sources(cdata, key));
                }
                children.extend(base.map(|base| (base, WriterOrigin::Native)));
                if let Some(arg) = cast {
                    children.push((arg.obj.clone(), WriterOrigin::Native));
                }
            } else if obj.downcast_ref::<PyMemoryView>().is_some() {
                children.push((buffer_owner(&obj).to_owned(), WriterOrigin::Native));
            }
            for (child, origin) in children {
                let mut path = path.clone();
                path.push((child.clone(), origin));
                pending.push((child, path));
            }
        }
        Ok(targets)
    }

    // Let the original storage find py_object values retained by a live writer.
    // Weak links preserve the pointer's ownership lifetime; a normal write to
    // the storage cannot leave a hidden strong reference to an obsolete value.
    pub(super) fn register_pyobject_writer(
        pointer: &PyObject,
        address: usize,
        size: usize,
        index: usize,
        vm: &VirtualMachine,
    ) -> PyResult<CDataWrite> {
        let storage = Self::find_writer_storage(pointer, address, size, vm)?;
        Ok(CDataWrite::new(storage, || {
            let root = Self::find_root_object(pointer);
            let slot = if root.is(pointer) {
                WriteSlot::Root(index)
            } else {
                WriteSlot::View(pointer.downcast_ref::<Self>().unwrap().unique_key(index))
            };
            WriteRange {
                root,
                slot,
                address,
                size,
            }
        }))
    }

    fn index_write(
        root: &PyObject,
        slot: &WriteSlot,
        address: usize,
        size: usize,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        if size == 0 {
            return Ok(());
        }
        let cdata = root.downcast_ref::<Self>().expect("ctypes retention root");
        let current = cdata.kept_refs.read().as_ref().is_some_and(|refs| {
            refs.writes.get(slot).is_some_and(|registration| {
                registration.record.address == address
                    && registration.record.size == size
                    && !registration.record.writer.is_dead()
            })
        });
        if current {
            return Ok(());
        }
        // Allocate before locking. An entry is a hint, never permission to
        // dereference native memory: readers still find a current owned object.
        let record = alloc::sync::Arc::new(BufferWrite {
            writer: root.downgrade(None, vm)?,
            address,
            size,
        });
        let registry = vm.state.ctypes_buffer_views.clone();
        // The Arc allocation stays unique while either the token or its Weak
        // is alive, so dropping an old token cannot remove a replacement.
        let key = RangeKey::new(address, size, alloc::sync::Arc::as_ptr(&record) as usize);
        let old = registry
            .writes
            .write()
            .insert(key, size, alloc::sync::Arc::downgrade(&record));
        debug_assert!(old.is_none());
        let registration = WriteRegistration {
            registry,
            key,
            record,
        };
        let old = cdata
            .kept_refs
            .write()
            .get_or_insert_with(Default::default)
            .writes
            .insert(slot.clone(), registration);
        // A slot owns at most its latest range; retargeting cannot accumulate
        // unbounded history. The full provenance fallback preserves older aliases.
        drop(old);
        Ok(())
    }

    fn find_writer_storage(
        pointer: &PyObject,
        address: usize,
        size: usize,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>> {
        let cache_root = Self::find_root_object(pointer);
        let writer = cache_root.downcast_ref::<Self>();
        let view_key = (!cache_root.is(pointer))
            .then(|| pointer.downcast_ref::<Self>().unwrap().unique_key(0)[2..].to_owned());
        let contents_key = view_key.as_ref().map(|key| format!("1:{key}"));

        // Register current contents separately: it may share the bytes with
        // an older cast source that remains reachable through another path.
        let direct = Self::register_contents_writer(
            &cache_root,
            contents_key.as_deref().unwrap_or("1"),
            pointer,
            address,
            size,
            vm,
        )?;
        let cached = writer.and_then(|writer| {
            writer
                .kept_refs
                .read()
                .as_ref()
                .and_then(|refs| match &view_key {
                    Some(key) => refs.view_writer_paths.get(key).cloned(),
                    None => refs.writer_paths.clone(),
                })
        });
        if let Some(paths) = cached {
            // The common pointer(value) case has exactly one root. Its current
            // contents and live registration were already checked above.
            if paths.len() == 1
                && paths[0].len() == 1
                && let Some(root) = &direct
                && view_key.is_none()
                && !root.downcast_ref::<Self>().unwrap().is_borrowed()
                && paths[0][0]
                    .source
                    .upgrade()
                    .is_some_and(|source| source.is(root))
            {
                return Ok(direct);
            }
            let mut target = None;
            let mut valid = true;
            let mut direct_covered = direct.is_none();
            for path in paths.iter() {
                let mut source = Some(cache_root.clone());
                for link in path {
                    let parent = source.as_ref().unwrap();
                    if !parent.is(pointer) {
                        // A view's base pointer may change contents while its
                        // original private byref path is still valid.
                        Self::register_contents_writer(parent, "1", pointer, address, size, vm)?;
                    }
                    source = Self::writer_link_source(parent, link, vm)?;
                    if source.is_none() {
                        break;
                    }
                }
                let Some(source) = source else {
                    valid = false;
                    break;
                };
                let Some(root) = Self::register_writer_source(&source, pointer, address, size, vm)?
                else {
                    continue;
                };
                // Resurrection may have cleared the registry's weakref even
                // though the cached source path still exists.
                if direct.as_ref().is_some_and(|direct| direct.is(&root)) {
                    direct_covered = true;
                }
                target = Some(root);
            }
            if valid && direct_covered && (direct.is_some() || target.is_some()) {
                return Ok(direct.or(target));
            }
        }
        let targets = Self::writer_paths(&cache_root, address, size, vm, view_key.as_deref())?;
        if !targets.is_empty() {
            let mut cache = Vec::new();
            let mut target = None;
            for (root, path) in targets {
                let destination = Self::register_writer_source(&root, pointer, address, size, vm)?;
                let overlaps = destination.is_some();
                // A field needs only paths to its own destination. Retargeting
                // invalidates these by identity/range and rediscovers the path;
                // retaining every sibling here would create N caches of N paths.
                if view_key.is_some() && !overlaps {
                    continue;
                }
                if overlaps {
                    target = destination;
                }
                cache.push(
                    path.iter()
                        .map(|(obj, origin)| {
                            Ok(WriterLink {
                                source: obj.downgrade(None, vm)?,
                                origin: origin.clone(),
                            })
                        })
                        .collect::<PyResult<Vec<_>>>()?,
                );
            }
            if let Some(writer) = writer {
                let old = {
                    let mut refs = writer.kept_refs.write();
                    let refs = refs.get_or_insert_with(Default::default);
                    match view_key {
                        Some(key) => refs.view_writer_paths.insert(key, cache.into()),
                        None => refs.writer_paths.replace(cache.into()),
                    }
                };
                drop(old);
            }
            if direct.is_some() || target.is_some() {
                return Ok(direct.or(target));
            }
        }
        let mut pending = Vec::new();
        Self::provenance_children(pointer, &mut pending);
        let mut visited = std::collections::HashSet::new();
        let mut target = None;
        while let Some(obj) = pending.pop() {
            if !visited.insert(obj.get_id()) {
                continue;
            }
            if let Some(root) = Self::register_writer_source(&obj, pointer, address, size, vm)? {
                let storage = root.downcast_ref::<Self>().unwrap();
                let owned = !storage.is_borrowed() && storage.contains_storage(address, size);
                target = Some(root);
                if owned {
                    continue;
                }
            }
            Self::provenance_children(&obj, &mut pending);
        }
        Ok(direct.or(target))
    }

    // A byref cast has separate ownership metadata and can write outside its
    // source view while remaining inside the shared exporter. Let other aliases
    // reach that writer through the source, including readers created later.
    // Only an overlapping source can also pin the write's destination.
    fn register_writer_source(
        source: &PyObject,
        pointer: &PyObject,
        address: usize,
        size: usize,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>> {
        let root = Self::find_root_object(source);
        let Some(storage) = root.downcast_ref::<Self>() else {
            return Ok(None);
        };
        let overlaps = storage.overlaps_storage(address, size);
        let borrowed = storage
            .kept_refs
            .read()
            .as_ref()
            .is_some_and(|refs| refs.buffer_alias.is_some());
        if overlaps || borrowed {
            Self::register_writer(&root, pointer, vm)?;
        }
        Ok(overlaps.then_some(root))
    }

    fn register_contents_writer(
        parent: &PyObject,
        key: &str,
        pointer: &PyObject,
        address: usize,
        size: usize,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>> {
        let objects = parent
            .downcast_ref::<Self>()
            .and_then(|parent| parent.objects.read().clone());
        let target = if let Some(dict) = objects
            .as_ref()
            .and_then(|obj| obj.downcast_ref::<PyDict>())
        {
            dict.inner_getitem_opt(&ObjectKey(key), vm)?
                .and_then(|owner| Self::overlapping_storage_root(&owner, address, size))
        } else {
            None
        };
        if let Some(root) = &target {
            Self::register_writer(root, pointer, vm)?;
        }
        Ok(target)
    }

    fn register_writer(root: &PyObject, pointer: &PyObject, vm: &VirtualMachine) -> PyResult<()> {
        // KeepRef stores a view's values in its root. The view itself may be a
        // temporary, so the reverse link must follow the same retention owner.
        let pointer = Self::find_root_object(pointer);
        if root.is(&pointer) {
            return Ok(());
        }
        let cdata = root.downcast_ref::<Self>().expect("ctypes storage root");
        let existing = cdata
            .kept_refs
            .read()
            .as_ref()
            .and_then(|refs| refs.first("@writers")?.downcast::<PyDict>().ok());
        let views = if let Some(views) = existing {
            views
        } else {
            // Allocation can collect and reenter, so do it before
            // locking, then reuse a registry installed by reentry.
            let candidate = vm.ctx.new_dict();
            let mut refs = cdata.kept_refs.write();
            let refs = refs.get_or_insert_with(Default::default);
            let existing = refs
                .first("@writers")
                .and_then(|obj| obj.downcast::<PyDict>().ok());
            if let Some(views) = existing {
                views
            } else {
                let old = refs.replace("@writers".to_owned(), vec![candidate.clone().into()]);
                debug_assert!(old.is_empty());
                candidate
            }
        };
        let registered = views
            .inner_getitem_opt(&ObjectIdKey(pointer.get_id()), vm)?
            .filter(|weak| {
                weak.downcast_ref::<crate::object::PyWeak>()
                    .and_then(|weak| weak.upgrade())
                    .is_some_and(|owner| owner.is(&pointer))
            });
        if registered.is_none() {
            // Amortize dead-link cleanup across new registrations;
            // scanning all live writers on every insert is quadratic.
            let interval = views.entries.len().max(64);
            let prune = {
                let mut refs = cdata.kept_refs.write();
                let refs = refs.as_mut().expect("registered writer dictionary");
                if refs.writer_prune_remaining == 0 {
                    refs.writer_prune_remaining = interval;
                    true
                } else {
                    refs.writer_prune_remaining -= 1;
                    false
                }
            };
            if prune {
                for (key, weak) in views.items_vec() {
                    if weak
                        .downcast_ref::<crate::object::PyWeak>()
                        .is_some_and(|weak| weak.is_dead())
                    {
                        let hash = PyDict::hash_or_unhashable(&*key, vm)?;
                        // Do not remove a replacement registered by another
                        // thread after this dead weakref was observed.
                        views.entries.delete_if(
                            vm,
                            &*key,
                            hash,
                            |current| Ok(current.is(&weak)),
                        )?;
                    }
                }
            }
            let key: PyObjectRef = vm.ctx.new_int(pointer.get_id()).into();
            views.set_item(&*key, pointer.downgrade(None, vm)?.into(), vm)?;
        }
        Ok(())
    }

    fn overlapping_storage_root(
        obj: &PyObject,
        address: usize,
        size: usize,
    ) -> Option<PyObjectRef> {
        let root = Self::find_root_object(obj);
        root.downcast_ref::<Self>()
            .is_some_and(|storage| storage.overlaps_storage(address, size))
            .then_some(root)
    }

    // Register every affected reader, even if the assignment is wider than its
    // view. This is provenance bookkeeping, not validation of the native write.
    fn overlaps_storage(&self, address: usize, size: usize) -> bool {
        let buffer = self.buffer.read();
        let start = buffer.as_ptr() as usize;
        self.base
            .read()
            .as_ref()
            .is_none_or(|base| base.downcast_ref::<Self>().is_none())
            && size != 0
            && !buffer.is_empty()
            && if address >= start {
                address - start < buffer.len()
            } else {
                start - address < size
            }
    }

    fn contains_storage(&self, address: usize, size: usize) -> bool {
        let buffer = self.buffer.read();
        self.base
            .read()
            .as_ref()
            .is_none_or(|base| base.downcast_ref::<Self>().is_none())
            && address
                .checked_sub(buffer.as_ptr() as usize)
                .and_then(|offset| offset.checked_add(size))
                .is_some_and(|end| end <= buffer.len())
    }

    // Writes into a root's own storage already retain their values there.
    // Follow only buffer/base links for borrowed storage, without searching
    // growing _objects dictionaries or user values for an unrelated target.
    fn register_storage_writer(
        writer: &PyObject,
        address: usize,
        size: usize,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>> {
        let Some(mut storage) = Self::overlapping_storage_root(writer, address, size) else {
            return Ok(None);
        };
        if !storage.downcast_ref::<Self>().unwrap().is_borrowed() {
            return Ok(Some(storage));
        }
        let mut source = storage.clone();
        let mut visited = std::collections::HashSet::new();
        loop {
            if !visited.insert(source.get_id()) {
                return Ok(Some(storage));
            }
            if let Some(cdata) = source.downcast_ref::<Self>() {
                let Some(root) = Self::overlapping_storage_root(&source, address, size) else {
                    // A buffer view can be based on a pointer whose contents
                    // require the regular cast/contents provenance search.
                    return Ok(None);
                };
                Self::register_writer(&root, writer, vm)?;
                storage = root;
                if !cdata.is_borrowed() {
                    return Ok(Some(storage));
                }
                let base = cdata.base.read().clone();
                let next = if base.is_some() {
                    base
                } else {
                    let objects = cdata.objects.read().clone();
                    if let Some(dict) = objects
                        .as_ref()
                        .and_then(|obj| obj.downcast_ref::<PyDict>())
                    {
                        dict.inner_getitem_opt(&ObjectKey("-1"), vm)?
                    } else {
                        None
                    }
                };
                let next = next.or_else(|| {
                    cdata
                        .kept_refs
                        .read()
                        .as_ref()
                        .and_then(|refs| refs.buffer_alias.as_ref())
                        .map(|alias| alias.source.clone())
                });
                let Some(next) = next else {
                    return Ok(Some(storage));
                };
                source = next;
            } else if source.downcast_ref::<PyMemoryView>().is_some() {
                source = buffer_owner(&source).to_owned();
            } else {
                return Ok(Some(storage));
            }
        }
    }

    pub(super) fn register_pyobject_view(
        &self,
        index: usize,
        offset: usize,
        size: usize,
        vm: &VirtualMachine,
    ) -> PyResult<CDataWrite> {
        let base = self.base.read().clone();
        if let Some(base) = base {
            let root = Self::find_root_object(&base);
            let address = self.buffer.read().as_ptr().wrapping_add(offset) as usize;
            let storage = match Self::register_storage_writer(&root, address, size, vm)? {
                Some(storage) => Some(storage),
                None => Self::find_writer_storage(&root, address, size, vm)?,
            };
            return Ok(CDataWrite::new(storage, || WriteRange {
                root,
                slot: WriteSlot::View(self.unique_key(index)),
                address,
                size,
            }));
        }
        Ok(CDataWrite::default())
    }

    fn update_object_index(
        &self,
        dict: &Py<PyDict>,
        key: &str,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        if self.kept_refs.read().as_ref().is_none_or(|refs| {
            refs.object_index
                .as_ref()
                .is_none_or(|index| index.dictionary != dict.get_id())
        }) {
            return Ok(());
        }
        // set_item can run a displaced owner's finalizer. Observe the value
        // after reentry, rather than publishing the outer assignment's value.
        let value = dict.inner_getitem_opt(&ObjectKey(key), vm)?;
        let len = dict.entries.len();
        let mut refs = self.kept_refs.write();
        if let Some(refs) = refs.as_mut()
            && let Some(index) = refs.object_index.as_mut()
            && index.dictionary == dict.get_id()
        {
            index.set(
                WriterKey::String(key.to_owned()),
                value.as_ref().map(|v| v.get_id()),
            );
            // Direct user edits can remove keys without passing through KeepRef.
            // Bound obsolete hints instead of accumulating assignment history.
            if index.slots.len() > len.saturating_mul(2).saturating_add(64) {
                refs.object_index = None;
            }
        }
        drop(refs);
        drop(value);
        Ok(())
    }

    fn indexed_object(&self, address: usize, vm: &VirtualMachine) -> PyResult<Option<PyObjectRef>> {
        // Private owners already have an identity index; do not snapshot them
        // merely to find a single retained object.
        let private = self
            .kept_refs
            .read()
            .as_ref()
            .and_then(|refs| refs.owners.get(&address).map(|(value, _)| value.clone()));
        if let Some(value) = private {
            return Ok(Some(value));
        }
        let objects = self.objects.read().clone();
        let Some(dict) = objects
            .as_ref()
            .and_then(|obj| obj.downcast_ref::<PyDict>())
        else {
            return Ok(None);
        };
        let build = self.kept_refs.read().as_ref().is_none_or(|refs| {
            refs.object_index
                .as_ref()
                .is_none_or(|index| index.dictionary != dict.get_id())
        });
        if build {
            let mut index = ObjectIndex {
                dictionary: dict.get_id(),
                slots: Default::default(),
                values: Default::default(),
            };
            // No metadata lock while snapshots release Python references.
            for (key, value) in dict.items_vec() {
                let key = if let Some(key) = key
                    .downcast_ref_if_exact::<PyStr>(vm)
                    .and_then(|key| key.as_wtf8().as_str().ok())
                {
                    WriterKey::String(key.to_owned())
                } else if let Some(key) = key
                    .downcast_ref_if_exact::<PyInt>(vm)
                    .and_then(|key| key.as_bigint().to_usize())
                {
                    WriterKey::Id(key)
                } else {
                    continue;
                };
                index.set(key, Some(value.get_id()));
            }
            self.kept_refs
                .write()
                .get_or_insert_with(Default::default)
                .object_index = Some(Box::new(index));
        }
        let attempts = self
            .kept_refs
            .read()
            .as_ref()
            .and_then(|refs| refs.object_index.as_ref())
            .filter(|index| index.dictionary == dict.get_id())
            .and_then(|index| index.values.get(&address))
            .map_or(0, ObjectKeys::len);
        // Concurrent writers can reinsert stale hints. Bound retries and fall
        // back to the ordinary graph search rather than spinning in the cache.
        for _ in 0..attempts {
            let key = self
                .kept_refs
                .read()
                .as_ref()
                .and_then(|refs| refs.object_index.as_ref())
                .filter(|index| index.dictionary == dict.get_id())
                .and_then(|index| index.values.get(&address).map(|keys| keys.first().clone()));
            let Some(key) = key else { return Ok(None) };
            let value = match &key {
                WriterKey::String(key) => dict.inner_getitem_opt(&ObjectKey(key), vm)?,
                WriterKey::Id(key) => dict.inner_getitem_opt(&ObjectIdKey(*key), vm)?,
                WriterKey::Index(_) => unreachable!("dictionary retention key"),
            };
            // A concurrent write or direct _objects mutation may stale a hint.
            // Only the current dictionary reference can establish ownership.
            if value
                .as_ref()
                .is_some_and(|value| value.get_id() == address)
            {
                return Ok(value);
            }
            let mut refs = self.kept_refs.write();
            if let Some(index) = refs.as_mut().and_then(|refs| refs.object_index.as_mut())
                && index.dictionary == dict.get_id()
                && index.slots.get(&key) == Some(&address)
            {
                index.set(key, value.as_ref().map(|value| value.get_id()));
            }
            drop(refs);
            drop(value);
        }
        Ok(None)
    }

    // Resolve a py_object through owned references, never by turning an
    // arbitrary integer from native memory into a Rust reference. Views can
    // reach the object through nested _objects dictionaries or cast cycles.
    // slot is the native location read, distinct from the Python object address.
    pub(super) fn object_at(
        &self,
        address: usize,
        slot: usize,
        index: Option<usize>,
        vm: &VirtualMachine,
    ) -> PyResult {
        if address == 0 {
            return Err(vm.new_value_error("PyObject is NULL"));
        }
        if address == vm.ctx.none.get_id() {
            return Ok(vm.ctx.none());
        }
        let objects = self.objects.read().clone();
        if let Some(obj) = &objects
            && obj.get_id() == address
        {
            return Ok(obj.clone());
        }
        let base = self.base.read().clone();
        let root = base.as_ref().map(|base| Self::find_root_object(base));
        let owners = root
            .as_ref()
            .and_then(|root| root.downcast_ref::<Self>())
            .and_then(|root| root.objects.read().clone())
            .or_else(|| objects.clone());
        if let Some(dict) = owners.as_ref().and_then(|obj| obj.downcast_ref::<PyDict>()) {
            let key = self.unique_key(index.unwrap_or(0));
            // A scalar view's setter stores under "0:<view path>", whereas
            // assignment through its parent stores under "<view path>".
            let parent_key = if index.is_none() {
                key.strip_prefix("0:")
            } else {
                None
            };
            for key in core::iter::once(key.as_str()).chain(parent_key) {
                if let Some(obj) = dict.inner_getitem_opt(&ObjectKey(key), vm)?
                    && obj.get_id() == address
                {
                    return Ok(obj);
                }
            }
        }
        // Casts, copied aggregates and buffer views can retain the referent
        // through other keys or containers. Search those only on a direct miss.
        let mut pending: Vec<_> = objects.into_iter().collect();
        pending.extend(base);
        if let Some(refs) = self.kept_refs.read().as_ref() {
            pending.extend(refs.snapshot());
            pending.extend(refs.buffer_alias.as_ref().map(|alias| alias.source.clone()));
        }
        // Written ranges are hints to live retention roots, including writes
        // outside a source view and copies whose original exporter view died.
        // Only current Python owners with the matching identity can be returned.
        pending.extend(vm.state.ctypes_buffer_views.writers(slot));
        let mut visited = std::collections::HashSet::new();
        let mut exporters = std::collections::HashSet::new();
        let mut deferred_exporters: Vec<PyObjectRef> = Vec::new();
        loop {
            let Some(obj) = pending.pop() else {
                let Some(exporter) = deferred_exporters.pop() else {
                    break;
                };
                // A cast can write beyond its source view's declared extent.
                // Preserve the full provenance search only after indexed paths
                // miss; ordinary disjoint views never scan all sibling aliases.
                pending.extend(vm.state.ctypes_buffer_views.owners(&exporter, None));
                continue;
            };
            if obj.get_id() == address {
                return Ok(obj);
            }
            if !visited.insert(obj.get_id()) {
                continue;
            }
            if let Some(cdata) = obj.downcast_ref::<Self>()
                && let Some(value) = cdata.indexed_object(address, vm)?
            {
                return Ok(value);
            }
            if let Some(view) = obj.downcast_ref::<PyMemoryView>()
                && exporters.insert(buffer_owner(view.as_object()).get_id())
            {
                let exporter = buffer_owner(view.as_object());
                pending.extend(vm.state.ctypes_buffer_views.owners(exporter, Some(slot)));
                deferred_exporters.push(exporter.to_owned());
            }
            Self::provenance_children(&obj, &mut pending);
        }
        Err(vm.new_value_error("PyObject is NULL"))
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
        let array_code = PyCField::array_type_code(proto);

        // For c_char arrays with bytes input, copy only up to first null
        if array_code == Some('c') {
            if let Some(bytes_val) = value.downcast_ref::<PyBytes>() {
                let src = bytes_val.as_bytes();
                let to_copy = char_array_assignment_bytes(src);
                let copy_len = core::cmp::min(to_copy.len(), size);
                self.write_bytes_at_offset(offset, &to_copy[..copy_len]);
                self.keep_ref(index, value, vm)?;
                return Ok(());
            }
            return Err(vm.new_type_error("bytes expected instead of str instance"));
        }

        // For c_wchar arrays with str input, convert to wchar_t
        if array_code == Some('u') {
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

        let proto_type = proto
            .downcast_ref::<PyType>()
            .ok_or_else(|| vm.new_type_error("field proto is not a type"))?;
        let write = if proto_type
            .stg_info(vm)?
            .flags
            .intersects(StgInfoFlags::TYPEFLAG_ISPOINTER | StgInfoFlags::TYPEFLAG_HASPOINTER)
        {
            self.register_pyobject_view(index, offset, size, vm)?
        } else {
            CDataWrite::default()
        };
        let prepared = CDataValue::new(proto_type, &value, size, needs_swap, vm)?;
        write.publish(vm)?;
        self.write_bytes_at_offset(offset, &prepared.bytes);
        prepared.keep(self, index, vm)
    }

    pub(super) fn into_view(self, cls: PyTypeRef, vm: &VirtualMachine) -> PyResult {
        if cls.fast_issubclass(super::function::PyCFuncPtr::static_type()) {
            super::function::PyCFuncPtr::from_cdata(self)
                .into_ref_with_type(vm, cls)
                .map(Into::into)
        } else {
            self.into_ref_with_type(vm, cls).map(Into::into)
        }
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
            if PyCField::array_type_code(proto) == Some('c') {
                let data = &buffer[offset..offset + size];
                return Ok(vm
                    .ctx
                    .new_bytes(char_array_field_value(data).to_vec())
                    .into());
            }

            // c_wchar array → return str
            if PyCField::array_type_code(proto) == Some('u') {
                let data = &buffer[offset..offset + size];
                return Ok(vm.ctx.new_str(wchar_array_field_value(data)).into());
            }

            // Array fields share their storage with the containing object.
            // SAFETY: the field lies within the currently locked base buffer.
            // The base reference retains the owner, not the allocation address;
            // a reallocating resize can invalidate this view, as with other ctypes views.
            let cdata_obj = unsafe {
                Self::from_base_obj(
                    buffer.as_ptr().add(offset).cast_mut(),
                    size,
                    base_obj,
                    index,
                )
            };
            cdata_obj.base_offset.store(offset);
            cdata_obj.length.store(stg.length);
            drop(buffer);
            let array_type: PyTypeRef = proto
                .to_owned()
                .downcast()
                .map_err(|_| vm.new_type_error("expected array type"))?;

            return super::array::PyCArray(cdata_obj)
                .into_ref_with_type(vm, array_type)
                .map(Into::into);
        }

        let buffer_data = buffer[offset..offset + size].to_vec();
        let slot = buffer.as_ptr().wrapping_add(offset) as usize;
        drop(buffer);

        // Get proto as type
        let proto_type: PyTypeRef = proto
            .to_owned()
            .downcast()
            .map_err(|_| vm.new_type_error("field proto is not a type"))?;

        let proto_metaclass = proto_type.class();

        // Simple types: return primitive value
        if super::simple::returns_scalar(&proto_type) {
            if proto_type.type_code(vm).as_deref() == Some("O") {
                let address = rustpython_host_env::ctypes::read_pointer_from_buffer(&buffer_data);
                return self.object_at(address, slot, Some(index), vm);
            }
            // Check for byte swapping
            let needs_swap = super::simple::is_swapped(&proto_type);

            let data = if needs_swap && size > 1 {
                let mut swapped = buffer_data;
                swapped.reverse();
                swapped
            } else {
                buffer_data
            };

            return Ok(bytes_to_pyobject(&proto_type, &data, vm));
        }

        // Complex types: create ctypes instance via PyCData_FromBaseObj
        let ptr = self.buffer.read().as_ptr().wrapping_add(offset) as *mut u8;
        let cdata_obj = unsafe { Self::from_base_obj(ptr, size, base_obj, index) };

        if proto_metaclass.fast_issubclass(super::simple::PyCSimpleType::static_type())
            || proto_metaclass.fast_issubclass(super::structure::PyCStructType::static_type())
            || proto_metaclass.fast_issubclass(super::union::PyCUnionType::static_type())
            || proto_metaclass.fast_issubclass(super::pointer::PyCPointerType::static_type())
            || proto_type.fast_issubclass(super::function::PyCFuncPtr::static_type())
        {
            cdata_obj.into_view(proto_type, vm)
        } else {
            // Fallback
            Ok(vm.ctx.new_int(0).into())
        }
    }
}

#[pyclass(flags(BASETYPE))]
impl Py<PyCData> {
    #[pygetset]
    fn _objects(&self) -> Option<PyObjectRef> {
        self.objects.read().clone()
    }

    #[pygetset]
    fn _b_base_(&self) -> Option<PyObjectRef> {
        self.base.read().clone()
    }

    #[pymethod]
    fn __ctypes_from_outparam__(zelf: PyRef<PyCData>, _vm: &VirtualMachine) -> PyObjectRef {
        zelf.into()
    }

    #[pygetset]
    fn _b_needsfree_(&self) -> i32 {
        // Borrowed (from_address) or has base object → 0 (don't free)
        // Owned and no base → 1 (need to free)
        i32::from(!(self.is_borrowed() || self.base.read().is_some()))
    }

    // CDataType_methods - shared across all ctypes types

    #[pyclassmethod]
    pub(super) fn from_buffer(
        cls: PyTypeRef,
        source: PyObjectRef,
        offset: OptionalArg<isize>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let cdata = PyCData::from_buffer_impl(&cls, source, offset.unwrap_or(0), vm)?;
        let cdata = cdata.into_ref_with_type(vm, cls)?;
        let objects = cdata
            .objects
            .read()
            .as_ref()
            .unwrap()
            .clone()
            .downcast::<PyDict>()
            .unwrap();
        let source = objects.inner_getitem_opt(&ObjectKey("-1"), vm)?.unwrap();
        let exporter = buffer_owner(&source);
        let weak = cdata.as_object().downgrade(None, vm)?;
        let exporter_id = exporter.get_id();
        let (address, size) = {
            let buffer = cdata.buffer.read();
            (buffer.as_ptr() as usize, buffer.len())
        };
        let key = RangeKey::new(address, size, cdata.get_id());
        {
            let mut refs = cdata.kept_refs.write();
            let refs = refs.get_or_insert_with(Default::default);
            // Scalar assignment can replace _objects. Keep the buffer export
            // for the borrowed slice's lifetime and prevent identity reuse.
            refs.buffer_alias = Some(BufferAlias {
                registry: vm.state.ctypes_buffer_views.clone(),
                exporter: exporter_id,
                key,
                source,
            });
        }
        let old = vm
            .state
            .ctypes_buffer_views
            .views
            .write()
            .entry(exporter_id)
            .or_default()
            .insert(key, size, weak);
        drop(old);
        // A native CData exporter must also see direct scalar assignments;
        // those replace the view's _objects without going through a pointer.
        if PyCData::register_storage_writer(cdata.as_object(), address, size, vm)?.is_none() {
            PyCData::find_writer_storage(cdata.as_object(), address, size, vm)?;
        }
        Ok(cdata.into())
    }

    #[pyclassmethod]
    pub(super) fn from_buffer_copy(
        cls: PyTypeRef,
        source: ArgBytesLike,
        offset: OptionalArg<isize>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let cdata =
            PyCData::from_buffer_copy_impl(&cls, &source.borrow_buf(), offset.unwrap_or(0), vm)?;
        cdata.into_ref_with_type(vm, cls).map(Into::into)
    }

    #[pyclassmethod]
    pub(super) fn from_address(
        cls: PyTypeRef,
        address: ArgStrictInt<isize>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let address = address.value;
        let size = {
            let stg_info = cls.stg_info(vm)?;
            stg_info.size
        };

        if size == 0 {
            return Err(vm.new_type_error("abstract class"));
        }

        // PyCData_AtAddress
        let cdata = unsafe { PyCData::at_address(address as *const u8, size) };
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
        let cdata = unsafe { PyCData::at_address(ptr, size) };
        cdata.into_ref_with_type(vm, cls).map(Into::into)
    }
}

// PyCField - Field descriptor for Structure/Union types

#[pyclass(name = "CField", module = "ctypes", traverse = "manual")]
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

// SAFETY: proto is the descriptor's only owned Python reference.
unsafe impl Traverse for PyCField {
    fn traverse(&self, tracer: &mut TraverseFn<'_>) {
        self.proto.traverse(tracer);
    }
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

    pub(super) fn checked_bitfield_size(
        name: &str,
        proto: &Py<PyType>,
        value: &PyObject,
        vm: &VirtualMachine,
    ) -> PyResult<u16> {
        let bits = value.try_int(vm)?.as_bigint().to_u16();
        let info = proto.stg_info(vm)?;
        let bits = bits
            .filter(|&bits| bits > 0 && usize::from(bits) <= info.size * 8)
            .ok_or_else(|| {
                vm.new_value_error(format!("number of bits invalid for bit field '{name}'"))
            })?;
        if !matches!(
            info.scalar_code,
            Some('b' | 'B' | 'h' | 'H' | 'i' | 'I' | 'l' | 'L' | 'q' | 'Q' | '?')
        ) {
            return Err(
                vm.new_type_error(format!("bit fields not allowed for type {}", proto.name()))
            );
        }
        Ok(bits)
    }

    /// Get the byte size of the field's underlying type
    pub fn get_byte_size(&self) -> usize {
        self.byte_size_val as usize
    }

    /// Create a new CField from an existing field with adjusted offset and index
    /// Used by MakeFields to promote anonymous fields
    pub(crate) fn new_from_field(fdescr: &Self, index_offset: usize, offset_delta: isize) -> Self {
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
            v.try_to_bool(vm)?
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
        let type_size = super::base::get_field_size(field_type.as_object(), vm) as isize;
        if byte_size != type_size {
            return Err(vm.new_value_error(format!(
                "byte_size {byte_size} does not match type size {type_size}"
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
            if !matches!(
                field_type.stg_info(vm)?.scalar_code,
                Some('b' | 'B' | 'h' | 'H' | 'i' | 'I' | 'l' | 'L' | 'q' | 'Q' | '?')
            ) {
                return Err(vm.new_type_error(format!(
                    "bit fields not allowed for type {}",
                    field_type.name()
                )));
            }
            if bs <= 0 {
                return Err(vm.new_value_error("number of bits invalid for bit field"));
            }
            let bo = bit_offset_val.unwrap_or(0);
            if bo < 0 {
                return Err(vm.new_value_error("bit_offset must be >= 0"));
            }
            let type_bits = byte_size * 8;
            if bs > type_bits || bo > type_bits - bs {
                return Err(vm.new_value_error(format!(
                    "bit field '{name}' overflows its type ({bo} + {bs} > {type_bits})"
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
        zelf: &PyObject,
        obj: Option<&PyObject>,
        _cls: Option<&PyObject>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let zelf = zelf
            .to_owned()
            .downcast::<Self>()
            .map_err(|_| vm.new_type_error("expected CField"))?;

        // If obj is None, return the descriptor itself (class attribute access)
        let obj = match obj {
            Some(obj) if !vm.is_none(obj) => obj.to_owned(),
            _ => return Ok(zelf.into()),
        };

        let offset = zelf.offset as usize;
        let size = zelf.get_byte_size();

        // Get PyCData from obj (works for both Structure and Union)
        let cdata = Self::get_cdata_from_obj(&obj, vm)?;

        if zelf.bitfield_size > 0 && zelf.proto.stg_info(vm)?.scalar_code != Some('?') {
            let buffer = cdata.buffer.read();
            let data = buffer
                .get(offset..offset.saturating_add(size))
                .ok_or_else(|| vm.new_value_error("field outside the buffer"))?;
            let value = zelf.bitfield_word(data);
            let bits = zelf.bitfield_size as u32;
            let value = (value >> zelf.bit_offset_val) & (u64::MAX >> (64 - bits));
            let signed = matches!(
                zelf.proto.stg_info(vm)?.scalar_code,
                Some('b' | 'h' | 'i' | 'l' | 'q')
            );
            drop(buffer);
            return Ok(if signed {
                vm.ctx
                    .new_int(((value << (64 - bits)) as i64) >> (64 - bits))
                    .into()
            } else {
                vm.ctx.new_int(value).into()
            });
        }

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
    fn bitfield_word(&self, data: &[u8]) -> u64 {
        let mut bytes = [0; 8];
        if self
            .proto
            .stg_info_opt()
            .is_some_and(|info| info.big_endian)
        {
            bytes[8 - data.len()..].copy_from_slice(data);
            u64::from_be_bytes(bytes)
        } else {
            bytes[..data.len()].copy_from_slice(data);
            u64::from_le_bytes(bytes)
        }
    }

    // Character-array fields use the element's initialized scalar converter.
    fn array_type_code(proto: &PyObject) -> Option<char> {
        let info = proto.downcast_ref::<PyType>()?.stg_info_opt()?;
        info.element_type.as_ref()?.stg_info_opt()?.scalar_code
    }
}

impl PyCField {
    /// Get PyCData from object (works for both Structure and Union)
    fn get_cdata_from_obj<'a>(obj: &'a PyObject, vm: &VirtualMachine) -> PyResult<&'a PyCData> {
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
}

#[pyclass(flags(IMMUTABLETYPE), with(Representable, GetDescriptor, Constructor))]
impl Py<PyCField> {
    // PyCField_set
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
        let cdata = PyCField::get_cdata_from_obj(&obj, vm)?;

        match value {
            PySetterValue::Assign(value) => {
                // Check if needs byte swapping
                let needs_swap = super::simple::is_swapped(&zelf.proto) && size > 1;

                let code = zelf.proto.stg_info(vm)?.scalar_code;
                if zelf.bitfield_size > 0 && code != Some('?') {
                    let code = code.ok_or_else(|| vm.new_type_error("invalid bitfield type"))?;
                    let mut encoded = [0; 4];
                    // __index__ may reenter and change adjacent fields. Convert
                    // before locking, then merge into the current storage.
                    let (bytes, _, _) = super::simple::assignment_value(
                        code.encode_utf8(&mut encoded),
                        &value,
                        needs_swap,
                        vm,
                    )?;
                    let value = zelf.bitfield_word(&bytes);
                    let bits = zelf.bitfield_size as u32;
                    let mask = (u64::MAX >> (64 - bits)) << zelf.bit_offset_val;
                    let mut buffer = cdata.buffer.write();
                    let data = buffer
                        .get(offset..offset.saturating_add(size))
                        .ok_or_else(|| vm.new_value_error("field outside the buffer"))?;
                    let word = (zelf.bitfield_word(data) & !mask)
                        | ((value << zelf.bit_offset_val) & mask);
                    let big_endian = zelf.proto.stg_info(vm)?.big_endian;
                    let bytes = if big_endian {
                        word.to_be_bytes()
                    } else {
                        word.to_le_bytes()
                    };
                    let data = if big_endian {
                        &bytes[8 - size..]
                    } else {
                        &bytes[..size]
                    };
                    write_cow_bytes_at_offset(&mut buffer, offset, data);
                    return Ok(());
                }

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
        ParamFunc::Structure | ParamFunc::Union => Ok(struct_union_paramfunc(obj, &stg_info, vm)),
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

    Ok(CArgObject {
        tag,
        value: CArgValue::typed(tag as char, &buffer),
        obj: obj.to_owned(),
        keep: None,
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
        value: CArgValue::pointer(ptr_val),
        obj: obj.to_owned(),
        keep: None,
        size: 0,
        offset: 0,
    })
}

/// PyCPointerType_paramfunc
fn pointer_paramfunc(obj: &PyObject, vm: &VirtualMachine) -> PyResult<CArgObject> {
    use super::function::PyCFuncPtr;
    use super::pointer::PyCPointer;

    // parg->value.p = *(void **)self->b_ptr
    let ptr_val = if let Some(ptr) = obj.downcast_ref::<PyCPointer>() {
        ptr.get_ptr_value()
    } else if let Some(funcptr) = obj.downcast_ref::<PyCFuncPtr>() {
        funcptr.get_func_ptr()
    } else {
        return Err(vm.new_type_error("expected pointer"));
    };

    Ok(CArgObject {
        tag: b'P',
        value: CArgValue::pointer(ptr_val),
        obj: obj.to_owned(),
        keep: None,
        size: 0,
        offset: 0,
    })
}

/// StructUnionType_paramfunc (for both Structure and Union)
fn struct_union_paramfunc(obj: &PyObject, stg_info: &StgInfo, _vm: &VirtualMachine) -> CArgObject {
    // Snapshot the instance bytes and pass the aggregate by value. The layout
    // is built here from the already-borrowed `stg_info` to avoid re-locking.
    let (bytes, size) = if let Some(cdata) = obj.downcast_ref::<PyCData>() {
        let buffer = cdata.buffer.read();
        (buffer.to_vec(), buffer.len())
    } else {
        (Vec::new(), stg_info.size)
    };

    let layout = if matches!(stg_info.paramfunc, ParamFunc::Union) {
        CTypeLayout::Union {
            fields: stg_info.field_layouts.clone(),
            size: stg_info.size,
        }
    } else {
        CTypeLayout::Struct {
            fields: stg_info.field_layouts.clone(),
            size: stg_info.size,
        }
    };

    CArgObject {
        tag: b'V',
        value: CArgValue::aggregate(layout, bytes),
        obj: obj.to_owned(),
        keep: None,
        size,
        offset: 0,
    }
}

// CArgValue - Owned foreign-call argument value

/// A foreign-call argument in a form the unified `call` entry point accepts: a
/// simple-typed scalar (its ctypes code plus a native-endian bytes snapshot),
/// an untyped int, or an address. Any object whose memory an address
/// refers to is kept alive by the enclosing `Argument`/`CArgObject`, not here.
#[derive(Debug, Clone)]
pub enum CArgValue {
    /// A value typed by its ctypes simple-type code, snapshotted as its bytes.
    Typed { code: char, bytes: Vec<u8> },
    /// Untyped Python int (ConvParam default: C int).
    Int(i32),
    /// Address-valued argument (pointer decay, byref, buffer copies, NULL = 0).
    Pointer(usize),
    /// By-value aggregate: its call layout plus a snapshot of its bytes.
    Aggregate { layout: CTypeLayout, bytes: Vec<u8> },
}

impl CArgValue {
    pub fn pointer(value: usize) -> Self {
        Self::Pointer(value)
    }

    /// Snapshot a simple-typed value from its code and buffer bytes.
    pub(super) fn typed(code: char, buffer: &[u8]) -> Self {
        Self::Typed {
            code,
            bytes: buffer.to_vec(),
        }
    }

    /// Snapshot an aggregate value from its call layout and buffer bytes.
    pub(super) fn aggregate(layout: CTypeLayout, bytes: Vec<u8>) -> Self {
        Self::Aggregate { layout, bytes }
    }

    /// Lower to a [`CallArg`], borrowing `code_buf` for the code's `&str`.
    pub(super) fn as_call_arg<'a>(
        &'a self,
        code_buf: &'a mut [u8; 4],
    ) -> rustpython_host_env::ctypes::CallArg<'a> {
        use rustpython_host_env::ctypes::CallArg;
        match self {
            Self::Typed { code, bytes } => CallArg::Typed {
                code: code.encode_utf8(code_buf),
                buffer: bytes,
            },
            Self::Int(value) => CallArg::Int(*value),
            Self::Pointer(value) => CallArg::Pointer(*value),
            Self::Aggregate { layout, bytes } => CallArg::Aggregate {
                layout,
                buffer: bytes,
            },
        }
    }
}

/// Convert bytes to appropriate Python object based on ctypes type
pub(super) fn bytes_to_pyobject(
    cls: &Py<PyType>,
    bytes: &[u8],
    vm: &VirtualMachine,
) -> PyObjectRef {
    if let Some(ty) = cls.type_code(vm) {
        return match rustpython_host_env::ctypes::decode_type_code(ty.as_str(), bytes) {
            rustpython_host_env::ctypes::DecodedValue::Bytes(value) => {
                vm.ctx.new_bytes(value).into()
            }
            rustpython_host_env::ctypes::DecodedValue::Signed(value) => {
                vm.ctx.new_int(value).into()
            }
            rustpython_host_env::ctypes::DecodedValue::Unsigned(value) => {
                vm.ctx.new_int(value).into()
            }
            rustpython_host_env::ctypes::DecodedValue::Float(value) => {
                vm.ctx.new_float(value).into()
            }
            rustpython_host_env::ctypes::DecodedValue::Bool(value) => vm.ctx.new_bool(value).into(),
            rustpython_host_env::ctypes::DecodedValue::Pointer(value) => {
                if value == 0 {
                    vm.ctx.none()
                } else {
                    vm.ctx.new_int(value).into()
                }
            }
            rustpython_host_env::ctypes::DecodedValue::String(value) => {
                vm.ctx.new_str(value).into()
            }
            rustpython_host_env::ctypes::DecodedValue::None => vm.ctx.none(),
        };
    }
    // Default: return bytes as-is
    vm.ctx.new_bytes(bytes.to_vec()).into()
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
    if type_ref.init_traced_type_data(stg_info.clone()).is_err()
        && let Some(mut existing) = type_ref.get_type_data_mut::<StgInfo>()
    {
        // Preserve pointer_type cache across StgInfo replacement
        let mut old = core::mem::replace(&mut *existing, stg_info);
        if existing.pointer_type.is_none() {
            existing.pointer_type = old.pointer_type.take();
        }
        drop(existing);
        drop(old);
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
pub(super) fn get_field_size(field_type: &PyObject, vm: &VirtualMachine) -> usize {
    if let Some(type_obj) = field_type.downcast_ref::<PyType>()
        && let Some(stg_info) = type_obj.stg_info_opt()
    {
        return stg_info.size;
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
        return size;
    }

    if let Some(s) = field_type
        .get_attr("size_of_instances", vm)
        .ok()
        .and_then(|size_method| size_method.call((), vm).ok())
        .and_then(|size| size.try_int(vm).ok())
        .and_then(|n| n.as_bigint().to_usize())
    {
        return s;
    }

    rustpython_host_env::ctypes::pointer_size()
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
        tuple.as_slice().to_vec()
    } else {
        return Err(vm.new_type_error("_fields_ must be a sequence"));
    };

    for pair in &fieldlist {
        let field_tuple = pair
            .downcast_ref::<PyTuple>()
            .ok_or_else(|| vm.new_type_error("_fields_ must contain tuples"))?;

        if field_tuple.as_slice().len() < 2 {
            continue;
        }

        let fname = field_tuple
            .as_slice()
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
        tuple.as_slice().to_vec()
    } else {
        return Err(vm.new_type_error("_anonymous_ must be a sequence"));
    };

    for fname_obj in &anon_names {
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

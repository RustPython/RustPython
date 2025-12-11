use crate::PyObjectRef;

/// Storage information for ctypes types
/// Stored in TypeDataSlot of heap types (PyType::init_type_data/get_type_data)
#[derive(Clone)]
pub struct StgInfo {
    pub initialized: bool,
    pub size: usize,                // number of bytes
    pub align: usize,               // alignment requirements
    pub length: usize,              // number of fields (for arrays/structures)
    pub proto: Option<PyObjectRef>, // Only for Pointer/ArrayObject
    pub flags: i32,                 // calling convention and such

    // Array-specific fields (moved from PyCArrayType)
    pub element_type: Option<PyObjectRef>, // _type_ for arrays
    pub element_size: usize,               // size of each element
}

// StgInfo is stored in type_data which requires Send + Sync.
// The PyObjectRef in proto/element_type fields is protected by the type system's locking mechanism.
// CPython: ctypes objects are not thread-safe by design; users must synchronize access.
unsafe impl Send for StgInfo {}
unsafe impl Sync for StgInfo {}

impl std::fmt::Debug for StgInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StgInfo")
            .field("initialized", &self.initialized)
            .field("size", &self.size)
            .field("align", &self.align)
            .field("length", &self.length)
            .field("proto", &self.proto)
            .field("flags", &self.flags)
            .field("element_type", &self.element_type)
            .field("element_size", &self.element_size)
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
            flags: 0,
            element_type: None,
            element_size: 0,
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
            flags: 0,
            element_type: None,
            element_size: 0,
        }
    }

    /// Create StgInfo for an array type
    pub fn new_array(
        size: usize,
        align: usize,
        length: usize,
        element_type: PyObjectRef,
        element_size: usize,
    ) -> Self {
        StgInfo {
            initialized: true,
            size,
            align,
            length,
            proto: None,
            flags: 0,
            element_type: Some(element_type),
            element_size,
        }
    }
}

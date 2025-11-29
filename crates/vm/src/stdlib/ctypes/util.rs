use crate::PyObjectRef;

/// Storage information for ctypes types
#[derive(Debug, Clone)]
pub struct StgInfo {
    #[allow(dead_code)]
    pub initialized: bool,
    pub size: usize,   // number of bytes
    pub align: usize,  // alignment requirements
    pub length: usize, // number of fields (for arrays/structures)
    #[allow(dead_code)]
    pub proto: Option<PyObjectRef>, // Only for Pointer/ArrayObject
    #[allow(dead_code)]
    pub flags: i32, // calling convention and such
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
        }
    }
}

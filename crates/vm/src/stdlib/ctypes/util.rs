use crate::PyObjectRef;

#[pyclass(name, module = "_ctypes")]
#[derive(Debug, PyPayload)]
pub struct StgInfo {
    pub initialized: i32,
    pub size: usize,   // number of bytes
    pub align: usize,  // alignment requirements
    pub length: usize, // number of fields
    // ffi_type_pointer: ffi::ffi_type,
    pub proto: PyObjectRef,           // Only for Pointer/ArrayObject
    pub setfunc: Option<PyObjectRef>, // Only for simple objects
    pub getfunc: Option<PyObjectRef>, // Only for simple objects
    pub paramfunc: Option<PyObjectRef>,

    /* Following fields only used by PyCFuncPtrType_Type instances */
    pub argtypes: Option<PyObjectRef>,   // tuple of CDataObjects
    pub converters: Option<PyObjectRef>, // tuple([t.from_param for t in argtypes])
    pub restype: Option<PyObjectRef>,    // CDataObject or NULL
    pub checker: Option<PyObjectRef>,
    pub module: Option<PyObjectRef>,
    pub flags: i32, // calling convention and such
    pub dict_final: u8,
}

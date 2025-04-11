#[pyclass]
#[derive(Debug, PyPayload)]
pub struct StgInfo {
    initialized: i32,
    size: usize,   // number of bytes
    align: usize,  // alignment requirements
    length: usize, // number of fields
    ffi_type_pointer: ffi::ffi_type,
    proto: PyObjectRef,           // Only for Pointer/ArrayObject
    setfunc: Option<PyObjectRef>, // Only for simple objects
    getfunc: Option<PyObjectRef>, // Only for simple objects
    paramfunc: Option<PyObjectRef>,

    /* Following fields only used by PyCFuncPtrType_Type instances */
    argtypes: Option<PyObjectRef>,   // tuple of CDataObjects
    converters: Option<PyObjectRef>, // tuple([t.from_param for t in argtypes])
    restype: Option<PyObjectRef>,    // CDataObject or NULL
    checker: Option<PyObjectRef>,
    module: Option<PyObjectRef>,
    flags: i32, // calling convention and such
    dict_final: u8,
}

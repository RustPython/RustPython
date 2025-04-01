//! Yes, really, this is not a typo.

// typedef struct {
//     PyObject_VAR_HEAD
//     ffi_closure *pcl_write; /* the C callable, writeable */
//     void *pcl_exec;         /* the C callable, executable */
//     ffi_cif cif;
//     int flags;
//     PyObject *converters;
//     PyObject *callable;
//     PyObject *restype;
//     SETFUNC setfunc;
//     ffi_type *ffi_restype;
//     ffi_type *atypes[1];
// } CThunkObject;

#[pyclass(name = "CThunkObject", module = "_ctypes")]
#[derive(Debug, PyPayload)]
pub struct PyCThunk {}

#[pyclass]
impl PyCThunk {}

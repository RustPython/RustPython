//! FFI callback (thunk) implementation for ctypes.
//!
//! This module implements CThunkObject which wraps Python callables
//! to be callable from C code via libffi closures.

use crate::builtins::{PyStr, PyType, PyTypeRef};
use crate::vm::thread::with_current_vm;
use crate::{PyObjectRef, PyPayload, PyResult, VirtualMachine};
use libffi::low;
use libffi::middle::{Cif, Closure, CodePtr, Type};
use num_traits::ToPrimitive;
use rustpython_common::lock::PyRwLock;
use std::ffi::c_void;
use std::fmt::Debug;

use super::base::ffi_type_from_str;
/// Userdata passed to the libffi callback.
/// This contains everything needed to invoke the Python callable.
pub struct ThunkUserData {
    /// The Python callable to invoke
    pub callable: PyObjectRef,
    /// Argument types for conversion
    pub arg_types: Vec<PyTypeRef>,
    /// Result type for conversion (None means void)
    pub res_type: Option<PyTypeRef>,
}

/// Get the type code string from a ctypes type
fn get_type_code(ty: &PyTypeRef, vm: &VirtualMachine) -> Option<String> {
    ty.get_attr(vm.ctx.intern_str("_type_"))
        .and_then(|t| t.downcast_ref::<PyStr>().map(|s| s.to_string()))
}

/// Convert a C value to a Python object based on the type code
fn ffi_to_python(ty: &PyTypeRef, ptr: *const c_void, vm: &VirtualMachine) -> PyObjectRef {
    let type_code = get_type_code(ty, vm);
    // SAFETY: ptr is guaranteed to be valid by libffi calling convention
    unsafe {
        match type_code.as_deref() {
            Some("b") => vm.ctx.new_int(*(ptr as *const i8) as i32).into(),
            Some("B") => vm.ctx.new_int(*(ptr as *const u8) as i32).into(),
            Some("c") => vm.ctx.new_bytes(vec![*(ptr as *const u8)]).into(),
            Some("h") => vm.ctx.new_int(*(ptr as *const i16) as i32).into(),
            Some("H") => vm.ctx.new_int(*(ptr as *const u16) as i32).into(),
            Some("i") => vm.ctx.new_int(*(ptr as *const i32)).into(),
            Some("I") => vm.ctx.new_int(*(ptr as *const u32)).into(),
            Some("l") => vm.ctx.new_int(*(ptr as *const libc::c_long)).into(),
            Some("L") => vm.ctx.new_int(*(ptr as *const libc::c_ulong)).into(),
            Some("q") => vm.ctx.new_int(*(ptr as *const libc::c_longlong)).into(),
            Some("Q") => vm.ctx.new_int(*(ptr as *const libc::c_ulonglong)).into(),
            Some("f") => vm.ctx.new_float(*(ptr as *const f32) as f64).into(),
            Some("d") => vm.ctx.new_float(*(ptr as *const f64)).into(),
            Some("P") | Some("z") | Some("Z") => vm.ctx.new_int(ptr as usize).into(),
            _ => vm.ctx.none(),
        }
    }
}

/// Convert a Python object to a C value and store it at the result pointer
fn python_to_ffi(obj: PyResult, ty: &PyTypeRef, result: *mut c_void, vm: &VirtualMachine) {
    let obj = match obj {
        Ok(o) => o,
        Err(_) => return, // Exception occurred, leave result as-is
    };

    let type_code = get_type_code(ty, vm);
    // SAFETY: result is guaranteed to be valid by libffi calling convention
    unsafe {
        match type_code.as_deref() {
            Some("b") => {
                if let Ok(i) = obj.try_int(vm) {
                    *(result as *mut i8) = i.as_bigint().to_i8().unwrap_or(0);
                }
            }
            Some("B") => {
                if let Ok(i) = obj.try_int(vm) {
                    *(result as *mut u8) = i.as_bigint().to_u8().unwrap_or(0);
                }
            }
            Some("c") => {
                if let Ok(i) = obj.try_int(vm) {
                    *(result as *mut u8) = i.as_bigint().to_u8().unwrap_or(0);
                }
            }
            Some("h") => {
                if let Ok(i) = obj.try_int(vm) {
                    *(result as *mut i16) = i.as_bigint().to_i16().unwrap_or(0);
                }
            }
            Some("H") => {
                if let Ok(i) = obj.try_int(vm) {
                    *(result as *mut u16) = i.as_bigint().to_u16().unwrap_or(0);
                }
            }
            Some("i") => {
                if let Ok(i) = obj.try_int(vm) {
                    *(result as *mut i32) = i.as_bigint().to_i32().unwrap_or(0);
                }
            }
            Some("I") => {
                if let Ok(i) = obj.try_int(vm) {
                    *(result as *mut u32) = i.as_bigint().to_u32().unwrap_or(0);
                }
            }
            Some("l") | Some("q") => {
                if let Ok(i) = obj.try_int(vm) {
                    *(result as *mut i64) = i.as_bigint().to_i64().unwrap_or(0);
                }
            }
            Some("L") | Some("Q") => {
                if let Ok(i) = obj.try_int(vm) {
                    *(result as *mut u64) = i.as_bigint().to_u64().unwrap_or(0);
                }
            }
            Some("f") => {
                if let Ok(f) = obj.try_float(vm) {
                    *(result as *mut f32) = f.to_f64() as f32;
                }
            }
            Some("d") => {
                if let Ok(f) = obj.try_float(vm) {
                    *(result as *mut f64) = f.to_f64();
                }
            }
            Some("P") | Some("z") | Some("Z") => {
                if let Ok(i) = obj.try_int(vm) {
                    *(result as *mut usize) = i.as_bigint().to_usize().unwrap_or(0);
                }
            }
            _ => {}
        }
    }
}

/// The callback function that libffi calls when the closure is invoked.
/// This function converts C arguments to Python objects, calls the Python
/// callable, and converts the result back to C.
unsafe extern "C" fn thunk_callback(
    _cif: &low::ffi_cif,
    result: &mut c_void,
    args: *const *const c_void,
    userdata: &ThunkUserData,
) {
    with_current_vm(|vm| {
        // Convert C arguments to Python objects
        let py_args: Vec<PyObjectRef> = userdata
            .arg_types
            .iter()
            .enumerate()
            .map(|(i, ty)| {
                let arg_ptr = unsafe { *args.add(i) };
                ffi_to_python(ty, arg_ptr, vm)
            })
            .collect();

        // Call the Python callable
        let py_result = userdata.callable.call(py_args, vm);

        // Convert result back to C type
        if let Some(ref res_type) = userdata.res_type {
            python_to_ffi(py_result, res_type, result as *mut c_void, vm);
        }
    });
}

/// Holds the closure and userdata together to ensure proper lifetime.
/// The userdata is leaked to create a 'static reference that the closure can use.
struct ThunkData {
    #[allow(dead_code)]
    closure: Closure<'static>,
    /// Raw pointer to the leaked userdata, for cleanup
    userdata_ptr: *mut ThunkUserData,
}

impl Drop for ThunkData {
    fn drop(&mut self) {
        // SAFETY: We created this with Box::into_raw, so we can reclaim it
        unsafe {
            drop(Box::from_raw(self.userdata_ptr));
        }
    }
}

/// CThunkObject wraps a Python callable to make it callable from C code.
#[pyclass(name = "CThunkObject", module = "_ctypes")]
#[derive(PyPayload)]
pub struct PyCThunk {
    /// The Python callable
    callable: PyObjectRef,
    /// The libffi closure (must be kept alive)
    #[allow(dead_code)]
    thunk_data: PyRwLock<Option<ThunkData>>,
    /// The code pointer for the closure
    code_ptr: CodePtr,
}

impl Debug for PyCThunk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PyCThunk")
            .field("callable", &self.callable)
            .finish()
    }
}

impl PyCThunk {
    /// Create a new thunk wrapping a Python callable.
    ///
    /// # Arguments
    /// * `callable` - The Python callable to wrap
    /// * `arg_types` - Optional sequence of argument types
    /// * `res_type` - Optional result type
    /// * `vm` - The virtual machine
    pub fn new(
        callable: PyObjectRef,
        arg_types: Option<PyObjectRef>,
        res_type: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<Self> {
        // Parse argument types
        let arg_type_vec: Vec<PyTypeRef> = if let Some(args) = arg_types {
            if vm.is_none(&args) {
                Vec::new()
            } else {
                let mut types = Vec::new();
                for item in args.try_to_value::<Vec<PyObjectRef>>(vm)? {
                    types.push(item.downcast::<PyType>().map_err(|_| {
                        vm.new_type_error("_argtypes_ must be a sequence of types".to_string())
                    })?);
                }
                types
            }
        } else {
            Vec::new()
        };

        // Parse result type
        let res_type_ref: Option<PyTypeRef> =
            if let Some(ref rt) = res_type {
                if vm.is_none(rt) {
                    None
                } else {
                    Some(rt.clone().downcast::<PyType>().map_err(|_| {
                        vm.new_type_error("restype must be a ctypes type".to_string())
                    })?)
                }
            } else {
                None
            };

        // Build FFI types
        let ffi_arg_types: Vec<Type> = arg_type_vec
            .iter()
            .map(|ty| {
                get_type_code(ty, vm)
                    .and_then(|code| ffi_type_from_str(&code))
                    .unwrap_or(Type::pointer())
            })
            .collect();

        let ffi_res_type = res_type_ref
            .as_ref()
            .and_then(|ty| get_type_code(ty, vm))
            .and_then(|code| ffi_type_from_str(&code))
            .unwrap_or(Type::void());

        // Create the CIF
        let cif = Cif::new(ffi_arg_types, ffi_res_type);

        // Create userdata and leak it to get a 'static reference
        let userdata = Box::new(ThunkUserData {
            callable: callable.clone(),
            arg_types: arg_type_vec,
            res_type: res_type_ref,
        });
        let userdata_ptr = Box::into_raw(userdata);

        // SAFETY: We maintain the userdata lifetime by storing it in ThunkData
        // and cleaning it up in Drop
        let userdata_ref: &'static ThunkUserData = unsafe { &*userdata_ptr };

        // Create the closure
        let closure = Closure::new(cif, thunk_callback, userdata_ref);

        // Get the code pointer
        let code_ptr = CodePtr(*closure.code_ptr() as *mut _);

        // Store closure and userdata together
        let thunk_data = ThunkData {
            closure,
            userdata_ptr,
        };

        Ok(Self {
            callable,
            thunk_data: PyRwLock::new(Some(thunk_data)),
            code_ptr,
        })
    }

    /// Get the code pointer for this thunk
    pub fn code_ptr(&self) -> CodePtr {
        self.code_ptr
    }
}

// SAFETY: PyCThunk is safe to send/sync because:
// - callable is a PyObjectRef which is Send+Sync
// - thunk_data contains the libffi closure which is heap-allocated
// - code_ptr is just a pointer to executable memory
unsafe impl Send for PyCThunk {}
unsafe impl Sync for PyCThunk {}

#[pyclass]
impl PyCThunk {
    #[pygetset]
    fn callable(&self) -> PyObjectRef {
        self.callable.clone()
    }
}

// spell-checker:disable

use super::{
    _ctypes::CArgObject,
    PyCArray, PyCData, PyCPointer, PyCStructure, StgInfo,
    base::{CDATA_BUFFER_METHODS, FfiArgValue, ParamFunc, StgInfoFlags},
    simple::PyCSimple,
    type_info,
};
use crate::{
    AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    builtins::{PyBytes, PyDict, PyNone, PyStr, PyTuple, PyType, PyTypeRef},
    class::StaticType,
    function::FuncArgs,
    protocol::{BufferDescriptor, PyBuffer},
    types::{AsBuffer, Callable, Constructor, Initializer, Representable},
    vm::thread::with_current_vm,
};
use alloc::borrow::Cow;
use core::ffi::c_void;
use core::fmt::Debug;
use libffi::{
    low,
    middle::{Arg, Cif, Closure, CodePtr, Type},
};
use libloading::Symbol;
use num_traits::{Signed, ToPrimitive};
use rustpython_common::lock::PyRwLock;

// Internal function addresses for special ctypes functions
pub(super) const INTERNAL_CAST_ADDR: usize = 1;
pub(super) const INTERNAL_STRING_AT_ADDR: usize = 2;
pub(super) const INTERNAL_WSTRING_AT_ADDR: usize = 3;

// Thread-local errno storage for ctypes
std::thread_local! {
    /// Thread-local storage for ctypes errno
    /// This is separate from the system errno - ctypes swaps them during FFI calls
    /// when use_errno=True is specified.
    static CTYPES_LOCAL_ERRNO: core::cell::Cell<i32> = const { core::cell::Cell::new(0) };
}

/// Get ctypes thread-local errno value
pub(super) fn get_errno_value() -> i32 {
    CTYPES_LOCAL_ERRNO.with(|e| e.get())
}

/// Set ctypes thread-local errno value, returns old value
pub(super) fn set_errno_value(value: i32) -> i32 {
    CTYPES_LOCAL_ERRNO.with(|e| {
        let old = e.get();
        e.set(value);
        old
    })
}

/// Save and restore errno around FFI call (called when use_errno=True)
/// Before: restore thread-local errno to system
/// After: save system errno to thread-local
#[cfg(not(windows))]
fn swap_errno<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    // Before call: restore thread-local errno to system
    let saved = CTYPES_LOCAL_ERRNO.with(|e| e.get());
    errno::set_errno(errno::Errno(saved));

    // Call the function
    let result = f();

    // After call: save system errno to thread-local
    let new_error = errno::errno().0;
    CTYPES_LOCAL_ERRNO.with(|e| e.set(new_error));

    result
}

#[cfg(windows)]
std::thread_local! {
    /// Thread-local storage for ctypes last_error (Windows only)
    static CTYPES_LOCAL_LAST_ERROR: core::cell::Cell<u32> = const { core::cell::Cell::new(0) };
}

#[cfg(windows)]
pub(super) fn get_last_error_value() -> u32 {
    CTYPES_LOCAL_LAST_ERROR.with(|e| e.get())
}

#[cfg(windows)]
pub(super) fn set_last_error_value(value: u32) -> u32 {
    CTYPES_LOCAL_LAST_ERROR.with(|e| {
        let old = e.get();
        e.set(value);
        old
    })
}

/// Save and restore last_error around FFI call (called when use_last_error=True)
#[cfg(windows)]
fn save_and_restore_last_error<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    // Before call: restore thread-local last_error to Windows
    let saved = CTYPES_LOCAL_LAST_ERROR.with(|e| e.get());
    unsafe { windows_sys::Win32::Foundation::SetLastError(saved) };

    // Call the function
    let result = f();

    // After call: save Windows last_error to thread-local
    let new_error = unsafe { windows_sys::Win32::Foundation::GetLastError() };
    CTYPES_LOCAL_LAST_ERROR.with(|e| e.set(new_error));

    result
}

type FP = unsafe extern "C" fn();

/// Get FFI type for a ctypes type code
fn get_ffi_type(ty: &str) -> Option<libffi::middle::Type> {
    type_info(ty).map(|t| (t.ffi_type_fn)())
}

// PyCFuncPtr - Function pointer implementation

/// Get FFI type from CArgObject tag character
fn ffi_type_from_tag(tag: u8) -> Type {
    match tag {
        b'c' | b'b' => Type::i8(),
        b'B' => Type::u8(),
        b'h' => Type::i16(),
        b'H' => Type::u16(),
        b'i' => Type::i32(),
        b'I' => Type::u32(),
        b'l' => {
            if core::mem::size_of::<libc::c_long>() == 8 {
                Type::i64()
            } else {
                Type::i32()
            }
        }
        b'L' => {
            if core::mem::size_of::<libc::c_ulong>() == 8 {
                Type::u64()
            } else {
                Type::u32()
            }
        }
        b'q' => Type::i64(),
        b'Q' => Type::u64(),
        b'f' => Type::f32(),
        b'd' | b'g' => Type::f64(),
        b'?' => Type::u8(),
        b'u' => {
            if core::mem::size_of::<super::WideChar>() == 2 {
                Type::u16()
            } else {
                Type::u32()
            }
        }
        _ => Type::pointer(), // 'P', 'V', 'z', 'Z', 'O', etc.
    }
}

/// Convert any object to a pointer value for c_void_p arguments
/// Follows ConvParam logic for pointer types
fn convert_to_pointer(value: &PyObject, vm: &VirtualMachine) -> PyResult<FfiArgValue> {
    // 0. CArgObject (from byref()) -> buffer address + offset
    if let Some(carg) = value.downcast_ref::<CArgObject>() {
        // Get buffer address from the underlying object
        let base_addr = if let Some(cdata) = carg.obj.downcast_ref::<PyCData>() {
            cdata.buffer.read().as_ptr() as usize
        } else {
            return Err(vm.new_type_error(format!(
                "byref() argument must be a ctypes instance, not '{}'",
                carg.obj.class().name()
            )));
        };
        let addr = (base_addr as isize + carg.offset) as usize;
        return Ok(FfiArgValue::Pointer(addr));
    }

    // 1. None -> NULL
    if value.is(&vm.ctx.none) {
        return Ok(FfiArgValue::Pointer(0));
    }

    // 2. PyCArray -> buffer address (PyCArrayType_paramfunc)
    if let Some(array) = value.downcast_ref::<PyCArray>() {
        let addr = array.0.buffer.read().as_ptr() as usize;
        return Ok(FfiArgValue::Pointer(addr));
    }

    // 3. PyCPointer -> stored pointer value
    if let Some(ptr) = value.downcast_ref::<PyCPointer>() {
        return Ok(FfiArgValue::Pointer(ptr.get_ptr_value()));
    }

    // 4. PyCStructure -> buffer address
    if let Some(struct_obj) = value.downcast_ref::<PyCStructure>() {
        let addr = struct_obj.0.buffer.read().as_ptr() as usize;
        return Ok(FfiArgValue::Pointer(addr));
    }

    // 5. PyCSimple (c_void_p, c_char_p, etc.) -> value from buffer
    if let Some(simple) = value.downcast_ref::<PyCSimple>() {
        let buffer = simple.0.buffer.read();
        if buffer.len() >= core::mem::size_of::<usize>() {
            let addr = super::base::read_ptr_from_buffer(&buffer);
            return Ok(FfiArgValue::Pointer(addr));
        }
    }

    // 6. bytes -> buffer address (PyBytes_AsString)
    if let Some(bytes) = value.downcast_ref::<crate::builtins::PyBytes>() {
        let addr = bytes.as_bytes().as_ptr() as usize;
        return Ok(FfiArgValue::Pointer(addr));
    }

    // 7. Integer -> direct value (PyLong_AsVoidPtr behavior)
    if let Ok(int_val) = value.try_int(vm) {
        let bigint = int_val.as_bigint();
        // Negative values: use signed conversion (allows -1 as 0xFFFF...)
        if bigint.is_negative() {
            if let Some(signed_val) = bigint.to_isize() {
                return Ok(FfiArgValue::Pointer(signed_val as usize));
            }
        } else if let Some(unsigned_val) = bigint.to_usize() {
            return Ok(FfiArgValue::Pointer(unsigned_val));
        }
        // Value out of range - raise OverflowError
        return Err(vm.new_overflow_error("int too large to convert to pointer".to_string()));
    }

    // 8. Check _as_parameter_ attribute ( recursive ConvParam)
    if let Ok(as_param) = value.get_attr("_as_parameter_", vm) {
        return convert_to_pointer(&as_param, vm);
    }

    Err(vm.new_type_error(format!(
        "cannot convert '{}' to c_void_p",
        value.class().name()
    )))
}

/// ConvParam-like conversion for when argtypes is None
/// Returns an Argument with FFI type, value, and optional keep object
fn conv_param(value: &PyObject, vm: &VirtualMachine) -> PyResult<Argument> {
    // 1. CArgObject (from byref() or paramfunc) -> use stored type and value
    if let Some(carg) = value.downcast_ref::<CArgObject>() {
        let ffi_type = ffi_type_from_tag(carg.tag);
        return Ok(Argument {
            ffi_type,
            keep: None,
            value: carg.value.clone(),
        });
    }

    // 2. None -> NULL pointer
    if value.is(&vm.ctx.none) {
        return Ok(Argument {
            ffi_type: Type::pointer(),
            keep: None,
            value: FfiArgValue::Pointer(0),
        });
    }

    // 3. ctypes objects -> use paramfunc
    if let Ok(carg) = super::base::call_paramfunc(value, vm) {
        let ffi_type = ffi_type_from_tag(carg.tag);
        return Ok(Argument {
            ffi_type,
            keep: None,
            value: carg.value.clone(),
        });
    }

    // 4. Python str -> wide string pointer (like PyUnicode_AsWideCharString)
    if let Some(s) = value.downcast_ref::<PyStr>() {
        // Convert to null-terminated UTF-16 (wide string)
        let wide: Vec<u16> = s
            .as_str()
            .encode_utf16()
            .chain(core::iter::once(0))
            .collect();
        let wide_bytes: Vec<u8> = wide.iter().flat_map(|&x| x.to_ne_bytes()).collect();
        let keep = vm.ctx.new_bytes(wide_bytes);
        let addr = keep.as_bytes().as_ptr() as usize;
        return Ok(Argument {
            ffi_type: Type::pointer(),
            keep: Some(keep.into()),
            value: FfiArgValue::Pointer(addr),
        });
    }

    // 9. Python bytes -> null-terminated buffer pointer
    // Need to ensure null termination like c_char_p
    if let Some(bytes) = value.downcast_ref::<PyBytes>() {
        let mut buffer = bytes.as_bytes().to_vec();
        buffer.push(0); // Add null terminator
        let keep = vm.ctx.new_bytes(buffer);
        let addr = keep.as_bytes().as_ptr() as usize;
        return Ok(Argument {
            ffi_type: Type::pointer(),
            keep: Some(keep.into()),
            value: FfiArgValue::Pointer(addr),
        });
    }

    // 10. Python int -> i32 (default integer type)
    if let Ok(int_val) = value.try_int(vm) {
        let val = int_val.as_bigint().to_i32().unwrap_or(0);
        return Ok(Argument {
            ffi_type: Type::i32(),
            keep: None,
            value: FfiArgValue::I32(val),
        });
    }

    // 11. Python float -> f64
    if let Ok(float_val) = value.try_float(vm) {
        return Ok(Argument {
            ffi_type: Type::f64(),
            keep: None,
            value: FfiArgValue::F64(float_val.to_f64()),
        });
    }

    // 12. Check _as_parameter_ attribute
    if let Ok(as_param) = value.get_attr("_as_parameter_", vm) {
        return conv_param(&as_param, vm);
    }

    Err(vm.new_type_error(format!(
        "Don't know how to convert parameter {}",
        value.class().name()
    )))
}

trait ArgumentType {
    fn to_ffi_type(&self, vm: &VirtualMachine) -> PyResult<Type>;
    fn convert_object(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<FfiArgValue>;
}

impl ArgumentType for PyTypeRef {
    fn to_ffi_type(&self, vm: &VirtualMachine) -> PyResult<Type> {
        use super::pointer::PyCPointer;
        use super::structure::PyCStructure;

        // CArgObject (from byref()) should be treated as pointer
        if self.fast_issubclass(CArgObject::static_type()) {
            return Ok(Type::pointer());
        }

        // Pointer types (POINTER(T)) are always pointer FFI type
        // Check if type is a subclass of _Pointer (PyCPointer)
        if self.fast_issubclass(PyCPointer::static_type()) {
            return Ok(Type::pointer());
        }

        // Structure types are passed as pointers
        if self.fast_issubclass(PyCStructure::static_type()) {
            return Ok(Type::pointer());
        }

        // Use get_attr to traverse MRO (for subclasses like MyInt(c_int))
        let typ = self
            .as_object()
            .get_attr(vm.ctx.intern_str("_type_"), vm)
            .ok()
            .ok_or(vm.new_type_error("Unsupported argument type"))?;
        let typ = typ
            .downcast_ref::<PyStr>()
            .ok_or(vm.new_type_error("Unsupported argument type"))?;
        let typ = typ.to_string();
        let typ = typ.as_str();
        get_ffi_type(typ)
            .ok_or_else(|| vm.new_type_error(format!("Unsupported argument type: {}", typ)))
    }

    fn convert_object(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<FfiArgValue> {
        // Call from_param first to convert the value
        // converter = PyTuple_GET_ITEM(argtypes, i);
        // v = PyObject_CallOneArg(converter, arg);
        let from_param = self
            .as_object()
            .get_attr(vm.ctx.intern_str("from_param"), vm)?;
        let converted = from_param.call((value.clone(),), vm)?;

        // Then pass the converted value to ConvParam logic
        // CArgObject (from from_param) -> use stored value directly
        if let Some(carg) = converted.downcast_ref::<CArgObject>() {
            return Ok(carg.value.clone());
        }

        // None -> NULL pointer
        if vm.is_none(&converted) {
            return Ok(FfiArgValue::Pointer(0));
        }

        // For pointer types (POINTER(T)), we need to pass the pointer VALUE stored in buffer
        if self.fast_issubclass(PyCPointer::static_type()) {
            if let Some(pointer) = converted.downcast_ref::<PyCPointer>() {
                return Ok(FfiArgValue::Pointer(pointer.get_ptr_value()));
            }
            return convert_to_pointer(&converted, vm);
        }

        // For structure types, convert to pointer to structure
        if self.fast_issubclass(PyCStructure::static_type()) {
            return convert_to_pointer(&converted, vm);
        }

        // Get the type code for this argument type
        let type_code = self
            .as_object()
            .get_attr(vm.ctx.intern_str("_type_"), vm)
            .ok()
            .and_then(|t| t.downcast_ref::<PyStr>().map(|s| s.to_string()));

        // For pointer types (c_void_p, c_char_p, c_wchar_p), handle as pointer
        if matches!(type_code.as_deref(), Some("P") | Some("z") | Some("Z")) {
            return convert_to_pointer(&converted, vm);
        }

        // PyCSimple (already a ctypes instance from from_param)
        if let Ok(simple) = converted.clone().downcast::<PyCSimple>() {
            let typ = ArgumentType::to_ffi_type(self, vm)?;
            let ffi_value = simple
                .to_ffi_value(typ, vm)
                .ok_or(vm.new_type_error("Unsupported argument type"))?;
            return Ok(ffi_value);
        }

        Err(vm.new_type_error("Unsupported argument type"))
    }
}

trait ReturnType {
    fn to_ffi_type(&self, vm: &VirtualMachine) -> Option<Type>;
}

impl ReturnType for PyTypeRef {
    fn to_ffi_type(&self, vm: &VirtualMachine) -> Option<Type> {
        // Try to get _type_ attribute first (for ctypes types like c_void_p)
        if let Ok(type_attr) = self.as_object().get_attr(vm.ctx.intern_str("_type_"), vm)
            && let Some(s) = type_attr.downcast_ref::<PyStr>()
            && let Some(ffi_type) = get_ffi_type(s.as_str())
        {
            return Some(ffi_type);
        }

        // Check for Structure/Array types (have StgInfo but no _type_)
        // _ctypes_get_ffi_type: returns appropriately sized type for struct returns
        if let Some(stg_info) = self.stg_info_opt() {
            let size = stg_info.size;
            // Small structs can be returned in registers
            // Match can_return_struct_as_int/can_return_struct_as_sint64
            return Some(if size <= 4 {
                Type::i32()
            } else if size <= 8 {
                Type::i64()
            } else {
                // Large structs: use pointer-sized return
                // (ABI typically returns via hidden pointer parameter)
                Type::pointer()
            });
        }

        // Fallback to class name
        get_ffi_type(self.name().to_string().as_str())
    }
}

impl ReturnType for PyNone {
    fn to_ffi_type(&self, _vm: &VirtualMachine) -> Option<Type> {
        get_ffi_type("void")
    }
}

// PyCFuncPtrType - Metaclass for function pointer types
// PyCFuncPtrType_init

#[pyclass(name = "PyCFuncPtrType", base = PyType, module = "_ctypes")]
#[derive(Debug)]
#[repr(transparent)]
pub(super) struct PyCFuncPtrType(PyType);

impl Initializer for PyCFuncPtrType {
    type Args = FuncArgs;

    fn init(zelf: PyRef<Self>, _args: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
        let obj: PyObjectRef = zelf.clone().into();
        let new_type: PyTypeRef = obj
            .downcast()
            .map_err(|_| vm.new_type_error("expected type"))?;

        new_type.check_not_initialized(vm)?;

        let ptr_size = core::mem::size_of::<usize>();
        let mut stg_info = StgInfo::new(ptr_size, ptr_size);
        stg_info.format = Some("X{}".to_string());
        stg_info.length = 1;
        stg_info.flags |= StgInfoFlags::TYPEFLAG_ISPOINTER;
        stg_info.paramfunc = ParamFunc::Pointer; // CFuncPtr is passed as a pointer

        let _ = new_type.init_type_data(stg_info);
        Ok(())
    }
}

#[pyclass(flags(IMMUTABLETYPE), with(Initializer))]
impl PyCFuncPtrType {}

/// PyCFuncPtr - Function pointer instance
/// Saved in _base.buffer
#[pyclass(
    module = "_ctypes",
    name = "CFuncPtr",
    base = PyCData,
    metaclass = "PyCFuncPtrType"
)]
#[repr(C)]
pub(super) struct PyCFuncPtr {
    pub _base: PyCData,
    /// Thunk for callbacks (keeps thunk alive)
    pub thunk: PyRwLock<Option<PyRef<PyCThunk>>>,
    /// Original Python callable (for callbacks)
    pub callable: PyRwLock<Option<PyObjectRef>>,
    /// Converters cache
    pub converters: PyRwLock<Option<PyObjectRef>>,
    /// Instance-level argtypes override
    pub argtypes: PyRwLock<Option<PyObjectRef>>,
    /// Instance-level restype override
    pub restype: PyRwLock<Option<PyObjectRef>>,
    /// Checker function
    pub checker: PyRwLock<Option<PyObjectRef>>,
    /// Error checking function
    pub errcheck: PyRwLock<Option<PyObjectRef>>,
    /// COM method vtable index
    /// When set, the function reads the function pointer from the vtable at call time
    #[cfg(windows)]
    pub index: PyRwLock<Option<usize>>,
    /// COM method IID (interface ID) for error handling
    #[cfg(windows)]
    pub iid: PyRwLock<Option<PyObjectRef>>,
    /// Parameter flags for COM methods (direction: IN=1, OUT=2, IN|OUT=4)
    /// Each element is (direction, name, default) tuple
    pub paramflags: PyRwLock<Option<PyObjectRef>>,
}

impl Debug for PyCFuncPtr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PyCFuncPtr")
            .field("func_ptr", &self.get_func_ptr())
            .finish()
    }
}

/// Extract pointer value from a ctypes argument (c_void_p conversion)
fn extract_ptr_from_arg(arg: &PyObject, vm: &VirtualMachine) -> PyResult<usize> {
    // Try to get pointer value from various ctypes types
    if let Some(ptr) = arg.downcast_ref::<PyCPointer>() {
        return Ok(ptr.get_ptr_value());
    }
    if let Some(simple) = arg.downcast_ref::<PyCSimple>() {
        let buffer = simple.0.buffer.read();
        if buffer.len() >= core::mem::size_of::<usize>() {
            return Ok(usize::from_ne_bytes(
                buffer[..core::mem::size_of::<usize>()].try_into().unwrap(),
            ));
        }
    }
    if let Some(cdata) = arg.downcast_ref::<PyCData>() {
        // For arrays/structures, return address of buffer
        return Ok(cdata.buffer.read().as_ptr() as usize);
    }
    // PyStr: return internal buffer address
    if let Some(s) = arg.downcast_ref::<PyStr>() {
        return Ok(s.as_str().as_ptr() as usize);
    }
    // PyBytes: return internal buffer address
    if let Some(bytes) = arg.downcast_ref::<PyBytes>() {
        return Ok(bytes.as_bytes().as_ptr() as usize);
    }
    // Try as integer
    if let Ok(int_val) = arg.try_int(vm) {
        return Ok(int_val.as_bigint().to_usize().unwrap_or(0));
    }
    Err(vm.new_type_error(format!(
        "cannot convert '{}' to pointer",
        arg.class().name()
    )))
}

/// string_at implementation - read bytes from memory at ptr
fn string_at_impl(ptr: usize, size: isize, vm: &VirtualMachine) -> PyResult {
    if ptr == 0 {
        return Err(vm.new_value_error("NULL pointer access"));
    }
    let ptr = ptr as *const u8;
    let len = if size < 0 {
        // size == -1 means use strlen
        unsafe { libc::strlen(ptr as _) }
    } else {
        // Overflow check for huge size values
        let size_usize = size as usize;
        if size_usize > isize::MAX as usize / 2 {
            return Err(vm.new_overflow_error("string too long"));
        }
        size_usize
    };
    let bytes = unsafe { core::slice::from_raw_parts(ptr, len) };
    Ok(vm.ctx.new_bytes(bytes.to_vec()).into())
}

/// wstring_at implementation - read wide string from memory at ptr
fn wstring_at_impl(ptr: usize, size: isize, vm: &VirtualMachine) -> PyResult {
    if ptr == 0 {
        return Err(vm.new_value_error("NULL pointer access"));
    }
    let w_ptr = ptr as *const libc::wchar_t;
    let len = if size < 0 {
        unsafe { libc::wcslen(w_ptr) }
    } else {
        // Overflow check for huge size values
        let size_usize = size as usize;
        if size_usize > isize::MAX as usize / core::mem::size_of::<libc::wchar_t>() {
            return Err(vm.new_overflow_error("string too long"));
        }
        size_usize
    };
    let wchars = unsafe { core::slice::from_raw_parts(w_ptr, len) };

    // Windows: wchar_t = u16 (UTF-16) -> use Wtf8Buf::from_wide
    // macOS/Linux: wchar_t = i32 (UTF-32) -> convert via char::from_u32
    #[cfg(windows)]
    {
        use rustpython_common::wtf8::Wtf8Buf;
        let wide: Vec<u16> = wchars.to_vec();
        let wtf8 = Wtf8Buf::from_wide(&wide);
        Ok(vm.ctx.new_str(wtf8).into())
    }
    #[cfg(not(windows))]
    {
        let s: String = wchars
            .iter()
            .filter_map(|&c| char::from_u32(c as u32))
            .collect();
        Ok(vm.ctx.new_str(s).into())
    }
}

// cast_check_pointertype
fn cast_check_pointertype(ctype: &PyObject, vm: &VirtualMachine) -> bool {
    use super::pointer::PyCPointerType;

    // PyCPointerTypeObject_Check
    if ctype.class().fast_issubclass(PyCPointerType::static_type()) {
        return true;
    }

    // PyCFuncPtrTypeObject_Check - TODO

    // simple pointer types via StgInfo.proto (c_void_p, c_char_p, etc.)
    if let Ok(type_attr) = ctype.get_attr("_type_", vm)
        && let Some(s) = type_attr.downcast_ref::<PyStr>()
    {
        let c = s.as_str();
        if c.len() == 1 && "sPzUZXO".contains(c) {
            return true;
        }
    }

    false
}

/// cast implementation
/// _ctypes.c cast()
pub(super) fn cast_impl(
    obj: PyObjectRef,
    src: PyObjectRef,
    ctype: PyObjectRef,
    vm: &VirtualMachine,
) -> PyResult {
    // 1. cast_check_pointertype
    if !cast_check_pointertype(&ctype, vm) {
        return Err(vm.new_type_error(format!(
            "cast() argument 2 must be a pointer type, not {}",
            ctype.class().name()
        )));
    }

    // 2. Extract pointer value - matches c_void_p_from_param_impl order
    let ptr_value: usize = if vm.is_none(&obj) {
        // None → NULL pointer
        0
    } else if let Ok(int_val) = obj.try_int(vm) {
        // int/long → direct pointer value
        int_val.as_bigint().to_usize().unwrap_or(0)
    } else if let Some(bytes) = obj.downcast_ref::<PyBytes>() {
        // bytes → buffer address (c_void_p_from_param: PyBytes_Check)
        bytes.as_bytes().as_ptr() as usize
    } else if let Some(s) = obj.downcast_ref::<PyStr>() {
        // unicode/str → buffer address (c_void_p_from_param: PyUnicode_Check)
        s.as_str().as_ptr() as usize
    } else if let Some(ptr) = obj.downcast_ref::<PyCPointer>() {
        // Pointer instance → contained pointer value
        ptr.get_ptr_value()
    } else if let Some(simple) = obj.downcast_ref::<PyCSimple>() {
        // Simple type (c_void_p, c_char_p, etc.) → value from buffer
        let buffer = simple.0.buffer.read();
        super::base::read_ptr_from_buffer(&buffer)
    } else if let Some(cdata) = obj.downcast_ref::<PyCData>() {
        // Array, Structure, Union → buffer address (b_ptr)
        cdata.buffer.read().as_ptr() as usize
    } else {
        return Err(vm.new_type_error(format!(
            "cast() argument 1 must be a ctypes instance, not {}",
            obj.class().name()
        )));
    };

    // 3. Create result instance
    let result = ctype.call((), vm)?;

    // 4. _objects reference tracking
    // Share _objects dict between source and result, add id(src): src
    if src.class().fast_issubclass(PyCData::static_type()) {
        // Get the source's _objects, create dict if needed
        let shared_objects: PyObjectRef = if let Some(src_cdata) = src.downcast_ref::<PyCData>() {
            let mut src_objects = src_cdata.objects.write();
            if src_objects.is_none() {
                // Create new dict
                let dict = vm.ctx.new_dict();
                *src_objects = Some(dict.clone().into());
                dict.into()
            } else if let Some(obj) = src_objects.as_ref() {
                if obj.downcast_ref::<PyDict>().is_none() {
                    // Convert to dict (keep existing reference)
                    let dict = vm.ctx.new_dict();
                    let id_key: PyObjectRef = vm.ctx.new_int(obj.get_id() as i64).into();
                    let _ = dict.set_item(&*id_key, obj.clone(), vm);
                    *src_objects = Some(dict.clone().into());
                    dict.into()
                } else {
                    obj.clone()
                }
            } else {
                vm.ctx.new_dict().into()
            }
        } else {
            vm.ctx.new_dict().into()
        };

        // Add id(src): src to the shared dict
        if let Some(dict) = shared_objects.downcast_ref::<PyDict>() {
            let id_key: PyObjectRef = vm.ctx.new_int(src.get_id() as i64).into();
            let _ = dict.set_item(&*id_key, src.clone(), vm);
        }

        // Set result's _objects to the shared dict
        if let Some(result_cdata) = result.downcast_ref::<PyCData>() {
            *result_cdata.objects.write() = Some(shared_objects);
        }
    }

    // 5. Store pointer value
    if let Some(ptr) = result.downcast_ref::<PyCPointer>() {
        ptr.set_ptr_value(ptr_value);
    } else if let Some(cdata) = result.downcast_ref::<PyCData>() {
        let bytes = ptr_value.to_ne_bytes();
        let mut buffer = cdata.buffer.write();
        let buf = buffer.to_mut();
        if buf.len() >= bytes.len() {
            buf[..bytes.len()].copy_from_slice(&bytes);
        }
    }

    Ok(result)
}

impl PyCFuncPtr {
    /// Get function pointer address from buffer
    fn get_func_ptr(&self) -> usize {
        let buffer = self._base.buffer.read();
        super::base::read_ptr_from_buffer(&buffer)
    }

    /// Get CodePtr from buffer for FFI calls
    fn get_code_ptr(&self) -> Option<CodePtr> {
        let addr = self.get_func_ptr();
        if addr != 0 {
            Some(CodePtr(addr as *mut _))
        } else {
            None
        }
    }

    /// Create buffer with function pointer address
    fn make_ptr_buffer(addr: usize) -> Vec<u8> {
        addr.to_ne_bytes().to_vec()
    }
}

impl Constructor for PyCFuncPtr {
    type Args = FuncArgs;

    fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        // Handle different argument forms:
        // 1. Empty args: create uninitialized (NULL pointer)
        // 2. One integer argument: function address
        // 3. Tuple argument: (name, dll) form
        // 4. Callable: callback creation

        let ptr_size = core::mem::size_of::<usize>();

        if args.args.is_empty() {
            return PyCFuncPtr {
                _base: PyCData::from_bytes(vec![0u8; ptr_size], None),
                thunk: PyRwLock::new(None),
                callable: PyRwLock::new(None),
                converters: PyRwLock::new(None),
                argtypes: PyRwLock::new(None),
                restype: PyRwLock::new(None),
                checker: PyRwLock::new(None),
                errcheck: PyRwLock::new(None),
                #[cfg(windows)]
                index: PyRwLock::new(None),
                #[cfg(windows)]
                iid: PyRwLock::new(None),
                paramflags: PyRwLock::new(None),
            }
            .into_ref_with_type(vm, cls)
            .map(Into::into);
        }

        let first_arg = &args.args[0];

        // Check for COM method form: (index, name, [paramflags], [iid])
        // First arg is integer (vtable index), second arg is string (method name)
        if args.args.len() >= 2
            && first_arg.try_int(vm).is_ok()
            && args.args[1].downcast_ref::<PyStr>().is_some()
        {
            #[cfg(windows)]
            let index = first_arg.try_int(vm)?.as_bigint().to_usize().unwrap_or(0);

            // args[3] is iid (GUID struct, optional)
            // Also check if args[2] is a GUID (has Data1 attribute) when args[3] is not present
            #[cfg(windows)]
            let iid = args.args.get(3).cloned().or_else(|| {
                args.args.get(2).and_then(|arg| {
                    // If it's a GUID struct (has Data1 attribute), use it as IID
                    if arg.get_attr("Data1", vm).is_ok() {
                        Some(arg.clone())
                    } else {
                        None
                    }
                })
            });

            // args[2] is paramflags (tuple or None)
            let paramflags = args.args.get(2).filter(|arg| !vm.is_none(arg)).cloned();

            return PyCFuncPtr {
                _base: PyCData::from_bytes(vec![0u8; ptr_size], None),
                thunk: PyRwLock::new(None),
                callable: PyRwLock::new(None),
                converters: PyRwLock::new(None),
                argtypes: PyRwLock::new(None),
                restype: PyRwLock::new(None),
                checker: PyRwLock::new(None),
                errcheck: PyRwLock::new(None),
                #[cfg(windows)]
                index: PyRwLock::new(Some(index)),
                #[cfg(windows)]
                iid: PyRwLock::new(iid),
                paramflags: PyRwLock::new(paramflags),
            }
            .into_ref_with_type(vm, cls)
            .map(Into::into);
        }

        // Check if first argument is an integer (function address)
        if let Ok(addr) = first_arg.try_int(vm) {
            let ptr_val = addr.as_bigint().to_usize().unwrap_or(0);
            return PyCFuncPtr {
                _base: PyCData::from_bytes(Self::make_ptr_buffer(ptr_val), None),
                thunk: PyRwLock::new(None),
                callable: PyRwLock::new(None),
                converters: PyRwLock::new(None),
                argtypes: PyRwLock::new(None),
                restype: PyRwLock::new(None),
                checker: PyRwLock::new(None),
                errcheck: PyRwLock::new(None),
                #[cfg(windows)]
                index: PyRwLock::new(None),
                #[cfg(windows)]
                iid: PyRwLock::new(None),
                paramflags: PyRwLock::new(None),
            }
            .into_ref_with_type(vm, cls)
            .map(Into::into);
        }

        // Check if first argument is a tuple (name, dll) form
        if let Some(tuple) = first_arg.downcast_ref::<PyTuple>() {
            let name = tuple
                .first()
                .ok_or(vm.new_type_error("Expected a tuple with at least 2 elements"))?
                .downcast_ref::<PyStr>()
                .ok_or(vm.new_type_error("Expected a string"))?
                .to_string();
            let dll = tuple
                .iter()
                .nth(1)
                .ok_or(vm.new_type_error("Expected a tuple with at least 2 elements"))?
                .clone();

            // Get library handle and load function
            let handle = dll.try_int(vm);
            let handle = match handle {
                Ok(handle) => handle.as_bigint().clone(),
                Err(_) => dll
                    .get_attr("_handle", vm)?
                    .try_int(vm)?
                    .as_bigint()
                    .clone(),
            };
            let library_cache = super::library::libcache().read();
            let library = library_cache
                .get_lib(
                    handle
                        .to_usize()
                        .ok_or(vm.new_value_error("Invalid handle"))?,
                )
                .ok_or_else(|| vm.new_value_error("Library not found"))?;
            let inner_lib = library.lib.lock();

            let terminated = format!("{}\0", &name);
            let ptr_val = if let Some(lib) = &*inner_lib {
                let pointer: Symbol<'_, FP> = unsafe {
                    lib.get(terminated.as_bytes())
                        .map_err(|err| err.to_string())
                        .map_err(|err| vm.new_attribute_error(err))?
                };
                let addr = *pointer as usize;
                // dlsym can return NULL for symbols that resolve to NULL (e.g., GNU IFUNC)
                // Treat NULL addresses as errors
                if addr == 0 {
                    return Err(vm.new_attribute_error(format!("function '{}' not found", name)));
                }
                addr
            } else {
                0
            };

            return PyCFuncPtr {
                _base: PyCData::from_bytes(Self::make_ptr_buffer(ptr_val), None),
                thunk: PyRwLock::new(None),
                callable: PyRwLock::new(None),
                converters: PyRwLock::new(None),
                argtypes: PyRwLock::new(None),
                restype: PyRwLock::new(None),
                checker: PyRwLock::new(None),
                errcheck: PyRwLock::new(None),
                #[cfg(windows)]
                index: PyRwLock::new(None),
                #[cfg(windows)]
                iid: PyRwLock::new(None),
                paramflags: PyRwLock::new(None),
            }
            .into_ref_with_type(vm, cls)
            .map(Into::into);
        }

        // Check if first argument is a Python callable (callback creation)
        if first_arg.is_callable() {
            // Get argument types and result type from the class
            let class_argtypes = cls.get_attr(vm.ctx.intern_str("_argtypes_"));
            let class_restype = cls.get_attr(vm.ctx.intern_str("_restype_"));
            let class_flags = cls
                .get_attr(vm.ctx.intern_str("_flags_"))
                .and_then(|f| f.try_to_value::<u32>(vm).ok())
                .unwrap_or(0);

            // Create the thunk (C-callable wrapper for the Python function)
            let thunk = PyCThunk::new(
                first_arg.clone(),
                class_argtypes.clone(),
                class_restype.clone(),
                class_flags,
                vm,
            )?;
            let code_ptr = thunk.code_ptr();
            let ptr_val = code_ptr.0 as usize;

            // Store the thunk as a Python object to keep it alive
            let thunk_ref: PyRef<PyCThunk> = thunk.into_ref(&vm.ctx);

            return PyCFuncPtr {
                _base: PyCData::from_bytes(Self::make_ptr_buffer(ptr_val), None),
                thunk: PyRwLock::new(Some(thunk_ref)),
                callable: PyRwLock::new(Some(first_arg.clone())),
                converters: PyRwLock::new(None),
                argtypes: PyRwLock::new(class_argtypes),
                restype: PyRwLock::new(class_restype),
                checker: PyRwLock::new(None),
                errcheck: PyRwLock::new(None),
                #[cfg(windows)]
                index: PyRwLock::new(None),
                #[cfg(windows)]
                iid: PyRwLock::new(None),
                paramflags: PyRwLock::new(None),
            }
            .into_ref_with_type(vm, cls)
            .map(Into::into);
        }

        Err(vm.new_type_error("Expected an integer address or a tuple"))
    }

    fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
        unimplemented!("use slot_new")
    }
}

// PyCFuncPtr call helpers (similar to callproc.c flow)

/// Handle internal function addresses (PYFUNCTYPE special cases)
/// Returns Some(result) if handled, None if should continue with normal call
fn handle_internal_func(addr: usize, args: &FuncArgs, vm: &VirtualMachine) -> Option<PyResult> {
    if addr == INTERNAL_CAST_ADDR {
        let result: PyResult<(PyObjectRef, PyObjectRef, PyObjectRef)> = args.clone().bind(vm);
        return Some(result.and_then(|(obj, src, ctype)| cast_impl(obj, src, ctype, vm)));
    }

    if addr == INTERNAL_STRING_AT_ADDR {
        let result: PyResult<(PyObjectRef, Option<PyObjectRef>)> = args.clone().bind(vm);
        return Some(result.and_then(|(ptr_arg, size_arg)| {
            let ptr = extract_ptr_from_arg(&ptr_arg, vm)?;
            let size = size_arg
                .and_then(|s| s.try_int(vm).ok())
                .and_then(|i| i.as_bigint().to_isize())
                .unwrap_or(-1);
            string_at_impl(ptr, size, vm)
        }));
    }

    if addr == INTERNAL_WSTRING_AT_ADDR {
        let result: PyResult<(PyObjectRef, Option<PyObjectRef>)> = args.clone().bind(vm);
        return Some(result.and_then(|(ptr_arg, size_arg)| {
            let ptr = extract_ptr_from_arg(&ptr_arg, vm)?;
            let size = size_arg
                .and_then(|s| s.try_int(vm).ok())
                .and_then(|i| i.as_bigint().to_isize())
                .unwrap_or(-1);
            wstring_at_impl(ptr, size, vm)
        }));
    }

    None
}

/// Call information extracted from PyCFuncPtr (argtypes, restype, etc.)
struct CallInfo {
    explicit_arg_types: Option<Vec<PyTypeRef>>,
    restype_obj: Option<PyObjectRef>,
    restype_is_none: bool,
    ffi_return_type: Type,
    is_pointer_return: bool,
}

/// Extract call information (argtypes, restype) from PyCFuncPtr
fn extract_call_info(zelf: &Py<PyCFuncPtr>, vm: &VirtualMachine) -> PyResult<CallInfo> {
    // Get argtypes - first from instance, then from type's _argtypes_
    let explicit_arg_types: Option<Vec<PyTypeRef>> =
        if let Some(argtypes_obj) = zelf.argtypes.read().as_ref() {
            if !vm.is_none(argtypes_obj) {
                Some(
                    argtypes_obj
                        .try_to_value::<Vec<PyObjectRef>>(vm)?
                        .into_iter()
                        .filter_map(|obj| obj.downcast::<PyType>().ok())
                        .collect(),
                )
            } else {
                None // argtypes is None -> use ConvParam
            }
        } else if let Some(class_argtypes) = zelf
            .as_object()
            .class()
            .get_attr(vm.ctx.intern_str("_argtypes_"))
            && !vm.is_none(&class_argtypes)
        {
            Some(
                class_argtypes
                    .try_to_value::<Vec<PyObjectRef>>(vm)?
                    .into_iter()
                    .filter_map(|obj| obj.downcast::<PyType>().ok())
                    .collect(),
            )
        } else {
            None // No argtypes -> use ConvParam
        };

    // Get restype - first from instance, then from class's _restype_
    let restype_obj = zelf.restype.read().clone().or_else(|| {
        zelf.as_object()
            .class()
            .get_attr(vm.ctx.intern_str("_restype_"))
    });

    // Check if restype is explicitly None (return void)
    let restype_is_none = restype_obj.as_ref().is_some_and(|t| vm.is_none(t));
    let ffi_return_type = if restype_is_none {
        Type::void()
    } else {
        restype_obj
            .as_ref()
            .and_then(|t| t.clone().downcast::<PyType>().ok())
            .and_then(|t| ReturnType::to_ffi_type(&t, vm))
            .unwrap_or_else(Type::i32)
    };

    // Check if return type is a pointer type via TYPEFLAG_ISPOINTER
    // This handles c_void_p, c_char_p, c_wchar_p, and POINTER(T) types
    let is_pointer_return = restype_obj
        .as_ref()
        .and_then(|t| t.clone().downcast::<PyType>().ok())
        .and_then(|t| {
            t.stg_info_opt()
                .map(|info| info.flags.contains(StgInfoFlags::TYPEFLAG_ISPOINTER))
        })
        .unwrap_or(false);

    Ok(CallInfo {
        explicit_arg_types,
        restype_obj,
        restype_is_none,
        ffi_return_type,
        is_pointer_return,
    })
}

/// Parsed paramflags: (direction, name, default) tuples
/// direction: 1=IN, 2=OUT, 4=IN|OUT (or 1|2=3)
type ParsedParamFlags = Vec<(u32, Option<String>, Option<PyObjectRef>)>;

/// Parse paramflags from PyCFuncPtr
fn parse_paramflags(
    zelf: &Py<PyCFuncPtr>,
    vm: &VirtualMachine,
) -> PyResult<Option<ParsedParamFlags>> {
    let Some(pf) = zelf.paramflags.read().as_ref().cloned() else {
        return Ok(None);
    };

    let pf_vec = pf.try_to_value::<Vec<PyObjectRef>>(vm)?;
    let parsed = pf_vec
        .into_iter()
        .map(|item| {
            let Some(tuple) = item.downcast_ref::<PyTuple>() else {
                // Single value means just the direction
                let direction = item
                    .try_int(vm)
                    .ok()
                    .and_then(|i| i.as_bigint().to_u32())
                    .unwrap_or(1);
                return (direction, None, None);
            };
            let direction = tuple
                .first()
                .and_then(|d| d.try_int(vm).ok())
                .and_then(|i| i.as_bigint().to_u32())
                .unwrap_or(1);
            let name = tuple
                .get(1)
                .and_then(|n| n.downcast_ref::<PyStr>().map(|s| s.to_string()));
            let default = tuple.get(2).cloned();
            (direction, name, default)
        })
        .collect();
    Ok(Some(parsed))
}

/// Resolve COM method pointer from vtable (Windows only)
/// Returns (Some(CodePtr), true) if this is a COM method call, (None, false) otherwise
#[cfg(windows)]
fn resolve_com_method(
    zelf: &Py<PyCFuncPtr>,
    args: &FuncArgs,
    vm: &VirtualMachine,
) -> PyResult<(Option<CodePtr>, bool)> {
    let com_index = zelf.index.read();
    let Some(idx) = *com_index else {
        return Ok((None, false));
    };

    // First arg must be the COM object pointer
    if args.args.is_empty() {
        return Err(
            vm.new_type_error("COM method requires at least one argument (self)".to_string())
        );
    }

    // Extract COM pointer value from first argument
    let self_arg = &args.args[0];
    let com_ptr = if let Some(simple) = self_arg.downcast_ref::<PyCSimple>() {
        let buffer = simple.0.buffer.read();
        if buffer.len() >= std::mem::size_of::<usize>() {
            super::base::read_ptr_from_buffer(&buffer)
        } else {
            0
        }
    } else if let Ok(int_val) = self_arg.try_int(vm) {
        int_val.as_bigint().to_usize().unwrap_or(0)
    } else {
        return Err(
            vm.new_type_error("COM method first argument must be a COM pointer".to_string())
        );
    };

    if com_ptr == 0 {
        return Err(vm.new_value_error("NULL COM pointer access"));
    }

    // Read vtable pointer from COM object: vtable = *(void**)com_ptr
    let vtable_ptr = unsafe { *(com_ptr as *const usize) };
    if vtable_ptr == 0 {
        return Err(vm.new_value_error("NULL vtable pointer"));
    }

    // Read function pointer from vtable: func = vtable[index]
    let fptr = unsafe {
        let vtable = vtable_ptr as *const usize;
        *vtable.add(idx)
    };

    if fptr == 0 {
        return Err(vm.new_value_error("NULL function pointer in vtable"));
    }

    Ok((Some(CodePtr(fptr as *mut _)), true))
}

/// Single argument for FFI call
// struct argument
struct Argument {
    ffi_type: Type,
    value: FfiArgValue,
    #[allow(dead_code)]
    keep: Option<PyObjectRef>, // Object to keep alive during call
}

/// Out buffers for paramflags OUT parameters
type OutBuffers = Vec<(usize, PyObjectRef)>;

/// Get buffer address from a ctypes object
fn get_buffer_addr(obj: &PyObjectRef) -> Option<usize> {
    obj.downcast_ref::<PyCSimple>()
        .map(|s| s.0.buffer.read().as_ptr() as usize)
        .or_else(|| {
            obj.downcast_ref::<super::structure::PyCStructure>()
                .map(|s| s.0.buffer.read().as_ptr() as usize)
        })
        .or_else(|| {
            obj.downcast_ref::<PyCPointer>()
                .map(|s| s.0.buffer.read().as_ptr() as usize)
        })
}

/// Create OUT buffer for a parameter type
fn create_out_buffer(arg_type: &PyTypeRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
    // For POINTER(T) types, create T instance (the pointed-to type)
    if arg_type.fast_issubclass(PyCPointer::static_type())
        && let Some(stg_info) = arg_type.stg_info_opt()
        && let Some(ref proto) = stg_info.proto
    {
        return proto.as_object().call((), vm);
    }
    // Not a pointer type or no proto, create instance directly
    arg_type.as_object().call((), vm)
}

/// Build callargs when no argtypes specified (use ConvParam)
fn build_callargs_no_argtypes(
    args: &FuncArgs,
    vm: &VirtualMachine,
) -> PyResult<(Vec<Argument>, OutBuffers)> {
    let arguments: Vec<Argument> = args
        .args
        .iter()
        .map(|arg| conv_param(arg, vm))
        .collect::<PyResult<Vec<_>>>()?;
    Ok((arguments, Vec::new()))
}

/// Build callargs for regular function with argtypes (no paramflags)
fn build_callargs_simple(
    args: &FuncArgs,
    arg_types: &[PyTypeRef],
    vm: &VirtualMachine,
) -> PyResult<(Vec<Argument>, OutBuffers)> {
    let arguments: Vec<Argument> = args
        .args
        .iter()
        .enumerate()
        .map(|(n, arg)| {
            let arg_type = arg_types
                .get(n)
                .ok_or_else(|| vm.new_type_error("argument amount mismatch"))?;
            let ffi_type = ArgumentType::to_ffi_type(arg_type, vm)?;
            let value = arg_type.convert_object(arg.clone(), vm)?;
            Ok(Argument {
                ffi_type,
                keep: None,
                value,
            })
        })
        .collect::<PyResult<Vec<_>>>()?;
    Ok((arguments, Vec::new()))
}

/// Build callargs with paramflags (handles IN/OUT parameters)
fn build_callargs_with_paramflags(
    args: &FuncArgs,
    arg_types: &[PyTypeRef],
    paramflags: &ParsedParamFlags,
    skip_first_arg: bool, // true for COM methods
    vm: &VirtualMachine,
) -> PyResult<(Vec<Argument>, OutBuffers)> {
    let mut arguments = Vec::new();
    let mut out_buffers = Vec::new();

    // For COM methods, first arg is self (pointer)
    let mut caller_arg_idx = if skip_first_arg {
        if !args.args.is_empty() {
            let arg = conv_param(&args.args[0], vm)?;
            arguments.push(arg);
        }
        1usize
    } else {
        0usize
    };

    // Process parameters based on paramflags
    for (param_idx, (direction, _name, default)) in paramflags.iter().enumerate() {
        let arg_type = arg_types
            .get(param_idx)
            .ok_or_else(|| vm.new_type_error("paramflags/argtypes mismatch"))?;

        let is_out = (*direction & 2) != 0; // OUT flag
        let is_in = (*direction & 1) != 0 || *direction == 0; // IN flag or default

        let ffi_type = ArgumentType::to_ffi_type(arg_type, vm)?;

        if is_out && !is_in {
            // Pure OUT parameter: create buffer, don't consume caller arg
            let buffer = create_out_buffer(arg_type, vm)?;
            let addr = get_buffer_addr(&buffer).ok_or_else(|| {
                vm.new_type_error("Cannot create OUT buffer for this type".to_string())
            })?;
            arguments.push(Argument {
                ffi_type,
                keep: None,
                value: FfiArgValue::Pointer(addr),
            });
            out_buffers.push((param_idx, buffer));
        } else {
            // IN or IN|OUT: get from caller args or default
            let arg = if caller_arg_idx < args.args.len() {
                caller_arg_idx += 1;
                args.args[caller_arg_idx - 1].clone()
            } else if let Some(def) = default {
                def.clone()
            } else {
                return Err(vm.new_type_error(format!("required argument {} missing", param_idx)));
            };

            if is_out {
                // IN|OUT: track for return
                out_buffers.push((param_idx, arg.clone()));
            }
            let value = arg_type.convert_object(arg, vm)?;
            arguments.push(Argument {
                ffi_type,
                keep: None,
                value,
            });
        }
    }

    Ok((arguments, out_buffers))
}

/// Build call arguments (main dispatcher)
fn build_callargs(
    args: &FuncArgs,
    call_info: &CallInfo,
    paramflags: Option<&ParsedParamFlags>,
    is_com_method: bool,
    vm: &VirtualMachine,
) -> PyResult<(Vec<Argument>, OutBuffers)> {
    let Some(ref arg_types) = call_info.explicit_arg_types else {
        // No argtypes: use ConvParam
        return build_callargs_no_argtypes(args, vm);
    };

    if let Some(pflags) = paramflags {
        // Has paramflags: handle IN/OUT
        build_callargs_with_paramflags(args, arg_types, pflags, is_com_method, vm)
    } else if is_com_method {
        // COM method without paramflags
        let mut arguments = Vec::new();
        if !args.args.is_empty() {
            arguments.push(conv_param(&args.args[0], vm)?);
        }
        for (n, arg) in args.args.iter().skip(1).enumerate() {
            let arg_type = arg_types
                .get(n)
                .ok_or_else(|| vm.new_type_error("argument amount mismatch"))?;
            let ffi_type = ArgumentType::to_ffi_type(arg_type, vm)?;
            let value = arg_type.convert_object(arg.clone(), vm)?;
            arguments.push(Argument {
                ffi_type,
                keep: None,
                value,
            });
        }
        Ok((arguments, Vec::new()))
    } else {
        // Regular function
        build_callargs_simple(args, arg_types, vm)
    }
}

/// Raw result from FFI call
enum RawResult {
    Void,
    Pointer(usize),
    Value(libffi::low::ffi_arg),
}

/// Execute FFI call
fn ctypes_callproc(code_ptr: CodePtr, arguments: &[Argument], call_info: &CallInfo) -> RawResult {
    let ffi_arg_types: Vec<Type> = arguments.iter().map(|a| a.ffi_type.clone()).collect();
    let cif = Cif::new(ffi_arg_types, call_info.ffi_return_type.clone());
    let ffi_args: Vec<Arg<'_>> = arguments.iter().map(|a| a.value.as_arg()).collect();

    if call_info.restype_is_none {
        unsafe { cif.call::<()>(code_ptr, &ffi_args) };
        RawResult::Void
    } else if call_info.is_pointer_return {
        let result = unsafe { cif.call::<usize>(code_ptr, &ffi_args) };
        RawResult::Pointer(result)
    } else {
        let result = unsafe { cif.call::<libffi::low::ffi_arg>(code_ptr, &ffi_args) };
        RawResult::Value(result)
    }
}

/// Check and handle HRESULT errors (Windows)
#[cfg(windows)]
fn check_hresult(hresult: i32, zelf: &Py<PyCFuncPtr>, vm: &VirtualMachine) -> PyResult<()> {
    if hresult >= 0 {
        return Ok(());
    }

    if zelf.iid.read().is_some() {
        // Raise COMError
        let ctypes_module = vm.import("_ctypes", 0)?;
        let com_error_type = ctypes_module.get_attr("COMError", vm)?;
        let com_error_type = com_error_type
            .downcast::<PyType>()
            .map_err(|_| vm.new_type_error("COMError is not a type"))?;
        let hresult_obj: PyObjectRef = vm.ctx.new_int(hresult).into();
        let text: PyObjectRef = vm
            .ctx
            .new_str(format!("HRESULT: 0x{:08X}", hresult as u32))
            .into();
        let details: PyObjectRef = vm.ctx.none();
        let exc = vm.invoke_exception(
            com_error_type.to_owned(),
            vec![text.clone(), details.clone()],
        )?;
        let _ = exc.as_object().set_attr("hresult", hresult_obj, vm);
        let _ = exc.as_object().set_attr("text", text, vm);
        let _ = exc.as_object().set_attr("details", details, vm);
        Err(exc)
    } else {
        // Raise OSError
        let exc = vm.new_os_error(format!("HRESULT: 0x{:08X}", hresult as u32));
        let _ = exc
            .as_object()
            .set_attr("winerror", vm.ctx.new_int(hresult), vm);
        Err(exc)
    }
}

/// Convert raw FFI result to Python object
// = GetResult
fn convert_raw_result(
    raw_result: &mut RawResult,
    call_info: &CallInfo,
    vm: &VirtualMachine,
) -> Option<PyObjectRef> {
    // Get result as bytes for type conversion
    let (result_bytes, result_size) = match raw_result {
        RawResult::Void => return None,
        RawResult::Pointer(ptr) => {
            let bytes = ptr.to_ne_bytes();
            (bytes.to_vec(), core::mem::size_of::<usize>())
        }
        RawResult::Value(val) => {
            let bytes = val.to_ne_bytes();
            (bytes.to_vec(), core::mem::size_of::<i64>())
        }
    };

    // 1. No restype → return as int
    let restype = match &call_info.restype_obj {
        None => {
            // Default: return as int
            let val = match raw_result {
                RawResult::Pointer(p) => *p as isize,
                RawResult::Value(v) => *v as isize,
                RawResult::Void => return None,
            };
            return Some(vm.ctx.new_int(val).into());
        }
        Some(r) => r,
    };

    // 2. restype is None → return None
    if restype.is(&vm.ctx.none()) {
        return None;
    }

    // 3. Get restype as PyType
    let restype_type = match restype.clone().downcast::<PyType>() {
        Ok(t) => t,
        Err(_) => {
            // Not a type, call it with int result
            let val = match raw_result {
                RawResult::Pointer(p) => *p as isize,
                RawResult::Value(v) => *v as isize,
                RawResult::Void => return None,
            };
            return restype.call((val,), vm).ok();
        }
    };

    // 4. Get StgInfo
    let stg_info = restype_type.stg_info_opt();

    // No StgInfo → call restype with int
    if stg_info.is_none() {
        let val = match raw_result {
            RawResult::Pointer(p) => *p as isize,
            RawResult::Value(v) => *v as isize,
            RawResult::Void => return None,
        };
        return restype_type.as_object().call((val,), vm).ok();
    }

    let info = stg_info.unwrap();

    // 5. Simple type with getfunc → use bytes_to_pyobject (info->getfunc)
    // is_simple_instance returns TRUE for c_int, c_void_p, etc.
    if super::base::is_simple_instance(&restype_type) {
        return super::base::bytes_to_pyobject(&restype_type, &result_bytes, vm).ok();
    }

    // 6. Complex type → create ctypes instance (PyCData_FromBaseObj)
    // This handles POINTER(T), Structure, Array, etc.

    // Special handling for POINTER(T) types - set pointer value directly
    if info.flags.contains(StgInfoFlags::TYPEFLAG_ISPOINTER)
        && info.proto.is_some()
        && let RawResult::Pointer(ptr) = raw_result
        && let Ok(instance) = restype_type.as_object().call((), vm)
    {
        if let Some(pointer) = instance.downcast_ref::<PyCPointer>() {
            pointer.set_ptr_value(*ptr);
        }
        return Some(instance);
    }

    // Create instance and copy result data
    pycdata_from_ffi_result(&restype_type, &result_bytes, result_size, vm).ok()
}

/// Create a ctypes instance from FFI result (PyCData_FromBaseObj equivalent)
fn pycdata_from_ffi_result(
    typ: &PyTypeRef,
    result_bytes: &[u8],
    size: usize,
    vm: &VirtualMachine,
) -> PyResult {
    // Create instance
    let instance = PyType::call(typ, ().into(), vm)?;

    // Copy result data into instance buffer
    if let Some(cdata) = instance.downcast_ref::<PyCData>() {
        let mut buffer = cdata.buffer.write();
        let copy_size = size.min(buffer.len()).min(result_bytes.len());
        if copy_size > 0 {
            buffer.to_mut()[..copy_size].copy_from_slice(&result_bytes[..copy_size]);
        }
    }

    Ok(instance)
}

/// Extract values from OUT buffers
fn extract_out_values(
    out_buffers: Vec<(usize, PyObjectRef)>,
    vm: &VirtualMachine,
) -> Vec<PyObjectRef> {
    out_buffers
        .into_iter()
        .map(|(_, buffer)| buffer.get_attr("value", vm).unwrap_or(buffer))
        .collect()
}

/// Build final result (main function)
fn build_result(
    mut raw_result: RawResult,
    call_info: &CallInfo,
    out_buffers: OutBuffers,
    zelf: &Py<PyCFuncPtr>,
    args: &FuncArgs,
    vm: &VirtualMachine,
) -> PyResult {
    // Check HRESULT on Windows
    #[cfg(windows)]
    if let RawResult::Value(val) = raw_result {
        let is_hresult = call_info
            .restype_obj
            .as_ref()
            .and_then(|t| t.clone().downcast::<PyType>().ok())
            .is_some_and(|t| t.name().to_string() == "HRESULT");
        if is_hresult {
            check_hresult(val as i32, zelf, vm)?;
        }
    }

    // Convert raw result to Python object
    let mut result = convert_raw_result(&mut raw_result, call_info, vm);

    // Apply errcheck if set
    if let Some(errcheck) = zelf.errcheck.read().as_ref() {
        let args_tuple = PyTuple::new_ref(args.args.clone(), &vm.ctx);
        let func_obj = zelf.as_object().to_owned();
        let result_obj = result.clone().unwrap_or_else(|| vm.ctx.none());
        result = Some(errcheck.call((result_obj, func_obj, args_tuple), vm)?);
    }

    // Handle OUT parameter return values
    if out_buffers.is_empty() {
        return result.map(Ok).unwrap_or_else(|| Ok(vm.ctx.none()));
    }

    let out_values = extract_out_values(out_buffers, vm);
    Ok(match <[PyObjectRef; 1]>::try_from(out_values) {
        Ok([single]) => single,
        Err(v) => PyTuple::new_ref(v, &vm.ctx).into(),
    })
}

impl Callable for PyCFuncPtr {
    type Args = FuncArgs;
    fn call(zelf: &Py<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult {
        // 1. Check for internal PYFUNCTYPE addresses
        if let Some(result) = handle_internal_func(zelf.get_func_ptr(), &args, vm) {
            return result;
        }

        // 2. Resolve function pointer (COM or direct)
        #[cfg(windows)]
        let (func_ptr, is_com_method) = resolve_com_method(zelf, &args, vm)?;
        #[cfg(not(windows))]
        let (func_ptr, is_com_method) = (None::<CodePtr>, false);

        // 3. Extract call info (argtypes, restype)
        let call_info = extract_call_info(zelf, vm)?;

        // 4. Parse paramflags
        let paramflags = parse_paramflags(zelf, vm)?;

        // 5. Build call arguments
        let (arguments, out_buffers) =
            build_callargs(&args, &call_info, paramflags.as_ref(), is_com_method, vm)?;

        // 6. Get code pointer
        let code_ptr = match func_ptr.or_else(|| zelf.get_code_ptr()) {
            Some(cp) => cp,
            None => {
                debug_assert!(false, "NULL function pointer");
                // In release mode, this will crash
                CodePtr(core::ptr::null_mut())
            }
        };

        // 7. Get flags to check for use_last_error/use_errno
        let flags = PyCFuncPtr::_flags_(zelf, vm);

        // 8. Call the function (with use_last_error/use_errno handling)
        #[cfg(not(windows))]
        let raw_result = {
            if flags & super::base::StgInfoFlags::FUNCFLAG_USE_ERRNO.bits() != 0 {
                swap_errno(|| ctypes_callproc(code_ptr, &arguments, &call_info))
            } else {
                ctypes_callproc(code_ptr, &arguments, &call_info)
            }
        };

        #[cfg(windows)]
        let raw_result = {
            if flags & super::base::StgInfoFlags::FUNCFLAG_USE_LASTERROR.bits() != 0 {
                save_and_restore_last_error(|| ctypes_callproc(code_ptr, &arguments, &call_info))
            } else {
                ctypes_callproc(code_ptr, &arguments, &call_info)
            }
        };

        // 9. Build result
        build_result(raw_result, &call_info, out_buffers, zelf, &args, vm)
    }
}

// PyCFuncPtr_repr
impl Representable for PyCFuncPtr {
    fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
        let type_name = zelf.class().name();
        // Use object id, not function pointer address
        let addr = zelf.get_id();
        Ok(format!("<{} object at {:#x}>", type_name, addr))
    }
}

// PyCData_NewGetBuffer
impl AsBuffer for PyCFuncPtr {
    fn as_buffer(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<PyBuffer> {
        // CFuncPtr types may not have StgInfo if PyCFuncPtrType metaclass is not used
        // Use default values for function pointers: format="X{}", size=sizeof(pointer)
        let (format, itemsize) = if let Some(stg_info) = zelf.class().stg_info_opt() {
            (
                stg_info
                    .format
                    .clone()
                    .map(Cow::Owned)
                    .unwrap_or(Cow::Borrowed("X{}")),
                stg_info.size,
            )
        } else {
            (Cow::Borrowed("X{}"), core::mem::size_of::<usize>())
        };
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

#[pyclass(flags(BASETYPE), with(Callable, Constructor, Representable, AsBuffer))]
impl PyCFuncPtr {
    // restype getter/setter
    #[pygetset]
    fn restype(&self) -> Option<PyObjectRef> {
        self.restype.read().clone()
    }

    #[pygetset(setter)]
    fn set_restype(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        // Must be type, callable, or None
        if vm.is_none(&value) {
            *self.restype.write() = None;
        } else if value.downcast_ref::<PyType>().is_some() || value.is_callable() {
            *self.restype.write() = Some(value);
        } else {
            return Err(vm.new_type_error("restype must be a type, a callable, or None"));
        }
        Ok(())
    }

    // argtypes getter/setter
    #[pygetset]
    fn argtypes(&self, vm: &VirtualMachine) -> PyObjectRef {
        self.argtypes
            .read()
            .clone()
            .unwrap_or_else(|| vm.ctx.empty_tuple.clone().into())
    }

    #[pygetset(name = "argtypes", setter)]
    fn set_argtypes(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        if vm.is_none(&value) {
            *self.argtypes.write() = None;
        } else {
            // Store the argtypes object directly as it is
            *self.argtypes.write() = Some(value);
        }
        Ok(())
    }

    // errcheck getter/setter
    #[pygetset]
    fn errcheck(&self) -> Option<PyObjectRef> {
        self.errcheck.read().clone()
    }

    #[pygetset(setter)]
    fn set_errcheck(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        if vm.is_none(&value) {
            *self.errcheck.write() = None;
        } else if value.is_callable() {
            *self.errcheck.write() = Some(value);
        } else {
            return Err(vm.new_type_error("errcheck must be a callable or None"));
        }
        Ok(())
    }

    // _flags_ getter (read-only, from type's class attribute or StgInfo)
    #[pygetset]
    fn _flags_(zelf: &Py<Self>, vm: &VirtualMachine) -> u32 {
        // First try to get _flags_ from type's class attribute (for dynamically created types)
        // This is how CDLL sets use_errno: class _FuncPtr(_CFuncPtr): _flags_ = flags
        if let Ok(flags_attr) = zelf.class().as_object().get_attr("_flags_", vm)
            && let Ok(flags_int) = flags_attr.try_to_value::<u32>(vm)
        {
            return flags_int;
        }

        // Fallback to StgInfo for native types
        zelf.class()
            .stg_info_opt()
            .map(|stg| stg.flags.bits())
            .unwrap_or(StgInfoFlags::empty().bits())
    }

    // bool conversion - check if function pointer is set
    #[pymethod]
    fn __bool__(&self) -> bool {
        self.get_func_ptr() != 0
    }
}

// CThunkObject - FFI callback (thunk) implementation

/// Userdata passed to the libffi callback.
struct ThunkUserData {
    /// The Python callable to invoke
    callable: PyObjectRef,
    /// Argument types for conversion
    arg_types: Vec<PyTypeRef>,
    /// Result type for conversion (None means void)
    pub res_type: Option<PyTypeRef>,
    /// Function flags (FUNCFLAG_USE_ERRNO, etc.)
    pub flags: u32,
}

/// Check if ty is a subclass of a simple type (like MyInt(c_int)).
fn is_simple_subclass(ty: &Py<PyType>, vm: &VirtualMachine) -> bool {
    let Ok(base) = ty.as_object().get_attr(vm.ctx.intern_str("__base__"), vm) else {
        return false;
    };
    base.get_attr(vm.ctx.intern_str("_type_"), vm).is_ok()
}

/// Convert a C value to a Python object based on the type code.
fn ffi_to_python(ty: &Py<PyType>, ptr: *const c_void, vm: &VirtualMachine) -> PyObjectRef {
    let type_code = ty.type_code(vm);
    let raw_value: PyObjectRef = unsafe {
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
            Some("z") => {
                // c_char_p: C string pointer → Python bytes
                let cstr_ptr = *(ptr as *const *const libc::c_char);
                if cstr_ptr.is_null() {
                    vm.ctx.none()
                } else {
                    let cstr = core::ffi::CStr::from_ptr(cstr_ptr);
                    vm.ctx.new_bytes(cstr.to_bytes().to_vec()).into()
                }
            }
            Some("Z") => {
                // c_wchar_p: wchar_t* → Python str
                let wstr_ptr = *(ptr as *const *const libc::wchar_t);
                if wstr_ptr.is_null() {
                    vm.ctx.none()
                } else {
                    let mut len = 0;
                    while *wstr_ptr.add(len) != 0 {
                        len += 1;
                    }
                    let slice = core::slice::from_raw_parts(wstr_ptr, len);
                    // Windows: wchar_t = u16 (UTF-16) -> use Wtf8Buf::from_wide
                    // Unix: wchar_t = i32 (UTF-32) -> convert via char::from_u32
                    #[cfg(windows)]
                    {
                        use rustpython_common::wtf8::Wtf8Buf;
                        let wide: Vec<u16> = slice.to_vec();
                        let wtf8 = Wtf8Buf::from_wide(&wide);
                        vm.ctx.new_str(wtf8).into()
                    }
                    #[cfg(not(windows))]
                    {
                        let s: String = slice
                            .iter()
                            .filter_map(|&c| char::from_u32(c as u32))
                            .collect();
                        vm.ctx.new_str(s).into()
                    }
                }
            }
            Some("P") => vm.ctx.new_int(*(ptr as *const usize)).into(),
            Some("?") => vm.ctx.new_bool(*(ptr as *const u8) != 0).into(),
            _ => return vm.ctx.none(),
        }
    };

    if !is_simple_subclass(ty, vm) {
        return raw_value;
    }
    ty.as_object()
        .call((raw_value.clone(),), vm)
        .unwrap_or(raw_value)
}

/// Convert a Python object to a C value and store it at the result pointer
fn python_to_ffi(obj: PyResult, ty: &Py<PyType>, result: *mut c_void, vm: &VirtualMachine) {
    let Ok(obj) = obj else { return };

    let type_code = ty.type_code(vm);
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
                    let val = i.as_bigint().to_i32().unwrap_or(0);
                    *(result as *mut libffi::low::ffi_arg) = val as libffi::low::ffi_arg;
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
            Some("?") => {
                if let Ok(b) = obj.is_true(vm) {
                    *(result as *mut u8) = u8::from(b);
                }
            }
            _ => {}
        }
    }
}

/// The callback function that libffi calls when the closure is invoked.
unsafe extern "C" fn thunk_callback(
    _cif: &low::ffi_cif,
    result: &mut c_void,
    args: *const *const c_void,
    userdata: &ThunkUserData,
) {
    with_current_vm(|vm| {
        // Swap errno before call if FUNCFLAG_USE_ERRNO is set
        let use_errno = userdata.flags & StgInfoFlags::FUNCFLAG_USE_ERRNO.bits() != 0;
        let saved_errno = if use_errno {
            let current = rustpython_common::os::get_errno();
            // TODO: swap with ctypes stored errno (thread-local)
            Some(current)
        } else {
            None
        };

        let py_args: Vec<PyObjectRef> = userdata
            .arg_types
            .iter()
            .enumerate()
            .map(|(i, ty)| {
                let arg_ptr = unsafe { *args.add(i) };
                ffi_to_python(ty, arg_ptr, vm)
            })
            .collect();

        let py_result = userdata.callable.call(py_args, vm);

        // Swap errno back after call
        if use_errno {
            let _current = rustpython_common::os::get_errno();
            // TODO: store current errno to ctypes storage
            if let Some(saved) = saved_errno {
                rustpython_common::os::set_errno(saved);
            }
        }

        // Call unraisable hook if exception occurred
        if let Err(exc) = &py_result {
            let repr = userdata
                .callable
                .repr(vm)
                .map(|s| s.to_string())
                .unwrap_or_else(|_| "<unknown>".to_string());
            let msg = format!(
                "Exception ignored on calling ctypes callback function {}",
                repr
            );
            vm.run_unraisable(exc.clone(), Some(msg), vm.ctx.none());
        }

        if let Some(ref res_type) = userdata.res_type {
            python_to_ffi(py_result, res_type, result as *mut c_void, vm);
        }
    });
}

/// Holds the closure and userdata together to ensure proper lifetime.
struct ThunkData {
    #[allow(dead_code)]
    closure: Closure<'static>,
    userdata_ptr: *mut ThunkUserData,
}

impl Drop for ThunkData {
    fn drop(&mut self) {
        unsafe {
            drop(Box::from_raw(self.userdata_ptr));
        }
    }
}

/// CThunkObject wraps a Python callable to make it callable from C code.
#[pyclass(name = "CThunkObject", module = "_ctypes")]
#[derive(PyPayload)]
pub(super) struct PyCThunk {
    callable: PyObjectRef,
    #[allow(dead_code)]
    thunk_data: PyRwLock<Option<ThunkData>>,
    code_ptr: CodePtr,
}

impl Debug for PyCThunk {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PyCThunk")
            .field("callable", &self.callable)
            .finish()
    }
}

impl PyCThunk {
    pub fn new(
        callable: PyObjectRef,
        arg_types: Option<PyObjectRef>,
        res_type: Option<PyObjectRef>,
        flags: u32,
        vm: &VirtualMachine,
    ) -> PyResult<Self> {
        let arg_type_vec: Vec<PyTypeRef> = match arg_types {
            Some(args) if !vm.is_none(&args) => args
                .try_to_value::<Vec<PyObjectRef>>(vm)?
                .into_iter()
                .map(|item| {
                    item.downcast::<PyType>()
                        .map_err(|_| vm.new_type_error("_argtypes_ must be a sequence of types"))
                })
                .collect::<PyResult<Vec<_>>>()?,
            _ => Vec::new(),
        };

        let res_type_ref: Option<PyTypeRef> = match res_type {
            Some(ref rt) if !vm.is_none(rt) => Some(
                rt.clone()
                    .downcast::<PyType>()
                    .map_err(|_| vm.new_type_error("restype must be a ctypes type"))?,
            ),
            _ => None,
        };

        let ffi_arg_types: Vec<Type> = arg_type_vec
            .iter()
            .map(|ty| {
                ty.type_code(vm)
                    .and_then(|code| get_ffi_type(&code))
                    .unwrap_or(Type::pointer())
            })
            .collect();

        let ffi_res_type = res_type_ref
            .as_ref()
            .and_then(|ty| ty.type_code(vm))
            .and_then(|code| get_ffi_type(&code))
            .unwrap_or(Type::void());

        let cif = Cif::new(ffi_arg_types, ffi_res_type);

        let userdata = Box::new(ThunkUserData {
            callable: callable.clone(),
            arg_types: arg_type_vec,
            res_type: res_type_ref,
            flags,
        });
        let userdata_ptr = Box::into_raw(userdata);
        let userdata_ref: &'static ThunkUserData = unsafe { &*userdata_ptr };

        let closure = Closure::new(cif, thunk_callback, userdata_ref);
        let code_ptr = CodePtr(*closure.code_ptr() as *mut _);

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

    pub fn code_ptr(&self) -> CodePtr {
        self.code_ptr
    }
}

unsafe impl Send for PyCThunk {}
unsafe impl Sync for PyCThunk {}

#[pyclass]
impl PyCThunk {
    #[pygetset]
    fn callable(&self) -> PyObjectRef {
        self.callable.clone()
    }
}

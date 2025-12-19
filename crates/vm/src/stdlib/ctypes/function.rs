// spell-checker:disable

use super::{
    _ctypes::CArgObject, PyCArray, PyCData, PyCPointer, PyCStructure, base::FfiArgValue,
    simple::PyCSimple, type_info,
};
use crate::{
    AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    builtins::{PyBytes, PyDict, PyNone, PyStr, PyTuple, PyType, PyTypeRef},
    class::StaticType,
    convert::ToPyObject,
    function::FuncArgs,
    types::{Callable, Constructor, Representable},
    vm::thread::with_current_vm,
};
use libffi::{
    low,
    middle::{Arg, Cif, Closure, CodePtr, Type},
};
use libloading::Symbol;
use num_traits::ToPrimitive;
use rustpython_common::lock::PyRwLock;
use std::ffi::{self, c_void};
use std::fmt::Debug;

// Internal function addresses for special ctypes functions
pub(super) const INTERNAL_CAST_ADDR: usize = 1;
pub(super) const INTERNAL_STRING_AT_ADDR: usize = 2;
pub(super) const INTERNAL_WSTRING_AT_ADDR: usize = 3;

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
            if std::mem::size_of::<libc::c_long>() == 8 {
                Type::i64()
            } else {
                Type::i32()
            }
        }
        b'L' => {
            if std::mem::size_of::<libc::c_ulong>() == 8 {
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
            if std::mem::size_of::<super::WideChar>() == 2 {
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
        if buffer.len() >= std::mem::size_of::<usize>() {
            let addr = super::base::read_ptr_from_buffer(&buffer);
            return Ok(FfiArgValue::Pointer(addr));
        }
    }

    // 6. bytes -> buffer address (PyBytes_AsString)
    if let Some(bytes) = value.downcast_ref::<crate::builtins::PyBytes>() {
        let addr = bytes.as_bytes().as_ptr() as usize;
        return Ok(FfiArgValue::Pointer(addr));
    }

    // 7. Integer -> direct value
    if let Ok(int_val) = value.try_int(vm) {
        return Ok(FfiArgValue::Pointer(
            int_val.as_bigint().to_usize().unwrap_or(0),
        ));
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
/// Returns both the FFI type and the converted value
fn conv_param(value: &PyObject, vm: &VirtualMachine) -> PyResult<(Type, FfiArgValue)> {
    // 1. CArgObject (from byref() or paramfunc) -> use stored type and value
    if let Some(carg) = value.downcast_ref::<CArgObject>() {
        let ffi_type = ffi_type_from_tag(carg.tag);
        return Ok((ffi_type, carg.value.clone()));
    }

    // 2. None -> NULL pointer
    if value.is(&vm.ctx.none) {
        return Ok((Type::pointer(), FfiArgValue::Pointer(0)));
    }

    // 3. ctypes objects -> use paramfunc
    if let Ok(carg) = super::base::call_paramfunc(value, vm) {
        let ffi_type = ffi_type_from_tag(carg.tag);
        return Ok((ffi_type, carg.value.clone()));
    }

    // 4. Python str -> pointer (use internal UTF-8 buffer)
    if let Some(s) = value.downcast_ref::<PyStr>() {
        let addr = s.as_str().as_ptr() as usize;
        return Ok((Type::pointer(), FfiArgValue::Pointer(addr)));
    }

    // 9. Python bytes -> pointer to buffer
    if let Some(bytes) = value.downcast_ref::<PyBytes>() {
        let addr = bytes.as_bytes().as_ptr() as usize;
        return Ok((Type::pointer(), FfiArgValue::Pointer(addr)));
    }

    // 10. Python int -> i32 (default integer type)
    if let Ok(int_val) = value.try_int(vm) {
        let val = int_val.as_bigint().to_i32().unwrap_or(0);
        return Ok((Type::i32(), FfiArgValue::I32(val)));
    }

    // 11. Python float -> f64
    if let Ok(float_val) = value.try_float(vm) {
        return Ok((Type::f64(), FfiArgValue::F64(float_val.to_f64())));
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
        // Call from_param first to convert the value (like CPython's callproc.c:1235)
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

        // For pointer types (POINTER(T)), we need to pass the ADDRESS of the value's buffer
        if self.fast_issubclass(PyCPointer::static_type()) {
            if let Some(cdata) = converted.downcast_ref::<PyCData>() {
                let addr = cdata.buffer.read().as_ptr() as usize;
                return Ok(FfiArgValue::Pointer(addr));
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
    #[allow(clippy::wrong_self_convention)]
    fn from_ffi_type(
        &self,
        value: *mut ffi::c_void,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>>;
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

    fn from_ffi_type(
        &self,
        value: *mut ffi::c_void,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>> {
        // Get the type code from _type_ attribute (use get_attr to traverse MRO)
        let type_code = self
            .as_object()
            .get_attr(vm.ctx.intern_str("_type_"), vm)
            .ok()
            .and_then(|t| t.downcast_ref::<PyStr>().map(|s| s.to_string()));

        let result = match type_code.as_deref() {
            Some("b") => vm
                .ctx
                .new_int(unsafe { *(value as *const i8) } as i32)
                .into(),
            Some("B") => vm
                .ctx
                .new_int(unsafe { *(value as *const u8) } as i32)
                .into(),
            Some("c") => vm
                .ctx
                .new_bytes(vec![unsafe { *(value as *const u8) }])
                .into(),
            Some("h") => vm
                .ctx
                .new_int(unsafe { *(value as *const i16) } as i32)
                .into(),
            Some("H") => vm
                .ctx
                .new_int(unsafe { *(value as *const u16) } as i32)
                .into(),
            Some("i") => vm.ctx.new_int(unsafe { *(value as *const i32) }).into(),
            Some("I") => vm.ctx.new_int(unsafe { *(value as *const u32) }).into(),
            Some("l") => vm
                .ctx
                .new_int(unsafe { *(value as *const libc::c_long) })
                .into(),
            Some("L") => vm
                .ctx
                .new_int(unsafe { *(value as *const libc::c_ulong) })
                .into(),
            Some("q") => vm
                .ctx
                .new_int(unsafe { *(value as *const libc::c_longlong) })
                .into(),
            Some("Q") => vm
                .ctx
                .new_int(unsafe { *(value as *const libc::c_ulonglong) })
                .into(),
            Some("f") => vm
                .ctx
                .new_float(unsafe { *(value as *const f32) } as f64)
                .into(),
            Some("d") => vm.ctx.new_float(unsafe { *(value as *const f64) }).into(),
            Some("P") | Some("z") | Some("Z") => {
                vm.ctx.new_int(unsafe { *(value as *const usize) }).into()
            }
            Some("?") => vm
                .ctx
                .new_bool(unsafe { *(value as *const u8) } != 0)
                .into(),
            None => {
                // No _type_ attribute - check for Structure/Array types
                // GetResult: PyCData_FromBaseObj creates instance from memory
                if let Some(stg_info) = self.stg_info_opt() {
                    let size = stg_info.size;
                    // Create instance of the ctypes type
                    let instance = self.as_object().call((), vm)?;

                    // Copy return value memory into instance buffer
                    // Use a block to properly scope the borrow
                    {
                        let src = unsafe { std::slice::from_raw_parts(value as *const u8, size) };
                        if let Some(cdata) = instance.downcast_ref::<PyCData>() {
                            let mut buffer = cdata.buffer.write();
                            if buffer.len() >= size {
                                buffer.to_mut()[..size].copy_from_slice(src);
                            }
                        } else if let Some(structure) = instance.downcast_ref::<PyCStructure>() {
                            let mut buffer = structure.0.buffer.write();
                            if buffer.len() >= size {
                                buffer.to_mut()[..size].copy_from_slice(src);
                            }
                        } else if let Some(array) = instance.downcast_ref::<PyCArray>() {
                            let mut buffer = array.0.buffer.write();
                            if buffer.len() >= size {
                                buffer.to_mut()[..size].copy_from_slice(src);
                            }
                        }
                    }
                    return Ok(Some(instance));
                }
                // Not a ctypes type - call type with int result
                return self
                    .as_object()
                    .call((unsafe { *(value as *const i32) },), vm)
                    .map(Some);
            }
            _ => return Err(vm.new_type_error("Unsupported return type")),
        };
        Ok(Some(result))
    }
}

impl ReturnType for PyNone {
    fn to_ffi_type(&self, _vm: &VirtualMachine) -> Option<Type> {
        get_ffi_type("void")
    }

    fn from_ffi_type(
        &self,
        _value: *mut ffi::c_void,
        _vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>> {
        Ok(None)
    }
}

/// PyCFuncPtr - Function pointer instance
/// Saved in _base.buffer
#[pyclass(module = "_ctypes", name = "CFuncPtr", base = PyCData)]
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
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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
        if buffer.len() >= std::mem::size_of::<usize>() {
            return Ok(usize::from_ne_bytes(
                buffer[..std::mem::size_of::<usize>()].try_into().unwrap(),
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
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
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
        if size_usize > isize::MAX as usize / std::mem::size_of::<libc::wchar_t>() {
            return Err(vm.new_overflow_error("string too long"));
        }
        size_usize
    };
    let wchars = unsafe { std::slice::from_raw_parts(w_ptr, len) };

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

        let ptr_size = std::mem::size_of::<usize>();

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
                *pointer as usize
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

            // Create the thunk (C-callable wrapper for the Python function)
            let thunk = PyCThunk::new(
                first_arg.clone(),
                class_argtypes.clone(),
                class_restype.clone(),
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

    // Check if return type is a pointer type (P, z, Z) - need special handling on 64-bit
    let is_pointer_return = restype_obj
        .as_ref()
        .and_then(|t| t.clone().downcast::<PyType>().ok())
        .and_then(|t| t.as_object().get_attr(vm.ctx.intern_str("_type_"), vm).ok())
        .and_then(|t| t.downcast_ref::<PyStr>().map(|s| s.to_string()))
        .is_some_and(|tc| matches!(tc.as_str(), "P" | "z" | "Z"));

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

/// Prepared arguments for FFI call
struct PreparedArgs {
    ffi_arg_types: Vec<Type>,
    ffi_values: Vec<FfiArgValue>,
    out_buffers: Vec<(usize, PyObjectRef)>,
}

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
fn build_callargs_no_argtypes(args: &FuncArgs, vm: &VirtualMachine) -> PyResult<PreparedArgs> {
    let results: Vec<(Type, FfiArgValue)> = args
        .args
        .iter()
        .map(|arg| conv_param(arg, vm))
        .collect::<PyResult<Vec<_>>>()?;
    let (ffi_arg_types, ffi_values) = results.into_iter().unzip();
    Ok(PreparedArgs {
        ffi_arg_types,
        ffi_values,
        out_buffers: Vec::new(),
    })
}

/// Build callargs for regular function with argtypes (no paramflags)
fn build_callargs_simple(
    args: &FuncArgs,
    arg_types: &[PyTypeRef],
    vm: &VirtualMachine,
) -> PyResult<PreparedArgs> {
    let ffi_arg_types = arg_types
        .iter()
        .map(|t| ArgumentType::to_ffi_type(t, vm))
        .collect::<PyResult<Vec<_>>>()?;
    let ffi_values = args
        .args
        .iter()
        .enumerate()
        .map(|(n, arg)| {
            let arg_type = arg_types
                .get(n)
                .ok_or_else(|| vm.new_type_error("argument amount mismatch"))?;
            arg_type.convert_object(arg.clone(), vm)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(PreparedArgs {
        ffi_arg_types,
        ffi_values,
        out_buffers: Vec::new(),
    })
}

/// Build callargs with paramflags (handles IN/OUT parameters)
fn build_callargs_with_paramflags(
    args: &FuncArgs,
    arg_types: &[PyTypeRef],
    paramflags: &ParsedParamFlags,
    skip_first_arg: bool, // true for COM methods
    vm: &VirtualMachine,
) -> PyResult<PreparedArgs> {
    let mut ffi_arg_types = Vec::new();
    let mut ffi_values = Vec::new();
    let mut out_buffers = Vec::new();

    // For COM methods, first arg is self (pointer)
    let mut caller_arg_idx = if skip_first_arg {
        ffi_arg_types.push(Type::pointer());
        if !args.args.is_empty() {
            ffi_values.push(conv_param(&args.args[0], vm)?.1);
        }
        1usize
    } else {
        0usize
    };

    // Add FFI types for all argtypes
    for arg_type in arg_types {
        ffi_arg_types.push(ArgumentType::to_ffi_type(arg_type, vm)?);
    }

    // Process parameters based on paramflags
    for (param_idx, (direction, _name, default)) in paramflags.iter().enumerate() {
        let arg_type = arg_types
            .get(param_idx)
            .ok_or_else(|| vm.new_type_error("paramflags/argtypes mismatch"))?;

        let is_out = (*direction & 2) != 0; // OUT flag
        let is_in = (*direction & 1) != 0 || *direction == 0; // IN flag or default

        if is_out && !is_in {
            // Pure OUT parameter: create buffer, don't consume caller arg
            let buffer = create_out_buffer(arg_type, vm)?;
            let addr = get_buffer_addr(&buffer).ok_or_else(|| {
                vm.new_type_error("Cannot create OUT buffer for this type".to_string())
            })?;
            ffi_values.push(FfiArgValue::Pointer(addr));
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
            ffi_values.push(arg_type.convert_object(arg, vm)?);
        }
    }

    Ok(PreparedArgs {
        ffi_arg_types,
        ffi_values,
        out_buffers,
    })
}

/// Build call arguments (main dispatcher)
fn build_callargs(
    args: &FuncArgs,
    call_info: &CallInfo,
    paramflags: Option<&ParsedParamFlags>,
    is_com_method: bool,
    vm: &VirtualMachine,
) -> PyResult<PreparedArgs> {
    let Some(ref arg_types) = call_info.explicit_arg_types else {
        // No argtypes: use ConvParam
        return build_callargs_no_argtypes(args, vm);
    };

    if let Some(pflags) = paramflags {
        // Has paramflags: handle IN/OUT
        build_callargs_with_paramflags(args, arg_types, pflags, is_com_method, vm)
    } else if is_com_method {
        // COM method without paramflags
        let mut ffi_types = vec![Type::pointer()];
        ffi_types.extend(
            arg_types
                .iter()
                .map(|t| ArgumentType::to_ffi_type(t, vm))
                .collect::<PyResult<Vec<_>>>()?,
        );
        let mut ffi_vals = Vec::new();
        if !args.args.is_empty() {
            ffi_vals.push(conv_param(&args.args[0], vm)?.1);
        }
        for (n, arg) in args.args.iter().skip(1).enumerate() {
            let arg_type = arg_types
                .get(n)
                .ok_or_else(|| vm.new_type_error("argument amount mismatch"))?;
            ffi_vals.push(arg_type.convert_object(arg.clone(), vm)?);
        }
        Ok(PreparedArgs {
            ffi_arg_types: ffi_types,
            ffi_values: ffi_vals,
            out_buffers: Vec::new(),
        })
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
fn ctypes_callproc(code_ptr: CodePtr, prepared: &PreparedArgs, call_info: &CallInfo) -> RawResult {
    let cif = Cif::new(
        prepared.ffi_arg_types.clone(),
        call_info.ffi_return_type.clone(),
    );
    let ffi_args: Vec<Arg> = prepared.ffi_values.iter().map(|v| v.as_arg()).collect();

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
fn convert_raw_result(
    raw_result: &mut RawResult,
    call_info: &CallInfo,
    vm: &VirtualMachine,
) -> Option<PyObjectRef> {
    match raw_result {
        RawResult::Void => None,
        RawResult::Pointer(ptr) => {
            // Get type code from restype to determine conversion method
            let type_code = call_info
                .restype_obj
                .as_ref()
                .and_then(|t| t.clone().downcast::<PyType>().ok())
                .and_then(|t| t.as_object().get_attr(vm.ctx.intern_str("_type_"), vm).ok())
                .and_then(|t| t.downcast_ref::<PyStr>().map(|s| s.to_string()));

            match type_code.as_deref() {
                Some("z") => {
                    // c_char_p: NULL -> None, otherwise read C string -> bytes
                    if *ptr == 0 {
                        Some(vm.ctx.none())
                    } else {
                        let cstr = unsafe { std::ffi::CStr::from_ptr(*ptr as _) };
                        Some(vm.ctx.new_bytes(cstr.to_bytes().to_vec()).into())
                    }
                }
                Some("Z") => {
                    // c_wchar_p: NULL -> None, otherwise read wide string -> str
                    if *ptr == 0 {
                        Some(vm.ctx.none())
                    } else {
                        let wstr_ptr = *ptr as *const libc::wchar_t;
                        let mut len = 0;
                        unsafe {
                            while *wstr_ptr.add(len) != 0 {
                                len += 1;
                            }
                        }
                        let slice = unsafe { std::slice::from_raw_parts(wstr_ptr, len) };
                        let s: String = slice
                            .iter()
                            .filter_map(|&c| char::from_u32(c as u32))
                            .collect();
                        Some(vm.ctx.new_str(s).into())
                    }
                }
                _ => {
                    // c_void_p ("P") and other pointer types: NULL -> None, otherwise int
                    if *ptr == 0 {
                        Some(vm.ctx.none())
                    } else {
                        Some(vm.ctx.new_int(*ptr).into())
                    }
                }
            }
        }
        RawResult::Value(val) => call_info
            .restype_obj
            .as_ref()
            .and_then(|f| f.clone().downcast::<PyType>().ok())
            .map(|f| {
                f.from_ffi_type(val as *mut _ as *mut c_void, vm)
                    .ok()
                    .flatten()
            })
            .unwrap_or_else(|| Some(vm.ctx.new_int(*val as usize).as_object().to_pyobject(vm))),
    }
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
    prepared: PreparedArgs,
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
    if prepared.out_buffers.is_empty() {
        return result.map(Ok).unwrap_or_else(|| Ok(vm.ctx.none()));
    }

    let out_values = extract_out_values(prepared.out_buffers, vm);
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
        let prepared = build_callargs(&args, &call_info, paramflags.as_ref(), is_com_method, vm)?;

        // 6. Get code pointer
        let code_ptr = match func_ptr.or_else(|| zelf.get_code_ptr()) {
            Some(cp) => cp,
            None => {
                debug_assert!(false, "NULL function pointer");
                // In release mode, this will crash like CPython
                CodePtr(std::ptr::null_mut())
            }
        };

        // 7. Call the function
        let raw_result = ctypes_callproc(code_ptr, &prepared, &call_info);

        // 8. Build result
        build_result(raw_result, &call_info, prepared, zelf, &args, vm)
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

#[pyclass(flags(BASETYPE), with(Callable, Constructor, Representable))]
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
        use super::base::StgInfoFlags;
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
    res_type: Option<PyTypeRef>,
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
                    let cstr = std::ffi::CStr::from_ptr(cstr_ptr);
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
                    let slice = std::slice::from_raw_parts(wstr_ptr, len);
                    let s: String = slice
                        .iter()
                        .filter_map(|&c| char::from_u32(c as u32))
                        .collect();
                    vm.ctx.new_str(s).into()
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
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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

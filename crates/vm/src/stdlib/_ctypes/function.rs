// spell-checker:disable
#![allow(unreachable_pub)]

use crate::object::{Traverse, TraverseFn};

use super::{
    _ctypes::CArgObject,
    PyCArray, PyCData, PyCPointer, PyCStructure, PyCUnion, StgInfo,
    base::{CArgValue, CDATA_BUFFER_METHODS, ParamFunc, StgInfoFlags},
    simple::PyCSimple,
};
use crate::{
    AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    builtins::{PyBaseExceptionRef, PyBytes, PyDict, PyInt, PyStr, PyTuple, PyType, PyTypeRef},
    class::StaticType,
    function::FuncArgs,
    protocol::{BufferDescriptor, PyBuffer, PyNumberMethods, PySequenceMethods},
    types::{AsBuffer, AsNumber, AsSequence, Callable, Constructor, Initializer, Representable},
    vm::thread::with_current_vm,
};
use alloc::borrow::Cow;
use core::ffi::{c_int, c_long, c_ulong, c_void};
use core::fmt::Debug;
use core::ptr::NonNull;
use num_traits::{Signed, ToPrimitive};
use rustpython_common::lock::PyRwLock;
#[cfg(windows)]
use rustpython_host_env::ctypes::ComMethodError;
use rustpython_host_env::ctypes::{
    CTypeLayout, CallError, CallOptions, CallRet, CallValue, FfiCif, FfiCodePtr, FfiType,
    RawMemoryView, RawMemoryViewError, StringAtError, call, ffi_pointer_type, ffi_type_from_code,
    ffi_void_type, has_pointer_width, pointer_bytes, pointer_format, pointer_size,
    simple_type_is_pointer, write_pointer_to_buffer_at, write_prefix_limited,
};

// Internal function addresses for special ctypes functions
pub(super) const INTERNAL_CAST_ADDR: usize = 1;
pub(super) const INTERNAL_STRING_AT_ADDR: usize = 2;
pub(super) const INTERNAL_WSTRING_AT_ADDR: usize = 3;
pub(super) const INTERNAL_MEMORYVIEW_AT_ADDR: usize = 4;

// PyCFuncPtr - Function pointer implementation

/// Convert any object to a pointer value for c_void_p arguments
/// Follows ConvParam logic for pointer types
fn convert_to_pointer(
    value: &PyObject,
    vm: &VirtualMachine,
) -> PyResult<(CArgValue, Option<PyObjectRef>)> {
    let pointer = |address| Ok((CArgValue::pointer(address), Some(value.to_owned())));
    // 0. CArgObject (from byref()) -> buffer address + offset
    if let Some(carg) = value.downcast_ref::<CArgObject>() {
        return Ok((carg.value.clone(), Some(value.to_owned())));
    }

    // 1. None -> NULL
    if value.is(&vm.ctx.none) {
        return Ok((CArgValue::pointer(0), None));
    }

    // 2. PyCArray -> buffer address (PyCArrayType_paramfunc)
    if let Some(array) = value.downcast_ref::<PyCArray>() {
        let addr = array.0.buffer.read().as_ptr() as usize;
        return pointer(addr);
    }

    // 3. PyCPointer -> stored pointer value
    if let Some(ptr) = value.downcast_ref::<PyCPointer>() {
        return pointer(ptr.get_ptr_value());
    }

    // 4. PyCStructure -> buffer address
    if let Some(struct_obj) = value.downcast_ref::<PyCStructure>() {
        let addr = struct_obj.0.buffer.read().as_ptr() as usize;
        return pointer(addr);
    }

    // 5. PyCSimple (c_void_p, c_char_p, etc.) -> value from buffer
    if let Some(simple) = value.downcast_ref::<PyCSimple>() {
        let buffer = simple.0.buffer.read();
        if has_pointer_width(&buffer) {
            let addr = rustpython_host_env::ctypes::read_pointer_from_buffer(&buffer);
            return pointer(addr);
        }
    }

    // 6. bytes -> owned, null-terminated buffer
    if let Some(bytes) = value.downcast_ref::<crate::builtins::PyBytes>() {
        let (owner, address) = super::base::ensure_z_null_terminated(bytes, vm);
        return Ok((CArgValue::pointer(address), Some(owner)));
    }

    // 7. Integer -> direct value (PyLong_AsVoidPtr behavior)
    if let Ok(int_val) = value.try_int(vm) {
        let bigint = int_val.as_bigint();
        // Negative values: use signed conversion (allows -1 as 0xFFFF...)
        if bigint.is_negative() {
            if let Some(signed_val) = bigint.to_isize() {
                return pointer(signed_val as usize);
            }
        } else if let Some(unsigned_val) = bigint.to_usize() {
            return pointer(unsigned_val);
        }
        // Value out of range - raise OverflowError
        return Err(vm.new_overflow_error("int too large to convert to pointer"));
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
/// Returns an Argument with value and optional keep object
fn conv_param(value: &PyObject, vm: &VirtualMachine) -> PyResult<Argument> {
    // 1. CArgObject (from byref() or paramfunc) -> use stored value
    if let Some(carg) = value.downcast_ref::<CArgObject>() {
        return Ok(Argument {
            // Keep the wrapper, which owns both the source and any auxiliary
            // pointer storage. `_as_parameter_` may have created it on demand.
            keep: Some(value.to_owned()),
            value: carg.value.clone(),
        });
    }

    // 2. None -> NULL pointer
    if value.is(&vm.ctx.none) {
        return Ok(Argument {
            keep: None,
            value: CArgValue::pointer(0),
        });
    }

    // 3. ctypes objects -> use paramfunc
    if let Ok(carg) = super::base::call_paramfunc(value, vm) {
        let value = carg.value.clone();
        return Ok(Argument {
            keep: Some(carg.into_ref(&vm.ctx).into()),
            value,
        });
    }

    // 4. Python str -> wide string pointer (like PyUnicode_AsWideCharString)
    if let Some(s) = value.downcast_ref::<PyStr>() {
        let wide_bytes: Vec<u8> = rustpython_host_env::ctypes::utf16z_bytes(s.as_wtf8());
        let keep = vm.ctx.new_bytes(wide_bytes);
        let addr = keep.as_bytes().as_ptr() as usize;
        return Ok(Argument {
            keep: Some(keep.into()),
            value: CArgValue::pointer(addr),
        });
    }

    // 9. Python bytes -> null-terminated buffer pointer
    // Need to ensure null termination like c_char_p
    if let Some(bytes) = value.downcast_ref::<PyBytes>() {
        let buffer = rustpython_host_env::ctypes::clone_as_null_terminated(bytes.as_bytes());
        let keep = vm.ctx.new_bytes(buffer);
        let addr = keep.as_bytes().as_ptr() as usize;
        return Ok(Argument {
            keep: Some(keep.into()),
            value: CArgValue::pointer(addr),
        });
    }

    // 10. Python int -> i32 (default integer type)
    // PyLong_Check: only an int (or a subclass) converts. Going through
    // `__int__` would accept a float and pass its truncated value where the
    // callee expects a pointer.
    if let Some(int_val) = value.downcast_ref::<PyInt>() {
        // CPython first accepts the platform's unsigned long / signed long
        // range, then stores the low C-int bits in the argument carrier.
        let bigint = int_val.as_bigint();
        #[allow(
            clippy::useless_conversion,
            clippy::unnecessary_cast,
            reason = "C long is 32 bits on Windows and 64 bits on LP64 targets"
        )]
        let val = if bigint.is_negative() {
            bigint
                .to_i64()
                .and_then(|value| c_long::try_from(value).ok())
                .map(|value| value as c_int)
        } else {
            bigint
                .to_u64()
                .and_then(|value| c_ulong::try_from(value).ok())
                .map(|value| value as c_int)
        }
        .ok_or_else(|| vm.new_overflow_error("int too long to convert"))?;
        return Ok(Argument {
            keep: None,
            value: CArgValue::Int(val),
        });
    }

    // 11. Check _as_parameter_ attribute
    if let Ok(as_param) = value.get_attr("_as_parameter_", vm) {
        return conv_param(&as_param, vm);
    }

    Err(vm.new_type_error(format!(
        "Don't know how to convert parameter {}",
        value.class().name()
    )))
}

fn wrap_arg_error(
    error: PyBaseExceptionRef,
    index: usize,
    vm: &VirtualMachine,
) -> PyBaseExceptionRef {
    let Ok(module) = vm.import("_ctypes", 0) else {
        return error;
    };
    let Ok(arg_err_attr) = module.get_attr("ArgumentError", vm) else {
        return error;
    };
    let Ok(error_type) = arg_err_attr.downcast::<PyType>() else {
        return error;
    };
    let msg_str = error
        .as_object()
        .str(vm)
        .map_or_else(|_| String::new(), |s| s.to_string());
    let msg = format!(
        "argument {}: {}: {msg_str}",
        index + 1,
        error.class().name()
    );
    let exc = vm.new_exception_msg(error_type, msg.into());
    exc.set_cause(Some(error));
    exc
}

fn conv_param_for_call(value: &PyObject, index: usize, vm: &VirtualMachine) -> PyResult<Argument> {
    match conv_param(value, vm) {
        Ok(argument) => Ok(argument),
        Err(error) => Err(wrap_arg_error(error, index, vm)),
    }
}

fn convert_arg_for_call(
    converter: &PyObject,
    arg_type: &PyObject,
    arg: PyObjectRef,
    index: usize,
    vm: &VirtualMachine,
) -> PyResult<Argument> {
    match convert_object(converter, arg_type, arg, vm) {
        Ok((value, keep)) => Ok(Argument { value, keep }),
        Err(error) => Err(wrap_arg_error(error, index, vm)),
    }
}

fn convert_object(
    converter: &PyObject,
    arg_type: &PyObject,
    value: PyObjectRef,
    vm: &VirtualMachine,
) -> PyResult<(CArgValue, Option<PyObjectRef>)> {
    // Call the cached from_param converter directly
    let converted = converter.call((value,), vm)?;

    // Then pass the converted value to ConvParam logic
    // 1. CArgObject (from from_param or byref()) -> use stored value and keepalive directly
    if let Some(carg) = converted.downcast_ref::<CArgObject>() {
        return Ok((carg.value.clone(), Some(converted)));
    }

    // 2. None -> NULL pointer
    if vm.is_none(&converted) {
        return Ok((CArgValue::pointer(0), None));
    }

    // 3. PyCFuncPtr
    if converted.class().fast_issubclass(PyCFuncPtr::static_type()) {
        let argument = conv_param(&converted, vm)?;
        return Ok((argument.value, argument.keep));
    }

    // 4. PyCPointer: pass pointer VALUE stored in buffer
    if converted.class().fast_issubclass(PyCPointer::static_type()) {
        if let Some(pointer) = converted.downcast_ref::<PyCPointer>() {
            return Ok((CArgValue::pointer(pointer.get_ptr_value()), Some(converted)));
        }
        return convert_to_pointer(&converted, vm);
    }

    // 5. Structure / Union: pass aggregate by value
    if converted
        .class()
        .fast_issubclass(PyCStructure::static_type())
        || converted.class().fast_issubclass(PyCUnion::static_type())
    {
        if let Some(cdata) = converted.downcast_ref::<PyCData>() {
            let bytes = cdata.buffer.read().to_vec();
            let layout = converted.class().stg_info_opt().map_or_else(
                || CTypeLayout::Opaque { size: bytes.len() },
                |stg| super::base::type_layout(&stg),
            );
            // Keep the converted instance alive through the call: the
            // snapshot may embed pointers into buffers its keep-alive set
            // owns, which must outlive the foreign call.
            return Ok((CArgValue::aggregate(layout, bytes), Some(converted)));
        }
        return convert_to_pointer(&converted, vm);
    }

    // 6. PyCArray -> buffer address
    if let Some(array) = converted.downcast_ref::<PyCArray>() {
        let addr = array.0.buffer.read().as_ptr() as usize;
        return Ok((CArgValue::pointer(addr), Some(converted)));
    }

    // 7. PyCSimple -> simple ctypes value or pointer
    if let Ok(simple) = converted.clone().downcast::<PyCSimple>() {
        let code = converted.class().type_code(vm).or_else(|| {
            arg_type
                .downcast_ref::<PyType>()
                .and_then(|t| t.type_code(vm))
        });
        if matches!(code.as_deref(), Some("P" | "z" | "Z")) {
            return convert_to_pointer(&converted, vm);
        }
        if let Some(first_char) = code.as_deref().and_then(|s| s.chars().next()) {
            return Ok((simple.to_carg_value(first_char), Some(simple.into())));
        }
    }

    // 8. If arg_type requested a pointer type, try convert_to_pointer
    if let Some(self_ty) = arg_type.downcast_ref::<PyType>() {
        if self_ty.fast_issubclass(PyCPointer::static_type()) {
            return convert_to_pointer(&converted, vm);
        }
        if let Some(code) = self_ty.type_code(vm)
            && matches!(code.as_str(), "P" | "z" | "Z")
        {
            return convert_to_pointer(&converted, vm);
        }
    }

    // 9. Fallback to ConvParam (int, str, bytes, _as_parameter_, etc.)
    let argument = conv_param(&converted, vm)?;
    let keep = match argument.keep {
        Some(inner) => Some(vm.ctx.new_tuple(vec![converted, inner]).into()),
        None => Some(converted),
    };
    Ok((argument.value, keep))
}

// PyCFuncPtrType - Metaclass for function pointer types
// PyCFuncPtrType_init

#[pyclass(name = "PyCFuncPtrType", base = PyType, module = "_ctypes")]
#[derive(Debug)]
#[repr(transparent)]
pub(super) struct PyCFuncPtrType(PyType);

impl Initializer for PyCFuncPtrType {
    type Args = FuncArgs;

    fn init(zelf: &Py<Self>, _args: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
        let obj: PyObjectRef = zelf.to_owned().into();
        let new_type: PyTypeRef = obj
            .downcast()
            .map_err(|_| vm.new_type_error("expected type"))?;

        new_type.check_not_initialized(vm)?;

        let ptr_size = pointer_size();
        let mut stg_info = StgInfo::new(ptr_size, ptr_size);
        stg_info.format = Some("X{}".to_string());
        stg_info.length = 1;
        stg_info.flags |= StgInfoFlags::TYPEFLAG_ISPOINTER;
        stg_info.paramfunc = ParamFunc::Pointer; // CFuncPtr is passed as a pointer

        let _ = new_type.init_traced_type_data(stg_info);
        Ok(())
    }
}

#[pyclass(flags(IMMUTABLETYPE), with(AsSequence, Initializer))]
impl PyCFuncPtrType {
    #[pymethod]
    fn from_param(zelf: PyTypeRef, value: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if value.is_instance(zelf.as_object(), vm)? {
            return Ok(value);
        }

        if let Some(as_parameter) = vm.get_attribute_opt(&value, "_as_parameter_")? {
            return vm.with_recursion(" while processing _as_parameter_", || {
                Self::from_param(zelf, as_parameter, vm)
            });
        }

        Err(vm.new_type_error(format!(
            "expected {} instance instead of {}",
            zelf.name(),
            value.class().name()
        )))
    }

    #[pygetset(name = "__pointer_type__")]
    fn pointer_type(zelf: PyTypeRef, vm: &VirtualMachine) -> PyResult {
        super::base::pointer_type_get(&zelf, vm)
    }

    #[pygetset(name = "__pointer_type__", setter)]
    fn set_pointer_type(zelf: PyTypeRef, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        super::base::pointer_type_set(&zelf, value, vm)
    }
}

// Saved in _base.buffer
#[pyclass(
    module = "_ctypes",
    name = "CFuncPtr",
    base = PyCData,
    metaclass = "PyCFuncPtrType",
    traverse = "manual"
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

// SAFETY: each stored reference is an independent strong edge, including
// thunk when it is also retained by _base.objects.
unsafe impl Traverse for PyCFuncPtr {
    fn traverse(&self, tracer: &mut TraverseFn<'_>) {
        self._base.traverse(tracer);
        self.thunk.traverse(tracer);
        self.callable.traverse(tracer);
        self.converters.traverse(tracer);
        self.argtypes.traverse(tracer);
        self.restype.traverse(tracer);
        self.checker.traverse(tracer);
        self.errcheck.traverse(tracer);
        self.paramflags.traverse(tracer);
        #[cfg(windows)]
        self.iid.traverse(tracer);
    }

    fn clear(&mut self, out: &mut Vec<PyObjectRef>) {
        self._base.clear(out);
        if let Some(mut value) = self.thunk.try_write() {
            out.extend(value.take().map(Into::into));
        }
        if let Some(mut value) = self.callable.try_write() {
            out.extend(value.take());
        }
        if let Some(mut value) = self.converters.try_write() {
            out.extend(value.take());
        }
        if let Some(mut value) = self.argtypes.try_write() {
            out.extend(value.take());
        }
        if let Some(mut value) = self.restype.try_write() {
            out.extend(value.take());
        }
        if let Some(mut value) = self.checker.try_write() {
            out.extend(value.take());
        }
        if let Some(mut value) = self.errcheck.try_write() {
            out.extend(value.take());
        }
        if let Some(mut value) = self.paramflags.try_write() {
            out.extend(value.take());
        }
        #[cfg(windows)]
        if let Some(mut value) = self.iid.try_write() {
            out.extend(value.take());
        }
    }
}

impl Debug for PyCFuncPtr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PyCFuncPtr")
            .field("func_ptr", &self.get_func_ptr())
            .finish()
    }
}

// c_void_p accepts only P-tagged argument wrappers.
fn carg_pointer(arg: &PyObject) -> Option<usize> {
    let carg = arg.downcast_ref::<CArgObject>()?;
    if carg.tag != b'P' {
        return None;
    }
    match &carg.value {
        CArgValue::Pointer(address) => Some(*address),
        CArgValue::Typed { code: 'P', bytes } => {
            Some(rustpython_host_env::ctypes::read_pointer_from_buffer(bytes))
        }
        _ => None,
    }
}

/// Extract pointer value from a ctypes argument (c_void_p conversion)
fn extract_ptr_from_arg(arg: &PyObject, vm: &VirtualMachine) -> PyResult<usize> {
    // byref() has already stored the address, including its byte offset.
    if let Some(address) = carg_pointer(arg) {
        return Ok(address);
    }
    // Try to get pointer value from various ctypes types
    if let Some(ptr) = arg.downcast_ref::<PyCPointer>() {
        return Ok(ptr.get_ptr_value());
    }
    if let Some(simple) = arg.downcast_ref::<PyCSimple>() {
        let buffer = simple.0.buffer.read();
        if buffer.first_chunk::<{ size_of::<usize>() }>().is_some() {
            return Ok(rustpython_host_env::ctypes::read_pointer_from_buffer(
                &buffer,
            ));
        }
    }
    if let Some(cdata) = arg.downcast_ref::<PyCData>() {
        // For arrays/structures, return address of buffer
        return Ok(cdata.buffer.read().as_ptr() as usize);
    }
    // PyStr: return internal buffer address
    if let Some(s) = arg.downcast_ref::<PyStr>() {
        return Ok(s.as_bytes().as_ptr() as usize);
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
    match rustpython_host_env::ctypes::string_at(ptr, size) {
        Ok(bytes) => Ok(vm.ctx.new_bytes(bytes).into()),
        Err(StringAtError::NullPointer) => Err(vm.new_value_error("NULL pointer access")),
        Err(StringAtError::TooLong) => Err(vm.new_overflow_error("string too long")),
    }
}

/// wstring_at implementation - read wide string from memory at ptr
fn wstring_at_impl(ptr: usize, size: isize, vm: &VirtualMachine) -> PyResult {
    match rustpython_host_env::ctypes::wstring_at(ptr, size) {
        Ok(text) => Ok(vm.ctx.new_str(text).into()),
        Err(StringAtError::NullPointer) => Err(vm.new_value_error("NULL pointer access")),
        Err(StringAtError::TooLong) => Err(vm.new_overflow_error("string too long")),
    }
}

// A buffer wrapping raw memory at a given pointer, for zero-copy memoryview.
#[pyclass(name = "_RawMemoryBuffer", module = "_ctypes")]
#[derive(Debug, PyPayload)]
pub(super) struct RawMemoryBuffer {
    memory: RawMemoryView,
}

static RAW_MEMORY_BUFFER_METHODS: crate::protocol::BufferMethods = crate::protocol::BufferMethods {
    obj_bytes: |buffer| {
        let raw = buffer.obj_as::<RawMemoryBuffer>();
        let slice = unsafe { raw.memory.bytes() };
        rustpython_common::borrow::BorrowedValue::Ref(slice)
    },
    obj_bytes_mut: |buffer| {
        let raw = buffer.obj_as::<RawMemoryBuffer>();
        let slice = unsafe { raw.memory.bytes_mut() };
        rustpython_common::borrow::BorrowedValueMut::RefMut(slice)
    },
    release: |_| {},
    retain: |_| {},
};

#[pyclass(with(AsBuffer))]
impl RawMemoryBuffer {}

impl AsBuffer for RawMemoryBuffer {
    fn as_buffer(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<PyBuffer> {
        Ok(PyBuffer::new(
            zelf.to_owned().into(),
            BufferDescriptor::simple(zelf.memory.size(), zelf.memory.readonly()),
            &RAW_MEMORY_BUFFER_METHODS,
        ))
    }
}

/// memoryview_at implementation - create a memoryview from memory at ptr
fn memoryview_at_impl(ptr: usize, size: isize, readonly: bool, vm: &VirtualMachine) -> PyResult {
    use crate::builtins::PyMemoryView;

    let memory = RawMemoryView::new(ptr, size, readonly).map_err(|err| match err {
        RawMemoryViewError::NullPointer => vm.new_value_error("NULL pointer access"),
        RawMemoryViewError::NegativeSize => vm.new_value_error("negative size"),
    })?;
    let raw_buf = RawMemoryBuffer { memory }.into_pyobject(vm);
    let mv = PyMemoryView::from_object(&raw_buf, vm)?;
    Ok(mv.into_pyobject(vm))
}

// cast_check_pointertype
fn cast_check_pointertype(ctype: &PyObject, vm: &VirtualMachine) -> bool {
    use super::pointer::PyCPointerType;

    // PyCPointerTypeObject_Check
    if ctype.class().fast_issubclass(PyCPointerType::static_type()) {
        return true;
    }

    // PyCFuncPtrTypeObject_Check
    if ctype.class().fast_issubclass(PyCFuncPtrType::static_type()) {
        return true;
    }

    // Simple pointer types retain the representation selected at creation,
    // even if the user later changes the Python-visible _type_ attribute.
    if let Some(ty) = ctype.downcast_ref::<PyType>()
        && let Some(code) = ty.type_code(vm)
    {
        return code.len() == 1 && "sPzUZXO".contains(code.as_str());
    }

    false
}

/// cast implementation
/// _ctypes.c cast()
pub(super) fn cast_impl(
    obj: &PyObject,
    src: PyObjectRef,
    ctype: &PyObject,
    vm: &VirtualMachine,
) -> PyResult {
    // 1. cast_check_pointertype
    if !cast_check_pointertype(ctype, vm) {
        return Err(vm.new_type_error(format!(
            "cast() argument 2 must be a pointer type, not {}",
            ctype.class().name()
        )));
    }

    // 2. Extract pointer value - matches c_void_p_from_param_impl order
    let ptr_value: usize = if vm.is_none(obj) {
        // None → NULL pointer
        0
    } else if let Some(address) = carg_pointer(obj) {
        address
    } else if let Ok(int_val) = obj.try_int(vm) {
        // int/long → direct pointer value
        int_val.as_bigint().to_usize().unwrap_or(0)
    } else if let Some(bytes) = obj.downcast_ref::<PyBytes>() {
        // bytes → buffer address (c_void_p_from_param: PyBytes_Check)
        bytes.as_bytes().as_ptr() as usize
    } else if let Some(s) = obj.downcast_ref::<PyStr>() {
        // unicode/str → buffer address (c_void_p_from_param: PyUnicode_Check)
        s.as_bytes().as_ptr() as usize
    } else if let Some(ptr) = obj.downcast_ref::<PyCPointer>() {
        // Pointer instance → contained pointer value
        ptr.get_ptr_value()
    } else if let Some(simple) = obj.downcast_ref::<PyCSimple>() {
        // Simple type (c_void_p, c_char_p, etc.) → value from buffer
        let buffer = simple.0.buffer.read();
        rustpython_host_env::ctypes::read_pointer_from_buffer(&buffer)
    } else if let Some(funcptr) = obj.downcast_ref::<PyCFuncPtr>() {
        // A function pointer stores the executable address in its buffer.
        funcptr.get_func_ptr()
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
            let _ = dict.set_item(&*id_key, src, vm);
        }

        // Set result's _objects to the shared dict
        if let Some(result_cdata) = result.downcast_ref::<PyCData>() {
            *result_cdata.objects.write() = Some(shared_objects);
        }
    }

    if obj.downcast_ref::<CArgObject>().is_some()
        && let Some(cdata) = result.downcast_ref::<PyCData>()
    {
        // Preserve provenance privately: byref casts have no public _objects.
        cdata.keep_cast_owner(obj.to_owned());
    }

    // 5. Store pointer value
    if let Some(ptr) = result.downcast_ref::<PyCPointer>() {
        ptr.set_ptr_value(ptr_value);
    } else if let Some(cdata) = result.downcast_ref::<PyCData>() {
        let mut buffer = cdata.buffer.write();
        write_pointer_to_buffer_at(buffer.to_mut(), 0, pointer_size(), ptr_value);
    }

    Ok(result)
}

impl PyCFuncPtr {
    pub(super) fn from_cdata(data: PyCData) -> Self {
        Self {
            _base: data,
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
    }

    /// Get function pointer address from buffer
    pub(super) fn get_func_ptr(&self) -> usize {
        let buffer = self._base.buffer.read();
        rustpython_host_env::ctypes::read_pointer_from_buffer(&buffer)
    }

    /// Create buffer with function pointer address
    fn make_ptr_buffer(addr: usize) -> Vec<u8> {
        pointer_bytes(addr)
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

        let ptr_size = pointer_size();

        if args.args.is_empty() {
            return Self::from_cdata(PyCData::from_bytes(vec![0u8; ptr_size], None))
                .into_ref_with_type(vm, cls)
                .map(Into::into);
        }

        let first_arg = &args.args[0];

        let class_argtypes = cls.get_attr(vm.ctx.intern_str("_argtypes_"));
        let class_restype = cls.get_attr(vm.ctx.intern_str("_restype_"));
        let initial_converters = if let Some(ref argtypes) = class_argtypes
            && !vm.is_none(argtypes)
        {
            if let Ok((types, converters)) = extract_arg_types_and_converters(argtypes, vm) {
                let snapshot = vm.ctx.new_tuple(vec![
                    vm.ctx.new_tuple(types).into(),
                    vm.ctx.new_tuple(converters).into(),
                ]);
                Some(snapshot.into())
            } else {
                None
            }
        } else {
            None
        };

        let make_converters = || PyRwLock::new(initial_converters.clone());

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

            return Self {
                _base: PyCData::from_bytes(vec![0u8; ptr_size], None),
                thunk: PyRwLock::new(None),
                callable: PyRwLock::new(None),
                converters: make_converters(),
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
            return Self {
                _base: PyCData::from_bytes(Self::make_ptr_buffer(ptr_val), None),
                thunk: PyRwLock::new(None),
                callable: PyRwLock::new(None),
                converters: make_converters(),
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
                .as_slice()
                .first()
                .ok_or_else(|| vm.new_type_error("Expected a tuple with at least 2 elements"))?
                .downcast_ref::<PyStr>()
                .ok_or_else(|| vm.new_type_error("Expected a string"))?
                .to_string();
            let dll = tuple
                .as_slice()
                .get(1)
                .ok_or_else(|| vm.new_type_error("Expected a tuple with at least 2 elements"))?
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
            let terminated = format!("{name}\0");
            let ptr_val = match rustpython_host_env::ctypes::lookup_function_symbol_addr(
                handle
                    .to_usize()
                    .ok_or_else(|| vm.new_value_error("Invalid handle"))?,
                terminated.as_bytes(),
            ) {
                Ok(addr) => {
                    if addr == 0 {
                        return Err(vm.new_attribute_error(format!("function '{name}' not found")));
                    }
                    addr
                }
                Err(rustpython_host_env::ctypes::LookupSymbolError::LibraryNotFound) => {
                    return Err(vm.new_value_error("Library not found"));
                }
                Err(rustpython_host_env::ctypes::LookupSymbolError::LibraryClosed) => 0,
                Err(rustpython_host_env::ctypes::LookupSymbolError::Load(err)) => {
                    return Err(vm.new_attribute_error(err));
                }
            };

            return Self {
                _base: PyCData::from_bytes(Self::make_ptr_buffer(ptr_val), None),
                thunk: PyRwLock::new(None),
                callable: PyRwLock::new(None),
                converters: make_converters(),
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
            let class_flags = cls
                .get_attr(vm.ctx.intern_str("_flags_"))
                .and_then(|f| f.try_to_value::<u32>(vm).ok())
                .unwrap_or(0);

            // Create the thunk (C-callable wrapper for the Python function)
            let thunk = PyCThunk::new(
                first_arg.clone(),
                class_argtypes.as_deref(),
                class_restype.as_deref(),
                class_flags,
                vm,
            )?;
            let code_ptr = thunk.code_ptr();
            let ptr_val = code_ptr.0 as usize;

            // Store the thunk as a Python object to keep it alive
            let thunk_ref: PyRef<PyCThunk> = thunk.into_ref(&vm.ctx);
            let base = PyCData::from_bytes(Self::make_ptr_buffer(ptr_val), None);
            let objects = vm.ctx.new_dict();
            objects.set_item("0", thunk_ref.clone().into(), vm)?;
            *base.objects.write() = Some(objects.into());

            return Self {
                _base: base,
                thunk: PyRwLock::new(Some(thunk_ref)),
                callable: PyRwLock::new(Some(first_arg.clone())),
                converters: make_converters(),
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
        return Some(result.and_then(|(obj, src, ctype)| cast_impl(&obj, src, &ctype, vm)));
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

    if addr == INTERNAL_MEMORYVIEW_AT_ADDR {
        let result: PyResult<(PyObjectRef, PyObjectRef, Option<PyObjectRef>)> =
            args.clone().bind(vm);
        return Some(result.and_then(|(ptr_arg, size_arg, readonly_arg)| {
            let ptr = extract_ptr_from_arg(&ptr_arg, vm)?;
            let size_int = size_arg.try_int(vm)?;
            let size = size_int
                .as_bigint()
                .to_isize()
                .ok_or_else(|| vm.new_value_error("size too large"))?;
            let readonly = readonly_arg
                .and_then(|r| r.try_int(vm).ok())
                .and_then(|i| i.as_bigint().to_i32())
                .unwrap_or(0)
                != 0;
            memoryview_at_impl(ptr, size, readonly, vm)
        }));
    }

    None
}

/// How the foreign call's return value is retrieved (mirrors `CallRet`).
enum RetSpec {
    /// restype is None: void return.
    Void,
    /// Pointer-valued return (a `TYPEFLAG_ISPOINTER` restype, or an oversized
    /// by-value struct approximated as a pointer-sized register).
    Pointer,
    /// A scalar retrieved as the given ctypes simple-type code.
    Code(char),
    /// A by-value aggregate (struct/union) return with the given call layout.
    Aggregate(CTypeLayout),
}

/// Call information extracted from PyCFuncPtr (argtypes, restype, etc.)
struct CallInfo {
    explicit_arg_types: Option<Vec<PyObjectRef>>,
    explicit_converters: Option<Vec<PyObjectRef>>,
    restype_obj: Option<PyObjectRef>,
    ret: RetSpec,
}

fn extract_arg_types_and_converters(
    argtypes: &PyObject,
    vm: &VirtualMachine,
) -> PyResult<(Vec<PyObjectRef>, Vec<PyObjectRef>)> {
    let error = || vm.new_type_error("_argtypes_ must be a sequence of types");
    let sequence = argtypes.try_sequence(vm).map_err(|_| error())?;
    let length = sequence.length(vm).map_err(|_| error())?;
    let mut types = Vec::new();
    let mut converters = Vec::new();
    types
        .try_reserve(length)
        .map_err(|_| vm.no_memory_error())?;
    converters
        .try_reserve(length)
        .map_err(|_| vm.no_memory_error())?;

    for index in 0..length {
        let item = sequence.get_item(index as isize, vm).map_err(|_| error())?;
        match item.get_attr(vm.ctx.intern_str("from_param"), vm) {
            Ok(from_param) => {
                types.push(item);
                converters.push(from_param);
            }
            Err(e) if e.fast_isinstance(vm.ctx.exceptions.attribute_error) => {
                return Err(vm.new_type_error(format!(
                    "item {} in _argtypes_ has no from_param method",
                    index + 1
                )));
            }
            Err(e) => return Err(e),
        }
    }

    Ok((types, converters))
}

/// Determine how to retrieve the return value from restype, reproducing the
/// prior `ffi_return_type` + `is_pointer_return` dispatch.
fn compute_ret_spec(
    restype_is_none: bool,
    restype_obj: Option<&PyObject>,
    vm: &VirtualMachine,
) -> RetSpec {
    if restype_is_none {
        return RetSpec::Void;
    }
    let Some(restype_type) = restype_obj.and_then(|t| t.to_owned().downcast::<PyType>().ok())
    else {
        return RetSpec::Code('i');
    };

    // Pointer return via TYPEFLAG_ISPOINTER (c_void_p, c_char_p, c_wchar_p, POINTER(T))
    if restype_type
        .stg_info_opt()
        .is_some_and(|info| info.flags.contains(StgInfoFlags::TYPEFLAG_ISPOINTER))
    {
        return RetSpec::Pointer;
    }

    // Simple type via its initialized scalar converter.
    if let Some(code) = restype_type
        .type_code(vm)
        .filter(|code| ffi_type_from_code(code).is_some())
    {
        return if simple_type_is_pointer(&code) {
            RetSpec::Pointer
        } else {
            RetSpec::Code(code.chars().next().unwrap_or('i'))
        };
    }

    // Structure/Union (StgInfo, no _type_): returned by value as an aggregate.
    // The layout is built from the held guard to avoid re-locking the type.
    if let Some(stg_info) = restype_type.stg_info_opt() {
        return match stg_info.paramfunc {
            ParamFunc::Structure | ParamFunc::Union => {
                RetSpec::Aggregate(super::base::type_layout(&stg_info))
            }
            // Any other aggregate-ish StgInfo without a code: size-approximated
            // register return, as before.
            _ => {
                let size = stg_info.size;
                if size <= 4 {
                    RetSpec::Code('i')
                } else if size <= 8 {
                    RetSpec::Code('q')
                } else {
                    RetSpec::Pointer
                }
            }
        };
    }

    RetSpec::Code('i')
}

/// Extract call information (argtypes, converters, restype) from PyCFuncPtr
fn extract_call_info(zelf: &Py<PyCFuncPtr>, vm: &VirtualMachine) -> PyResult<CallInfo> {
    // Get argtypes & converters - first check if instance has cached converters
    let (instance_argtypes, instance_converters) = {
        let argtypes_guard = zelf.argtypes.read();
        let converters_guard = zelf.converters.read();
        (argtypes_guard.clone(), converters_guard.clone())
    };

    let (explicit_arg_types, explicit_converters) =
        if let Some(converters_obj) = instance_converters.as_ref() {
            if !vm.is_none(converters_obj) {
                let pair = converters_obj.try_sequence(vm)?;
                let types_obj = pair.get_item(0, vm)?;
                let convs_obj = pair.get_item(1, vm)?;

                let seq_types = types_obj.try_sequence(vm)?;
                let len_types = seq_types.length(vm)?;
                let mut types = Vec::with_capacity(len_types);
                for i in 0..len_types {
                    types.push(seq_types.get_item(i as isize, vm)?);
                }

                let seq_conv = convs_obj.try_sequence(vm)?;
                let len_conv = seq_conv.length(vm)?;
                let mut converters = Vec::with_capacity(len_conv);
                for i in 0..len_conv {
                    converters.push(seq_conv.get_item(i as isize, vm)?);
                }
                (Some(types), Some(converters))
            } else {
                (None, None) // argtypes is None -> use ConvParam
            }
        } else if let Some(argtypes_obj) = instance_argtypes.as_ref() {
            if !vm.is_none(argtypes_obj) {
                let (types, converters) = extract_arg_types_and_converters(argtypes_obj, vm)?;
                let current_argtypes = zelf.argtypes.read();
                let mut conv_guard = zelf.converters.write();
                if current_argtypes
                    .as_ref()
                    .is_some_and(|cur| cur.is(argtypes_obj))
                {
                    let snapshot = vm.ctx.new_tuple(vec![
                        vm.ctx.new_tuple(types.clone()).into(),
                        vm.ctx.new_tuple(converters.clone()).into(),
                    ]);
                    *conv_guard = Some(snapshot.into());
                }
                (Some(types), Some(converters))
            } else {
                (None, None) // argtypes is explicitly None -> use ConvParam
            }
        } else if let Some(class_argtypes) = zelf
            .as_object()
            .class()
            .get_attr(vm.ctx.intern_str("_argtypes_"))
            && !vm.is_none(&class_argtypes)
        {
            let (types, converters) = extract_arg_types_and_converters(&class_argtypes, vm)?;
            let current_argtypes = zelf.argtypes.read();
            let mut conv_guard = zelf.converters.write();
            if current_argtypes.is_none() && conv_guard.is_none() {
                let snapshot = vm.ctx.new_tuple(vec![
                    vm.ctx.new_tuple(types.clone()).into(),
                    vm.ctx.new_tuple(converters.clone()).into(),
                ]);
                *conv_guard = Some(snapshot.into());
            }
            (Some(types), Some(converters))
        } else {
            (None, None) // No argtypes -> use ConvParam
        };

    // Get restype - first from instance, then from class's _restype_
    let restype_obj = zelf.restype.read().clone().or_else(|| {
        zelf.as_object()
            .class()
            .get_attr(vm.ctx.intern_str("_restype_"))
    });

    // Check if restype is explicitly None (return void)
    let restype_is_none = restype_obj.as_deref().is_some_and(|t| vm.is_none(t));
    let ret = compute_ret_spec(restype_is_none, restype_obj.as_deref(), vm);

    Ok(CallInfo {
        explicit_arg_types,
        explicit_converters,
        restype_obj,
        ret,
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
                .as_slice()
                .first()
                .and_then(|d| d.try_int(vm).ok())
                .and_then(|i| i.as_bigint().to_u32())
                .unwrap_or(1);
            let name = tuple
                .as_slice()
                .get(1)
                .and_then(|n| n.downcast_ref::<PyStr>().map(|s| s.to_string()));
            let default = tuple.as_slice().get(2).cloned();
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
) -> PyResult<(Option<FfiCodePtr>, bool)> {
    let com_index = zelf.index.read();
    let Some(idx) = *com_index else {
        return Ok((None, false));
    };

    // First arg must be the COM object pointer
    if args.args.is_empty() {
        return Err(vm.new_type_error("COM method requires at least one argument (self)"));
    }

    // Extract COM pointer value from first argument
    let self_arg = &args.args[0];
    let com_ptr = if let Some(simple) = self_arg.downcast_ref::<PyCSimple>() {
        let buffer = simple.0.buffer.read();
        if has_pointer_width(&buffer) {
            rustpython_host_env::ctypes::read_pointer_from_buffer(&buffer)
        } else {
            0
        }
    } else if let Ok(int_val) = self_arg.try_int(vm) {
        int_val.as_bigint().to_usize().unwrap_or(0)
    } else {
        return Err(vm.new_type_error("COM method first argument must be a COM pointer"));
    };

    let code_ptr = match rustpython_host_env::ctypes::resolve_com_vtable_entry(com_ptr, idx) {
        Ok(code_ptr) => code_ptr,
        Err(ComMethodError::NullComPointer) => {
            return Err(vm.new_value_error("NULL COM pointer access"));
        }
        Err(ComMethodError::NullVtablePointer) => {
            return Err(vm.new_value_error("NULL vtable pointer"));
        }
        Err(ComMethodError::NullFunctionPointer) => {
            return Err(vm.new_value_error("NULL function pointer in vtable"));
        }
    };

    Ok((Some(code_ptr), true))
}

/// Single argument for FFI call
// struct argument
struct Argument {
    value: CArgValue,
    #[allow(dead_code)]
    keep: Option<PyObjectRef>, // Object to keep alive during call
}

/// Out buffers for paramflags OUT parameters
type OutBuffers = Vec<(usize, PyObjectRef)>;

/// Get buffer address from a ctypes object
fn get_buffer_addr(obj: &PyObject) -> Option<usize> {
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
fn create_out_buffer(arg_type: &PyObject, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
    // For POINTER(T) types, create T instance (the pointed-to type)
    if let Some(ty) = arg_type.downcast_ref::<PyType>()
        && ty.fast_issubclass(PyCPointer::static_type())
        && let Some(stg_info) = ty.stg_info_opt()
        && let Some(ref proto) = stg_info.proto
    {
        return proto.as_object().call((), vm);
    }
    // Not a pointer type or no proto, create instance directly
    arg_type.call((), vm)
}

/// Build callargs when no argtypes specified (use ConvParam)
fn build_callargs_no_argtypes(
    args: &FuncArgs,
    vm: &VirtualMachine,
) -> PyResult<(Vec<Argument>, OutBuffers)> {
    let arguments: Vec<Argument> = args
        .args
        .iter()
        .enumerate()
        .map(|(index, arg)| conv_param_for_call(arg, index, vm))
        .collect::<PyResult<Vec<_>>>()?;
    Ok((arguments, Vec::new()))
}

/// Build callargs for regular function with argtypes (no paramflags)
fn build_callargs_simple(
    args: &FuncArgs,
    arg_types: &[PyObjectRef],
    converters: &[PyObjectRef],
    allow_variadic: bool,
    vm: &VirtualMachine,
) -> PyResult<(Vec<Argument>, OutBuffers)> {
    if args.args.len() < arg_types.len() || (!allow_variadic && args.args.len() != arg_types.len())
    {
        return Err(vm.new_type_error(format!(
            "this function takes {}{} argument{} ({} given)",
            if allow_variadic { "at least " } else { "" },
            arg_types.len(),
            if arg_types.len() == 1 { "" } else { "s" },
            args.args.len(),
        )));
    }
    let arguments: Vec<Argument> = args
        .args
        .iter()
        .enumerate()
        .map(|(n, arg)| {
            if let (Some(converter), Some(arg_type)) = (converters.get(n), arg_types.get(n)) {
                convert_arg_for_call(converter, arg_type, arg.clone(), n, vm)
            } else {
                conv_param_for_call(arg, n, vm)
            }
        })
        .collect::<PyResult<Vec<_>>>()?;
    Ok((arguments, Vec::new()))
}

/// Build callargs with paramflags (handles IN/OUT parameters)
fn build_callargs_with_paramflags(
    args: &FuncArgs,
    arg_types: &[PyObjectRef],
    converters: &[PyObjectRef],
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
        let converter = converters
            .get(param_idx)
            .ok_or_else(|| vm.new_type_error("paramflags/argtypes mismatch"))?;

        let is_out = (*direction & 2) != 0; // OUT flag
        let is_in = (*direction & 1) != 0 || *direction == 0; // IN flag or default

        if is_out && !is_in {
            // Pure OUT parameter: create buffer, don't consume caller arg
            let buffer = create_out_buffer(arg_type, vm)?;
            let addr = get_buffer_addr(&buffer)
                .ok_or_else(|| vm.new_type_error("Cannot create OUT buffer for this type"))?;
            arguments.push(Argument {
                keep: None,
                value: CArgValue::pointer(addr),
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
                return Err(vm.new_type_error(format!("required argument {param_idx} missing")));
            };

            if is_out {
                // IN|OUT: track for return
                out_buffers.push((param_idx, arg.clone()));
            }
            let argument = convert_arg_for_call(converter, arg_type, arg, param_idx, vm)?;
            arguments.push(argument);
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
    allow_variadic: bool,
    vm: &VirtualMachine,
) -> PyResult<(Vec<Argument>, OutBuffers)> {
    let (Some(arg_types), Some(converters)) = (
        &call_info.explicit_arg_types,
        &call_info.explicit_converters,
    ) else {
        // No argtypes: use ConvParam
        return build_callargs_no_argtypes(args, vm);
    };

    if let Some(pflags) = paramflags {
        // Has paramflags: handle IN/OUT
        build_callargs_with_paramflags(args, arg_types, converters, pflags, is_com_method, vm)
    } else if is_com_method {
        // COM method without paramflags
        let mut arguments = Vec::new();
        if !args.args.is_empty() {
            arguments.push(conv_param(&args.args[0], vm)?);
        }
        for (n, arg) in args.args.iter().skip(1).enumerate() {
            let converter = converters
                .get(n)
                .ok_or_else(|| vm.new_type_error("argument amount mismatch"))?;
            let arg_type = arg_types
                .get(n)
                .ok_or_else(|| vm.new_type_error("argument amount mismatch"))?;
            let argument = convert_arg_for_call(converter, arg_type, arg.clone(), n + 1, vm)?;
            arguments.push(argument);
        }
        Ok((arguments, Vec::new()))
    } else {
        // Regular function
        build_callargs_simple(args, arg_types, converters, allow_variadic, vm)
    }
}

/// Execute the foreign call through the unified `call` entry point.
fn ctypes_callproc(
    addr: usize,
    arguments: &[Argument],
    ret: &RetSpec,
    options: CallOptions,
) -> Result<CallValue, CallError> {
    // Encode each simple-type code into its own buffer so the `&str` borrowed
    // by `CallArg::Typed` outlives the call.
    let mut code_bufs = vec![[0u8; 4]; arguments.len()];
    let call_args: Vec<_> = arguments
        .iter()
        .zip(code_bufs.iter_mut())
        .map(|(arg, code_buf)| arg.value.as_call_arg(code_buf))
        .collect();

    let mut ret_code_buf = [0u8; 4];
    let call_ret = match ret {
        RetSpec::Void => CallRet::Void,
        RetSpec::Pointer => CallRet::Pointer,
        RetSpec::Code(code) => CallRet::Code(code.encode_utf8(&mut ret_code_buf)),
        RetSpec::Aggregate(layout) => CallRet::Aggregate(layout),
    };

    call(addr, &call_args, call_ret, options)
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
        let exc = vm.invoke_exception(&com_error_type, vec![text.to_owned(), details.clone()])?;
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

/// Convert the foreign-call result to a Python object
// = GetResult
fn convert_raw_result(
    result: &CallValue,
    call_info: &CallInfo,
    vm: &VirtualMachine,
) -> PyResult<Option<PyObjectRef>> {
    // Result register image as bytes + size (None for void): pointer/scalar
    // returns are pointer/register sized.
    let (result_bytes, result_size) = match result {
        CallValue::Void => return Ok(None),
        CallValue::Pointer(ptr) => (ptr.to_ne_bytes().to_vec(), size_of::<usize>()),
        CallValue::Scalar(bytes) | CallValue::Aggregate(bytes) => (bytes.clone(), bytes.len()),
    };

    // Integer view of the return register, for the fallback branches below.
    let result_word: usize = match result {
        CallValue::Pointer(ptr) => *ptr,
        CallValue::Scalar(bytes) | CallValue::Aggregate(bytes) => {
            let mut word = [0u8; size_of::<usize>()];
            let n = bytes.len().min(word.len());
            word[..n].copy_from_slice(&bytes[..n]);
            usize::from_ne_bytes(word)
        }
        CallValue::Void => 0,
    };

    // 1. No restype → return as int
    let restype = match &call_info.restype_obj {
        None => {
            // Default: return as int
            return Ok(Some(vm.ctx.new_int(result_word as isize).into()));
        }
        Some(r) => r,
    };

    // 2. restype is None → return None
    if restype.is(&vm.ctx.none()) {
        return Ok(None);
    }

    // 3. Get restype as PyType
    let restype_type = match restype.clone().downcast::<PyType>() {
        Ok(t) => t,
        Err(_) => {
            // Not a type, call it with int result
            return restype.call((result_word as isize,), vm).map(Some);
        }
    };

    // 4. Get StgInfo
    let stg_info = restype_type.stg_info_opt();

    // No StgInfo → call restype with int
    if stg_info.is_none() {
        return restype_type
            .as_object()
            .call((result_word as isize,), vm)
            .map(Some);
    }

    let info = stg_info.unwrap();
    // Extract what's needed and release the read guard before constructing any
    // instance below: instance construction write-locks the type's StgInfo (to
    // finalize it), which would self-deadlock against a held read guard.
    let is_pointer_type = info.flags.contains(StgInfoFlags::TYPEFLAG_ISPOINTER);
    let has_proto = info.proto.is_some();
    let scalar_code = info.scalar_code;
    drop(info);

    // py_object: interpret return value as PyObject* and materialize it.
    if scalar_code == Some('O') {
        let ptr = NonNull::new(result_word as *mut PyObject)
            .ok_or_else(|| vm.new_value_error("PyObject is NULL"))?;
        unsafe {
            let obj = PyObjectRef::from_raw(ptr);
            return Ok(Some(obj));
        }
    }

    // 5. Simple type with getfunc → use bytes_to_pyobject (info->getfunc)
    // is_simple_instance returns TRUE for c_int, c_void_p, etc.
    if super::base::is_simple_instance(&restype_type) {
        return Ok(Some(super::base::bytes_to_pyobject(
            &restype_type,
            &result_bytes,
            vm,
        )));
    }

    // 6. Complex type → create ctypes instance (PyCData_FromBaseObj)
    // This handles POINTER(T), Structure, Array, etc.

    // Special handling for POINTER(T) types - set pointer value directly
    if is_pointer_type
        && has_proto
        && let CallValue::Pointer(ptr) = result
    {
        let instance = restype_type.as_object().call((), vm)?;
        if let Some(pointer) = instance.downcast_ref::<PyCPointer>() {
            pointer.set_ptr_value(*ptr);
        }
        return Ok(Some(instance));
    }

    // Create instance and copy result data
    pycdata_from_ffi_result(&restype_type, &result_bytes, result_size, vm).map(Some)
}

/// Create a ctypes instance from FFI result (PyCData_FromBaseObj equivalent)
fn pycdata_from_ffi_result(
    typ: &Py<PyType>,
    result_bytes: &[u8],
    size: usize,
    vm: &VirtualMachine,
) -> PyResult {
    // Create instance
    let instance = PyType::call(typ, ().into(), vm)?;

    // Copy result data into instance buffer
    if let Some(cdata) = instance.downcast_ref::<PyCData>() {
        let mut buffer = cdata.buffer.write();
        write_prefix_limited(buffer.to_mut(), result_bytes, size);
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
    call_result: CallValue,
    call_info: &CallInfo,
    out_buffers: OutBuffers,
    zelf: &Py<PyCFuncPtr>,
    args: &FuncArgs,
    vm: &VirtualMachine,
) -> PyResult {
    // Check HRESULT on Windows
    #[cfg(windows)]
    if let CallValue::Scalar(bytes) = &call_result {
        let is_hresult = call_info
            .restype_obj
            .as_ref()
            .and_then(|t| t.to_owned().downcast::<PyType>().ok())
            .is_some_and(|t| t.name().to_string() == "HRESULT");
        if is_hresult {
            let mut word = [0u8; size_of::<usize>()];
            let n = bytes.len().min(word.len());
            word[..n].copy_from_slice(&bytes[..n]);
            check_hresult(usize::from_ne_bytes(word) as i32, zelf, vm)?;
        }
    }

    // Convert the foreign-call result to a Python object
    let mut result = convert_raw_result(&call_result, call_info, vm)?;

    // Apply errcheck if set
    if let Some(errcheck) = zelf.errcheck.read().as_ref() {
        let args_tuple = PyTuple::new_ref(args.args.clone(), &vm.ctx);
        let func_obj = zelf.as_object().to_owned();
        let result_obj = result.clone().unwrap_or_else(|| vm.ctx.none());
        let checked = errcheck.call((result_obj, func_obj, args_tuple.clone()), vm)?;
        // Returning the original args tuple requests normal result processing.
        if !checked.is(&args_tuple) {
            result = Some(checked);
        }
    }

    // Handle OUT parameter return values
    if out_buffers.is_empty() {
        return result.map_or_else(|| Ok(vm.ctx.none()), Ok);
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
        let (func_ptr, is_com_method) = (None::<FfiCodePtr>, false);

        // 3. Extract call info (argtypes, restype)
        let call_info = extract_call_info(zelf, vm)?;

        // 4. Parse paramflags
        let paramflags = parse_paramflags(zelf, vm)?;

        // 5. Build call arguments
        let flags = Py::<Self>::_flags_(zelf, vm);
        let allow_variadic = flags & StgInfoFlags::FUNCFLAG_CDECL.bits() != 0;
        let (arguments, out_buffers) = build_callargs(
            &args,
            &call_info,
            paramflags.as_ref(),
            is_com_method,
            allow_variadic,
            vm,
        )?;

        // 6. Function address (usize); the unified `call` rejects a NULL address.
        let addr = match func_ptr {
            Some(cp) => cp.0 as usize,
            None => zelf.get_func_ptr(),
        };

        // 7. Errno / last-error swap options from flags
        let options = CallOptions {
            fixed_arg_count: call_info
                .explicit_arg_types
                .as_ref()
                .map(Vec::len)
                .filter(|&fixed| allow_variadic && fixed != 0 && arguments.len() > fixed),
            use_errno: flags & super::base::StgInfoFlags::FUNCFLAG_USE_ERRNO.bits() != 0,
            use_last_error: flags & super::base::StgInfoFlags::FUNCFLAG_USE_LASTERROR.bits() != 0,
        };

        // Native calls may block while another thread runs Python. Callbacks
        // reattach before executing Python; PythonAPI calls stay attached.
        let native_call = || ctypes_callproc(addr, &arguments, &call_info.ret, options);
        let call_result = if flags & StgInfoFlags::FUNCFLAG_PYTHONAPI.bits() != 0 {
            native_call()
        } else {
            vm.allow_threads(native_call)
        }
        .map_err(|err| match err {
            CallError::NullFunctionPointer => vm.new_value_error("NULL function pointer"),
            CallError::InvalidCallInterface { variadic } => vm.new_runtime_error(if variadic {
                "ffi_prep_cif_var failed"
            } else {
                "ffi_prep_cif failed"
            }),
            CallError::UnknownTypeCode(code) => {
                vm.new_type_error(format!("Unsupported argument type: {code}"))
            }
            CallError::BufferTooSmall { expected, got } => vm.new_value_error(format!(
                "argument buffer too small: expected {expected}, got {got}"
            )),
        })?;

        // PythonAPI functions report errors through the native raised-error
        // slot. Check it before a NULL PyObject* can be decoded, and consume
        // the exception exactly once without touching handled exceptions.
        if flags & StgInfoFlags::FUNCFLAG_PYTHONAPI.bits() != 0
            && let Some(exception) = vm.take_raised_exception()
        {
            return Err(exception);
        }

        // 9. Build result
        build_result(call_result, &call_info, out_buffers, zelf, &args, vm)
    }
}

// PyCFuncPtr_repr
impl Representable for PyCFuncPtr {
    fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
        let type_name = zelf.class().name();
        // Use object id, not function pointer address
        let addr = zelf.get_id();
        Ok(format!("<{type_name} object at {addr:#x}>"))
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
                    .map_or(Cow::Borrowed("X{}"), Cow::Owned),
                stg_info.size,
            )
        } else {
            (Cow::Borrowed(pointer_format()), pointer_size())
        };
        let desc = BufferDescriptor::from_dim_desc(itemsize, 0, false, itemsize, format, vec![]);
        let buf = PyBuffer::new(zelf.to_owned().into(), desc, &CDATA_BUFFER_METHODS);
        Ok(buf)
    }
}

#[pyclass(
    flags(BASETYPE),
    with(Callable, Constructor, AsNumber, Representable, AsBuffer)
)]
impl Py<PyCFuncPtr> {
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
        self.argtypes.read().clone().unwrap_or_else(|| {
            self.as_object()
                .class()
                .get_attr(vm.ctx.intern_str("_argtypes_"))
                .unwrap_or_else(|| vm.ctx.empty_tuple.clone().into())
        })
    }

    #[pygetset(name = "argtypes", setter)]
    fn set_argtypes(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        if vm.is_none(&value) {
            let mut arg_guard = self.argtypes.write();
            let mut conv_guard = self.converters.write();
            *arg_guard = Some(vm.ctx.none());
            *conv_guard = None;
        } else {
            let (types, converters) = extract_arg_types_and_converters(&value, vm)?;
            let snapshot = vm.ctx.new_tuple(vec![
                vm.ctx.new_tuple(types).into(),
                vm.ctx.new_tuple(converters).into(),
            ]);
            let mut arg_guard = self.argtypes.write();
            let mut conv_guard = self.converters.write();
            *arg_guard = Some(value);
            *conv_guard = Some(snapshot.into());
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
    fn _flags_(zelf: &Self, vm: &VirtualMachine) -> u32 {
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
            .map_or(StgInfoFlags::empty().bits(), |stg| stg.flags.bits())
    }
}

impl AsNumber for PyCFuncPtr {
    fn as_number() -> &'static PyNumberMethods {
        static AS_NUMBER: PyNumberMethods = PyNumberMethods {
            boolean: Some(|number, _vm| {
                let zelf = number.obj.downcast_ref::<PyCFuncPtr>().unwrap();
                Ok(zelf.get_func_ptr() != 0)
            }),
            ..PyNumberMethods::NOT_IMPLEMENTED
        };
        &AS_NUMBER
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
fn ffi_to_python(
    ty: &Py<PyType>,
    args: *const *const c_void,
    index: usize,
    vm: &VirtualMachine,
) -> PyResult {
    if ty.fast_issubclass(PyCPointer::static_type()) {
        // POINTER(T) uses a pointer argument in the CIF, even though its
        // `_type_` is a type object rather than a simple type code.
        let rustpython_host_env::ctypes::DecodedValue::Pointer(address) =
            (unsafe { rustpython_host_env::ctypes::callback_arg_value_at(Some("P"), args, index) })
        else {
            return Err(vm.new_type_error("invalid pointer callback argument"));
        };
        let instance = ty.as_object().call((), vm)?;
        let pointer = instance
            .downcast_ref::<PyCPointer>()
            .ok_or_else(|| vm.new_type_error("pointer constructor returned a non-pointer"))?;
        pointer.set_ptr_value(address);
        return Ok(instance);
    }

    let type_code = ty.type_code(vm);
    let raw_value: PyObjectRef = match unsafe {
        rustpython_host_env::ctypes::callback_arg_value_at(type_code.as_deref(), args, index)
    } {
        rustpython_host_env::ctypes::DecodedValue::Bytes(value) => vm.ctx.new_bytes(value).into(),
        rustpython_host_env::ctypes::DecodedValue::Signed(value) => vm.ctx.new_int(value).into(),
        rustpython_host_env::ctypes::DecodedValue::Unsigned(value) => vm.ctx.new_int(value).into(),
        rustpython_host_env::ctypes::DecodedValue::Float(value) => vm.ctx.new_float(value).into(),
        rustpython_host_env::ctypes::DecodedValue::Bool(value) => vm.ctx.new_bool(value).into(),
        rustpython_host_env::ctypes::DecodedValue::Pointer(value) => vm.ctx.new_int(value).into(),
        rustpython_host_env::ctypes::DecodedValue::String(value) => vm.ctx.new_str(value).into(),
        rustpython_host_env::ctypes::DecodedValue::None => vm.ctx.none(),
    };

    if !is_simple_subclass(ty, vm) {
        return Ok(raw_value);
    }
    Ok(ty
        .as_object()
        .call((raw_value.clone(),), vm)
        .unwrap_or(raw_value))
}

/// Convert a Python object to a C value and store it at the result pointer
fn python_to_ffi(obj: PyResult, ty: &Py<PyType>, result: *mut c_void, vm: &VirtualMachine) {
    let Ok(obj) = obj else { return };

    let type_code = ty.type_code(vm);
    match type_code.as_deref() {
        Some("b" | "h" | "i" | "l" | "q") => {
            if let Ok(i) = obj.try_int(vm) {
                unsafe {
                    rustpython_host_env::ctypes::write_callback_result(
                        type_code.as_deref(),
                        result,
                        rustpython_host_env::ctypes::CallbackResultValue::Signed(
                            i.as_bigint().to_i64().unwrap_or(0),
                        ),
                    );
                }
            }
        }
        Some("B" | "c" | "H" | "I" | "L" | "Q") => {
            if let Ok(i) = obj.try_int(vm) {
                unsafe {
                    rustpython_host_env::ctypes::write_callback_result(
                        type_code.as_deref(),
                        result,
                        rustpython_host_env::ctypes::CallbackResultValue::Unsigned(
                            i.as_bigint().to_u64().unwrap_or(0),
                        ),
                    );
                }
            }
        }
        Some("f" | "d") => {
            if let Ok(f) = obj.try_float(vm) {
                unsafe {
                    rustpython_host_env::ctypes::write_callback_result(
                        type_code.as_deref(),
                        result,
                        rustpython_host_env::ctypes::CallbackResultValue::Float(f.to_f64()),
                    );
                }
            }
        }
        Some("P" | "z" | "Z") => {
            if let Ok(i) = obj.try_int(vm) {
                unsafe {
                    rustpython_host_env::ctypes::write_callback_result(
                        type_code.as_deref(),
                        result,
                        rustpython_host_env::ctypes::CallbackResultValue::Pointer(
                            i.as_bigint().to_usize().unwrap_or(0),
                        ),
                    );
                }
            }
        }
        Some("?") => {
            if let Ok(b) = obj.is_true(vm) {
                unsafe {
                    rustpython_host_env::ctypes::write_callback_result(
                        type_code.as_deref(),
                        result,
                        rustpython_host_env::ctypes::CallbackResultValue::Bool(b),
                    );
                }
            }
        }
        _ => {}
    }
}

/// The callback function that libffi calls when the closure is invoked.
unsafe extern "C" fn thunk_callback(
    _cif: &FfiCif,
    result: &mut c_void,
    args: *const *const c_void,
    userdata: &ThunkUserData,
) {
    with_current_vm(|vm| {
        vm.attach_for_callback(|| {
            let use_errno = userdata.flags & StgInfoFlags::FUNCFLAG_USE_ERRNO.bits() != 0;
            let py_result =
                rustpython_host_env::ctypes::with_callback_errno_preserved(use_errno, || {
                    let py_args: Vec<PyObjectRef> = userdata
                        .arg_types
                        .iter()
                        .enumerate()
                        .map(|(i, ty)| ffi_to_python(ty, args, i, vm))
                        .collect::<PyResult<Vec<_>>>()?;

                    userdata.callable.call(py_args, vm)
                });

            // Call unraisable hook if exception occurred
            if let Err(exc) = &py_result {
                let repr = userdata
                    .callable
                    .repr(vm)
                    .map_or_else(|_| "<unknown>".to_string(), |s| s.to_string());
                let msg =
                    format!("Exception ignored while calling ctypes callback function {repr}");
                vm.run_unraisable(exc.clone(), Some(msg), vm.ctx.none());
            }

            if let Some(ref res_type) = userdata.res_type {
                python_to_ffi(py_result, res_type, result as *mut c_void, vm);
            }
        });
    });
}

// CThunkObject wraps a Python callable to make it callable from C code.
#[pyclass(
    name = "CThunkObject",
    module = "_ctypes",
    traverse = "manual",
    clear = false
)]
#[derive(PyPayload)]
pub(super) struct PyCThunk {
    callable: PyObjectRef,
    #[allow(dead_code)]
    thunk_data: PyRwLock<Option<rustpython_host_env::ctypes::CallbackThunk<ThunkUserData>>>,
    code_ptr: FfiCodePtr,
}

// SAFETY: the thunk owns both callable references, and all type references in
// its userdata. No references are inferred from executable or native pointers.
unsafe impl Traverse for PyCThunk {
    fn traverse(&self, tracer: &mut TraverseFn<'_>) {
        self.callable.traverse(tracer);
        if let Some(data) = self.thunk_data.try_read_recursive()
            && let Some(data) = data.as_ref()
        {
            let data = data.userdata();
            data.callable.traverse(tracer);
            data.arg_types.traverse(tracer);
            data.res_type.traverse(tracer);
        }
    }
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
        arg_types: Option<&PyObject>,
        res_type: Option<&PyObject>,
        flags: u32,
        vm: &VirtualMachine,
    ) -> PyResult<Self> {
        let arg_type_vec: Vec<PyTypeRef> = match arg_types {
            Some(args) if !vm.is_none(args) => {
                let error = || vm.new_type_error("callback argtypes must be ctypes types");
                let sequence = args
                    .try_sequence(vm)
                    .map_err(|_| vm.new_type_error("_argtypes_ must be a sequence of types"))?;
                let length = sequence
                    .length(vm)
                    .map_err(|_| vm.new_type_error("_argtypes_ must be a sequence of types"))?;
                let mut types = Vec::new();
                types
                    .try_reserve(length)
                    .map_err(|_| vm.no_memory_error())?;
                for index in 0..length {
                    let item = sequence.get_item(index as isize, vm).map_err(|_| error())?;
                    let ty = item.downcast::<PyType>().map_err(|_| error())?;
                    if !ty.fast_issubclass(PyCPointer::static_type()) && ty.type_code(vm).is_none()
                    {
                        return Err(error());
                    }
                    types.push(ty);
                }
                types
            }
            _ => Vec::new(),
        };

        let res_type_ref: Option<PyTypeRef> = match res_type {
            Some(rt) if !vm.is_none(rt) => Some(
                rt.to_owned()
                    .downcast::<PyType>()
                    .map_err(|_| vm.new_type_error("restype must be a ctypes type"))?,
            ),
            _ => None,
        };

        let ffi_arg_types: Vec<FfiType> = arg_type_vec
            .iter()
            .map(|ty| {
                if ty.fast_issubclass(PyCPointer::static_type()) {
                    ffi_pointer_type()
                } else {
                    ty.type_code(vm)
                        .and_then(|code| ffi_type_from_code(&code))
                        .unwrap_or_else(ffi_pointer_type)
                }
            })
            .collect();

        let ffi_res_type = res_type_ref
            .as_ref()
            .and_then(|ty| ty.type_code(vm))
            .and_then(|code| ffi_type_from_code(&code))
            .unwrap_or_else(ffi_void_type);

        let thunk_data = rustpython_host_env::ctypes::CallbackThunk::new(
            ffi_arg_types,
            ffi_res_type,
            Box::new(ThunkUserData {
                callable: callable.clone(),
                arg_types: arg_type_vec,
                res_type: res_type_ref,
                flags,
            }),
            thunk_callback,
        );
        let code_ptr = thunk_data.code_ptr();

        Ok(Self {
            callable,
            thunk_data: PyRwLock::new(Some(thunk_data)),
            code_ptr,
        })
    }

    pub fn code_ptr(&self) -> FfiCodePtr {
        self.code_ptr
    }
}

unsafe impl Send for PyCThunk {}
unsafe impl Sync for PyCThunk {}

/// `ffi_type`: `size_t` size, two `unsigned short`s, then a pointer.
#[repr(C)]
#[allow(dead_code)]
struct FfiTypeLayout {
    _size: usize,
    _alignment: u16,
    _type_code: u16,
    _elements: *mut *mut u8,
}

#[pyclass(itemsize = core::mem::size_of::<FfiTypeLayout>())]
impl Py<PyCThunk> {
    #[pygetset]
    fn callable(&self) -> PyObjectRef {
        self.callable.clone()
    }
}

impl AsSequence for PyCFuncPtrType {
    fn as_sequence() -> &'static PySequenceMethods {
        &super::array::CTYPE_SEQUENCE_METHODS
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{TryFromObject, class::PyClassImpl, function::ArgBytesLike};

    #[test]
    fn unlocked_raw_buffers_are_copied_even_when_readonly() {
        crate::Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let _class = RawMemoryBuffer::make_static_type();
            for readonly in [false, true] {
                let mut storage = *b"before";
                let view = memoryview_at_impl(
                    storage.as_mut_ptr() as usize,
                    storage.len() as isize,
                    readonly,
                    vm,
                )
                .unwrap();
                let buffer = ArgBytesLike::try_from_object(vm, view).unwrap();
                let snapshot = buffer.borrow_buf_unlocked(vm).unwrap();
                assert_ne!(snapshot.as_ptr(), storage.as_ptr());
                storage.copy_from_slice(b"after!");
                assert_eq!(&*snapshot, b"before");
            }

            let bytes = vm.ctx.new_bytes(b"bytes".to_vec());
            let view = crate::builtins::PyMemoryView::from_object(bytes.as_object(), vm)
                .unwrap()
                .into_pyobject(vm);
            for obj in [bytes.clone().into(), view] {
                let buffer = ArgBytesLike::try_from_object(vm, obj).unwrap();
                let borrowed = buffer.borrow_buf_unlocked(vm).unwrap();
                assert_eq!(borrowed.as_ptr(), bytes.as_bytes().as_ptr());
            }
        });
    }

    #[test]
    fn custom_converter_argtypes_and_from_param() {
        let builder = crate::Interpreter::builder(Default::default());
        let def = super::super::_ctypes::module_def(&builder.ctx);
        builder.add_native_module(def).build().enter(|vm| {
            vm.import("_ctypes", 0).unwrap();
            let scope = vm.new_scope_with_builtins();
            let code = r#"
import _ctypes

class ValidConverter:
    @classmethod
    def from_param(cls, val):
        return int(val)

class InvalidConverter:
    pass

class c_int(_ctypes._SimpleCData):
    _type_ = "i"

class Point(_ctypes.Structure):
    _fields_ = [("x", c_int), ("y", c_int)]

class PointConverter:
    @classmethod
    def from_param(cls, val):
        return Point(val[0], val[1])

class RaisingConverter:
    def __getattr__(self, name):
        raise ValueError("lookup failed")

valid = ValidConverter
invalid = InvalidConverter
point_conv = PointConverter
raising = RaisingConverter()
"#;
            vm.run_string(scope.clone(), code, "test.py").unwrap();
            let valid = scope.globals.get_item("valid", vm).unwrap();
            let invalid = scope.globals.get_item("invalid", vm).unwrap();
            let point_conv = scope.globals.get_item("point_conv", vm).unwrap();
            let raising = scope.globals.get_item("raising", vm).unwrap();

            let tuple_valid = vm.ctx.new_tuple(vec![valid.clone()]);
            let res = extract_arg_types_and_converters(tuple_valid.as_object(), vm);
            assert!(res.is_ok());
            let (types, converters) = res.unwrap();

            let tuple_invalid = vm.ctx.new_tuple(vec![invalid.clone()]);
            let err = extract_arg_types_and_converters(tuple_invalid.as_object(), vm).unwrap_err();
            assert!(
                err.as_object()
                    .str(vm)
                    .unwrap()
                    .to_string()
                    .contains("has no from_param method")
            );

            // Non-AttributeError exceptions are propagated directly
            let tuple_raising = vm.ctx.new_tuple(vec![raising]);
            let err_raising =
                extract_arg_types_and_converters(tuple_raising.as_object(), vm).unwrap_err();
            assert!(err_raising.fast_isinstance(vm.ctx.exceptions.value_error));

            let (val, keep) =
                convert_object(&converters[0], &types[0], vm.ctx.new_int(42).into(), vm).unwrap();
            match val {
                CArgValue::Int(i) => assert_eq!(i, 42),
                other => panic!("expected CArgValue::Int, got {other:?}"),
            }
            assert!(keep.is_some());

            // Custom converter returning Structure produces aggregate by value
            let (point_types, point_convs) = extract_arg_types_and_converters(
                vm.ctx.new_tuple(vec![point_conv]).as_object(),
                vm,
            )
            .unwrap();
            let pair = vm
                .ctx
                .new_tuple(vec![vm.ctx.new_int(10).into(), vm.ctx.new_int(20).into()]);
            let (val_point, keep_point) =
                convert_object(&point_convs[0], &point_types[0], pair.into(), vm).unwrap();
            match val_point {
                CArgValue::Aggregate { bytes, .. } => {
                    assert_eq!(bytes.len(), 8);
                }
                other => panic!("expected CArgValue::Aggregate, got {other:?}"),
            }
            assert!(keep_point.is_some());
        });
    }
}

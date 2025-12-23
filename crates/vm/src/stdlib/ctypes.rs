// spell-checker:disable

mod array;
mod base;
mod function;
mod library;
mod pointer;
mod simple;
mod structure;
mod union;

use crate::{
    AsObject, Py, PyObjectRef, PyRef, PyResult, VirtualMachine,
    builtins::{PyModule, PyStr, PyType},
    class::PyClassImpl,
    types::TypeDataRef,
};
use std::ffi::{
    c_double, c_float, c_int, c_long, c_longlong, c_schar, c_short, c_uchar, c_uint, c_ulong,
    c_ulonglong, c_ushort,
};
use std::mem;
use widestring::WideChar;

pub use array::PyCArray;
pub use base::{FfiArgValue, PyCData, PyCField, StgInfo, StgInfoFlags};
pub use pointer::PyCPointer;
pub use simple::{PyCSimple, PyCSimpleType};
pub use structure::PyCStructure;
pub use union::PyCUnion;

/// Extension for PyType to get StgInfo
/// PyStgInfo_FromType
impl Py<PyType> {
    /// Get StgInfo from a ctypes type object
    ///
    /// Returns a TypeDataRef to StgInfo if the type has one and is initialized, error otherwise.
    /// Abstract classes (whose metaclass __init__ was not called) will have uninitialized StgInfo.
    fn stg_info<'a>(&'a self, vm: &VirtualMachine) -> PyResult<TypeDataRef<'a, StgInfo>> {
        self.stg_info_opt()
            .ok_or_else(|| vm.new_type_error("abstract class"))
    }

    /// Get StgInfo if initialized, None otherwise.
    fn stg_info_opt(&self) -> Option<TypeDataRef<'_, StgInfo>> {
        self.get_type_data::<StgInfo>()
            .filter(|info| info.initialized)
    }

    /// Get _type_ attribute as String (type code like "i", "d", etc.)
    fn type_code(&self, vm: &VirtualMachine) -> Option<String> {
        self.as_object()
            .get_attr("_type_", vm)
            .ok()
            .and_then(|t: PyObjectRef| t.downcast_ref::<PyStr>().map(|s| s.to_string()))
    }

    /// Mark all base classes as finalized
    fn mark_bases_final(&self) {
        for base in self.bases.read().iter() {
            if let Some(mut stg) = base.get_type_data_mut::<StgInfo>() {
                stg.flags |= StgInfoFlags::DICTFLAG_FINAL;
            } else {
                let mut stg = StgInfo::default();
                stg.flags |= StgInfoFlags::DICTFLAG_FINAL;
                let _ = base.init_type_data(stg);
            }
        }
    }
}

impl PyType {
    /// Check if StgInfo is already initialized - prevent double initialization
    pub(crate) fn check_not_initialized(&self, vm: &VirtualMachine) -> PyResult<()> {
        if let Some(stg_info) = self.get_type_data::<StgInfo>()
            && stg_info.initialized
        {
            return Err(vm.new_exception_msg(
                vm.ctx.exceptions.system_error.to_owned(),
                format!("StgInfo of '{}' is already initialized.", self.name()),
            ));
        }
        Ok(())
    }
}

// Dynamic type check helpers for PyCData
// These check if an object's type's metaclass is a subclass of a specific metaclass

pub(crate) fn make_module(vm: &VirtualMachine) -> PyRef<PyModule> {
    let module = _ctypes::make_module(vm);
    let ctx = &vm.ctx;
    PyCSimpleType::make_class(ctx);
    array::PyCArrayType::make_class(ctx);
    pointer::PyCPointerType::make_class(ctx);
    structure::PyCStructType::make_class(ctx);
    union::PyCUnionType::make_class(ctx);
    function::PyCFuncPtrType::make_class(ctx);
    extend_module!(vm, &module, {
        "_CData" => PyCData::make_class(ctx),
        "_SimpleCData" => PyCSimple::make_class(ctx),
        "Array" => PyCArray::make_class(ctx),
        "CField" => PyCField::make_class(ctx),
        "CFuncPtr" => function::PyCFuncPtr::make_class(ctx),
        "_Pointer" => PyCPointer::make_class(ctx),
        "_pointer_type_cache" => ctx.new_dict(),
        "_array_type_cache" => ctx.new_dict(),
        "Structure" => PyCStructure::make_class(ctx),
        "CThunkObject" => function::PyCThunk::make_class(ctx),
        "Union" => PyCUnion::make_class(ctx),
    });
    module
}

/// Size of long double - platform dependent
/// x86_64 macOS/Linux: 16 bytes (80-bit extended + padding)
/// ARM64: 16 bytes (128-bit)
/// Windows: 8 bytes (same as double)
#[cfg(all(
    any(target_arch = "x86_64", target_arch = "aarch64"),
    not(target_os = "windows")
))]
const LONG_DOUBLE_SIZE: usize = 16;

#[cfg(target_os = "windows")]
const LONG_DOUBLE_SIZE: usize = mem::size_of::<c_double>();

#[cfg(not(any(
    all(
        any(target_arch = "x86_64", target_arch = "aarch64"),
        not(target_os = "windows")
    ),
    target_os = "windows"
)))]
const LONG_DOUBLE_SIZE: usize = mem::size_of::<c_double>();

/// Type information for ctypes simple types
struct TypeInfo {
    pub size: usize,
    pub ffi_type_fn: fn() -> libffi::middle::Type,
}

/// Get type information (size and ffi_type) for a ctypes type code
fn type_info(ty: &str) -> Option<TypeInfo> {
    use libffi::middle::Type;
    match ty {
        "c" => Some(TypeInfo {
            size: mem::size_of::<c_schar>(),
            ffi_type_fn: Type::u8,
        }),
        "u" => Some(TypeInfo {
            size: mem::size_of::<WideChar>(),
            ffi_type_fn: if mem::size_of::<WideChar>() == 2 {
                Type::u16
            } else {
                Type::u32
            },
        }),
        "b" => Some(TypeInfo {
            size: mem::size_of::<c_schar>(),
            ffi_type_fn: Type::i8,
        }),
        "B" => Some(TypeInfo {
            size: mem::size_of::<c_uchar>(),
            ffi_type_fn: Type::u8,
        }),
        "h" | "v" => Some(TypeInfo {
            size: mem::size_of::<c_short>(),
            ffi_type_fn: Type::i16,
        }),
        "H" => Some(TypeInfo {
            size: mem::size_of::<c_ushort>(),
            ffi_type_fn: Type::u16,
        }),
        "i" => Some(TypeInfo {
            size: mem::size_of::<c_int>(),
            ffi_type_fn: Type::i32,
        }),
        "I" => Some(TypeInfo {
            size: mem::size_of::<c_uint>(),
            ffi_type_fn: Type::u32,
        }),
        "l" => Some(TypeInfo {
            size: mem::size_of::<c_long>(),
            ffi_type_fn: if mem::size_of::<c_long>() == 8 {
                Type::i64
            } else {
                Type::i32
            },
        }),
        "L" => Some(TypeInfo {
            size: mem::size_of::<c_ulong>(),
            ffi_type_fn: if mem::size_of::<c_ulong>() == 8 {
                Type::u64
            } else {
                Type::u32
            },
        }),
        "q" => Some(TypeInfo {
            size: mem::size_of::<c_longlong>(),
            ffi_type_fn: Type::i64,
        }),
        "Q" => Some(TypeInfo {
            size: mem::size_of::<c_ulonglong>(),
            ffi_type_fn: Type::u64,
        }),
        "f" => Some(TypeInfo {
            size: mem::size_of::<c_float>(),
            ffi_type_fn: Type::f32,
        }),
        "d" => Some(TypeInfo {
            size: mem::size_of::<c_double>(),
            ffi_type_fn: Type::f64,
        }),
        "g" => Some(TypeInfo {
            // long double - platform dependent size
            // x86_64 macOS/Linux: 16 bytes (80-bit extended + padding)
            // ARM64: 16 bytes (128-bit)
            // Windows: 8 bytes (same as double)
            // Note: Use f64 as FFI type since Rust doesn't support long double natively
            size: LONG_DOUBLE_SIZE,
            ffi_type_fn: Type::f64,
        }),
        "?" => Some(TypeInfo {
            size: mem::size_of::<c_uchar>(),
            ffi_type_fn: Type::u8,
        }),
        "z" | "Z" | "P" | "X" | "O" => Some(TypeInfo {
            size: mem::size_of::<usize>(),
            ffi_type_fn: Type::pointer,
        }),
        "void" => Some(TypeInfo {
            size: 0,
            ffi_type_fn: Type::void,
        }),
        _ => None,
    }
}

/// Get size for a ctypes type code
fn get_size(ty: &str) -> usize {
    type_info(ty).map(|t| t.size).expect("invalid type code")
}

/// Get alignment for simple type codes from type_info().
/// For primitive C types (c_int, c_long, etc.), alignment equals size.
fn get_align(ty: &str) -> usize {
    get_size(ty)
}

#[pymodule]
pub(crate) mod _ctypes {
    use super::library;
    use super::{PyCArray, PyCData, PyCPointer, PyCSimple, PyCStructure, PyCUnion};
    use crate::builtins::{PyType, PyTypeRef};
    use crate::class::StaticType;
    use crate::convert::ToPyObject;
    use crate::function::{Either, OptionalArg};
    use crate::types::Representable;
    use crate::{AsObject, Py, PyObjectRef, PyPayload, PyResult, VirtualMachine};
    use num_traits::ToPrimitive;

    /// CArgObject - returned by byref() and paramfunc
    /// tagPyCArgObject
    #[pyclass(name = "CArgObject", module = "_ctypes", no_attr)]
    #[derive(Debug, PyPayload)]
    pub struct CArgObject {
        /// Type tag ('P', 'V', 'i', 'd', etc.)
        pub tag: u8,
        /// The actual FFI value (mirrors union value)
        pub value: super::FfiArgValue,
        /// Reference to original object (for memory safety)
        pub obj: PyObjectRef,
        /// Size for struct/union ('V' tag)
        #[allow(dead_code)]
        pub size: usize,
        /// Offset for byref()
        pub offset: isize,
    }

    /// is_literal_char - check if character is printable literal (not \\ or ')
    fn is_literal_char(c: u8) -> bool {
        c < 128 && c.is_ascii_graphic() && c != b'\\' && c != b'\''
    }

    impl Representable for CArgObject {
        // PyCArg_repr - use tag and value fields directly
        fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            use super::base::FfiArgValue;

            let tag_char = zelf.tag as char;

            // Format value based on tag
            match zelf.tag {
                b'b' | b'h' | b'i' | b'l' | b'q' => {
                    // Signed integers
                    let n = match zelf.value {
                        FfiArgValue::I8(v) => v as i64,
                        FfiArgValue::I16(v) => v as i64,
                        FfiArgValue::I32(v) => v as i64,
                        FfiArgValue::I64(v) => v,
                        _ => 0,
                    };
                    Ok(format!("<cparam '{}' ({})>", tag_char, n))
                }
                b'B' | b'H' | b'I' | b'L' | b'Q' => {
                    // Unsigned integers
                    let n = match zelf.value {
                        FfiArgValue::U8(v) => v as u64,
                        FfiArgValue::U16(v) => v as u64,
                        FfiArgValue::U32(v) => v as u64,
                        FfiArgValue::U64(v) => v,
                        _ => 0,
                    };
                    Ok(format!("<cparam '{}' ({})>", tag_char, n))
                }
                b'f' => {
                    let v = match zelf.value {
                        FfiArgValue::F32(v) => v as f64,
                        _ => 0.0,
                    };
                    Ok(format!("<cparam '{}' ({})>", tag_char, v))
                }
                b'd' | b'g' => {
                    let v = match zelf.value {
                        FfiArgValue::F64(v) => v,
                        FfiArgValue::F32(v) => v as f64,
                        _ => 0.0,
                    };
                    Ok(format!("<cparam '{}' ({})>", tag_char, v))
                }
                b'c' => {
                    // c_char - single byte
                    let byte = match zelf.value {
                        FfiArgValue::I8(v) => v as u8,
                        FfiArgValue::U8(v) => v,
                        _ => 0,
                    };
                    if is_literal_char(byte) {
                        Ok(format!("<cparam '{}' ('{}')>", tag_char, byte as char))
                    } else {
                        Ok(format!("<cparam '{}' ('\\x{:02x}')>", tag_char, byte))
                    }
                }
                b'z' | b'Z' | b'P' | b'V' => {
                    // Pointer types
                    let ptr = match zelf.value {
                        FfiArgValue::Pointer(v) => v,
                        _ => 0,
                    };
                    if ptr == 0 {
                        Ok(format!("<cparam '{}' (nil)>", tag_char))
                    } else {
                        Ok(format!("<cparam '{}' ({:#x})>", tag_char, ptr))
                    }
                }
                _ => {
                    // Default fallback
                    let addr = zelf.get_id();
                    if is_literal_char(zelf.tag) {
                        Ok(format!("<cparam '{}' at {:#x}>", tag_char, addr))
                    } else {
                        Ok(format!("<cparam {:#04x} at {:#x}>", zelf.tag, addr))
                    }
                }
            }
        }
    }

    #[pyclass(with(Representable))]
    impl CArgObject {
        #[pygetset]
        fn _obj(&self) -> PyObjectRef {
            self.obj.clone()
        }
    }

    #[pyattr(name = "__version__")]
    const __VERSION__: &str = "1.1.0";

    // TODO: get properly
    #[pyattr]
    const RTLD_LOCAL: i32 = 0;

    // TODO: get properly
    #[pyattr]
    const RTLD_GLOBAL: i32 = 0;

    #[pyattr]
    const SIZEOF_TIME_T: usize = std::mem::size_of::<libc::time_t>();

    #[pyattr]
    const CTYPES_MAX_ARGCOUNT: usize = 1024;

    #[pyattr]
    const FUNCFLAG_STDCALL: u32 = 0x0;
    #[pyattr]
    const FUNCFLAG_CDECL: u32 = 0x1;
    #[pyattr]
    const FUNCFLAG_HRESULT: u32 = 0x2;
    #[pyattr]
    const FUNCFLAG_PYTHONAPI: u32 = 0x4;
    #[pyattr]
    const FUNCFLAG_USE_ERRNO: u32 = 0x8;
    #[pyattr]
    const FUNCFLAG_USE_LASTERROR: u32 = 0x10;

    #[pyattr]
    const TYPEFLAG_ISPOINTER: u32 = 0x100;
    #[pyattr]
    const TYPEFLAG_HASPOINTER: u32 = 0x200;

    #[pyattr]
    const DICTFLAG_FINAL: u32 = 0x1000;

    #[pyattr(name = "ArgumentError", once)]
    fn argument_error(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.new_exception_type(
            "_ctypes",
            "ArgumentError",
            Some(vec![vm.ctx.exceptions.exception_type.to_owned()]),
        )
    }

    #[cfg(target_os = "windows")]
    #[pyattr(name = "COMError", once)]
    fn com_error(vm: &VirtualMachine) -> PyTypeRef {
        use crate::builtins::type_::PyAttributes;
        use crate::function::FuncArgs;
        use crate::types::{PyTypeFlags, PyTypeSlots};

        // Sets hresult, text, details as instance attributes in __init__
        // This function has InitFunc signature for direct slots.init use
        fn comerror_init(zelf: PyObjectRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
            let (hresult, text, details): (
                Option<PyObjectRef>,
                Option<PyObjectRef>,
                Option<PyObjectRef>,
            ) = args.bind(vm)?;
            let hresult = hresult.unwrap_or_else(|| vm.ctx.none());
            let text = text.unwrap_or_else(|| vm.ctx.none());
            let details = details.unwrap_or_else(|| vm.ctx.none());

            // Set instance attributes
            zelf.set_attr("hresult", hresult.clone(), vm)?;
            zelf.set_attr("text", text.clone(), vm)?;
            zelf.set_attr("details", details.clone(), vm)?;

            // self.args = args[1:] = (text, details)
            // via: PyObject_SetAttrString(self, "args", PySequence_GetSlice(args, 1, size))
            let args_tuple: PyObjectRef = vm.ctx.new_tuple(vec![text, details]).into();
            zelf.set_attr("args", args_tuple, vm)?;

            Ok(())
        }

        // Create exception type with IMMUTABLETYPE flag
        let mut attrs = PyAttributes::default();
        attrs.insert(
            vm.ctx.intern_str("__module__"),
            vm.ctx.new_str("_ctypes").into(),
        );
        attrs.insert(
            vm.ctx.intern_str("__doc__"),
            vm.ctx
                .new_str("Raised when a COM method call failed.")
                .into(),
        );

        // Create slots with IMMUTABLETYPE flag
        let slots = PyTypeSlots {
            name: "COMError",
            flags: PyTypeFlags::heap_type_flags()
                | PyTypeFlags::HAS_DICT
                | PyTypeFlags::IMMUTABLETYPE,
            ..PyTypeSlots::default()
        };

        let exc_type = PyType::new_heap(
            "COMError",
            vec![vm.ctx.exceptions.exception_type.to_owned()],
            attrs,
            slots,
            vm.ctx.types.type_type.to_owned(),
            &vm.ctx,
        )
        .unwrap();

        // Set our custom init after new_heap, which runs init_slots that would
        // otherwise overwrite slots.init with init_wrapper (due to __init__ in MRO).
        exc_type.slots.init.store(Some(comerror_init));

        exc_type
    }

    /// Get the size of a ctypes type or instance
    #[pyfunction]
    pub fn sizeof(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
        use super::structure::PyCStructType;
        use super::union::PyCUnionType;

        // 1. Check if obj is a TYPE object (not instance) - PyStgInfo_FromType
        if let Some(type_obj) = obj.downcast_ref::<PyType>() {
            // Type object - return StgInfo.size
            if let Some(stg_info) = type_obj.stg_info_opt() {
                return Ok(stg_info.size);
            }
            // Fallback for type objects without StgInfo
            // Array types
            if type_obj
                .class()
                .fast_issubclass(super::array::PyCArrayType::static_type())
                && let Ok(stg) = type_obj.stg_info(vm)
            {
                return Ok(stg.size);
            }
            // Structure types
            if type_obj
                .class()
                .fast_issubclass(PyCStructType::static_type())
            {
                return super::structure::calculate_struct_size(type_obj, vm);
            }
            // Union types
            if type_obj
                .class()
                .fast_issubclass(PyCUnionType::static_type())
            {
                return super::union::calculate_union_size(type_obj, vm);
            }
            // Simple types
            if type_obj.fast_issubclass(PyCSimple::static_type()) {
                if let Ok(type_attr) = type_obj.as_object().get_attr("_type_", vm)
                    && let Ok(type_str) = type_attr.str(vm)
                {
                    return Ok(super::get_size(type_str.as_ref()));
                }
                return Ok(std::mem::size_of::<usize>());
            }
            // Pointer types
            if type_obj.fast_issubclass(PyCPointer::static_type()) {
                return Ok(std::mem::size_of::<usize>());
            }
            return Err(vm.new_type_error("this type has no size"));
        }

        // 2. Instance object - return actual buffer size (b_size)
        // CDataObject_Check + return obj->b_size
        if let Some(cdata) = obj.downcast_ref::<PyCData>() {
            return Ok(cdata.size());
        }
        if obj.fast_isinstance(PyCPointer::static_type()) {
            return Ok(std::mem::size_of::<usize>());
        }

        Err(vm.new_type_error("this type has no size"))
    }

    #[cfg(windows)]
    #[pyfunction(name = "LoadLibrary")]
    fn load_library_windows(
        name: String,
        _load_flags: OptionalArg<i32>,
        vm: &VirtualMachine,
    ) -> PyResult<usize> {
        // TODO: audit functions first
        // TODO: load_flags
        let cache = library::libcache();
        let mut cache_write = cache.write();
        let (id, _) = cache_write.get_or_insert_lib(&name, vm).unwrap();
        Ok(id)
    }

    #[cfg(not(windows))]
    #[pyfunction(name = "dlopen")]
    fn load_library_unix(
        name: Option<crate::function::FsPath>,
        load_flags: OptionalArg<i32>,
        vm: &VirtualMachine,
    ) -> PyResult<usize> {
        // Default mode: RTLD_NOW | RTLD_LOCAL, always force RTLD_NOW
        let mode = load_flags.unwrap_or(libc::RTLD_NOW | libc::RTLD_LOCAL) | libc::RTLD_NOW;

        match name {
            Some(name) => {
                let cache = library::libcache();
                let mut cache_write = cache.write();
                let os_str = name.as_os_str(vm)?;
                let (id, _) = cache_write
                    .get_or_insert_lib_with_mode(&*os_str, mode, vm)
                    .map_err(|e| {
                        let name_str = os_str.to_string_lossy();
                        vm.new_os_error(format!("{}: {}", name_str, e))
                    })?;
                Ok(id)
            }
            None => {
                // dlopen(NULL, mode) to get the current process handle (for pythonapi)
                let handle = unsafe { libc::dlopen(std::ptr::null(), mode) };
                if handle.is_null() {
                    let err = unsafe { libc::dlerror() };
                    let msg = if err.is_null() {
                        "dlopen() error".to_string()
                    } else {
                        unsafe { std::ffi::CStr::from_ptr(err).to_string_lossy().into_owned() }
                    };
                    return Err(vm.new_os_error(msg));
                }
                // Add to library cache so symbol lookup works
                let cache = library::libcache();
                let mut cache_write = cache.write();
                let id = cache_write.insert_raw_handle(handle);
                Ok(id)
            }
        }
    }

    #[pyfunction(name = "FreeLibrary")]
    fn free_library(handle: usize) -> PyResult<()> {
        let cache = library::libcache();
        let mut cache_write = cache.write();
        cache_write.drop_lib(handle);
        Ok(())
    }

    #[cfg(not(windows))]
    #[pyfunction]
    fn dlclose(handle: usize, _vm: &VirtualMachine) -> PyResult<()> {
        // Remove from cache, which triggers SharedLibrary drop.
        // libloading::Library calls dlclose automatically on Drop.
        let cache = library::libcache();
        let mut cache_write = cache.write();
        cache_write.drop_lib(handle);
        Ok(())
    }

    #[cfg(not(windows))]
    #[pyfunction]
    fn dlsym(
        handle: usize,
        name: crate::builtins::PyStrRef,
        vm: &VirtualMachine,
    ) -> PyResult<usize> {
        let symbol_name = std::ffi::CString::new(name.as_str())
            .map_err(|_| vm.new_value_error("symbol name contains null byte"))?;

        // Clear previous error
        unsafe { libc::dlerror() };

        let ptr = unsafe { libc::dlsym(handle as *mut libc::c_void, symbol_name.as_ptr()) };

        // Check for error via dlerror first
        let err = unsafe { libc::dlerror() };
        if !err.is_null() {
            let msg = unsafe { std::ffi::CStr::from_ptr(err).to_string_lossy().into_owned() };
            return Err(vm.new_os_error(msg));
        }

        // Treat NULL symbol address as error
        // This handles cases like GNU IFUNCs that resolve to NULL
        if ptr.is_null() {
            return Err(vm.new_os_error(format!("symbol '{}' not found", name.as_str())));
        }

        Ok(ptr as usize)
    }

    #[pyfunction(name = "POINTER")]
    fn create_pointer_type(cls: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        use crate::builtins::PyStr;

        // Get the _pointer_type_cache
        let ctypes_module = vm.import("_ctypes", 0)?;
        let cache = ctypes_module.get_attr("_pointer_type_cache", vm)?;

        // Check if already in cache using __getitem__
        if let Ok(cached) = vm.call_method(&cache, "__getitem__", (cls.clone(),))
            && !vm.is_none(&cached)
        {
            return Ok(cached);
        }

        // Get the _Pointer base class
        let pointer_base = ctypes_module.get_attr("_Pointer", vm)?;

        // Create a new type that inherits from _Pointer
        let pointer_base_type = pointer_base
            .clone()
            .downcast::<crate::builtins::PyType>()
            .map_err(|_| vm.new_type_error("_Pointer must be a type"))?;
        let metaclass = pointer_base_type.class().to_owned();

        let bases = vm.ctx.new_tuple(vec![pointer_base]);
        let dict = vm.ctx.new_dict();

        // PyUnicode_CheckExact(cls) - string creates incomplete pointer type
        if let Some(s) = cls.downcast_ref::<PyStr>() {
            // Incomplete pointer type: _type_ not set, cache key is id(result)
            let name = format!("LP_{}", s.as_str());

            let new_type = metaclass
                .as_object()
                .call((vm.ctx.new_str(name), bases, dict), vm)?;

            // Store with id(result) as key for incomplete pointer types
            let id_key: PyObjectRef = vm.ctx.new_int(new_type.get_id() as i64).into();
            vm.call_method(&cache, "__setitem__", (id_key, new_type.clone()))?;

            return Ok(new_type);
        }

        // PyType_Check(cls) - type creates complete pointer type
        if !cls.class().fast_issubclass(vm.ctx.types.type_type.as_ref()) {
            return Err(vm.new_type_error("must be a ctypes type"));
        }

        // Create the name for the pointer type
        let name = if let Ok(type_obj) = cls.get_attr("__name__", vm) {
            format!("LP_{}", type_obj.str(vm)?)
        } else {
            "LP_unknown".to_string()
        };

        // Complete pointer type: set _type_ attribute
        dict.set_item("_type_", cls.clone(), vm)?;

        // Call the metaclass (PyCPointerType) to create the new type
        let new_type = metaclass
            .as_object()
            .call((vm.ctx.new_str(name), bases, dict), vm)?;

        // Store in cache with cls as key
        vm.call_method(&cache, "__setitem__", (cls, new_type.clone()))?;

        Ok(new_type)
    }

    #[pyfunction]
    fn pointer(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        // Get the type of the object
        let obj_type = obj.class().to_owned();

        // Create pointer type for this object's type
        let ptr_type = create_pointer_type(obj_type.into(), vm)?;

        // Create an instance of the pointer type with the object
        ptr_type.call((obj,), vm)
    }

    #[pyfunction]
    fn _pointer_type_cache() -> PyObjectRef {
        todo!()
    }

    #[cfg(target_os = "windows")]
    #[pyfunction(name = "_check_HRESULT")]
    fn check_hresult(_self: PyObjectRef, hr: i32, _vm: &VirtualMachine) -> PyResult<i32> {
        // TODO: fixme
        if hr < 0 {
            // vm.ctx.new_windows_error(hr)
            todo!();
        } else {
            Ok(hr)
        }
    }

    #[pyfunction]
    fn addressof(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
        // All ctypes objects should return cdata buffer pointer
        if let Some(cdata) = obj.downcast_ref::<PyCData>() {
            Ok(cdata.buffer.read().as_ptr() as usize)
        } else {
            Err(vm.new_type_error("expected a ctypes instance"))
        }
    }

    #[pyfunction]
    pub fn byref(obj: PyObjectRef, offset: OptionalArg<isize>, vm: &VirtualMachine) -> PyResult {
        use super::FfiArgValue;

        // Check if obj is a ctypes instance
        if !obj.fast_isinstance(PyCData::static_type())
            && !obj.fast_isinstance(PyCSimple::static_type())
        {
            return Err(vm.new_type_error(format!(
                "byref() argument must be a ctypes instance, not '{}'",
                obj.class().name()
            )));
        }

        let offset_val = offset.unwrap_or(0);

        // Get buffer address: (char *)((CDataObject *)obj)->b_ptr + offset
        let ptr_val = if let Some(simple) = obj.downcast_ref::<PyCSimple>() {
            let buffer = simple.0.buffer.read();
            (buffer.as_ptr() as isize + offset_val) as usize
        } else if let Some(cdata) = obj.downcast_ref::<PyCData>() {
            let buffer = cdata.buffer.read();
            (buffer.as_ptr() as isize + offset_val) as usize
        } else {
            0
        };

        // Create CArgObject to hold the reference
        Ok(CArgObject {
            tag: b'P',
            value: FfiArgValue::Pointer(ptr_val),
            obj,
            size: 0,
            offset: offset_val,
        }
        .to_pyobject(vm))
    }

    #[pyfunction]
    fn alignment(tp: Either<PyTypeRef, PyObjectRef>, vm: &VirtualMachine) -> PyResult<usize> {
        use crate::builtins::PyType;

        let obj = match &tp {
            Either::A(t) => t.as_object(),
            Either::B(o) => o.as_ref(),
        };

        // 1. Check TypeDataSlot on class (for instances)
        if let Some(stg_info) = obj.class().stg_info_opt() {
            return Ok(stg_info.align);
        }

        // 2. Check TypeDataSlot on type itself (for type objects)
        if let Some(type_obj) = obj.downcast_ref::<PyType>()
            && let Some(stg_info) = type_obj.stg_info_opt()
        {
            return Ok(stg_info.align);
        }

        // 3. Fallback for simple types
        if obj.fast_isinstance(PyCSimple::static_type())
            && let Ok(stg) = obj.class().stg_info(vm)
        {
            return Ok(stg.align);
        }
        if obj.fast_isinstance(PyCArray::static_type())
            && let Ok(stg) = obj.class().stg_info(vm)
        {
            return Ok(stg.align);
        }
        if obj.fast_isinstance(PyCStructure::static_type()) {
            // Calculate alignment from _fields_
            let cls = obj.class();
            return alignment(Either::A(cls.to_owned()), vm);
        }
        if obj.fast_isinstance(PyCPointer::static_type()) {
            // Pointer alignment is always pointer size
            return Ok(std::mem::align_of::<usize>());
        }
        if obj.fast_isinstance(PyCUnion::static_type()) {
            // Calculate alignment from _fields_
            let cls = obj.class();
            return alignment(Either::A(cls.to_owned()), vm);
        }

        // Get the type object to check
        let type_obj: PyObjectRef = match &tp {
            Either::A(t) => t.clone().into(),
            Either::B(obj) => obj.class().to_owned().into(),
        };

        // For type objects, try to get alignment from _type_ attribute
        if let Ok(type_attr) = type_obj.get_attr("_type_", vm) {
            // Array/Pointer: _type_ is the element type (a PyType)
            if let Ok(elem_type) = type_attr.clone().downcast::<crate::builtins::PyType>() {
                return alignment(Either::A(elem_type), vm);
            }
            // Simple type: _type_ is a single character string
            if let Ok(s) = type_attr.str(vm) {
                let ty = s.to_string();
                if ty.len() == 1 && super::simple::SIMPLE_TYPE_CHARS.contains(ty.as_str()) {
                    return Ok(super::get_align(&ty));
                }
            }
        }

        // Structure/Union: max alignment of fields
        if let Ok(fields_attr) = type_obj.get_attr("_fields_", vm)
            && let Ok(fields) = fields_attr.try_to_value::<Vec<PyObjectRef>>(vm)
        {
            let mut max_align = 1usize;
            for field in fields.iter() {
                if let Some(tuple) = field.downcast_ref::<crate::builtins::PyTuple>()
                    && let Some(field_type) = tuple.get(1)
                {
                    let align =
                        if let Ok(ft) = field_type.clone().downcast::<crate::builtins::PyType>() {
                            alignment(Either::A(ft), vm).unwrap_or(1)
                        } else {
                            1
                        };
                    max_align = max_align.max(align);
                }
            }
            return Ok(max_align);
        }

        // For instances, delegate to their class
        if let Either::B(obj) = &tp
            && !obj.class().is(vm.ctx.types.type_type.as_ref())
        {
            return alignment(Either::A(obj.class().to_owned()), vm);
        }

        // No alignment info found
        Err(vm.new_type_error("no alignment info"))
    }

    #[pyfunction]
    fn resize(obj: PyObjectRef, size: isize, vm: &VirtualMachine) -> PyResult<()> {
        use std::borrow::Cow;

        // 1. Get StgInfo from object's class (validates ctypes instance)
        let stg_info = obj
            .class()
            .stg_info_opt()
            .ok_or_else(|| vm.new_type_error("expected ctypes instance"))?;

        // 2. Validate size
        if size < 0 || (size as usize) < stg_info.size {
            return Err(vm.new_value_error(format!("minimum size is {}", stg_info.size)));
        }

        // 3. Get PyCData via upcast (works for all ctypes types due to repr(transparent))
        let cdata = obj
            .downcast_ref::<PyCData>()
            .ok_or_else(|| vm.new_type_error("expected ctypes instance"))?;

        // 4. Check if buffer is owned (not borrowed from external memory)
        {
            let buffer = cdata.buffer.read();
            if matches!(&*buffer, Cow::Borrowed(_)) {
                return Err(vm.new_value_error(
                    "Memory cannot be resized because this object doesn't own it".to_owned(),
                ));
            }
        }

        // 5. Resize the buffer
        let new_size = size as usize;
        let mut buffer = cdata.buffer.write();
        let old_data = buffer.to_vec();
        let mut new_data = vec![0u8; new_size];
        let copy_len = old_data.len().min(new_size);
        new_data[..copy_len].copy_from_slice(&old_data[..copy_len]);
        *buffer = Cow::Owned(new_data);

        Ok(())
    }

    #[pyfunction]
    fn get_errno() -> i32 {
        super::function::get_errno_value()
    }

    #[pyfunction]
    fn set_errno(value: i32) -> i32 {
        super::function::set_errno_value(value)
    }

    #[cfg(windows)]
    #[pyfunction]
    fn get_last_error() -> PyResult<u32> {
        Ok(super::function::get_last_error_value())
    }

    #[cfg(windows)]
    #[pyfunction]
    fn set_last_error(value: u32) -> u32 {
        super::function::set_last_error_value(value)
    }

    #[pyattr]
    fn _memmove_addr(_vm: &VirtualMachine) -> usize {
        let f = libc::memmove;
        f as usize
    }

    #[pyattr]
    fn _memset_addr(_vm: &VirtualMachine) -> usize {
        let f = libc::memset;
        f as usize
    }

    #[pyattr]
    fn _string_at_addr(_vm: &VirtualMachine) -> usize {
        super::function::INTERNAL_STRING_AT_ADDR
    }

    #[pyattr]
    fn _wstring_at_addr(_vm: &VirtualMachine) -> usize {
        super::function::INTERNAL_WSTRING_AT_ADDR
    }

    #[pyattr]
    fn _cast_addr(_vm: &VirtualMachine) -> usize {
        super::function::INTERNAL_CAST_ADDR
    }

    #[pyfunction]
    fn _cast(
        obj: PyObjectRef,
        src: PyObjectRef,
        ctype: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult {
        super::function::cast_impl(obj, src, ctype, vm)
    }

    /// Python-level cast function (PYFUNCTYPE wrapper)
    #[pyfunction]
    fn cast(obj: PyObjectRef, typ: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        super::function::cast_impl(obj.clone(), obj, typ, vm)
    }

    /// Return buffer interface information for a ctypes type or object.
    /// Returns a tuple (format, ndim, shape) where:
    /// - format: PEP 3118 format string
    /// - ndim: number of dimensions
    /// - shape: tuple of dimension sizes
    #[pyfunction]
    fn buffer_info(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        // Determine if obj is a type or an instance
        let is_type = obj.class().fast_issubclass(vm.ctx.types.type_type.as_ref());
        let cls = if is_type {
            obj.clone()
        } else {
            obj.class().to_owned().into()
        };

        // Get format from type - try _type_ first (for simple types), then _stg_info_format_
        let format = if let Ok(type_attr) = cls.get_attr("_type_", vm) {
            type_attr.str(vm)?.to_string()
        } else if let Ok(format_attr) = cls.get_attr("_stg_info_format_", vm) {
            format_attr.str(vm)?.to_string()
        } else {
            return Err(vm.new_type_error("not a ctypes type or object"));
        };

        // Non-array types have ndim=0 and empty shape
        // TODO: Implement ndim/shape for arrays when StgInfo supports it
        let ndim = 0;
        let shape: Vec<PyObjectRef> = vec![];

        let shape_tuple = vm.ctx.new_tuple(shape);
        Ok(vm
            .ctx
            .new_tuple(vec![
                vm.ctx.new_str(format).into(),
                vm.ctx.new_int(ndim).into(),
                shape_tuple.into(),
            ])
            .into())
    }

    /// Unpickle a ctypes object.
    #[pyfunction]
    fn _unpickle(typ: PyObjectRef, state: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if !state.class().is(vm.ctx.types.tuple_type.as_ref()) {
            return Err(vm.new_type_error("state must be a tuple"));
        }
        let obj = vm.call_method(&typ, "__new__", (typ.clone(),))?;
        vm.call_method(&obj, "__setstate__", (state,))?;
        Ok(obj)
    }

    /// Call a function at the given address with the given arguments.
    #[pyfunction]
    fn call_function(
        func_addr: usize,
        args: crate::builtins::PyTupleRef,
        vm: &VirtualMachine,
    ) -> PyResult {
        call_function_internal(func_addr, args, 0, vm)
    }

    /// Call a cdecl function at the given address with the given arguments.
    #[pyfunction]
    fn call_cdeclfunction(
        func_addr: usize,
        args: crate::builtins::PyTupleRef,
        vm: &VirtualMachine,
    ) -> PyResult {
        call_function_internal(func_addr, args, FUNCFLAG_CDECL, vm)
    }

    fn call_function_internal(
        func_addr: usize,
        args: crate::builtins::PyTupleRef,
        _flags: u32,
        vm: &VirtualMachine,
    ) -> PyResult {
        use libffi::middle::{Arg, Cif, CodePtr, Type};

        if func_addr == 0 {
            return Err(vm.new_value_error("NULL function pointer"));
        }

        let mut ffi_args: Vec<Arg<'_>> = Vec::with_capacity(args.len());
        let mut arg_values: Vec<isize> = Vec::with_capacity(args.len());
        let mut arg_types: Vec<Type> = Vec::with_capacity(args.len());

        for arg in args.iter() {
            if vm.is_none(arg) {
                arg_values.push(0);
                arg_types.push(Type::pointer());
            } else if let Ok(int_val) = arg.try_int(vm) {
                let val = int_val.as_bigint().to_i64().unwrap_or(0) as isize;
                arg_values.push(val);
                arg_types.push(Type::isize());
            } else if let Some(bytes) = arg.downcast_ref::<crate::builtins::PyBytes>() {
                let ptr = bytes.as_bytes().as_ptr() as isize;
                arg_values.push(ptr);
                arg_types.push(Type::pointer());
            } else if let Some(s) = arg.downcast_ref::<crate::builtins::PyStr>() {
                let ptr = s.as_str().as_ptr() as isize;
                arg_values.push(ptr);
                arg_types.push(Type::pointer());
            } else {
                return Err(vm.new_type_error(format!(
                    "Don't know how to convert parameter of type '{}'",
                    arg.class().name()
                )));
            }
        }

        for val in &arg_values {
            ffi_args.push(Arg::new(val));
        }

        let cif = Cif::new(arg_types, Type::c_int());
        let code_ptr = CodePtr::from_ptr(func_addr as *const _);
        let result: libc::c_int = unsafe { cif.call(code_ptr, &ffi_args) };
        Ok(vm.ctx.new_int(result).into())
    }

    /// Convert a pointer (as integer) to a Python object.
    #[pyfunction(name = "PyObj_FromPtr")]
    fn py_obj_from_ptr(ptr: usize, vm: &VirtualMachine) -> PyResult {
        if ptr == 0 {
            return Err(vm.new_value_error("NULL pointer access"));
        }
        let raw_ptr = ptr as *mut crate::object::PyObject;
        unsafe {
            let obj = crate::PyObjectRef::from_raw(std::ptr::NonNull::new_unchecked(raw_ptr));
            let obj = std::mem::ManuallyDrop::new(obj);
            Ok((*obj).clone())
        }
    }

    #[pyfunction(name = "Py_INCREF")]
    fn py_incref(obj: PyObjectRef, _vm: &VirtualMachine) -> PyObjectRef {
        // TODO:
        obj
    }

    #[pyfunction(name = "Py_DECREF")]
    fn py_decref(obj: PyObjectRef, _vm: &VirtualMachine) -> PyObjectRef {
        // TODO:
        obj
    }

    #[cfg(target_os = "macos")]
    #[pyfunction]
    fn _dyld_shared_cache_contains_path(
        path: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<bool> {
        use std::ffi::CString;

        let path = match path {
            Some(p) if !vm.is_none(&p) => p,
            _ => return Ok(false),
        };

        let path_str = path.str(vm)?.to_string();
        let c_path =
            CString::new(path_str).map_err(|_| vm.new_value_error("path contains null byte"))?;

        unsafe extern "C" {
            fn _dyld_shared_cache_contains_path(path: *const libc::c_char) -> bool;
        }

        let result = unsafe { _dyld_shared_cache_contains_path(c_path.as_ptr()) };
        Ok(result)
    }

    #[cfg(windows)]
    #[pyfunction(name = "FormatError")]
    fn format_error_func(code: OptionalArg<u32>, _vm: &VirtualMachine) -> PyResult<String> {
        use windows_sys::Win32::Foundation::{GetLastError, LocalFree};
        use windows_sys::Win32::System::Diagnostics::Debug::{
            FORMAT_MESSAGE_ALLOCATE_BUFFER, FORMAT_MESSAGE_FROM_SYSTEM,
            FORMAT_MESSAGE_IGNORE_INSERTS, FormatMessageW,
        };

        let error_code = code.unwrap_or_else(|| unsafe { GetLastError() });

        let mut buffer: *mut u16 = std::ptr::null_mut();
        let len = unsafe {
            FormatMessageW(
                FORMAT_MESSAGE_ALLOCATE_BUFFER
                    | FORMAT_MESSAGE_FROM_SYSTEM
                    | FORMAT_MESSAGE_IGNORE_INSERTS,
                std::ptr::null(),
                error_code,
                0,
                &mut buffer as *mut *mut u16 as *mut u16,
                0,
                std::ptr::null(),
            )
        };

        if len == 0 || buffer.is_null() {
            return Ok("<no description>".to_string());
        }

        let message = unsafe {
            let slice = std::slice::from_raw_parts(buffer, len as usize);
            let msg = String::from_utf16_lossy(slice).trim_end().to_string();
            LocalFree(buffer as *mut _);
            msg
        };

        Ok(message)
    }

    #[cfg(windows)]
    #[pyfunction(name = "CopyComPointer")]
    fn copy_com_pointer(src: PyObjectRef, dst: PyObjectRef, vm: &VirtualMachine) -> PyResult<i32> {
        use windows_sys::Win32::Foundation::{E_POINTER, S_OK};

        // 1. Extract pointer-to-pointer address from dst (byref() result)
        let pdst: usize = if let Some(carg) = dst.downcast_ref::<CArgObject>() {
            // byref() result: object buffer address + offset
            let base = if let Some(cdata) = carg.obj.downcast_ref::<PyCData>() {
                cdata.buffer.read().as_ptr() as usize
            } else {
                return Ok(E_POINTER);
            };
            (base as isize + carg.offset) as usize
        } else {
            return Ok(E_POINTER);
        };

        if pdst == 0 {
            return Ok(E_POINTER);
        }

        // 2. Extract COM pointer value from src
        let src_ptr: usize = if vm.is_none(&src) {
            0
        } else if let Some(cdata) = src.downcast_ref::<PyCData>() {
            // c_void_p etc: read pointer value from buffer
            let buffer = cdata.buffer.read();
            if buffer.len() >= std::mem::size_of::<usize>() {
                usize::from_ne_bytes(
                    buffer[..std::mem::size_of::<usize>()]
                        .try_into()
                        .unwrap_or([0; std::mem::size_of::<usize>()]),
                )
            } else {
                0
            }
        } else {
            return Ok(E_POINTER);
        };

        // 3. Call IUnknown::AddRef if src is non-NULL
        if src_ptr != 0 {
            unsafe {
                // IUnknown vtable: [QueryInterface, AddRef, Release, ...]
                let iunknown = src_ptr as *mut *const usize;
                let vtable = *iunknown;
                debug_assert!(!vtable.is_null(), "IUnknown vtable is null");
                let addref_fn: extern "system" fn(*mut std::ffi::c_void) -> u32 =
                    std::mem::transmute(*vtable.add(1)); // AddRef is index 1
                addref_fn(src_ptr as *mut std::ffi::c_void);
            }
        }

        // 4. Copy pointer: *pdst = src
        unsafe {
            *(pdst as *mut usize) = src_ptr;
        }

        Ok(S_OK)
    }
}

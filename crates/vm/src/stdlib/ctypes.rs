// spell-checker:disable

pub(crate) mod array;
pub(crate) mod base;
pub(crate) mod field;
pub(crate) mod function;
pub(crate) mod library;
pub(crate) mod pointer;
pub(crate) mod structure;
pub(crate) mod thunk;
pub(crate) mod union;
pub(crate) mod util;

use crate::builtins::PyModule;
use crate::class::PyClassImpl;
use crate::{Py, PyRef, VirtualMachine};

pub use crate::stdlib::ctypes::base::{PyCData, PyCSimple, PyCSimpleType};

pub fn extend_module_nodes(vm: &VirtualMachine, module: &Py<PyModule>) {
    let ctx = &vm.ctx;
    PyCSimpleType::make_class(ctx);
    array::PyCArrayType::make_class(ctx);
    field::PyCFieldType::make_class(ctx);
    pointer::PyCPointerType::make_class(ctx);
    structure::PyCStructType::make_class(ctx);
    union::PyCUnionType::make_class(ctx);
    extend_module!(vm, module, {
        "_CData" => PyCData::make_class(ctx),
        "_SimpleCData" => PyCSimple::make_class(ctx),
        "Array" => array::PyCArray::make_class(ctx),
        "CField" => field::PyCField::make_class(ctx),
        "CFuncPtr" => function::PyCFuncPtr::make_class(ctx),
        "_Pointer" => pointer::PyCPointer::make_class(ctx),
        "_pointer_type_cache" => ctx.new_dict(),
        "Structure" => structure::PyCStructure::make_class(ctx),
        "CThunkObject" => thunk::PyCThunk::make_class(ctx),
        "Union" => union::PyCUnion::make_class(ctx),
    })
}

pub(crate) fn make_module(vm: &VirtualMachine) -> PyRef<PyModule> {
    let module = _ctypes::make_module(vm);
    extend_module_nodes(vm, &module);
    module
}

#[pymodule]
pub(crate) mod _ctypes {
    use super::base::{CDataObject, PyCSimple};
    use crate::builtins::PyTypeRef;
    use crate::class::StaticType;
    use crate::convert::ToPyObject;
    use crate::function::{Either, FuncArgs, OptionalArg};
    use crate::stdlib::ctypes::library;
    use crate::{AsObject, PyObjectRef, PyPayload, PyResult, VirtualMachine};
    use crossbeam_utils::atomic::AtomicCell;
    use std::ffi::{
        c_double, c_float, c_int, c_long, c_longlong, c_schar, c_short, c_uchar, c_uint, c_ulong,
        c_ulonglong, c_ushort,
    };
    use std::mem;
    use widestring::WideChar;

    /// CArgObject - returned by byref()
    #[pyclass(name = "CArgObject", module = "_ctypes", no_attr)]
    #[derive(Debug, PyPayload)]
    pub struct CArgObject {
        pub obj: PyObjectRef,
        #[allow(dead_code)]
        pub offset: isize,
    }

    #[pyclass]
    impl CArgObject {
        #[pygetset]
        fn _obj(&self) -> PyObjectRef {
            self.obj.clone()
        }
    }

    #[pyattr(name = "__version__")]
    const __VERSION__: &str = "1.1.0";

    // TODO: get properly
    #[pyattr(name = "RTLD_LOCAL")]
    const RTLD_LOCAL: i32 = 0;

    // TODO: get properly
    #[pyattr(name = "RTLD_GLOBAL")]
    const RTLD_GLOBAL: i32 = 0;

    #[cfg(target_os = "windows")]
    #[pyattr(name = "SIZEOF_TIME_T")]
    pub const SIZEOF_TIME_T: usize = 8;
    #[cfg(not(target_os = "windows"))]
    #[pyattr(name = "SIZEOF_TIME_T")]
    pub const SIZEOF_TIME_T: usize = 4;

    #[pyattr(name = "CTYPES_MAX_ARGCOUNT")]
    pub const CTYPES_MAX_ARGCOUNT: usize = 1024;

    #[pyattr]
    pub const FUNCFLAG_STDCALL: u32 = 0x0;
    #[pyattr]
    pub const FUNCFLAG_CDECL: u32 = 0x1;
    #[pyattr]
    pub const FUNCFLAG_HRESULT: u32 = 0x2;
    #[pyattr]
    pub const FUNCFLAG_PYTHONAPI: u32 = 0x4;
    #[pyattr]
    pub const FUNCFLAG_USE_ERRNO: u32 = 0x8;
    #[pyattr]
    pub const FUNCFLAG_USE_LASTERROR: u32 = 0x10;

    #[pyattr]
    pub const TYPEFLAG_ISPOINTER: u32 = 0x100;
    #[pyattr]
    pub const TYPEFLAG_HASPOINTER: u32 = 0x200;

    #[pyattr]
    pub const DICTFLAG_FINAL: u32 = 0x1000;

    #[pyattr(name = "ArgumentError", once)]
    fn argument_error(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.new_exception_type(
            "_ctypes",
            "ArgumentError",
            Some(vec![vm.ctx.exceptions.exception_type.to_owned()]),
        )
    }

    #[pyattr(name = "FormatError", once)]
    fn format_error(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.new_exception_type(
            "_ctypes",
            "FormatError",
            Some(vec![vm.ctx.exceptions.exception_type.to_owned()]),
        )
    }

    pub fn get_size(ty: &str) -> usize {
        match ty {
            "u" => mem::size_of::<WideChar>(),
            "c" | "b" => mem::size_of::<c_schar>(),
            "h" => mem::size_of::<c_short>(),
            "H" => mem::size_of::<c_short>(),
            "i" => mem::size_of::<c_int>(),
            "I" => mem::size_of::<c_uint>(),
            "l" => mem::size_of::<c_long>(),
            "q" => mem::size_of::<c_longlong>(),
            "L" => mem::size_of::<c_ulong>(),
            "Q" => mem::size_of::<c_ulonglong>(),
            "f" => mem::size_of::<c_float>(),
            "d" | "g" => mem::size_of::<c_double>(),
            "?" | "B" => mem::size_of::<c_uchar>(),
            "P" | "z" | "Z" => mem::size_of::<usize>(),
            "O" => mem::size_of::<PyObjectRef>(),
            _ => unreachable!(),
        }
    }

    /// Get alignment for a simple type - for C types, alignment equals size
    pub fn get_align(ty: &str) -> usize {
        get_size(ty)
    }

    /// Get the size of a ctypes type from its type object
    #[allow(dead_code)]
    pub fn get_size_from_type(cls: &PyTypeRef, vm: &VirtualMachine) -> PyResult<usize> {
        // Try to get _type_ attribute for simple types
        if let Ok(type_attr) = cls.as_object().get_attr("_type_", vm)
            && let Ok(s) = type_attr.str(vm)
        {
            let s = s.to_string();
            if s.len() == 1 && SIMPLE_TYPE_CHARS.contains(s.as_str()) {
                return Ok(get_size(&s));
            }
        }
        // Fall back to sizeof
        size_of(cls.clone().into(), vm)
    }

    /// Convert bytes to appropriate Python object based on ctypes type
    pub fn bytes_to_pyobject(
        cls: &PyTypeRef,
        bytes: &[u8],
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        // Try to get _type_ attribute
        if let Ok(type_attr) = cls.as_object().get_attr("_type_", vm)
            && let Ok(s) = type_attr.str(vm)
        {
            let ty = s.to_string();
            return match ty.as_str() {
                "c" => {
                    // c_char - single byte
                    Ok(vm.ctx.new_bytes(bytes.to_vec()).into())
                }
                "b" => {
                    // c_byte - signed char
                    let val = if !bytes.is_empty() { bytes[0] as i8 } else { 0 };
                    Ok(vm.ctx.new_int(val).into())
                }
                "B" => {
                    // c_ubyte - unsigned char
                    let val = if !bytes.is_empty() { bytes[0] } else { 0 };
                    Ok(vm.ctx.new_int(val).into())
                }
                "h" => {
                    // c_short
                    const SIZE: usize = mem::size_of::<c_short>();
                    let val = if bytes.len() >= SIZE {
                        c_short::from_ne_bytes(bytes[..SIZE].try_into().expect("size checked"))
                    } else {
                        0
                    };
                    Ok(vm.ctx.new_int(val).into())
                }
                "H" => {
                    // c_ushort
                    const SIZE: usize = mem::size_of::<c_ushort>();
                    let val = if bytes.len() >= SIZE {
                        c_ushort::from_ne_bytes(bytes[..SIZE].try_into().expect("size checked"))
                    } else {
                        0
                    };
                    Ok(vm.ctx.new_int(val).into())
                }
                "i" => {
                    // c_int
                    const SIZE: usize = mem::size_of::<c_int>();
                    let val = if bytes.len() >= SIZE {
                        c_int::from_ne_bytes(bytes[..SIZE].try_into().expect("size checked"))
                    } else {
                        0
                    };
                    Ok(vm.ctx.new_int(val).into())
                }
                "I" => {
                    // c_uint
                    const SIZE: usize = mem::size_of::<c_uint>();
                    let val = if bytes.len() >= SIZE {
                        c_uint::from_ne_bytes(bytes[..SIZE].try_into().expect("size checked"))
                    } else {
                        0
                    };
                    Ok(vm.ctx.new_int(val).into())
                }
                "l" => {
                    // c_long
                    const SIZE: usize = mem::size_of::<c_long>();
                    let val = if bytes.len() >= SIZE {
                        c_long::from_ne_bytes(bytes[..SIZE].try_into().expect("size checked"))
                    } else {
                        0
                    };
                    Ok(vm.ctx.new_int(val).into())
                }
                "L" => {
                    // c_ulong
                    const SIZE: usize = mem::size_of::<c_ulong>();
                    let val = if bytes.len() >= SIZE {
                        c_ulong::from_ne_bytes(bytes[..SIZE].try_into().expect("size checked"))
                    } else {
                        0
                    };
                    Ok(vm.ctx.new_int(val).into())
                }
                "q" => {
                    // c_longlong
                    const SIZE: usize = mem::size_of::<c_longlong>();
                    let val = if bytes.len() >= SIZE {
                        c_longlong::from_ne_bytes(bytes[..SIZE].try_into().expect("size checked"))
                    } else {
                        0
                    };
                    Ok(vm.ctx.new_int(val).into())
                }
                "Q" => {
                    // c_ulonglong
                    const SIZE: usize = mem::size_of::<c_ulonglong>();
                    let val = if bytes.len() >= SIZE {
                        c_ulonglong::from_ne_bytes(bytes[..SIZE].try_into().expect("size checked"))
                    } else {
                        0
                    };
                    Ok(vm.ctx.new_int(val).into())
                }
                "f" => {
                    // c_float
                    const SIZE: usize = mem::size_of::<c_float>();
                    let val = if bytes.len() >= SIZE {
                        c_float::from_ne_bytes(bytes[..SIZE].try_into().expect("size checked"))
                    } else {
                        0.0
                    };
                    Ok(vm.ctx.new_float(val as f64).into())
                }
                "d" | "g" => {
                    // c_double
                    const SIZE: usize = mem::size_of::<c_double>();
                    let val = if bytes.len() >= SIZE {
                        c_double::from_ne_bytes(bytes[..SIZE].try_into().expect("size checked"))
                    } else {
                        0.0
                    };
                    Ok(vm.ctx.new_float(val).into())
                }
                "?" => {
                    // c_bool
                    let val = !bytes.is_empty() && bytes[0] != 0;
                    Ok(vm.ctx.new_bool(val).into())
                }
                "P" | "z" | "Z" => {
                    // Pointer types - return as integer address
                    let val = if bytes.len() >= mem::size_of::<libc::uintptr_t>() {
                        const UINTPTR_LEN: usize = mem::size_of::<libc::uintptr_t>();
                        let mut arr = [0u8; UINTPTR_LEN];
                        arr[..bytes.len().min(UINTPTR_LEN)]
                            .copy_from_slice(&bytes[..bytes.len().min(UINTPTR_LEN)]);
                        usize::from_ne_bytes(arr)
                    } else {
                        0
                    };
                    Ok(vm.ctx.new_int(val).into())
                }
                "u" => {
                    // c_wchar - wide character
                    let val = if bytes.len() >= mem::size_of::<WideChar>() {
                        let wc = if mem::size_of::<WideChar>() == 2 {
                            u16::from_ne_bytes([bytes[0], bytes[1]]) as u32
                        } else {
                            u32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
                        };
                        char::from_u32(wc).unwrap_or('\0')
                    } else {
                        '\0'
                    };
                    Ok(vm.ctx.new_str(val.to_string()).into())
                }
                _ => Ok(vm.ctx.none()),
            };
        }
        // Default: return bytes as-is
        Ok(vm.ctx.new_bytes(bytes.to_vec()).into())
    }

    const SIMPLE_TYPE_CHARS: &str = "cbBhHiIlLdfguzZPqQ?O";

    pub fn new_simple_type(
        cls: Either<&PyObjectRef, &PyTypeRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyCSimple> {
        let cls = match cls {
            Either::A(obj) => obj,
            Either::B(typ) => typ.as_object(),
        };

        if let Ok(_type_) = cls.get_attr("_type_", vm) {
            if _type_.is_instance((&vm.ctx.types.str_type).as_ref(), vm)? {
                let tp_str = _type_.str(vm)?.to_string();

                if tp_str.len() != 1 {
                    Err(vm.new_value_error(
                        format!("class must define a '_type_' attribute which must be a string of length 1, str: {tp_str}"),
                    ))
                } else if !SIMPLE_TYPE_CHARS.contains(tp_str.as_str()) {
                    Err(vm.new_attribute_error(format!("class must define a '_type_' attribute which must be\n a single character string containing one of {SIMPLE_TYPE_CHARS}, currently it is {tp_str}.")))
                } else {
                    let size = get_size(&tp_str);
                    Ok(PyCSimple {
                        _type_: tp_str,
                        value: AtomicCell::new(vm.ctx.none()),
                        cdata: rustpython_common::lock::PyRwLock::new(CDataObject::from_bytes(
                            vec![0u8; size],
                            None,
                        )),
                    })
                }
            } else {
                Err(vm.new_type_error("class must define a '_type_' string attribute"))
            }
        } else {
            Err(vm.new_attribute_error("class must define a '_type_' attribute"))
        }
    }

    /// Get the size of a ctypes type or instance
    #[pyfunction(name = "sizeof")]
    pub fn size_of(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
        use super::array::{PyCArray, PyCArrayType};
        use super::pointer::PyCPointer;
        use super::structure::{PyCStructType, PyCStructure};
        use super::union::{PyCUnion, PyCUnionType};

        // 1. Instances with stg_info
        if obj.fast_isinstance(PyCArray::static_type()) {
            // Get stg_info from the type
            if let Some(type_obj) = obj.class().as_object().downcast_ref::<PyCArrayType>() {
                return Ok(type_obj.stg_info.size);
            }
        }
        if let Some(structure) = obj.downcast_ref::<PyCStructure>() {
            return Ok(structure.cdata.read().size());
        }
        if obj.fast_isinstance(PyCUnion::static_type()) {
            // Get stg_info from the type
            if let Some(type_obj) = obj.class().as_object().downcast_ref::<PyCUnionType>() {
                return Ok(type_obj.stg_info.size);
            }
        }
        if let Some(simple) = obj.downcast_ref::<PyCSimple>() {
            return Ok(simple.cdata.read().size());
        }
        if obj.fast_isinstance(PyCPointer::static_type()) {
            return Ok(std::mem::size_of::<usize>());
        }

        // 2. Types (metatypes with stg_info)
        if let Some(array_type) = obj.downcast_ref::<PyCArrayType>() {
            return Ok(array_type.stg_info.size);
        }

        // 3. Type objects
        if let Ok(type_ref) = obj.clone().downcast::<crate::builtins::PyType>() {
            // Structure types - check if metaclass is or inherits from PyCStructType
            if type_ref
                .class()
                .fast_issubclass(PyCStructType::static_type())
            {
                return calculate_struct_size(&type_ref, vm);
            }
            // Union types - check if metaclass is or inherits from PyCUnionType
            if type_ref
                .class()
                .fast_issubclass(PyCUnionType::static_type())
            {
                return calculate_union_size(&type_ref, vm);
            }
            // Simple types (c_int, c_char, etc.)
            if type_ref.fast_issubclass(PyCSimple::static_type()) {
                let instance = new_simple_type(Either::B(&type_ref), vm)?;
                return Ok(get_size(&instance._type_));
            }
            // Pointer types
            if type_ref.fast_issubclass(PyCPointer::static_type()) {
                return Ok(std::mem::size_of::<usize>());
            }
        }

        Err(vm.new_type_error("this type has no size"))
    }

    /// Calculate Structure type size from _fields_ (sum of field sizes)
    fn calculate_struct_size(
        cls: &crate::builtins::PyTypeRef,
        vm: &VirtualMachine,
    ) -> PyResult<usize> {
        use crate::AsObject;

        if let Ok(fields_attr) = cls.as_object().get_attr("_fields_", vm) {
            let fields: Vec<PyObjectRef> = fields_attr.try_to_value(vm).unwrap_or_default();
            let mut total_size = 0usize;

            for field in fields.iter() {
                if let Some(tuple) = field.downcast_ref::<crate::builtins::PyTuple>()
                    && let Some(field_type) = tuple.get(1)
                {
                    // Recursively calculate field type size
                    total_size += size_of(field_type.clone(), vm)?;
                }
            }
            return Ok(total_size);
        }
        Ok(0)
    }

    /// Calculate Union type size from _fields_ (max field size)
    fn calculate_union_size(
        cls: &crate::builtins::PyTypeRef,
        vm: &VirtualMachine,
    ) -> PyResult<usize> {
        use crate::AsObject;

        if let Ok(fields_attr) = cls.as_object().get_attr("_fields_", vm) {
            let fields: Vec<PyObjectRef> = fields_attr.try_to_value(vm).unwrap_or_default();
            let mut max_size = 0usize;

            for field in fields.iter() {
                if let Some(tuple) = field.downcast_ref::<crate::builtins::PyTuple>()
                    && let Some(field_type) = tuple.get(1)
                {
                    let field_size = size_of(field_type.clone(), vm)?;
                    max_size = max_size.max(field_size);
                }
            }
            return Ok(max_size);
        }
        Ok(0)
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
        name: Option<String>,
        _load_flags: OptionalArg<i32>,
        vm: &VirtualMachine,
    ) -> PyResult<usize> {
        // TODO: audit functions first
        // TODO: load_flags
        match name {
            Some(name) => {
                let cache = library::libcache();
                let mut cache_write = cache.write();
                let (id, _) = cache_write
                    .get_or_insert_lib(&name, vm)
                    .map_err(|e| vm.new_os_error(e.to_string()))?;
                Ok(id)
            }
            None => {
                // If None, call libc::dlopen(null, mode) to get the current process handle
                let handle = unsafe { libc::dlopen(std::ptr::null(), libc::RTLD_NOW) };
                if handle.is_null() {
                    return Err(vm.new_os_error("dlopen() error"));
                }
                Ok(handle as usize)
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

    #[pyfunction(name = "POINTER")]
    pub fn create_pointer_type(cls: PyObjectRef, vm: &VirtualMachine) -> PyResult {
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

        // Create the name for the pointer type
        let name = if let Ok(type_obj) = cls.get_attr("__name__", vm) {
            format!("LP_{}", type_obj.str(vm)?)
        } else if let Ok(s) = cls.str(vm) {
            format!("LP_{}", s)
        } else {
            "LP_unknown".to_string()
        };

        // Create a new type that inherits from _Pointer
        let type_type = &vm.ctx.types.type_type;
        let bases = vm.ctx.new_tuple(vec![pointer_base]);
        let dict = vm.ctx.new_dict();
        dict.set_item("_type_", cls.clone(), vm)?;

        let new_type = type_type
            .as_object()
            .call((vm.ctx.new_str(name), bases, dict), vm)?;

        // Store in cache using __setitem__
        vm.call_method(&cache, "__setitem__", (cls, new_type.clone()))?;

        Ok(new_type)
    }

    #[pyfunction(name = "pointer")]
    pub fn create_pointer_inst(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
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
    pub fn check_hresult(_self: PyObjectRef, hr: i32, _vm: &VirtualMachine) -> PyResult<i32> {
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
        if obj.is_instance(PyCSimple::static_type().as_ref(), vm)? {
            let simple = obj.downcast_ref::<PyCSimple>().unwrap();
            Ok(simple.value.as_ptr() as usize)
        } else {
            Err(vm.new_type_error("expected a ctypes instance"))
        }
    }

    #[pyfunction]
    fn byref(obj: PyObjectRef, offset: OptionalArg<isize>, vm: &VirtualMachine) -> PyResult {
        use super::base::PyCData;
        use crate::class::StaticType;

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

        // Create CArgObject to hold the reference
        Ok(CArgObject {
            obj,
            offset: offset_val,
        }
        .to_pyobject(vm))
    }

    #[pyfunction]
    fn alignment(tp: Either<PyTypeRef, PyObjectRef>, vm: &VirtualMachine) -> PyResult<usize> {
        use super::array::{PyCArray, PyCArrayType};
        use super::base::PyCSimpleType;
        use super::pointer::PyCPointer;
        use super::structure::PyCStructure;
        use super::union::PyCUnion;

        let obj = match &tp {
            Either::A(t) => t.as_object(),
            Either::B(o) => o.as_ref(),
        };

        // Try to get alignment from stg_info directly (for instances)
        if let Some(array_type) = obj.downcast_ref::<PyCArrayType>() {
            return Ok(array_type.stg_info.align);
        }
        if obj.fast_isinstance(PyCSimple::static_type()) {
            // Get stg_info from the type by reading _type_ attribute
            let cls = obj.class().to_owned();
            let stg_info = PyCSimpleType::get_stg_info(&cls, vm);
            return Ok(stg_info.align);
        }
        if obj.fast_isinstance(PyCArray::static_type()) {
            // Get stg_info from the type
            if let Some(type_obj) = obj.class().as_object().downcast_ref::<PyCArrayType>() {
                return Ok(type_obj.stg_info.align);
            }
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
                if ty.len() == 1 && SIMPLE_TYPE_CHARS.contains(ty.as_str()) {
                    return Ok(get_align(&ty));
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
    fn resize(_args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
        // TODO: RUSTPYTHON
        Err(vm.new_value_error("not implemented"))
    }

    #[pyfunction]
    fn get_errno() -> i32 {
        errno::errno().0
    }

    #[pyfunction]
    fn set_errno(value: i32) {
        errno::set_errno(errno::Errno(value));
    }

    #[cfg(windows)]
    #[pyfunction]
    fn get_last_error() -> PyResult<u32> {
        Ok(unsafe { windows_sys::Win32::Foundation::GetLastError() })
    }

    #[cfg(windows)]
    #[pyfunction]
    fn set_last_error(value: u32) -> PyResult<()> {
        unsafe { windows_sys::Win32::Foundation::SetLastError(value) };
        Ok(())
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
        let f = libc::strnlen;
        f as usize
    }

    #[pyattr]
    fn _wstring_at_addr(_vm: &VirtualMachine) -> usize {
        // Return address of wcsnlen or similar wide string function
        #[cfg(not(target_os = "windows"))]
        {
            let f = libc::wcslen;
            f as usize
        }
        #[cfg(target_os = "windows")]
        {
            // FIXME: On Windows, use wcslen from ucrt
            0
        }
    }

    #[pyattr]
    fn _cast_addr(_vm: &VirtualMachine) -> usize {
        // todo!("Implement _cast_addr")
        0
    }

    #[pyfunction(name = "_cast")]
    pub fn pycfunction_cast(
        obj: PyObjectRef,
        _obj2: PyObjectRef,
        ctype: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult {
        use super::array::PyCArray;
        use super::base::PyCData;
        use super::pointer::PyCPointer;
        use crate::class::StaticType;

        // Python signature: _cast(obj, obj, ctype)
        // Python passes the same object twice (obj and _obj2 are the same)
        // We ignore _obj2 as it's redundant

        // Check if this is a pointer type (has _type_ attribute)
        if ctype.get_attr("_type_", vm).is_err() {
            return Err(vm.new_type_error("cast() argument 2 must be a pointer type".to_string()));
        }

        // Create an instance of the target pointer type with no arguments
        let result = ctype.call((), vm)?;

        // Get the pointer value from the source object
        // If obj is a CData instance (including arrays), use the object itself
        // If obj is an integer, use it directly as the pointer value
        let ptr_value: PyObjectRef = if obj.fast_isinstance(PyCData::static_type())
            || obj.fast_isinstance(PyCArray::static_type())
            || obj.fast_isinstance(PyCPointer::static_type())
        {
            // For CData objects (including arrays and pointers), store the object itself
            obj.clone()
        } else if let Ok(int_val) = obj.try_int(vm) {
            // For integers, treat as pointer address
            vm.ctx.new_int(int_val.as_bigint().clone()).into()
        } else {
            return Err(vm.new_type_error(format!(
                "cast() argument 1 must be a ctypes instance or an integer, not {}",
                obj.class().name()
            )));
        };

        // Set the contents of the pointer by setting the attribute
        result.set_attr("contents", ptr_value, vm)?;

        Ok(result)
    }
}

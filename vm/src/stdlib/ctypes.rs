// cspell:disable

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
use crate::stdlib::ctypes::base::{PyCData, PyCSimple, PyCSimpleType};
use crate::{Py, PyRef, VirtualMachine};

pub fn extend_module_nodes(vm: &VirtualMachine, module: &Py<PyModule>) {
    let ctx = &vm.ctx;
    PyCSimpleType::make_class(ctx);
    array::PyCArrayType::make_class(ctx);
    field::PyCFieldType::make_class(ctx);
    pointer::PyCPointerType::make_class(ctx);
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
    use super::base::PyCSimple;
    use crate::builtins::PyTypeRef;
    use crate::class::StaticType;
    use crate::function::{Either, FuncArgs, OptionalArg};
    use crate::stdlib::ctypes::library;
    use crate::{AsObject, PyObjectRef, PyResult, TryFromObject, VirtualMachine};
    use crossbeam_utils::atomic::AtomicCell;
    use std::ffi::{
        c_double, c_float, c_int, c_long, c_longlong, c_schar, c_short, c_uchar, c_uint, c_ulong,
        c_ulonglong,
    };
    use std::mem;
    use widestring::WideChar;

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
                    Ok(PyCSimple {
                        _type_: tp_str,
                        value: AtomicCell::new(vm.ctx.none()),
                    })
                }
            } else {
                Err(vm.new_type_error("class must define a '_type_' string attribute".to_string()))
            }
        } else {
            Err(vm.new_attribute_error("class must define a '_type_' attribute".to_string()))
        }
    }

    #[pyfunction(name = "sizeof")]
    pub fn size_of(tp: Either<PyTypeRef, PyObjectRef>, vm: &VirtualMachine) -> PyResult<usize> {
        match tp {
            Either::A(type_) if type_.fast_issubclass(PyCSimple::static_type()) => {
                let zelf = new_simple_type(Either::B(&type_), vm)?;
                Ok(get_size(zelf._type_.as_str()))
            }
            Either::B(obj) if obj.has_attr("size_of_instances", vm)? => {
                let size_of_method = obj.get_attr("size_of_instances", vm)?;
                let size_of_return = size_of_method.call(vec![], vm)?;
                Ok(usize::try_from_object(vm, size_of_return)?)
            }
            _ => Err(vm.new_type_error("this type has no size".to_string())),
        }
    }

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

    #[pyfunction(name = "dlopen")]
    fn load_library_unix(
        name: String,
        _load_flags: OptionalArg<i32>,
        vm: &VirtualMachine,
    ) -> PyResult<usize> {
        // TODO: audit functions first
        // TODO: load_flags
        let cache = library::libcache();
        let mut cache_write = cache.write();
        let (id, _) = cache_write
            .get_or_insert_lib(&name, vm)
            .map_err(|e| vm.new_os_error(e.to_string()))?;
        Ok(id)
    }

    #[pyfunction(name = "FreeLibrary")]
    fn free_library(handle: usize) -> PyResult<()> {
        let cache = library::libcache();
        let mut cache_write = cache.write();
        cache_write.drop_lib(handle);
        Ok(())
    }

    #[pyfunction(name = "POINTER")]
    pub fn pointer(_cls: PyTypeRef) {}

    #[pyfunction(name = "pointer")]
    pub fn pointer_fn(_inst: PyObjectRef) {}

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
            Err(vm.new_type_error("expected a ctypes instance".to_string()))
        }
    }

    #[pyfunction]
    fn byref(_args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
        // TODO: RUSTPYTHON
        Err(vm.new_value_error("not implemented".to_string()))
    }

    #[pyfunction]
    fn alignment(_args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
        // TODO: RUSTPYTHON
        Err(vm.new_value_error("not implemented".to_string()))
    }

    #[pyfunction]
    fn resize(_args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
        // TODO: RUSTPYTHON
        Err(vm.new_value_error("not implemented".to_string()))
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
    fn _cast_addr(_vm: &VirtualMachine) -> usize {
        // TODO: RUSTPYTHON
        0
    }
}

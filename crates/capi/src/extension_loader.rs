use crate::PyObject;
use crate::pystate::with_vm;
use alloc::ffi::CString;
use core::{
    ffi::{CStr, c_char, c_int, c_void},
    mem,
};
use libloading::Library;
#[cfg(unix)]
use libloading::os::unix::Library as UnixLibrary;
use rustpython_vm::{
    AsObject, PyObjectRef, PyResult, builtins::{PyStrRef, PyUtf8StrRef},
};
use std::sync::{Mutex, OnceLock};

const PY_MOD_CREATE: c_int = 1;
const PY_MOD_EXEC: c_int = 2;

#[repr(C)]
struct RawPyObject {
    _private: [usize; 2],
}

#[repr(C)]
struct RawPyModuleDefBase {
    ob_base: RawPyObject,
    m_init: Option<unsafe extern "C" fn() -> *mut PyObject>,
    m_index: isize,
    m_copy: *mut PyObject,
}

#[repr(C)]
struct RawPyModuleDefSlot {
    slot: c_int,
    value: *mut c_void,
}

#[repr(C)]
struct RawPyModuleDef {
    m_base: RawPyModuleDefBase,
    m_name: *const c_char,
    m_doc: *const c_char,
    m_size: isize,
    m_methods: *mut c_void,
    m_slots: *mut RawPyModuleDefSlot,
    m_traverse: *mut c_void,
    m_clear: *mut c_void,
    m_free: *mut c_void,
}

type RawPyInitFn = unsafe extern "C" fn() -> *mut PyObject;
type RawPyModuleExec = unsafe extern "C" fn(*mut PyObject) -> c_int;

fn dynamic_libs() -> &'static Mutex<Vec<Library>> {
    static DYNAMIC_LIBS: OnceLock<Mutex<Vec<Library>>> = OnceLock::new();
    DYNAMIC_LIBS.get_or_init(|| Mutex::new(Vec::new()))
}

#[unsafe(no_mangle)]
pub extern "C" fn RustPython_CreateDynamicExtension(spec: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| -> PyResult<PyObjectRef> {
        let spec = unsafe { &*spec }.to_owned();
        let name: PyUtf8StrRef = spec.get_attr("name", vm)?.try_into_value(vm)?;
        let origin: PyStrRef = spec.get_attr("origin", vm)?.try_into_value(vm)?;
        let short_name = name
            .as_str()
            .rsplit('.')
            .next()
            .expect("module name should not be empty");
        let init_symbol = format!("PyInit_{short_name}");

        #[cfg(unix)]
        let lib: Library = unsafe {
            UnixLibrary::open(
                Some(origin.to_str().ok_or_else(|| {
                    vm.new_import_error(
                        "module origin is not valid UTF-8",
                        name.clone().into_wtf8(),
                    )
                })?),
                libc::RTLD_NOW | libc::RTLD_GLOBAL,
            )
            .map_err(|err| vm.new_import_error(err.to_string(), name.clone().into_wtf8()))?
            .into()
        };

        let init: libloading::Symbol<'_, RawPyInitFn> = unsafe { lib.get(init_symbol.as_bytes()) }
            .map_err(|err| vm.new_import_error(err.to_string(), name.clone().into_wtf8()))?;

        let raw = unsafe { init() };
        if raw.is_null() {
            let err_symbol_name = CString::new("PyErr_GetRaisedException").unwrap();
            let err_symbol = unsafe { libc::dlsym(libc::RTLD_DEFAULT, err_symbol_name.as_ptr()) };
            if !err_symbol.is_null() {
                let get_exc: unsafe extern "C" fn() -> *mut PyObject = unsafe { mem::transmute(err_symbol) };
                let raw_exc = unsafe { get_exc() };
                if !raw_exc.is_null() {
                    let exc_obj = unsafe { &*raw_exc }.to_owned();
                    if let Ok(err) = exc_obj.downcast() {
                        return Err(err);
                    }
                }
            }
            return Err(vm.new_import_error(
                "native module init returned NULL",
                name.clone().into_wtf8(),
            ));
        }

        let raw_def = unsafe { &*raw.cast::<RawPyModuleDef>() };
        let module_name = if raw_def.m_name.is_null() {
            name.as_str()
        } else {
            unsafe { CStr::from_ptr(raw_def.m_name) }
                .to_str()
                .unwrap_or(name.as_str())
        };
        let doc = if raw_def.m_doc.is_null() {
            None
        } else {
            Some(vm.ctx.new_str(
                unsafe { CStr::from_ptr(raw_def.m_doc) }
                    .to_str()
                    .unwrap_or_default(),
            ))
        };

        let module = vm.new_module(module_name, vm.ctx.new_dict(), doc);
        let sys_modules = vm.sys_module.get_attr("modules", vm)?;
        sys_modules.set_item(name.as_pystr(), module.clone().into(), vm)?;

        let mut slot = raw_def.m_slots;
        while !slot.is_null() {
            let current = unsafe { &*slot };
            if current.slot == 0 {
                break;
            }
            match current.slot {
                PY_MOD_CREATE => {
                    // PyO3 currently uses exec slots in this experiment.
                }
                PY_MOD_EXEC => {
                    let exec: RawPyModuleExec = unsafe { mem::transmute(current.value) };
                    let rc = unsafe { exec(module.as_object().as_raw().cast_mut()) };
                    if rc != 0 {
                        let err_symbol_name = CString::new("PyErr_GetRaisedException").unwrap();
                        let err_symbol =
                            unsafe { libc::dlsym(libc::RTLD_DEFAULT, err_symbol_name.as_ptr()) };
                        if !err_symbol.is_null() {
                            let get_exc: unsafe extern "C" fn() -> *mut PyObject =
                                unsafe { mem::transmute(err_symbol) };
                            let raw_exc = unsafe { get_exc() };
                            if !raw_exc.is_null() {
                                let exc_obj = unsafe { &*raw_exc }.to_owned();
                                if let Ok(err) = exc_obj.downcast() {
                                    return Err(err);
                                }
                            }
                        }
                        return Err(vm.new_import_error(
                            format!("native module exec failed for {}", name.as_str()),
                            name.clone().into_wtf8(),
                        ));
                    }
                }
                _ => {}
            }
            slot = unsafe { slot.add(1) };
        }

        dynamic_libs().lock().unwrap().push(lib);
        Ok(module.into())
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn RustPython_ExecDynamicExtension(_module: *mut PyObject) -> c_int {
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn RustPython_TakeDynamicExtensionError() -> *mut PyObject {
    let symbol_name = CString::new("PyErr_GetRaisedException").unwrap();
    let symbol = unsafe { libc::dlsym(libc::RTLD_DEFAULT, symbol_name.as_ptr()) };
    if symbol.is_null() {
        return core::ptr::null_mut();
    }
    let get_exc: unsafe extern "C" fn() -> *mut PyObject = unsafe { mem::transmute(symbol) };
    unsafe { get_exc() }
}

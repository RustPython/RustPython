#![allow(non_snake_case)]

use crate::{PyRef, VirtualMachine, builtins::PyModule};

pub(crate) fn make_module(vm: &VirtualMachine) -> PyRef<PyModule> {
    let module = winreg::make_module(vm);
    module
}

#[pymodule]
mod winreg {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::sync::Arc;

    use crate::common::lock::PyRwLock;
    use crate::protocol::PyNumberMethods;
    use crate::types::AsNumber;
    use crate::{PyPayload, PyRef, PyResult, VirtualMachine};
    use windows_sys::Win32::Foundation;
    use windows_sys::Win32::System::Registry;

    pub(crate) fn to_utf16<P: AsRef<OsStr>>(s: P) -> Vec<u16> {
        s.as_ref().encode_wide().chain(Some(0)).collect()
    }

    // access rights
    #[pyattr]
    pub use windows_sys::Win32::System::Registry::{
        KEY_ALL_ACCESS, KEY_CREATE_LINK, KEY_CREATE_SUB_KEY, KEY_ENUMERATE_SUB_KEYS, KEY_EXECUTE,
        KEY_NOTIFY, KEY_QUERY_VALUE, KEY_READ, KEY_SET_VALUE, KEY_WOW64_32KEY, KEY_WOW64_64KEY,
        KEY_WRITE,
    };
    // value types
    #[pyattr]
    pub use windows_sys::Win32::System::Registry::{
        REG_BINARY, REG_CREATED_NEW_KEY, REG_DWORD, REG_DWORD_BIG_ENDIAN, REG_DWORD_LITTLE_ENDIAN,
        REG_EXPAND_SZ, REG_FULL_RESOURCE_DESCRIPTOR, REG_LINK, REG_MULTI_SZ, REG_NONE,
        REG_NOTIFY_CHANGE_ATTRIBUTES, REG_NOTIFY_CHANGE_LAST_SET, REG_NOTIFY_CHANGE_NAME,
        REG_NOTIFY_CHANGE_SECURITY, REG_OPENED_EXISTING_KEY, REG_OPTION_BACKUP_RESTORE,
        REG_OPTION_CREATE_LINK, REG_OPTION_NON_VOLATILE, REG_OPTION_OPEN_LINK, REG_OPTION_RESERVED,
        REG_OPTION_VOLATILE, REG_QWORD, REG_QWORD_LITTLE_ENDIAN, REG_RESOURCE_LIST,
        REG_RESOURCE_REQUIREMENTS_LIST, REG_SZ, REG_WHOLE_HIVE_VOLATILE,
    };

    #[pyattr(once)]
    fn HKEY_CLASSES_ROOT(_vm: &VirtualMachine) -> PyHKEYObject {
        PyHKEYObject {
            hkey: Arc::new(PyRwLock::new(Registry::HKEY_CLASSES_ROOT)),
        }
    }

    #[pyattr(once)]
    fn HKEY_CURRENT_USER(_vm: &VirtualMachine) -> PyHKEYObject {
        PyHKEYObject {
            hkey: Arc::new(PyRwLock::new(Registry::HKEY_CURRENT_USER)),
        }
    }

    #[pyattr(once)]
    fn HKEY_LOCAL_MACHINE(_vm: &VirtualMachine) -> PyHKEYObject {
        PyHKEYObject {
            hkey: Arc::new(PyRwLock::new(Registry::HKEY_LOCAL_MACHINE)),
        }
    }

    #[pyattr(once)]
    fn HKEY_USERS(_vm: &VirtualMachine) -> PyHKEYObject {
        PyHKEYObject {
            hkey: Arc::new(PyRwLock::new(Registry::HKEY_USERS)),
        }
    }

    #[pyattr(once)]
    fn HKEY_PERFORMANCE_DATA(_vm: &VirtualMachine) -> PyHKEYObject {
        PyHKEYObject {
            hkey: Arc::new(PyRwLock::new(Registry::HKEY_PERFORMANCE_DATA)),
        }
    }

    #[pyattr(once)]
    fn HKEY_CURRENT_CONFIG(_vm: &VirtualMachine) -> PyHKEYObject {
        PyHKEYObject {
            hkey: Arc::new(PyRwLock::new(Registry::HKEY_CURRENT_CONFIG)),
        }
    }

    #[pyattr(once)]
    fn HKEY_DYN_DATA(_vm: &VirtualMachine) -> PyHKEYObject {
        PyHKEYObject {
            hkey: Arc::new(PyRwLock::new(Registry::HKEY_DYN_DATA)),
        }
    }

    #[pyattr]
    #[pyclass(name)]
    #[derive(Clone, Debug, PyPayload)]
    pub struct PyHKEYObject {
        hkey: Arc<PyRwLock<Registry::HKEY>>,
    }

    // TODO: Fix
    unsafe impl Send for PyHKEYObject {}
    unsafe impl Sync for PyHKEYObject {}

    #[pyclass(with(AsNumber))]
    impl PyHKEYObject {
        #[pymethod(magic)]
        fn bool(&self) -> bool {
            !self.hkey.read().is_null()
        }

        #[pymethod(magic)]
        fn int(&self) -> usize {
            *self.hkey.read() as usize
        }

        #[pymethod(magic)]
        fn str(&self) -> String {
            format!("<PyHKEY:{}>", *self.hkey.read() as usize)
        }

        #[pymethod]
        fn Close(&self, vm: &VirtualMachine) -> PyResult<()> {
            let res = unsafe { Registry::RegCloseKey(*self.hkey.write()) };
            if res == 0 {
                Ok(())
            } else {
                Err(vm.new_os_error("msg TODO".to_string()))
            }
        }

        #[pymethod]
        fn Detach(&self) -> PyResult<usize> {
            let hkey = *self.hkey.write();
            // std::mem::forget(self);
            // TODO: Fix this
            Ok(hkey as usize)
        }

        // fn AsHKEY(object: &PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        //     if vm.is_none(object) {
        //         return Err(vm.new_type_error("cannot convert None to HKEY".to_owned()))
        //     } else if let Some(hkey) = object.downcast_ref::<PyHKEYObject>() {
        //         Ok(true)
        //     } else {
        //         Err(vm.new_type_error("The object is not a PyHKEY object".to_owned()))
        //     }
        // }

        // TODO: __enter__ and __exit__
    }

    pub const HKEY_ERR_MSG: &str = "bad operand type";

    impl PyHKEYObject {
        pub fn unary_fail(vm: &VirtualMachine) -> PyResult {
            Err(vm.new_type_error(HKEY_ERR_MSG.to_owned()))
        }

        pub fn binary_fail(vm: &VirtualMachine) -> PyResult {
            Err(vm.new_type_error(HKEY_ERR_MSG.to_owned()))
        }

        pub fn ternary_fail(vm: &VirtualMachine) -> PyResult {
            Err(vm.new_type_error(HKEY_ERR_MSG.to_owned()))
        }
    }

    impl AsNumber for PyHKEYObject {
        fn as_number() -> &'static PyNumberMethods {
            static AS_NUMBER: PyNumberMethods = PyNumberMethods {
                add: Some(|_a, _b, vm| PyHKEYObject::binary_fail(vm)),
                subtract: Some(|_a, _b, vm| PyHKEYObject::binary_fail(vm)),
                multiply: Some(|_a, _b, vm| PyHKEYObject::binary_fail(vm)),
                remainder: Some(|_a, _b, vm| PyHKEYObject::binary_fail(vm)),
                divmod: Some(|_a, _b, vm| PyHKEYObject::binary_fail(vm)),
                power: Some(|_a, _b, _c, vm| PyHKEYObject::ternary_fail(vm)),
                negative: Some(|_a, vm| PyHKEYObject::unary_fail(vm)),
                positive: Some(|_a, vm| PyHKEYObject::unary_fail(vm)),
                absolute: Some(|_a, vm| PyHKEYObject::unary_fail(vm)),
                boolean: Some(|a, vm| {
                    if let Some(a) = a.downcast_ref::<PyHKEYObject>() {
                        Ok(a.bool())
                    } else {
                        PyHKEYObject::unary_fail(vm)?;
                        unreachable!()
                    }
                }),
                invert: Some(|_a, vm| PyHKEYObject::unary_fail(vm)),
                lshift: Some(|_a, _b, vm| PyHKEYObject::binary_fail(vm)),
                rshift: Some(|_a, _b, vm| PyHKEYObject::binary_fail(vm)),
                and: Some(|_a, _b, vm| PyHKEYObject::binary_fail(vm)),
                xor: Some(|_a, _b, vm| PyHKEYObject::binary_fail(vm)),
                or: Some(|_a, _b, vm| PyHKEYObject::binary_fail(vm)),
                int: Some(|a, vm| {
                    if let Some(a) = a.downcast_ref::<PyHKEYObject>() {
                        Ok(vm.new_pyobj(a.int()))
                    } else {
                        PyHKEYObject::unary_fail(vm)?;
                        unreachable!()
                    }
                }),
                float: Some(|_a, vm| PyHKEYObject::unary_fail(vm)),
                ..PyNumberMethods::NOT_IMPLEMENTED
            };
            &AS_NUMBER
        }
    }

    // TODO: Computer name can be `None``
    #[pyfunction]
    fn ConnectRegistry(computer_name: String, key: PyRef<PyHKEYObject>, vm: &VirtualMachine) -> PyResult<()> {
        let wide_computer_name = to_utf16(computer_name);
        let res = unsafe {
            Registry::RegConnectRegistryW(
                wide_computer_name.as_ptr(),
                *key.hkey.read(),
                std::ptr::null_mut(),
            )
        };
        if res == 0 {
            Ok(())
        } else {
            Err(vm.new_os_error(format!("error code: {}", res)))
        }
    }

    #[pyfunction]
    fn CreateKey(key: PyRef<PyHKEYObject>, sub_key: String, vm: &VirtualMachine) -> PyResult<()> {
        let mut wide_sub_key = to_utf16(sub_key);
        wide_sub_key.push(0);
        let res = unsafe {
            Registry::RegCreateKeyW(*key.hkey.read(), wide_sub_key.as_ptr(), std::ptr::null_mut())
        };
        if res == 0 {
            Ok(())
        } else {
            Err(vm.new_os_error(format!("error code: {}", res)))
        }
    }

    #[derive(FromArgs, Debug)]
    struct CreateKeyExArgs {
        #[pyarg(any)]
        key: PyRef<PyHKEYObject>,
        #[pyarg(any)]
        sub_key: String,
        #[pyarg(any, default = "0")]
        reserved: u32,
        #[pyarg(any, default = "windows_sys::Win32::System::Registry::KEY_WRITE")]
        access: u32,
    }

    #[pyfunction]
    fn CreateKeyEx(args: CreateKeyExArgs, vm: &VirtualMachine) -> PyResult<PyHKEYObject> {
        let wide_sub_key = to_utf16(args.sub_key);
        let res: *mut *mut std::ffi::c_void = core::ptr::null_mut();
        let err = unsafe {
            let key = *args.key.hkey.read();
            Registry::RegCreateKeyExW(
                key,
                wide_sub_key.as_ptr(),
                args.reserved,
                std::ptr::null_mut(),
                0,
                args.access,
                std::ptr::null_mut(),
                res,
                std::ptr::null_mut(),
            )
        };
        if err == 0 {
            unsafe {
                Ok(PyHKEYObject {
                    hkey: Arc::new(PyRwLock::new(*res)),
                })
            }
        } else {
            Err(vm.new_os_error(format!("error code: {}", err)))
        }
    }

    #[pyfunction]
    fn DeleteKey(key: PyRef<PyHKEYObject>, sub_key: String, vm: &VirtualMachine) -> PyResult<()> {
        let wide_sub_key = to_utf16(sub_key);
        let res = unsafe { Registry::RegDeleteKeyW(*key.hkey.read(), wide_sub_key.as_ptr()) };
        if res == 0 {
            Ok(())
        } else {
            Err(vm.new_os_error(format!("error code: {}", res)))
        }
    }

    #[derive(FromArgs, Debug)]
    struct DeleteKeyExArgs {
        #[pyarg(any)]
        key: PyRef<PyHKEYObject>,
        #[pyarg(any)]
        sub_key: String,
        #[pyarg(any, default = "0")]
        reserved: u32,
        #[pyarg(any, default = "windows_sys::Win32::System::Registry::KEY_WOW64_32KEY")]
        access: u32,
    }

    #[pyfunction]
    fn DeleteKeyEx(args: DeleteKeyExArgs, vm: &VirtualMachine) -> PyResult<()> {
        let wide_sub_key = to_utf16(args.sub_key);
        let res = unsafe {
            Registry::RegDeleteKeyExW(
                *args.key.hkey.read(),
                wide_sub_key.as_ptr(),
                args.reserved,
                args.access,
            )
        };
        if res == 0 {
            Ok(())
        } else {
            Err(vm.new_os_error(format!("error code: {}", res)))
        }
    }

    #[pyfunction]
    fn FlushKey(key: PyRef<PyHKEYObject>, vm: &VirtualMachine) -> PyResult<()> {
        let res = unsafe { Registry::RegFlushKey(*key.hkey.read()) };
        if res == 0 {
            Ok(())
        } else {
            Err(vm.new_os_error(format!("error code: {}", res)))
        }
    }

    #[pyfunction]
    fn LoadKey(
        key: PyRef<PyHKEYObject>,
        sub_key: String,
        file_name: String,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let sub_key = to_utf16(sub_key);
        let file_name = to_utf16(file_name);
        let res = unsafe {
            Registry::RegLoadKeyW(*key.hkey.read(), sub_key.as_ptr(), file_name.as_ptr())
        };
        if res == 0 {
            Ok(())
        } else {
            Err(vm.new_os_error(format!("error code: {}", res)))
        }
    }

    #[derive(Debug, FromArgs)]
    struct OpenKeyArgs {
        #[pyarg(any)]
        key: PyRef<PyHKEYObject>,
        #[pyarg(any)]
        sub_key: String,
        #[pyarg(any, default = "0")]
        reserved: u32,
        #[pyarg(any, default = "windows_sys::Win32::System::Registry::KEY_READ")]
        access: u32,
    }

    #[pyfunction]
    #[pyfunction(name = "OpenKeyEx")]
    fn OpenKey(args: OpenKeyArgs, vm: &VirtualMachine) -> PyResult<PyHKEYObject> {
        let wide_sub_key = to_utf16(args.sub_key);
        let res: *mut *mut std::ffi::c_void = core::ptr::null_mut();
        let err = unsafe {
            let key = *args.key.hkey.read();
            Registry::RegOpenKeyExW(key, wide_sub_key.as_ptr(), args.reserved, args.access, res)
        };
        if err == 0 {
            unsafe {
                Ok(PyHKEYObject {
                    hkey: Arc::new(PyRwLock::new(*res)),
                })
            }
        } else {
            Err(vm.new_os_error(format!("error code: {}", err)))
        }
    }

    #[pyfunction]
    fn QueryInfoKey(key: PyRef<PyHKEYObject>, vm: &VirtualMachine) -> PyResult<()> {
        let key = *key.hkey.read();
        let mut lpcsubkeys: u32 = 0;
        let mut lpcvalues: u32 = 0;
        let lpftlastwritetime: *mut Foundation::FILETIME = std::ptr::null_mut();
        let err = unsafe {
            Registry::RegQueryInfoKeyW(
                key,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                0 as _,
                &mut lpcsubkeys,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut lpcvalues,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                lpftlastwritetime,
            )
        };

        if err != 0 {
            Err(vm.new_os_error(format!("error code: {}", err)))
        } else {
            Ok(())
        }
    }

    #[pyfunction]
    fn QueryValue(key: PyRef<PyHKEYObject>, sub_key: String, vm: &VirtualMachine) -> PyResult<()> {
        let key = *key.hkey.read();
        let mut lpcbdata: i32 = 0;
        // let mut lpdata = 0;
        let wide_sub_key = to_utf16(sub_key);
        let err = unsafe {
            Registry::RegQueryValueW(key, wide_sub_key.as_ptr(), std::ptr::null_mut(), &mut lpcbdata)
        };

        if err != 0 {
            return Err(vm.new_os_error(format!("error code: {}", err)));
        }

        Ok(())
    }

    // TODO: QueryValueEx
    #[pyfunction]
    fn SaveKey(key: PyRef<PyHKEYObject>, file_name: String, vm: &VirtualMachine) -> PyResult<()> {
        let file_name = to_utf16(file_name);
        let res = unsafe {
            Registry::RegSaveKeyW(*key.hkey.read(), file_name.as_ptr(), std::ptr::null_mut())
        };
        if res == 0 {
            Ok(())
        } else {
            Err(vm.new_os_error(format!("error code: {}", res)))
        }
    }

    // #[pyfunction]
    // fn SetValue(key: PyRef<PyHKEYObject>, sub_key: String, typ: String, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
    //     let res = unsafe {
    //         Registry::RegSetValueA(
    //             *key.hkey.read(),
    //             sub_key.as_ptr(),
    //             Registry::REG_SZ,
    //             value.as_ptr(),
    //             value.len() as u32,
    //         )
    //     };
    //     if res == 0 {
    //         Ok(())
    //     } else {
    //         Err(vm.new_os_error("msg TODO".to_string()))
    //     }
    // }

    // TODO: SetValuEx

    #[pyfunction]
    fn EnableReflectionKey(key: PyRef<PyHKEYObject>, vm: &VirtualMachine) -> PyResult<()> {
        let res = unsafe { Registry::RegEnableReflectionKey(*key.hkey.read()) };
        if res == 0 {
            Ok(())
        } else {
            Err(vm.new_os_error(format!("error code: {}", res)))
        }
    }

    #[pyfunction]
    fn ExpandEnvironmentStrings(i: String) -> PyResult<String> {
        let mut out = vec![0; 1024];
        let r = unsafe {
            windows_sys::Win32::System::Environment::ExpandEnvironmentStringsA(
                i.as_ptr(),
                out.as_mut_ptr(),
                out.len() as u32,
            )
        };
        let s = String::from_utf8(out[..r as usize].to_vec())
            .unwrap()
            .replace("\0", "")
            .replace("\x02", "")
            .to_string();

        Ok(s)
    }
}

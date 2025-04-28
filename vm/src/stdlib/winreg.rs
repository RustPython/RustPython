// cspell:disable
#![allow(non_snake_case)]

use crate::{PyRef, VirtualMachine, builtins::PyModule};

pub(crate) fn make_module(vm: &VirtualMachine) -> PyRef<PyModule> {
    winreg::make_module(vm)
}

#[pymodule]
mod winreg {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr;
    use std::sync::Arc;

    use crate::builtins::{PyInt, PyTuple};
    use crate::common::lock::PyRwLock;
    use crate::function::FuncArgs;
    use crate::protocol::PyNumberMethods;
    use crate::types::AsNumber;
    use crate::{PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine};

    use windows_sys::Win32::Foundation::{self, ERROR_MORE_DATA};
    use windows_sys::Win32::System::Registry;

    use num_traits::ToPrimitive;

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
            #[allow(clippy::arc_with_non_send_sync)]
            hkey: Arc::new(PyRwLock::new(Registry::HKEY_CLASSES_ROOT)),
        }
    }

    #[pyattr(once)]
    fn HKEY_CURRENT_USER(_vm: &VirtualMachine) -> PyHKEYObject {
        PyHKEYObject {
            #[allow(clippy::arc_with_non_send_sync)]
            hkey: Arc::new(PyRwLock::new(Registry::HKEY_CURRENT_USER)),
        }
    }

    #[pyattr(once)]
    fn HKEY_LOCAL_MACHINE(_vm: &VirtualMachine) -> PyHKEYObject {
        PyHKEYObject {
            #[allow(clippy::arc_with_non_send_sync)]
            hkey: Arc::new(PyRwLock::new(Registry::HKEY_LOCAL_MACHINE)),
        }
    }

    #[pyattr(once)]
    fn HKEY_USERS(_vm: &VirtualMachine) -> PyHKEYObject {
        PyHKEYObject {
            #[allow(clippy::arc_with_non_send_sync)]
            hkey: Arc::new(PyRwLock::new(Registry::HKEY_USERS)),
        }
    }

    #[pyattr(once)]
    fn HKEY_PERFORMANCE_DATA(_vm: &VirtualMachine) -> PyHKEYObject {
        PyHKEYObject {
            #[allow(clippy::arc_with_non_send_sync)]
            hkey: Arc::new(PyRwLock::new(Registry::HKEY_PERFORMANCE_DATA)),
        }
    }

    #[pyattr(once)]
    fn HKEY_CURRENT_CONFIG(_vm: &VirtualMachine) -> PyHKEYObject {
        PyHKEYObject {
            #[allow(clippy::arc_with_non_send_sync)]
            hkey: Arc::new(PyRwLock::new(Registry::HKEY_CURRENT_CONFIG)),
        }
    }

    #[pyattr(once)]
    fn HKEY_DYN_DATA(_vm: &VirtualMachine) -> PyHKEYObject {
        PyHKEYObject {
            #[allow(clippy::arc_with_non_send_sync)]
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
        #[pygetset]
        fn handle(&self) -> usize {
            *self.hkey.read() as usize
        }

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
            *self.hkey.write() = std::ptr::null_mut();
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

        #[pymethod(magic)]
        fn enter(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
            Ok(zelf)
        }

        #[pymethod(magic)]
        fn exit(zelf: PyRef<Self>, _args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
            let res = unsafe { Registry::RegCloseKey(*zelf.hkey.write()) };
            *zelf.hkey.write() = std::ptr::null_mut();
            if res == 0 {
                Ok(())
            } else {
                Err(vm.new_os_error("msg TODO".to_string()))
            }
        }
    }

    impl Drop for PyHKEYObject {
        fn drop(&mut self) {
            unsafe {
                let hkey = *self.hkey.write();
                if !hkey.is_null() {
                    Registry::RegCloseKey(hkey);
                }
            }
        }
    }

    pub const HKEY_ERR_MSG: &str = "bad operand type";

    impl PyHKEYObject {
        pub fn new(hkey: *mut std::ffi::c_void) -> Self {
            Self {
                #[allow(clippy::arc_with_non_send_sync)]
                hkey: Arc::new(PyRwLock::new(hkey)),
            }
        }

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

    // TODO: Computer name can be `None`
    #[pyfunction]
    fn ConnectRegistry(
        computer_name: Option<String>,
        key: PyRef<PyHKEYObject>,
        vm: &VirtualMachine,
    ) -> PyResult<PyHKEYObject> {
        if let Some(computer_name) = computer_name {
            let mut ret_key = std::ptr::null_mut();
            let wide_computer_name = to_utf16(computer_name);
            let res = unsafe {
                Registry::RegConnectRegistryW(
                    wide_computer_name.as_ptr(),
                    *key.hkey.read(),
                    &mut ret_key,
                )
            };
            if res == 0 {
                Ok(PyHKEYObject::new(ret_key))
            } else {
                Err(vm.new_os_error(format!("error code: {}", res)))
            }
        } else {
            let mut ret_key = std::ptr::null_mut();
            let res = unsafe {
                Registry::RegConnectRegistryW(std::ptr::null_mut(), *key.hkey.read(), &mut ret_key)
            };
            if res == 0 {
                Ok(PyHKEYObject::new(ret_key))
            } else {
                Err(vm.new_os_error(format!("error code: {}", res)))
            }
        }
    }

    #[pyfunction]
    fn CreateKey(
        key: PyRef<PyHKEYObject>,
        sub_key: String,
        vm: &VirtualMachine,
    ) -> PyResult<PyHKEYObject> {
        let wide_sub_key = to_utf16(sub_key);
        let mut out_key = std::ptr::null_mut();
        let res = unsafe {
            Registry::RegCreateKeyW(*key.hkey.read(), wide_sub_key.as_ptr(), &mut out_key)
        };
        if res == 0 {
            Ok(PyHKEYObject::new(out_key))
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
        #[pyarg(any, default = 0)]
        reserved: u32,
        #[pyarg(any, default = windows_sys::Win32::System::Registry::KEY_WRITE)]
        access: u32,
    }

    #[pyfunction]
    fn CreateKeyEx(args: CreateKeyExArgs, vm: &VirtualMachine) -> PyResult<PyHKEYObject> {
        let wide_sub_key = to_utf16(args.sub_key);
        let mut res: *mut std::ffi::c_void = core::ptr::null_mut();
        let err = unsafe {
            let key = *args.key.hkey.read();
            Registry::RegCreateKeyExW(
                key,
                wide_sub_key.as_ptr(),
                args.reserved,
                std::ptr::null(),
                Registry::REG_OPTION_NON_VOLATILE,
                args.access,
                std::ptr::null(),
                &mut res,
                std::ptr::null_mut(),
            )
        };
        if err == 0 {
            Ok(PyHKEYObject {
                #[allow(clippy::arc_with_non_send_sync)]
                hkey: Arc::new(PyRwLock::new(res)),
            })
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
        #[pyarg(any, default = 0)]
        reserved: u32,
        #[pyarg(any, default = windows_sys::Win32::System::Registry::KEY_WOW64_32KEY)]
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

    // #[pyfunction]
    // fn EnumKey(key: PyRef<PyHKEYObject>, index: i32, vm: &VirtualMachine) -> PyResult<String> {
    //     let mut tmpbuf = [0u16; 257];
    //     let mut len = std::mem::sizeof(tmpbuf.len())/std::mem::sizeof(tmpbuf[0]);
    //     let res = unsafe {
    //         Registry::RegEnumKeyExW(
    //             *key.hkey.read(),
    //             index as u32,
    //             tmpbuf.as_mut_ptr(),
    //             &mut len,
    //             std::ptr::null_mut(),
    //             std::ptr::null_mut(),
    //             std::ptr::null_mut(),
    //             std::ptr::null_mut(),
    //         )
    //     };
    //     if res != 0 {
    //         return Err(vm.new_os_error(format!("error code: {}", res)));
    //     }
    //     let s = String::from_utf16(&tmpbuf[..len as usize])
    //         .map_err(|e| vm.new_value_error(format!("UTF16 error: {}", e)))?;
    //     Ok(s)
    // }

    #[pyfunction]
    fn EnumValue(hkey: PyRef<PyHKEYObject>, index: u32, vm: &VirtualMachine) -> PyResult {
        // Query registry for the required buffer sizes.
        let mut ret_value_size: u32 = 0;
        let mut ret_data_size: u32 = 0;
        let hkey: *mut std::ffi::c_void = *hkey.hkey.read();
        let rc = unsafe {
            Registry::RegQueryInfoKeyW(
                hkey,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                &mut ret_value_size as *mut u32,
                &mut ret_data_size as *mut u32,
                ptr::null_mut(),
                ptr::null_mut(),
            )
        };
        if rc != 0 {
            return Err(vm.new_os_error(format!("RegQueryInfoKeyW failed with error code {}", rc)));
        }

        // Include room for null terminators.
        ret_value_size += 1;
        ret_data_size += 1;
        let mut buf_value_size = ret_value_size;
        let mut buf_data_size = ret_data_size;

        // Allocate buffers.
        let mut ret_value_buf: Vec<u16> = vec![0; ret_value_size as usize];
        let mut ret_data_buf: Vec<u8> = vec![0; ret_data_size as usize];

        // Loop to enumerate the registry value.
        loop {
            let mut current_value_size = ret_value_size;
            let mut current_data_size = ret_data_size;
            let rc = unsafe {
                Registry::RegEnumValueW(
                    hkey,
                    index,
                    ret_value_buf.as_mut_ptr(),
                    &mut current_value_size as *mut u32,
                    ptr::null_mut(),
                    {
                        // typ will hold the registry data type.
                        let mut t = 0u32;
                        &mut t
                    },
                    ret_data_buf.as_mut_ptr(),
                    &mut current_data_size as *mut u32,
                )
            };
            if rc == ERROR_MORE_DATA {
                // Double the buffer sizes.
                buf_data_size *= 2;
                buf_value_size *= 2;
                ret_data_buf.resize(buf_data_size as usize, 0);
                ret_value_buf.resize(buf_value_size as usize, 0);
                // Reset sizes for next iteration.
                ret_value_size = buf_value_size;
                ret_data_size = buf_data_size;
                continue;
            }
            if rc != 0 {
                return Err(vm.new_os_error(format!("RegEnumValueW failed with error code {}", rc)));
            }

            // At this point, current_value_size and current_data_size have been updated.
            // Retrieve the registry type.
            let mut reg_type: u32 = 0;
            unsafe {
                Registry::RegEnumValueW(
                    hkey,
                    index,
                    ret_value_buf.as_mut_ptr(),
                    &mut current_value_size as *mut u32,
                    ptr::null_mut(),
                    &mut reg_type as *mut u32,
                    ret_data_buf.as_mut_ptr(),
                    &mut current_data_size as *mut u32,
                )
            };

            // Convert the registry value name from UTF‑16.
            let name_len = ret_value_buf
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(ret_value_buf.len());
            let name = String::from_utf16(&ret_value_buf[..name_len])
                .map_err(|e| vm.new_value_error(format!("UTF16 conversion error: {}", e)))?;

            // Slice the data buffer to the actual size returned.
            let data_slice = &ret_data_buf[..current_data_size as usize];
            let py_data = reg_to_py(vm, data_slice, reg_type)?;

            // Return tuple (value_name, data, type)
            return Ok(vm
                .ctx
                .new_tuple(vec![
                    vm.ctx.new_str(name).into(),
                    py_data,
                    vm.ctx.new_int(reg_type).into(),
                ])
                .into());
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
        #[pyarg(any, default = 0)]
        reserved: u32,
        #[pyarg(any, default = windows_sys::Win32::System::Registry::KEY_READ)]
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
                    #[allow(clippy::arc_with_non_send_sync)]
                    hkey: Arc::new(PyRwLock::new(*res)),
                })
            }
        } else {
            Err(vm.new_os_error(format!("error code: {}", err)))
        }
    }

    #[pyfunction]
    fn QueryInfoKey(key: PyRef<PyHKEYObject>, vm: &VirtualMachine) -> PyResult<PyRef<PyTuple>> {
        let key = *key.hkey.read();
        let mut lpcsubkeys: u32 = 0;
        let mut lpcvalues: u32 = 0;
        let mut lpftlastwritetime: Foundation::FILETIME = unsafe { std::mem::zeroed() };
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
                &mut lpftlastwritetime,
            )
        };

        if err != 0 {
            return Err(vm.new_os_error(format!("error code: {}", err)));
        }
        let l: u64 = (lpftlastwritetime.dwHighDateTime as u64) << 32
            | lpftlastwritetime.dwLowDateTime as u64;
        let tup: Vec<PyObjectRef> = vec![
            vm.ctx.new_int(lpcsubkeys).into(),
            vm.ctx.new_int(lpcvalues).into(),
            vm.ctx.new_int(l).into(),
        ];
        Ok(vm.ctx.new_tuple(tup))
    }

    #[pyfunction]
    fn QueryValue(key: PyRef<PyHKEYObject>, sub_key: String, vm: &VirtualMachine) -> PyResult<()> {
        let key = *key.hkey.read();
        let mut lpcbdata: i32 = 0;
        // let mut lpdata = 0;
        let wide_sub_key = to_utf16(sub_key);
        let err = unsafe {
            Registry::RegQueryValueW(
                key,
                wide_sub_key.as_ptr(),
                std::ptr::null_mut(),
                &mut lpcbdata,
            )
        };

        if err != 0 {
            return Err(vm.new_os_error(format!("error code: {}", err)));
        }

        Ok(())
    }

    #[pyfunction]
    fn QueryValueEx(
        key: PyRef<PyHKEYObject>,
        name: String,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        let wide_name = to_utf16(name);
        let mut buf_size = 0;
        let res = unsafe {
            Registry::RegQueryValueExW(
                *key.hkey.read(),
                wide_name.as_ptr(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut buf_size,
            )
        };
        // TODO: res == ERROR_MORE_DATA
        if res != 0 {
            return Err(vm.new_os_error(format!("error code: {}", res)));
        }
        let mut retBuf = Vec::with_capacity(buf_size as usize);
        let mut typ = 0;
        let res = unsafe {
            Registry::RegQueryValueExW(
                *key.hkey.read(),
                wide_name.as_ptr(),
                std::ptr::null_mut(),
                &mut typ,
                retBuf.as_mut_ptr(),
                &mut buf_size,
            )
        };
        // TODO: res == ERROR_MORE_DATA
        if res != 0 {
            return Err(vm.new_os_error(format!("error code: {}", res)));
        }
        let obj = reg_to_py(vm, retBuf.as_slice(), typ)?;
        Ok(obj)
    }

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

    #[pyfunction]
    fn SetValue(
        key: PyRef<PyHKEYObject>,
        sub_key: String,
        typ: u32,
        value: String,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        if typ != Registry::REG_SZ {
            return Err(vm.new_type_error("type must be winreg.REG_SZ".to_string()));
        }

        let wide_sub_key = to_utf16(sub_key);

        // TODO: Value check
        if *key.hkey.read() == Registry::HKEY_PERFORMANCE_DATA {
            return Err(vm.new_os_error("Cannot set value on HKEY_PERFORMANCE_DATA".to_string()));
        }

        // if (sub_key && sub_key[0]) {
        //     // TODO: create key
        // }

        let res = unsafe {
            Registry::RegSetValueExW(
                *key.hkey.read(),
                wide_sub_key.as_ptr(),
                0,
                typ,
                value.as_ptr(),
                value.len() as u32,
            )
        };

        if res == 0 {
            Ok(())
        } else {
            Err(vm.new_os_error(format!("error code: {}", res)))
        }
    }

    fn reg_to_py(vm: &VirtualMachine, ret_data: &[u8], typ: u32) -> PyResult {
        match typ {
            REG_DWORD => {
                // If there isn’t enough data, return 0.
                if ret_data.len() < std::mem::size_of::<u32>() {
                    Ok(vm.ctx.new_int(0).into())
                } else {
                    let val = u32::from_ne_bytes(ret_data[..4].try_into().unwrap());
                    Ok(vm.ctx.new_int(val).into())
                }
            }
            REG_QWORD => {
                if ret_data.len() < std::mem::size_of::<u64>() {
                    Ok(vm.ctx.new_int(0).into())
                } else {
                    let val = u64::from_ne_bytes(ret_data[..8].try_into().unwrap());
                    Ok(vm.ctx.new_int(val).into())
                }
            }
            REG_SZ | REG_EXPAND_SZ => {
                // Treat the data as a UTF-16 string.
                let u16_count = ret_data.len() / 2;
                let u16_slice = unsafe {
                    std::slice::from_raw_parts(ret_data.as_ptr() as *const u16, u16_count)
                };
                // Only use characters up to the first NUL.
                let len = u16_slice
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(u16_slice.len());
                let s = String::from_utf16(&u16_slice[..len])
                    .map_err(|e| vm.new_value_error(format!("UTF16 error: {}", e)))?;
                Ok(vm.ctx.new_str(s).into())
            }
            REG_MULTI_SZ => {
                if ret_data.is_empty() {
                    Ok(vm.ctx.new_list(vec![]).into())
                } else {
                    let u16_count = ret_data.len() / 2;
                    let u16_slice = unsafe {
                        std::slice::from_raw_parts(ret_data.as_ptr() as *const u16, u16_count)
                    };
                    let mut strings: Vec<PyObjectRef> = Vec::new();
                    let mut start = 0;
                    for (i, &c) in u16_slice.iter().enumerate() {
                        if c == 0 {
                            // An empty string signals the end.
                            if start == i {
                                break;
                            }
                            let s = String::from_utf16(&u16_slice[start..i])
                                .map_err(|e| vm.new_value_error(format!("UTF16 error: {}", e)))?;
                            strings.push(vm.ctx.new_str(s).into());
                            start = i + 1;
                        }
                    }
                    Ok(vm.ctx.new_list(strings).into())
                }
            }
            // For REG_BINARY and any other unknown types, return a bytes object if data exists.
            _ => {
                if ret_data.is_empty() {
                    Ok(vm.ctx.none())
                } else {
                    Ok(vm.ctx.new_bytes(ret_data.to_vec()).into())
                }
            }
        }
    }

    fn py2reg(value: PyObjectRef, typ: u32, vm: &VirtualMachine) -> PyResult<Option<Vec<u8>>> {
        match typ {
            REG_DWORD => {
                let val = value.downcast_ref::<PyInt>();
                if val.is_none() {
                    return Err(vm.new_type_error("value must be an integer".to_string()));
                }
                let val = val.unwrap().as_bigint().to_u32().unwrap();
                Ok(Some(val.to_le_bytes().to_vec()))
            }
            REG_QWORD => {
                let val = value.downcast_ref::<PyInt>();
                if val.is_none() {
                    return Err(vm.new_type_error("value must be an integer".to_string()));
                }
                let val = val.unwrap().as_bigint().to_u64().unwrap();
                Ok(Some(val.to_le_bytes().to_vec()))
            }
            // REG_SZ is fallthrough
            REG_EXPAND_SZ => {
                Err(vm
                    .new_type_error("TODO: RUSTPYTHON REG_EXPAND_SZ is not supported".to_string()))
            }
            REG_MULTI_SZ => {
                Err(vm.new_type_error("TODO: RUSTPYTHON REG_MULTI_SZ is not supported".to_string()))
            }
            // REG_BINARY is fallthrough
            _ => {
                if vm.is_none(&value) {
                    return Ok(None);
                }
                Err(vm.new_type_error("TODO: RUSTPYTHON Not supported".to_string()))
            }
        }
    }

    #[pyfunction]
    fn SetValueEx(
        key: PyRef<PyHKEYObject>,
        value_name: String,
        _reserved: u32,
        typ: u32,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        match py2reg(value, typ, vm) {
            Ok(Some(v)) => {
                let len = v.len() as u32;
                let ptr = v.as_ptr();
                let wide_value_name = to_utf16(value_name);
                let res = unsafe {
                    Registry::RegSetValueExW(
                        *key.hkey.read(),
                        wide_value_name.as_ptr(),
                        0,
                        typ,
                        ptr,
                        len,
                    )
                };
                if res != 0 {
                    return Err(vm.new_os_error(format!("error code: {}", res)));
                }
            }
            Ok(None) => {
                let len = 0;
                let ptr = std::ptr::null();
                let wide_value_name = to_utf16(value_name);
                let res = unsafe {
                    Registry::RegSetValueExW(
                        *key.hkey.read(),
                        wide_value_name.as_ptr(),
                        0,
                        typ,
                        ptr,
                        len,
                    )
                };
                if res != 0 {
                    return Err(vm.new_os_error(format!("error code: {}", res)));
                }
            }
            Err(_) => return Err(vm.new_type_error("value must be an integer".to_string())),
        }
        Ok(())
    }

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

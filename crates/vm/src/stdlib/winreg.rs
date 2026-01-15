// spell-checker:disable
#![allow(non_snake_case)]

use crate::{PyRef, VirtualMachine, builtins::PyModule};

pub(crate) fn make_module(vm: &VirtualMachine) -> PyRef<PyModule> {
    winreg::make_module(vm)
}

#[pymodule]
mod winreg {
    use crate::builtins::{PyInt, PyStr, PyTuple, PyTypeRef};
    use crate::common::hash::PyHash;
    use crate::common::windows::ToWideString;
    use crate::convert::TryFromObject;
    use crate::function::FuncArgs;
    use crate::object::AsObject;
    use crate::protocol::PyNumberMethods;
    use crate::types::{AsNumber, Hashable};
    use crate::{Py, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine};
    use crossbeam_utils::atomic::AtomicCell;
    use malachite_bigint::Sign;
    use num_traits::ToPrimitive;
    use std::ptr;
    use windows_sys::Win32::Foundation::{self, ERROR_MORE_DATA};
    use windows_sys::Win32::System::Registry;

    /// Atomic HKEY handle type for lock-free thread-safe access
    type AtomicHKEY = AtomicCell<Registry::HKEY>;

    /// Convert byte slice to UTF-16 slice (zero-copy when aligned)
    fn bytes_as_wide_slice(bytes: &[u8]) -> &[u16] {
        // SAFETY: Windows Registry API returns properly aligned UTF-16 data.
        // align_to handles any edge cases safely by returning empty prefix/suffix
        // if alignment doesn't match.
        let (prefix, u16_slice, suffix) = unsafe { bytes.align_to::<u16>() };
        debug_assert!(
            prefix.is_empty() && suffix.is_empty(),
            "Registry data should be u16-aligned"
        );
        u16_slice
    }

    // TODO: check if errno.rs can be reused here or not
    fn os_error_from_windows_code(
        vm: &VirtualMachine,
        code: i32,
        func_name: &str,
    ) -> crate::PyRef<crate::builtins::PyBaseException> {
        use windows_sys::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND};
        let msg = format!("[WinError {}] {}", code, func_name);
        let exc_type = match code as u32 {
            ERROR_FILE_NOT_FOUND => vm.ctx.exceptions.file_not_found_error.to_owned(),
            ERROR_ACCESS_DENIED => vm.ctx.exceptions.permission_error.to_owned(),
            _ => vm.ctx.exceptions.os_error.to_owned(),
        };
        vm.new_exception_msg(exc_type, msg)
    }

    /// Wrapper type for HKEY that can be created from PyHkey or int
    struct HKEYArg(Registry::HKEY);

    impl TryFromObject for HKEYArg {
        fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
            // Try PyHkey first
            if let Some(hkey_obj) = obj.downcast_ref::<PyHkey>() {
                return Ok(HKEYArg(hkey_obj.hkey.load()));
            }
            // Then try int
            let handle = usize::try_from_object(vm, obj)?;
            Ok(HKEYArg(handle as Registry::HKEY))
        }
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

    // Additional constants not in windows-sys
    #[pyattr]
    const REG_REFRESH_HIVE: u32 = 0x00000002;
    #[pyattr]
    const REG_NO_LAZY_FLUSH: u32 = 0x00000004;
    // REG_LEGAL_OPTION is a mask of all option flags
    #[pyattr]
    const REG_LEGAL_OPTION: u32 = Registry::REG_OPTION_RESERVED
        | Registry::REG_OPTION_NON_VOLATILE
        | Registry::REG_OPTION_VOLATILE
        | Registry::REG_OPTION_CREATE_LINK
        | Registry::REG_OPTION_BACKUP_RESTORE
        | Registry::REG_OPTION_OPEN_LINK;
    // REG_LEGAL_CHANGE_FILTER is a mask of all notify flags
    #[pyattr]
    const REG_LEGAL_CHANGE_FILTER: u32 = Registry::REG_NOTIFY_CHANGE_NAME
        | Registry::REG_NOTIFY_CHANGE_ATTRIBUTES
        | Registry::REG_NOTIFY_CHANGE_LAST_SET
        | Registry::REG_NOTIFY_CHANGE_SECURITY;

    // error is an alias for OSError (for backwards compatibility)
    #[pyattr]
    fn error(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.exceptions.os_error.to_owned()
    }

    #[pyattr(once)]
    fn HKEY_CLASSES_ROOT(vm: &VirtualMachine) -> PyRef<PyHkey> {
        PyHkey::new(Registry::HKEY_CLASSES_ROOT).into_ref(&vm.ctx)
    }

    #[pyattr(once)]
    fn HKEY_CURRENT_USER(vm: &VirtualMachine) -> PyRef<PyHkey> {
        PyHkey::new(Registry::HKEY_CURRENT_USER).into_ref(&vm.ctx)
    }

    #[pyattr(once)]
    fn HKEY_LOCAL_MACHINE(vm: &VirtualMachine) -> PyRef<PyHkey> {
        PyHkey::new(Registry::HKEY_LOCAL_MACHINE).into_ref(&vm.ctx)
    }

    #[pyattr(once)]
    fn HKEY_USERS(vm: &VirtualMachine) -> PyRef<PyHkey> {
        PyHkey::new(Registry::HKEY_USERS).into_ref(&vm.ctx)
    }

    #[pyattr(once)]
    fn HKEY_PERFORMANCE_DATA(vm: &VirtualMachine) -> PyRef<PyHkey> {
        PyHkey::new(Registry::HKEY_PERFORMANCE_DATA).into_ref(&vm.ctx)
    }

    #[pyattr(once)]
    fn HKEY_CURRENT_CONFIG(vm: &VirtualMachine) -> PyRef<PyHkey> {
        PyHkey::new(Registry::HKEY_CURRENT_CONFIG).into_ref(&vm.ctx)
    }

    #[pyattr(once)]
    fn HKEY_DYN_DATA(vm: &VirtualMachine) -> PyRef<PyHkey> {
        PyHkey::new(Registry::HKEY_DYN_DATA).into_ref(&vm.ctx)
    }

    #[pyattr]
    #[pyclass(name = "HKEYType")]
    #[derive(Debug, PyPayload)]
    struct PyHkey {
        hkey: AtomicHKEY,
    }

    unsafe impl Send for PyHkey {}
    unsafe impl Sync for PyHkey {}

    impl PyHkey {
        fn new(hkey: Registry::HKEY) -> Self {
            Self {
                hkey: AtomicHKEY::new(hkey),
            }
        }

        fn unary_fail(vm: &VirtualMachine) -> PyResult {
            Err(vm.new_type_error(HKEY_ERR_MSG.to_owned()))
        }

        fn binary_fail(vm: &VirtualMachine) -> PyResult {
            Err(vm.new_type_error(HKEY_ERR_MSG.to_owned()))
        }

        fn ternary_fail(vm: &VirtualMachine) -> PyResult {
            Err(vm.new_type_error(HKEY_ERR_MSG.to_owned()))
        }
    }

    #[pyclass(with(AsNumber, Hashable))]
    impl PyHkey {
        #[pygetset]
        fn handle(&self) -> usize {
            self.hkey.load() as usize
        }

        #[pymethod]
        fn Close(&self, vm: &VirtualMachine) -> PyResult<()> {
            // Atomically swap the handle with null and get the old value
            let old_hkey = self.hkey.swap(std::ptr::null_mut());
            // Already closed - silently succeed
            if old_hkey.is_null() {
                return Ok(());
            }
            let res = unsafe { Registry::RegCloseKey(old_hkey) };
            if res == 0 {
                Ok(())
            } else {
                Err(vm.new_os_error(format!("RegCloseKey failed with error code: {res}")))
            }
        }

        #[pymethod]
        fn Detach(&self) -> PyResult<usize> {
            // Atomically swap the handle with null and return the old value
            let old_hkey = self.hkey.swap(std::ptr::null_mut());
            Ok(old_hkey as usize)
        }

        #[pymethod]
        fn __enter__(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
            Ok(zelf)
        }

        #[pymethod]
        fn __exit__(zelf: PyRef<Self>, _args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
            zelf.Close(vm)
        }

        fn __int__(&self) -> usize {
            self.hkey.load() as usize
        }

        #[pymethod]
        fn __str__(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyRef<PyStr>> {
            Ok(vm.ctx.new_str(format!("<PyHkey:{:p}>", zelf.hkey.load())))
        }
    }

    impl Drop for PyHkey {
        fn drop(&mut self) {
            let hkey = self.hkey.swap(std::ptr::null_mut());
            if !hkey.is_null() {
                unsafe { Registry::RegCloseKey(hkey) };
            }
        }
    }

    impl Hashable for PyHkey {
        // CPython uses PyObject_GenericHash which hashes the object's address
        fn hash(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<PyHash> {
            Ok(zelf.get_id() as PyHash)
        }
    }

    pub const HKEY_ERR_MSG: &str = "bad operand type";

    impl AsNumber for PyHkey {
        fn as_number() -> &'static PyNumberMethods {
            static AS_NUMBER: PyNumberMethods = PyNumberMethods {
                add: Some(|_a, _b, vm| PyHkey::binary_fail(vm)),
                subtract: Some(|_a, _b, vm| PyHkey::binary_fail(vm)),
                multiply: Some(|_a, _b, vm| PyHkey::binary_fail(vm)),
                remainder: Some(|_a, _b, vm| PyHkey::binary_fail(vm)),
                divmod: Some(|_a, _b, vm| PyHkey::binary_fail(vm)),
                power: Some(|_a, _b, _c, vm| PyHkey::ternary_fail(vm)),
                negative: Some(|_a, vm| PyHkey::unary_fail(vm)),
                positive: Some(|_a, vm| PyHkey::unary_fail(vm)),
                absolute: Some(|_a, vm| PyHkey::unary_fail(vm)),
                boolean: Some(|a, _vm| {
                    let zelf = a.obj.downcast_ref::<PyHkey>().unwrap();
                    Ok(!zelf.hkey.load().is_null())
                }),
                invert: Some(|_a, vm| PyHkey::unary_fail(vm)),
                lshift: Some(|_a, _b, vm| PyHkey::binary_fail(vm)),
                rshift: Some(|_a, _b, vm| PyHkey::binary_fail(vm)),
                and: Some(|_a, _b, vm| PyHkey::binary_fail(vm)),
                xor: Some(|_a, _b, vm| PyHkey::binary_fail(vm)),
                or: Some(|_a, _b, vm| PyHkey::binary_fail(vm)),
                int: Some(|a, vm| {
                    if let Some(a) = a.downcast_ref::<PyHkey>() {
                        Ok(vm.new_pyobj(a.__int__()))
                    } else {
                        PyHkey::unary_fail(vm)?;
                        unreachable!()
                    }
                }),
                float: Some(|_a, vm| PyHkey::unary_fail(vm)),
                ..PyNumberMethods::NOT_IMPLEMENTED
            };
            &AS_NUMBER
        }
    }

    #[pyfunction]
    fn ConnectRegistry(
        computer_name: Option<String>,
        key: PyRef<PyHkey>,
        vm: &VirtualMachine,
    ) -> PyResult<PyHkey> {
        if let Some(computer_name) = computer_name {
            let mut ret_key = std::ptr::null_mut();
            let wide_computer_name = computer_name.to_wide_with_nul();
            let res = unsafe {
                Registry::RegConnectRegistryW(
                    wide_computer_name.as_ptr(),
                    key.hkey.load(),
                    &mut ret_key,
                )
            };
            if res == 0 {
                Ok(PyHkey::new(ret_key))
            } else {
                Err(vm.new_os_error(format!("error code: {res}")))
            }
        } else {
            let mut ret_key = std::ptr::null_mut();
            let res = unsafe {
                Registry::RegConnectRegistryW(std::ptr::null_mut(), key.hkey.load(), &mut ret_key)
            };
            if res == 0 {
                Ok(PyHkey::new(ret_key))
            } else {
                Err(vm.new_os_error(format!("error code: {res}")))
            }
        }
    }

    #[pyfunction]
    fn CreateKey(key: PyRef<PyHkey>, sub_key: String, vm: &VirtualMachine) -> PyResult<PyHkey> {
        let wide_sub_key = sub_key.to_wide_with_nul();
        let mut out_key = std::ptr::null_mut();
        let res = unsafe {
            Registry::RegCreateKeyW(key.hkey.load(), wide_sub_key.as_ptr(), &mut out_key)
        };
        if res == 0 {
            Ok(PyHkey::new(out_key))
        } else {
            Err(vm.new_os_error(format!("error code: {res}")))
        }
    }

    #[derive(FromArgs, Debug)]
    struct CreateKeyExArgs {
        #[pyarg(any)]
        key: PyRef<PyHkey>,
        #[pyarg(any)]
        sub_key: String,
        #[pyarg(any, default = 0)]
        reserved: u32,
        #[pyarg(any, default = windows_sys::Win32::System::Registry::KEY_WRITE)]
        access: u32,
    }

    #[pyfunction]
    fn CreateKeyEx(args: CreateKeyExArgs, vm: &VirtualMachine) -> PyResult<PyHkey> {
        let wide_sub_key = args.sub_key.to_wide_with_nul();
        let mut res: Registry::HKEY = core::ptr::null_mut();
        let err = unsafe {
            let key = args.key.hkey.load();
            Registry::RegCreateKeyExW(
                key,
                wide_sub_key.as_ptr(),
                args.reserved,
                core::ptr::null(),
                Registry::REG_OPTION_NON_VOLATILE,
                args.access,
                core::ptr::null(),
                &mut res,
                std::ptr::null_mut(),
            )
        };
        if err == 0 {
            Ok(PyHkey {
                #[allow(clippy::arc_with_non_send_sync)]
                hkey: AtomicHKEY::new(res),
            })
        } else {
            Err(vm.new_os_error(format!("error code: {err}")))
        }
    }

    #[pyfunction]
    fn CloseKey(hkey: PyRef<PyHkey>, vm: &VirtualMachine) -> PyResult<()> {
        hkey.Close(vm)
    }

    #[pyfunction]
    fn DeleteKey(key: PyRef<PyHkey>, sub_key: String, vm: &VirtualMachine) -> PyResult<()> {
        let wide_sub_key = sub_key.to_wide_with_nul();
        let res = unsafe { Registry::RegDeleteKeyW(key.hkey.load(), wide_sub_key.as_ptr()) };
        if res == 0 {
            Ok(())
        } else {
            Err(vm.new_os_error(format!("error code: {res}")))
        }
    }

    #[pyfunction]
    fn DeleteValue(key: PyRef<PyHkey>, value: Option<String>, vm: &VirtualMachine) -> PyResult<()> {
        let wide_value = value.map(|v| v.to_wide_with_nul());
        let value_ptr = wide_value
            .as_ref()
            .map_or(core::ptr::null(), |v| v.as_ptr());
        let res = unsafe { Registry::RegDeleteValueW(key.hkey.load(), value_ptr) };
        if res == 0 {
            Ok(())
        } else {
            Err(vm.new_os_error(format!("error code: {res}")))
        }
    }

    #[derive(FromArgs, Debug)]
    struct DeleteKeyExArgs {
        #[pyarg(any)]
        key: PyRef<PyHkey>,
        #[pyarg(any)]
        sub_key: String,
        #[pyarg(any, default = windows_sys::Win32::System::Registry::KEY_WOW64_64KEY)]
        access: u32,
        #[pyarg(any, default = 0)]
        reserved: u32,
    }

    #[pyfunction]
    fn DeleteKeyEx(args: DeleteKeyExArgs, vm: &VirtualMachine) -> PyResult<()> {
        let wide_sub_key = args.sub_key.to_wide_with_nul();
        let res = unsafe {
            Registry::RegDeleteKeyExW(
                args.key.hkey.load(),
                wide_sub_key.as_ptr(),
                args.access,
                args.reserved,
            )
        };
        if res == 0 {
            Ok(())
        } else {
            Err(vm.new_os_error(format!("error code: {res}")))
        }
    }

    #[pyfunction]
    fn EnumKey(key: PyRef<PyHkey>, index: i32, vm: &VirtualMachine) -> PyResult<String> {
        // The Windows docs claim that the max key name length is 255
        // characters, plus a terminating nul character.  However,
        // empirical testing demonstrates that it is possible to
        // create a 256 character key that is missing the terminating
        // nul.  RegEnumKeyEx requires a 257 character buffer to
        // retrieve such a key name.
        let mut tmpbuf = [0u16; 257];
        let mut len = tmpbuf.len() as u32;
        let res = unsafe {
            Registry::RegEnumKeyExW(
                key.hkey.load(),
                index as u32,
                tmpbuf.as_mut_ptr(),
                &mut len,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if res != 0 {
            return Err(vm.new_os_error(format!("error code: {res}")));
        }
        String::from_utf16(&tmpbuf[..len as usize])
            .map_err(|e| vm.new_value_error(format!("UTF16 error: {e}")))
    }

    #[pyfunction]
    fn EnumValue(hkey: PyRef<PyHkey>, index: u32, vm: &VirtualMachine) -> PyResult {
        // Query registry for the required buffer sizes.
        let mut ret_value_size: u32 = 0;
        let mut ret_data_size: u32 = 0;
        let hkey: Registry::HKEY = hkey.hkey.load();
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
            return Err(vm.new_os_error(format!("RegQueryInfoKeyW failed with error code {rc}")));
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
            let mut reg_type: u32 = 0;
            let rc = unsafe {
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
                return Err(vm.new_os_error(format!("RegEnumValueW failed with error code {rc}")));
            }

            // Convert the registry value name from UTF‑16.
            let name_len = ret_value_buf
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(ret_value_buf.len());
            let name = String::from_utf16(&ret_value_buf[..name_len])
                .map_err(|e| vm.new_value_error(format!("UTF16 conversion error: {e}")))?;

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
    fn FlushKey(key: PyRef<PyHkey>, vm: &VirtualMachine) -> PyResult<()> {
        let res = unsafe { Registry::RegFlushKey(key.hkey.load()) };
        if res == 0 {
            Ok(())
        } else {
            Err(vm.new_os_error(format!("error code: {res}")))
        }
    }

    #[pyfunction]
    fn LoadKey(
        key: PyRef<PyHkey>,
        sub_key: String,
        file_name: String,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let sub_key = sub_key.to_wide_with_nul();
        let file_name = file_name.to_wide_with_nul();
        let res =
            unsafe { Registry::RegLoadKeyW(key.hkey.load(), sub_key.as_ptr(), file_name.as_ptr()) };
        if res == 0 {
            Ok(())
        } else {
            Err(vm.new_os_error(format!("error code: {res}")))
        }
    }

    #[derive(Debug, FromArgs)]
    struct OpenKeyArgs {
        #[pyarg(any)]
        key: PyRef<PyHkey>,
        #[pyarg(any)]
        sub_key: String,
        #[pyarg(any, default = 0)]
        reserved: u32,
        #[pyarg(any, default = windows_sys::Win32::System::Registry::KEY_READ)]
        access: u32,
    }

    #[pyfunction]
    #[pyfunction(name = "OpenKeyEx")]
    fn OpenKey(args: OpenKeyArgs, vm: &VirtualMachine) -> PyResult<PyHkey> {
        let wide_sub_key = args.sub_key.to_wide_with_nul();
        let mut res: Registry::HKEY = std::ptr::null_mut();
        let err = unsafe {
            let key = args.key.hkey.load();
            Registry::RegOpenKeyExW(
                key,
                wide_sub_key.as_ptr(),
                args.reserved,
                args.access,
                &mut res,
            )
        };
        if err == 0 {
            Ok(PyHkey {
                #[allow(clippy::arc_with_non_send_sync)]
                hkey: AtomicHKEY::new(res),
            })
        } else {
            Err(os_error_from_windows_code(vm, err as i32, "RegOpenKeyEx"))
        }
    }

    #[pyfunction]
    fn QueryInfoKey(key: HKEYArg, vm: &VirtualMachine) -> PyResult<PyRef<PyTuple>> {
        let key = key.0;
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
            return Err(vm.new_os_error(format!("error code: {err}")));
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
    fn QueryValue(key: HKEYArg, sub_key: Option<String>, vm: &VirtualMachine) -> PyResult<String> {
        let hkey = key.0;

        if hkey == Registry::HKEY_PERFORMANCE_DATA {
            return Err(os_error_from_windows_code(
                vm,
                Foundation::ERROR_INVALID_HANDLE as i32,
                "RegQueryValue",
            ));
        }

        // Open subkey if provided and non-empty
        let child_key = if let Some(ref sk) = sub_key {
            if !sk.is_empty() {
                let wide_sub_key = sk.to_wide_with_nul();
                let mut out_key = std::ptr::null_mut();
                let res = unsafe {
                    Registry::RegOpenKeyExW(
                        hkey,
                        wide_sub_key.as_ptr(),
                        0,
                        Registry::KEY_QUERY_VALUE,
                        &mut out_key,
                    )
                };
                if res != 0 {
                    return Err(os_error_from_windows_code(vm, res as i32, "RegOpenKeyEx"));
                }
                Some(out_key)
            } else {
                None
            }
        } else {
            None
        };

        let target_key = child_key.unwrap_or(hkey);
        let mut buf_size: u32 = 256;
        let mut buffer: Vec<u8> = vec![0; buf_size as usize];
        let mut reg_type: u32 = 0;

        // Loop to handle ERROR_MORE_DATA
        let result = loop {
            let mut size = buf_size;
            let res = unsafe {
                Registry::RegQueryValueExW(
                    target_key,
                    core::ptr::null(), // NULL value name for default value
                    std::ptr::null_mut(),
                    &mut reg_type,
                    buffer.as_mut_ptr(),
                    &mut size,
                )
            };
            if res == ERROR_MORE_DATA {
                buf_size *= 2;
                buffer.resize(buf_size as usize, 0);
                continue;
            }
            if res == Foundation::ERROR_FILE_NOT_FOUND {
                // Return empty string if there's no default value
                break Ok(String::new());
            }
            if res != 0 {
                break Err(os_error_from_windows_code(
                    vm,
                    res as i32,
                    "RegQueryValueEx",
                ));
            }
            if reg_type != Registry::REG_SZ {
                break Err(os_error_from_windows_code(
                    vm,
                    Foundation::ERROR_INVALID_DATA as i32,
                    "RegQueryValue",
                ));
            }

            // Convert UTF-16 to String
            let u16_slice = bytes_as_wide_slice(&buffer[..size as usize]);
            let len = u16_slice
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(u16_slice.len());
            break String::from_utf16(&u16_slice[..len])
                .map_err(|e| vm.new_value_error(format!("UTF16 error: {e}")));
        };

        // Close child key if we opened one
        if let Some(ck) = child_key {
            unsafe { Registry::RegCloseKey(ck) };
        }

        result
    }

    #[pyfunction]
    fn QueryValueEx(key: HKEYArg, name: String, vm: &VirtualMachine) -> PyResult<PyRef<PyTuple>> {
        let hkey = key.0;
        let wide_name = name.to_wide_with_nul();
        let mut buf_size: u32 = 0;
        let res = unsafe {
            Registry::RegQueryValueExW(
                hkey,
                wide_name.as_ptr(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut buf_size,
            )
        };
        // Handle ERROR_MORE_DATA by using a default buffer size
        if res == ERROR_MORE_DATA || buf_size == 0 {
            buf_size = 256;
        } else if res != 0 {
            return Err(os_error_from_windows_code(
                vm,
                res as i32,
                "RegQueryValueEx",
            ));
        }

        let mut ret_buf = vec![0u8; buf_size as usize];
        let mut typ = 0;
        let mut ret_size: u32;

        // Loop to handle ERROR_MORE_DATA
        loop {
            ret_size = buf_size;
            let res = unsafe {
                Registry::RegQueryValueExW(
                    hkey,
                    wide_name.as_ptr(),
                    std::ptr::null_mut(),
                    &mut typ,
                    ret_buf.as_mut_ptr(),
                    &mut ret_size,
                )
            };

            if res != ERROR_MORE_DATA {
                if res != 0 {
                    return Err(os_error_from_windows_code(
                        vm,
                        res as i32,
                        "RegQueryValueEx",
                    ));
                }
                break;
            }

            // Double buffer size and retry
            buf_size *= 2;
            ret_buf.resize(buf_size as usize, 0);
        }

        // Only pass the bytes actually returned by the API
        let obj = reg_to_py(vm, &ret_buf[..ret_size as usize], typ)?;
        // Return tuple (value, type)
        Ok(vm.ctx.new_tuple(vec![obj, vm.ctx.new_int(typ).into()]))
    }

    #[pyfunction]
    fn SaveKey(key: PyRef<PyHkey>, file_name: String, vm: &VirtualMachine) -> PyResult<()> {
        let file_name = file_name.to_wide_with_nul();
        let res = unsafe {
            Registry::RegSaveKeyW(key.hkey.load(), file_name.as_ptr(), std::ptr::null_mut())
        };
        if res == 0 {
            Ok(())
        } else {
            Err(vm.new_os_error(format!("error code: {res}")))
        }
    }

    #[pyfunction]
    fn SetValue(
        key: PyRef<PyHkey>,
        sub_key: String,
        typ: u32,
        value: String,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        if typ != Registry::REG_SZ {
            return Err(vm.new_type_error("type must be winreg.REG_SZ"));
        }

        let hkey = key.hkey.load();
        if hkey == Registry::HKEY_PERFORMANCE_DATA {
            return Err(os_error_from_windows_code(
                vm,
                Foundation::ERROR_INVALID_HANDLE as i32,
                "RegSetValue",
            ));
        }

        // Create subkey if sub_key is non-empty
        let child_key = if !sub_key.is_empty() {
            let wide_sub_key = sub_key.to_wide_with_nul();
            let mut out_key = std::ptr::null_mut();
            let res = unsafe {
                Registry::RegCreateKeyExW(
                    hkey,
                    wide_sub_key.as_ptr(),
                    0,
                    core::ptr::null(),
                    0,
                    Registry::KEY_SET_VALUE,
                    core::ptr::null(),
                    &mut out_key,
                    std::ptr::null_mut(),
                )
            };
            if res != 0 {
                return Err(os_error_from_windows_code(vm, res as i32, "RegCreateKeyEx"));
            }
            Some(out_key)
        } else {
            None
        };

        let target_key = child_key.unwrap_or(hkey);
        // Convert value to UTF-16 for Wide API
        let wide_value = value.to_wide_with_nul();
        let res = unsafe {
            Registry::RegSetValueExW(
                target_key,
                core::ptr::null(), // value name is NULL
                0,
                typ,
                wide_value.as_ptr() as *const u8,
                (wide_value.len() * 2) as u32, // byte count
            )
        };

        // Close child key if we created one
        if let Some(ck) = child_key {
            unsafe { Registry::RegCloseKey(ck) };
        }

        if res == 0 {
            Ok(())
        } else {
            Err(os_error_from_windows_code(vm, res as i32, "RegSetValueEx"))
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
                let u16_slice = bytes_as_wide_slice(ret_data);
                // Only use characters up to the first NUL.
                let len = u16_slice
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(u16_slice.len());
                let s = String::from_utf16(&u16_slice[..len])
                    .map_err(|e| vm.new_value_error(format!("UTF16 error: {e}")))?;
                Ok(vm.ctx.new_str(s).into())
            }
            REG_MULTI_SZ => {
                if ret_data.is_empty() {
                    Ok(vm.ctx.new_list(vec![]).into())
                } else {
                    let u16_slice = bytes_as_wide_slice(ret_data);
                    let u16_count = u16_slice.len();

                    // Remove trailing null if present (like countStrings)
                    let len = if u16_count > 0 && u16_slice[u16_count - 1] == 0 {
                        u16_count - 1
                    } else {
                        u16_count
                    };

                    let mut strings: Vec<PyObjectRef> = Vec::new();
                    let mut start = 0;
                    for i in 0..len {
                        if u16_slice[i] == 0 {
                            let s = String::from_utf16(&u16_slice[start..i])
                                .map_err(|e| vm.new_value_error(format!("UTF16 error: {e}")))?;
                            strings.push(vm.ctx.new_str(s).into());
                            start = i + 1;
                        }
                    }
                    // Handle last string if not null-terminated
                    if start < len {
                        let s = String::from_utf16(&u16_slice[start..len])
                            .map_err(|e| vm.new_value_error(format!("UTF16 error: {e}")))?;
                        strings.push(vm.ctx.new_str(s).into());
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
                if vm.is_none(&value) {
                    return Ok(Some(0u32.to_le_bytes().to_vec()));
                }
                let val = value
                    .downcast_ref::<PyInt>()
                    .ok_or_else(|| vm.new_type_error("value must be an integer".to_string()))?;
                let bigint = val.as_bigint();
                // Check for negative value - raise OverflowError
                if bigint.sign() == Sign::Minus {
                    return Err(vm.new_overflow_error("int too big to convert".to_string()));
                }
                let val = bigint
                    .to_u32()
                    .ok_or_else(|| vm.new_overflow_error("int too big to convert".to_string()))?;
                Ok(Some(val.to_le_bytes().to_vec()))
            }
            REG_QWORD => {
                if vm.is_none(&value) {
                    return Ok(Some(0u64.to_le_bytes().to_vec()));
                }
                let val = value
                    .downcast_ref::<PyInt>()
                    .ok_or_else(|| vm.new_type_error("value must be an integer".to_string()))?;
                let bigint = val.as_bigint();
                // Check for negative value - raise OverflowError
                if bigint.sign() == Sign::Minus {
                    return Err(vm.new_overflow_error("int too big to convert".to_string()));
                }
                let val = bigint
                    .to_u64()
                    .ok_or_else(|| vm.new_overflow_error("int too big to convert".to_string()))?;
                Ok(Some(val.to_le_bytes().to_vec()))
            }
            REG_SZ | REG_EXPAND_SZ => {
                if vm.is_none(&value) {
                    // Return empty string as UTF-16 null terminator
                    return Ok(Some(vec![0u8, 0u8]));
                }
                let s = value
                    .downcast::<PyStr>()
                    .map_err(|_| vm.new_type_error("value must be a string".to_string()))?;
                let wide = s.as_str().to_wide_with_nul();
                // Convert Vec<u16> to Vec<u8>
                let bytes: Vec<u8> = wide.iter().flat_map(|&c| c.to_le_bytes()).collect();
                Ok(Some(bytes))
            }
            REG_MULTI_SZ => {
                if vm.is_none(&value) {
                    // Empty list = double null terminator
                    return Ok(Some(vec![0u8, 0u8, 0u8, 0u8]));
                }
                let list = value.downcast::<crate::builtins::PyList>().map_err(|_| {
                    vm.new_type_error("value must be a list of strings".to_string())
                })?;

                let mut bytes: Vec<u8> = Vec::new();
                for item in list.borrow_vec().iter() {
                    let s = item.downcast_ref::<PyStr>().ok_or_else(|| {
                        vm.new_type_error("list items must be strings".to_string())
                    })?;
                    let wide = s.as_str().to_wide_with_nul();
                    bytes.extend(wide.iter().flat_map(|&c| c.to_le_bytes()));
                }
                // Add final null terminator (double null at end)
                bytes.extend([0u8, 0u8]);
                Ok(Some(bytes))
            }
            // REG_BINARY and other types
            _ => {
                if vm.is_none(&value) {
                    return Ok(None);
                }
                // Try to get bytes
                if let Some(bytes) = value.downcast_ref::<crate::builtins::PyBytes>() {
                    return Ok(Some(bytes.as_bytes().to_vec()));
                }
                Err(vm.new_type_error(format!(
                    "Objects of type '{}' can not be used as binary registry values",
                    value.class().name()
                )))
            }
        }
    }

    #[pyfunction]
    fn SetValueEx(
        key: PyRef<PyHkey>,
        value_name: String,
        _reserved: PyObjectRef,
        typ: u32,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        match py2reg(value, typ, vm) {
            Ok(Some(v)) => {
                let len = v.len() as u32;
                let ptr = v.as_ptr();
                let wide_value_name = value_name.to_wide_with_nul();
                let res = unsafe {
                    Registry::RegSetValueExW(
                        key.hkey.load(),
                        wide_value_name.as_ptr(),
                        0,
                        typ,
                        ptr,
                        len,
                    )
                };
                if res != 0 {
                    return Err(vm.new_os_error(format!("error code: {res}")));
                }
            }
            Ok(None) => {
                let len = 0;
                let ptr = core::ptr::null();
                let wide_value_name = value_name.to_wide_with_nul();
                let res = unsafe {
                    Registry::RegSetValueExW(
                        key.hkey.load(),
                        wide_value_name.as_ptr(),
                        0,
                        typ,
                        ptr,
                        len,
                    )
                };
                if res != 0 {
                    return Err(vm.new_os_error(format!("error code: {res}")));
                }
            }
            Err(e) => return Err(e),
        }
        Ok(())
    }

    #[pyfunction]
    fn DisableReflectionKey(key: PyRef<PyHkey>, vm: &VirtualMachine) -> PyResult<()> {
        let res = unsafe { Registry::RegDisableReflectionKey(key.hkey.load()) };
        if res == 0 {
            Ok(())
        } else {
            Err(vm.new_os_error(format!("error code: {res}")))
        }
    }

    #[pyfunction]
    fn EnableReflectionKey(key: PyRef<PyHkey>, vm: &VirtualMachine) -> PyResult<()> {
        let res = unsafe { Registry::RegEnableReflectionKey(key.hkey.load()) };
        if res == 0 {
            Ok(())
        } else {
            Err(vm.new_os_error(format!("error code: {res}")))
        }
    }

    #[pyfunction]
    fn QueryReflectionKey(key: PyRef<PyHkey>, vm: &VirtualMachine) -> PyResult<bool> {
        let mut result: i32 = 0;
        let res = unsafe { Registry::RegQueryReflectionKey(key.hkey.load(), &mut result) };
        if res == 0 {
            Ok(result != 0)
        } else {
            Err(vm.new_os_error(format!("error code: {res}")))
        }
    }

    #[pyfunction]
    fn ExpandEnvironmentStrings(i: String, vm: &VirtualMachine) -> PyResult<String> {
        let wide_input = i.to_wide_with_nul();

        // First call with size=0 to get required buffer size
        let required_size = unsafe {
            windows_sys::Win32::System::Environment::ExpandEnvironmentStringsW(
                wide_input.as_ptr(),
                std::ptr::null_mut(),
                0,
            )
        };
        if required_size == 0 {
            return Err(vm.new_os_error("ExpandEnvironmentStringsW failed".to_string()));
        }

        // Allocate buffer with exact size and expand
        let mut out = vec![0u16; required_size as usize];
        let r = unsafe {
            windows_sys::Win32::System::Environment::ExpandEnvironmentStringsW(
                wide_input.as_ptr(),
                out.as_mut_ptr(),
                required_size,
            )
        };
        if r == 0 {
            return Err(vm.new_os_error("ExpandEnvironmentStringsW failed".to_string()));
        }

        let len = out.iter().position(|&c| c == 0).unwrap_or(out.len());
        String::from_utf16(&out[..len]).map_err(|e| vm.new_value_error(format!("UTF16 error: {e}")))
    }
}

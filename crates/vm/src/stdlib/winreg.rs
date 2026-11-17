// spell-checker:disable
#![allow(non_snake_case)]

pub(crate) use winreg::module_def;

#[pymodule]
mod winreg {
    use crate::builtins::{PyInt, PyStr, PyTuple, PyTypeRef};
    use crate::common::hash::PyHash;
    use crate::convert::{ToPyException, TryFromObject};
    use crate::exceptions::nul_char_error;
    use crate::function::PyComparisonValue;
    use crate::object::AsObject;
    use crate::protocol::PyNumberMethods;
    use crate::types::{AsNumber, Comparable, Hashable, PyComparisonOp};
    use crate::{Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine};
    use core::ptr;
    use crossbeam_utils::atomic::AtomicCell;
    use malachite_bigint::Sign;
    use num_traits::ToPrimitive;
    use rustpython_common::wtf8::Wtf8Buf;
    use rustpython_host_env::windows::ToWideString;
    use rustpython_host_env::winreg as host_winreg;
    use widestring::{WideCString, WideString};

    /// Atomic HKEY handle type for lock-free thread-safe access
    type AtomicHKEY = AtomicCell<host_winreg::HKEY>;

    fn os_error_from_windows_code(
        vm: &VirtualMachine,
        code: i32,
    ) -> crate::PyRef<crate::builtins::PyBaseException> {
        std::io::Error::from_raw_os_error(code).to_pyexception(vm)
    }

    // Snapshot the handle during argument conversion, keeping its owner alive
    // while later argument conversions or audit hooks can invoke Python code.
    #[derive(Debug)]
    struct HKEYArg(host_winreg::HKEY, #[allow(dead_code)] Option<PyRef<PyHkey>>);

    impl TryFromObject for HKEYArg {
        fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
            // Try PyHkey first
            let obj = match obj.downcast::<PyHkey>() {
                Ok(key) => return Ok(Self(key.hkey.load(), Some(key))),
                Err(obj) => obj,
            };
            if vm.is_none(&obj) {
                return Err(vm.new_type_error("None is not a valid HKEY in this context"));
            }
            let Some(value) = obj.downcast_ref::<PyInt>() else {
                return Err(vm.new_type_error("The object is not a PyHKEY object"));
            };
            let value = value.as_bigint();
            let negative = value.sign() == Sign::Minus;
            let handle = if negative {
                value.to_isize().map(|v| v as usize)
            } else {
                value.to_usize()
            }
            .ok_or_else(|| {
                let message = if cfg!(target_pointer_width = "64") {
                    "int too big to convert"
                } else if negative {
                    "Python int too large to convert to C long"
                } else {
                    "Python int too large to convert to C unsigned long"
                };
                vm.new_overflow_error(message)
            })?;
            Ok(Self(handle as host_winreg::HKEY, None))
        }
    }

    fn audit_wide(value: Option<&widestring::WideCStr>, vm: &VirtualMachine) -> PyObjectRef {
        value.map_or_else(
            || vm.ctx.none(),
            |value| vm.ctx.new_str(Wtf8Buf::from_wide(value.as_slice())).into(),
        )
    }

    // access rights
    #[pyattr]
    pub(super) use host_winreg::{
        KEY_ALL_ACCESS, KEY_CREATE_LINK, KEY_CREATE_SUB_KEY, KEY_ENUMERATE_SUB_KEYS, KEY_EXECUTE,
        KEY_NOTIFY, KEY_QUERY_VALUE, KEY_READ, KEY_SET_VALUE, KEY_WOW64_32KEY, KEY_WOW64_64KEY,
        KEY_WRITE,
    };
    // value types
    #[pyattr]
    pub(super) use host_winreg::{
        REG_BINARY, REG_CREATED_NEW_KEY, REG_DWORD, REG_DWORD_BIG_ENDIAN, REG_DWORD_LITTLE_ENDIAN,
        REG_EXPAND_SZ, REG_FULL_RESOURCE_DESCRIPTOR, REG_LINK, REG_MULTI_SZ, REG_NONE,
        REG_NOTIFY_CHANGE_ATTRIBUTES, REG_NOTIFY_CHANGE_LAST_SET, REG_NOTIFY_CHANGE_NAME,
        REG_NOTIFY_CHANGE_SECURITY, REG_OPENED_EXISTING_KEY, REG_OPTION_BACKUP_RESTORE,
        REG_OPTION_CREATE_LINK, REG_OPTION_NON_VOLATILE, REG_OPTION_OPEN_LINK, REG_OPTION_RESERVED,
        REG_OPTION_VOLATILE, REG_QWORD, REG_QWORD_LITTLE_ENDIAN, REG_RESOURCE_LIST,
        REG_RESOURCE_REQUIREMENTS_LIST, REG_SZ, REG_WHOLE_HIVE_VOLATILE,
    };

    #[pyattr]
    const REG_REFRESH_HIVE: u32 = host_winreg::REG_REFRESH_HIVE;
    #[pyattr]
    const REG_NO_LAZY_FLUSH: u32 = host_winreg::REG_NO_LAZY_FLUSH;
    #[pyattr]
    const REG_LEGAL_OPTION: u32 = host_winreg::REG_LEGAL_OPTION;
    #[pyattr]
    const REG_LEGAL_CHANGE_FILTER: u32 = host_winreg::REG_LEGAL_CHANGE_FILTER;

    // error is an alias for OSError (for backwards compatibility)
    #[pyattr]
    fn error(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.exceptions.os_error.to_owned()
    }

    #[pyattr(once)]
    fn HKEY_CLASSES_ROOT(vm: &VirtualMachine) -> PyRef<PyHkey> {
        PyHkey::new(host_winreg::HKEY_CLASSES_ROOT).into_ref(&vm.ctx)
    }

    #[pyattr(once)]
    fn HKEY_CURRENT_USER(vm: &VirtualMachine) -> PyRef<PyHkey> {
        PyHkey::new(host_winreg::HKEY_CURRENT_USER).into_ref(&vm.ctx)
    }

    #[pyattr(once)]
    fn HKEY_LOCAL_MACHINE(vm: &VirtualMachine) -> PyRef<PyHkey> {
        PyHkey::new(host_winreg::HKEY_LOCAL_MACHINE).into_ref(&vm.ctx)
    }

    #[pyattr(once)]
    fn HKEY_USERS(vm: &VirtualMachine) -> PyRef<PyHkey> {
        PyHkey::new(host_winreg::HKEY_USERS).into_ref(&vm.ctx)
    }

    #[pyattr(once)]
    fn HKEY_PERFORMANCE_DATA(vm: &VirtualMachine) -> PyRef<PyHkey> {
        PyHkey::new(host_winreg::HKEY_PERFORMANCE_DATA).into_ref(&vm.ctx)
    }

    #[pyattr(once)]
    fn HKEY_CURRENT_CONFIG(vm: &VirtualMachine) -> PyRef<PyHkey> {
        PyHkey::new(host_winreg::HKEY_CURRENT_CONFIG).into_ref(&vm.ctx)
    }

    #[pyattr(once)]
    fn HKEY_DYN_DATA(vm: &VirtualMachine) -> PyRef<PyHkey> {
        PyHkey::new(host_winreg::HKEY_DYN_DATA).into_ref(&vm.ctx)
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
        fn new(hkey: host_winreg::HKEY) -> Self {
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

    impl PyHkey {
        fn __int__(&self) -> usize {
            self.hkey.load() as usize
        }
    }

    #[pyclass(with(AsNumber, Comparable, Hashable))]
    impl Py<PyHkey> {
        #[pygetset]
        fn handle(&self) -> usize {
            self.hkey.load() as usize
        }

        #[pymethod]
        fn Close(&self, vm: &VirtualMachine) -> PyResult<()> {
            // Atomically swap the handle with null and get the old value
            let old_hkey = self.hkey.swap(core::ptr::null_mut());
            // Already closed - silently succeed
            if old_hkey.is_null() {
                return Ok(());
            }
            let res = host_winreg::close_key(old_hkey);
            if res == 0 {
                Ok(())
            } else {
                Err(os_error_from_windows_code(vm, res as i32))
            }
        }

        #[pymethod]
        fn Detach(&self, vm: &VirtualMachine) -> PyResult<usize> {
            vm.audit("winreg.PyHKEY.Detach", || (self.hkey.load() as isize,))?;
            // Read again after the hook: it may have detached or closed us.
            Ok(self.hkey.swap(core::ptr::null_mut()) as usize)
        }

        #[pymethod]
        fn __enter__(zelf: PyRef<PyHkey>, _vm: &VirtualMachine) -> PyRef<PyHkey> {
            zelf
        }

        #[pymethod]
        fn __exit__(
            zelf: PyRef<PyHkey>,
            _exc_type: PyObjectRef,
            _exc_value: PyObjectRef,
            _traceback: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            zelf.Close(vm)
        }

        #[pymethod]
        fn __str__(zelf: &Self, vm: &VirtualMachine) -> PyRef<PyStr> {
            vm.ctx.new_str(format!("<PyHkey:{:p}>", zelf.hkey.load()))
        }
    }

    impl Drop for PyHkey {
        fn drop(&mut self) {
            let hkey = self.hkey.swap(core::ptr::null_mut());
            if !hkey.is_null() {
                host_winreg::close_key(hkey);
            }
        }
    }

    impl Comparable for PyHkey {
        fn cmp(
            zelf: &Py<Self>,
            other: &PyObject,
            op: PyComparisonOp,
            _vm: &VirtualMachine,
        ) -> PyResult<PyComparisonValue> {
            op.eq_only(|| {
                if other.class().is(zelf.class())
                    && let Some(other) = other.downcast_ref::<Self>()
                {
                    return Ok(PyComparisonValue::Implemented(
                        zelf.hkey.load() == other.hkey.load(),
                    ));
                }
                Ok(PyComparisonValue::NotImplemented)
            })
        }
    }

    impl Hashable for PyHkey {
        // CPython uses PyObject_GenericHash which hashes the object's address
        fn hash(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<PyHash> {
            Ok(zelf.get_id() as PyHash)
        }
    }

    const HKEY_ERR_MSG: &str = "bad operand type";

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
        key: HKEYArg,
        vm: &VirtualMachine,
    ) -> PyResult<PyHkey> {
        let computer_name = computer_name
            .map(WideCString::from_str)
            .transpose()
            .map_err(|e| e.to_pyexception(vm))?;
        vm.audit("winreg.ConnectRegistry", || {
            (audit_wide(computer_name.as_deref(), vm), key.0 as isize)
        })?;
        let mut ret_key = core::ptr::null_mut();
        let res =
            unsafe { host_winreg::connect_registry(computer_name.as_deref(), key.0, &mut ret_key) };
        if res == 0 {
            Ok(PyHkey::new(ret_key))
        } else {
            Err(os_error_from_windows_code(vm, res as i32))
        }
    }

    #[pyfunction]
    fn CreateKey(
        key: HKEYArg,
        sub_key: Option<PyRef<PyStr>>,
        vm: &VirtualMachine,
    ) -> PyResult<PyHkey> {
        let sub_key = sub_key
            .as_ref()
            .map(|value| value.as_wtf8().to_wide_cstring())
            .transpose()
            .map_err(|_| nul_char_error(vm))?;
        vm.audit("winreg.CreateKey", || {
            (
                key.0 as isize,
                audit_wide(sub_key.as_deref(), vm),
                host_winreg::KEY_WRITE as isize,
            )
        })?;
        let mut out_key = core::ptr::null_mut();
        let res = unsafe { host_winreg::create_key(key.0, sub_key.as_deref(), &mut out_key) };
        if res == 0 {
            let result = PyHkey::new(out_key);
            vm.audit("winreg.OpenKey/result", || (out_key as isize,))?;
            Ok(result)
        } else {
            Err(os_error_from_windows_code(vm, res as i32))
        }
    }

    #[derive(FromArgs, Debug)]
    struct CreateKeyExArgs {
        #[pyarg(any)]
        key: HKEYArg,
        #[pyarg(any)]
        sub_key: Option<PyRef<PyStr>>,
        #[pyarg(any, default)]
        reserved: i32,
        #[pyarg(any, default = host_winreg::KEY_WRITE as i32, py_default = "winreg.KEY_WRITE")]
        access: i32,
    }

    #[pyfunction]
    fn CreateKeyEx(args: CreateKeyExArgs, vm: &VirtualMachine) -> PyResult<PyHkey> {
        let wide_sub_key = args
            .sub_key
            .as_ref()
            .map(|value| value.as_wtf8().to_wide_cstring())
            .transpose()
            .map_err(|_| nul_char_error(vm))?;
        vm.audit("winreg.CreateKey", || {
            (
                args.key.0 as isize,
                audit_wide(wide_sub_key.as_deref(), vm),
                args.access as u32 as isize,
            )
        })?;
        let mut res: host_winreg::HKEY = core::ptr::null_mut();
        let err = unsafe {
            let key = args.key.0;
            host_winreg::create_key_ex(
                key,
                wide_sub_key.as_deref(),
                args.reserved as u32,
                core::ptr::null_mut(),
                host_winreg::REG_OPTION_NON_VOLATILE,
                args.access as u32,
                core::ptr::null(),
                &mut res,
                core::ptr::null_mut(),
            )
        };
        if err == 0 {
            let result = PyHkey::new(res);
            vm.audit("winreg.OpenKey/result", || (res as isize,))?;
            Ok(result)
        } else {
            Err(os_error_from_windows_code(vm, err as i32))
        }
    }

    #[pyfunction]
    fn CloseKey(hkey: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        if let Some(key) = hkey.downcast_ref::<PyHkey>() {
            return key.Close(vm);
        }
        if vm.is_none(&hkey) {
            return Ok(());
        }
        let key = HKEYArg::try_from_object(vm, hkey)?.0;
        let res = if key.is_null() {
            0
        } else {
            host_winreg::close_key(key)
        };
        if res == 0 {
            Ok(())
        } else {
            Err(os_error_from_windows_code(vm, res as i32))
        }
    }

    #[pyfunction]
    fn DeleteKey(key: HKEYArg, sub_key: String, vm: &VirtualMachine) -> PyResult<()> {
        let sub_key = WideCString::from_str(sub_key).map_err(|e| e.to_pyexception(vm))?;
        vm.audit("winreg.DeleteKey", || {
            (key.0 as isize, audit_wide(Some(&sub_key), vm), 0)
        })?;
        let res = unsafe { host_winreg::delete_key(key.0, &sub_key) };
        if res == 0 {
            Ok(())
        } else {
            Err(os_error_from_windows_code(vm, res as i32))
        }
    }

    #[derive(FromArgs)]
    struct DeleteTreeSubKey {
        #[pyarg(positional, optional)]
        sub_key: Option<PyRef<PyStr>>,
    }

    #[pyfunction]
    fn DeleteTree(
        key: HKEYArg,
        DeleteTreeSubKey { sub_key }: DeleteTreeSubKey,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let wide_sub_key = sub_key
            .as_ref()
            .map(|s| WideCString::from_vec(s.as_wtf8().encode_wide().collect::<Vec<_>>()))
            .transpose()
            .map_err(|_| nul_char_error(vm))?;
        vm.audit("winreg.DeleteTree", || (key.0 as isize, sub_key))?;
        let res = vm
            .allow_threads(|| unsafe { host_winreg::delete_tree(key.0, wide_sub_key.as_deref()) });
        if res == 0 {
            Ok(())
        } else {
            Err(os_error_from_windows_code(vm, res as i32))
        }
    }

    #[pyfunction]
    fn DeleteValue(key: HKEYArg, value: Option<String>, vm: &VirtualMachine) -> PyResult<()> {
        let wide_value = value
            .map(WideCString::from_str)
            .transpose()
            .map_err(|e| e.to_pyexception(vm))?;
        vm.audit("winreg.DeleteValue", || {
            (key.0 as isize, audit_wide(wide_value.as_deref(), vm))
        })?;
        let res = unsafe { host_winreg::delete_value(key.0, wide_value.as_deref()) };
        if res == 0 {
            Ok(())
        } else {
            Err(os_error_from_windows_code(vm, res as i32))
        }
    }

    #[derive(FromArgs, Debug)]
    struct DeleteKeyExArgs {
        #[pyarg(any)]
        key: HKEYArg,
        #[pyarg(any)]
        sub_key: String,
        #[pyarg(any, default = host_winreg::KEY_WOW64_64KEY as i32, py_default = "winreg.KEY_WOW64_64KEY")]
        access: i32,
        #[pyarg(any, default)]
        reserved: i32,
    }

    #[pyfunction]
    fn DeleteKeyEx(args: DeleteKeyExArgs, vm: &VirtualMachine) -> PyResult<()> {
        let wide_sub_key = WideCString::from_str(args.sub_key).map_err(|e| e.to_pyexception(vm))?;
        vm.audit("winreg.DeleteKey", || {
            (
                args.key.0 as isize,
                audit_wide(Some(&wide_sub_key), vm),
                args.access as u32 as isize,
            )
        })?;
        let res = unsafe {
            host_winreg::delete_key_ex(
                args.key.0,
                &wide_sub_key,
                args.access as u32,
                args.reserved as u32,
            )
        };
        if res == 0 {
            Ok(())
        } else {
            Err(os_error_from_windows_code(vm, res as i32))
        }
    }

    #[pyfunction]
    fn EnumKey(key: HKEYArg, index: i32, vm: &VirtualMachine) -> PyResult<String> {
        let hkey = key.0;
        vm.audit("winreg.EnumKey", || (hkey as isize, index))?;
        // The Windows docs claim that the max key name length is 255
        // characters, plus a terminating nul character.  However,
        // empirical testing demonstrates that it is possible to
        // create a 256 character key that is missing the terminating
        // nul.  RegEnumKeyEx requires a 257 character buffer to
        // retrieve such a key name.
        let mut tmpbuf = [0u16; 257];
        let mut len = tmpbuf.len() as u32;
        let res =
            unsafe { host_winreg::enum_key_ex(hkey, index as u32, tmpbuf.as_mut_ptr(), &mut len) };
        if res != 0 {
            return Err(os_error_from_windows_code(vm, res as i32));
        }
        String::from_utf16(&tmpbuf[..len as usize])
            .map_err(|e| vm.new_value_error(format!("UTF16 error: {e}")))
    }

    #[pyfunction]
    fn EnumValue(key: HKEYArg, index: i32, vm: &VirtualMachine) -> PyResult {
        vm.audit("winreg.EnumValue", || (key.0 as isize, index))?;
        // Query registry for the required buffer sizes.
        let mut ret_value_size: u32 = 0;
        let mut ret_data_size: u32 = 0;
        let hkey: host_winreg::HKEY = key.0;
        let rc = unsafe {
            host_winreg::query_info_key(
                hkey,
                ptr::null_mut(),
                ptr::null_mut(),
                &mut ret_value_size as *mut u32,
                &mut ret_data_size as *mut u32,
            )
        };
        if rc != 0 {
            return Err(os_error_from_windows_code(vm, rc as i32));
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
                host_winreg::enum_value(
                    hkey,
                    index as u32,
                    ret_value_buf.as_mut_ptr(),
                    &mut current_value_size as *mut u32,
                    &mut reg_type as *mut u32,
                    ret_data_buf.as_mut_ptr(),
                    &mut current_data_size as *mut u32,
                )
            };
            if rc == host_winreg::ERROR_MORE_DATA {
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
                return Err(os_error_from_windows_code(vm, rc as i32));
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
            let py_data = reg_to_py(vm, data_slice, reg_type);

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
    fn FlushKey(key: HKEYArg, vm: &VirtualMachine) -> PyResult<()> {
        let res = host_winreg::flush_key(key.0);
        if res == 0 {
            Ok(())
        } else {
            Err(os_error_from_windows_code(vm, res as i32))
        }
    }

    #[pyfunction]
    fn LoadKey(
        key: HKEYArg,
        sub_key: String,
        file_name: String,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let (Ok(sub_key), Ok(file_name)) = (
            WideCString::from_str(sub_key),
            WideCString::from_str(file_name),
        ) else {
            return Err(nul_char_error(vm));
        };
        vm.audit("winreg.LoadKey", || {
            (
                key.0 as isize,
                audit_wide(Some(&sub_key), vm),
                audit_wide(Some(&file_name), vm),
            )
        })?;
        let res = unsafe { host_winreg::load_key(key.0, &sub_key, &file_name) };
        if res == 0 {
            Ok(())
        } else {
            Err(os_error_from_windows_code(vm, res as i32))
        }
    }

    #[derive(Debug, FromArgs)]
    struct OpenKeyArgs {
        #[pyarg(any)]
        key: HKEYArg,
        #[pyarg(any)]
        sub_key: Option<PyRef<PyStr>>,
        #[pyarg(any, default)]
        reserved: i32,
        #[pyarg(any, default = host_winreg::KEY_READ as i32, py_default = "winreg.KEY_READ")]
        access: i32,
    }

    #[pyfunction]
    #[pyfunction(name = "OpenKeyEx")]
    fn OpenKey(args: OpenKeyArgs, vm: &VirtualMachine) -> PyResult<PyHkey> {
        let sub_key = args
            .sub_key
            .as_ref()
            .map(|s| WideCString::from_vec(s.as_wtf8().encode_wide().collect::<Vec<_>>()))
            .transpose()
            .map_err(|_| nul_char_error(vm))?;
        let key = args.key.0;
        vm.audit("winreg.OpenKey", || {
            (key as isize, args.sub_key, args.access as u32 as isize)
        })?;
        let mut res: host_winreg::HKEY = core::ptr::null_mut();
        let err = vm.allow_threads(|| unsafe {
            host_winreg::open_key_ex(
                key,
                sub_key.as_deref(),
                args.reserved as u32,
                args.access as u32,
                &mut res,
            )
        });
        if err == 0 {
            let result = PyHkey::new(res);
            vm.audit("winreg.OpenKey/result", || (res as isize,))?;
            Ok(result)
        } else {
            Err(os_error_from_windows_code(vm, err as i32))
        }
    }

    #[pyfunction]
    fn QueryInfoKey(key: HKEYArg, vm: &VirtualMachine) -> PyResult<PyRef<PyTuple>> {
        let key = key.0;
        vm.audit("winreg.QueryInfoKey", || (key as isize,))?;
        let info = host_winreg::query_info_key_full(key)
            .map_err(|err| os_error_from_windows_code(vm, err as i32))?;
        let tup: Vec<PyObjectRef> = vec![
            vm.ctx.new_int(info.sub_keys).into(),
            vm.ctx.new_int(info.values).into(),
            vm.ctx.new_int(info.last_write_time).into(),
        ];
        Ok(vm.ctx.new_tuple(tup))
    }

    #[pyfunction]
    fn QueryValue(
        key: HKEYArg,
        sub_key: Option<Wtf8Buf>,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<PyStr>> {
        let hkey = key.0;

        let sub_key = sub_key
            .as_ref()
            .map(|s| s.to_wide_cstring().map_err(|_| nul_char_error(vm)))
            .transpose()?;
        vm.audit("winreg.QueryValue", || {
            let sub_key: PyObjectRef = sub_key.as_ref().map_or_else(
                || vm.ctx.none(),
                |s| vm.ctx.new_str(Wtf8Buf::from_wide(s.as_slice())).into(),
            );
            (hkey as isize, sub_key, vm.ctx.none())
        })?;

        if hkey == host_winreg::HKEY_PERFORMANCE_DATA {
            return Err(os_error_from_windows_code(
                vm,
                host_winreg::ERROR_INVALID_HANDLE as i32,
            ));
        }

        host_winreg::query_default_value_wtf8(hkey, sub_key.as_deref())
            .map(|value| vm.ctx.new_str(value))
            .map_err(|err| err.to_pyexception(vm))
    }

    #[pyfunction]
    fn QueryValueEx(
        key: HKEYArg,
        name: Option<PyRef<PyStr>>,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<PyTuple>> {
        let hkey = key.0;
        let wide_name = name
            .as_ref()
            .map(|name| {
                name.as_wtf8()
                    .to_wide_cstring()
                    .map_err(|_| nul_char_error(vm))
            })
            .transpose()?;
        vm.audit("winreg.QueryValue", || {
            let name: PyObjectRef = wide_name.as_ref().map_or_else(
                || vm.ctx.none(),
                |name| vm.ctx.new_str(Wtf8Buf::from_wide(name.as_slice())).into(),
            );
            (hkey as isize, vm.ctx.none(), name)
        })?;
        let (ret_buf, typ) = host_winreg::query_value_bytes(hkey, wide_name.as_deref())
            .map_err(|err| os_error_from_windows_code(vm, err as i32))?;
        let obj = reg_to_py(vm, &ret_buf, typ);
        // Return tuple (value, type)
        Ok(vm.ctx.new_tuple(vec![obj, vm.ctx.new_int(typ).into()]))
    }

    #[pyfunction]
    fn SaveKey(key: HKEYArg, file_name: String, vm: &VirtualMachine) -> PyResult<()> {
        let file_name = WideCString::from_str(file_name).map_err(|e| e.to_pyexception(vm))?;
        vm.audit("winreg.SaveKey", || {
            (key.0 as isize, audit_wide(Some(&file_name), vm))
        })?;
        let res = unsafe { host_winreg::save_key(key.0, &file_name) };
        if res == 0 {
            Ok(())
        } else {
            Err(os_error_from_windows_code(vm, res as i32))
        }
    }

    #[pyfunction]
    fn SetValue(
        key: HKEYArg,
        sub_key: String,
        r#type: u32,
        value: String,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        if r#type != host_winreg::REG_SZ {
            return Err(vm.new_type_error("type must be winreg.REG_SZ"));
        }

        let hkey = key.0;

        let sub_key = WideCString::from_str(sub_key).map_err(|e| e.to_pyexception(vm))?;
        // Value can contain interior NULs.
        let mut wide_value = WideString::with_capacity(value.len() + 1);
        wide_value.push_str(&value);
        wide_value.push_str("\0");
        vm.audit("winreg.SetValue", || {
            (
                hkey as isize,
                audit_wide(Some(&sub_key), vm),
                r#type as isize,
                value,
            )
        })?;
        if hkey == host_winreg::HKEY_PERFORMANCE_DATA {
            return Err(os_error_from_windows_code(
                vm,
                host_winreg::ERROR_INVALID_HANDLE as i32,
            ));
        }
        let res = host_winreg::set_default_value(hkey, &sub_key, r#type, &wide_value);

        if res == 0 {
            Ok(())
        } else {
            Err(os_error_from_windows_code(vm, res as i32))
        }
    }

    fn reg_to_py(vm: &VirtualMachine, ret_data: &[u8], typ: u32) -> PyObjectRef {
        match typ {
            REG_DWORD => {
                // If there isn’t enough data, return 0.
                let val = ret_data
                    .first_chunk::<4>()
                    .copied()
                    .map_or(0, u32::from_ne_bytes);
                vm.ctx.new_int(val).into()
            }
            REG_QWORD => {
                let val = ret_data
                    .first_chunk::<8>()
                    .copied()
                    .map_or(0, u64::from_ne_bytes);
                vm.ctx.new_int(val).into()
            }
            REG_SZ | REG_EXPAND_SZ => {
                let u16_slice = host_winreg::registry_wide_units(ret_data);
                // Only use characters up to the first NUL.
                let len = u16_slice
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(u16_slice.len());
                let s = Wtf8Buf::from_wide(&u16_slice[..len]);
                vm.ctx.new_str(s).into()
            }
            REG_MULTI_SZ => {
                if ret_data.is_empty() {
                    vm.ctx.new_list(vec![]).into()
                } else {
                    let u16_slice = host_winreg::registry_wide_units(ret_data);
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
                            let s = Wtf8Buf::from_wide(&u16_slice[start..i]);
                            strings.push(vm.ctx.new_str(s).into());
                            start = i + 1;
                        }
                    }
                    // Handle last string if not null-terminated
                    if start < len {
                        let s = Wtf8Buf::from_wide(&u16_slice[start..len]);
                        strings.push(vm.ctx.new_str(s).into());
                    }
                    vm.ctx.new_list(strings).into()
                }
            }
            // For REG_BINARY and any other unknown types, return a bytes object if data exists.
            _ => {
                if ret_data.is_empty() {
                    vm.ctx.none()
                } else {
                    vm.ctx.new_bytes(ret_data.to_vec()).into()
                }
            }
        }
    }

    fn py2reg(value: &PyObjectRef, typ: u32, vm: &VirtualMachine) -> PyResult<Option<Vec<u8>>> {
        let invalid_data =
            || vm.new_value_error("Could not convert the data to the specified type.");
        match typ {
            REG_DWORD => {
                if vm.is_none(value) {
                    return Ok(Some(0u32.to_le_bytes().to_vec()));
                }
                let val = value.downcast_ref::<PyInt>().ok_or_else(invalid_data)?;
                let bigint = val.as_bigint();
                // Check for negative value - raise OverflowError
                if bigint.sign() == Sign::Minus {
                    return Err(vm.new_overflow_error("int too big to convert"));
                }
                let val = bigint
                    .to_u32()
                    .ok_or_else(|| vm.new_overflow_error("int too big to convert"))?;
                Ok(Some(val.to_le_bytes().to_vec()))
            }
            REG_QWORD => {
                if vm.is_none(value) {
                    return Ok(Some(0u64.to_le_bytes().to_vec()));
                }
                let val = value.downcast_ref::<PyInt>().ok_or_else(invalid_data)?;
                let bigint = val.as_bigint();
                // Check for negative value - raise OverflowError
                if bigint.sign() == Sign::Minus {
                    return Err(vm.new_overflow_error("int too big to convert"));
                }
                let val = bigint
                    .to_u64()
                    .ok_or_else(|| vm.new_overflow_error("int too big to convert"))?;
                Ok(Some(val.to_le_bytes().to_vec()))
            }
            REG_SZ | REG_EXPAND_SZ => {
                if vm.is_none(value) {
                    // Return empty string as UTF-16 null terminator
                    return Ok(Some(vec![0u8, 0u8]));
                }
                // Registry values are allowed to contain interior NULs.
                let bytes: Vec<u8> = value
                    .downcast_ref::<PyStr>()
                    .ok_or_else(invalid_data)?
                    .as_wtf8()
                    .encode_wide()
                    .chain([0u16])
                    .flat_map(u16::to_le_bytes)
                    .collect();
                Ok(Some(bytes))
            }
            REG_MULTI_SZ => {
                if vm.is_none(value) {
                    return Ok(Some(vec![0u8, 0u8]));
                }
                let list = value
                    .downcast_ref::<crate::builtins::PyList>()
                    .ok_or_else(invalid_data)?;

                let items = list.borrow_vec();
                for item in items.iter() {
                    if !item.downcastable::<PyStr>() {
                        return Err(invalid_data());
                    }
                }
                let mut encoded = Vec::<u8>::new();
                for item in items.iter() {
                    let s = item
                        .downcast_ref::<PyStr>()
                        .expect("all list items were checked above");
                    for unit in s.as_wtf8().encode_wide().chain([0u16]) {
                        encoded.extend_from_slice(&unit.to_le_bytes());
                    }
                }
                encoded.extend_from_slice(&0u16.to_le_bytes());
                Ok(Some(encoded))
            }
            // REG_BINARY and other types
            _ => {
                if vm.is_none(value) {
                    return Ok(None);
                }
                // Exact bytes need no buffer acquisition; subclasses and other
                // exporters must use their buffer protocol implementation.
                if value.class().is(vm.ctx.types.bytes_type) {
                    let bytes = value
                        .downcast_ref::<crate::builtins::PyBytes>()
                        .expect("the exact bytes type has a bytes payload");
                    return Ok(Some(bytes.as_bytes().to_vec()));
                }
                if !value.check_buffer() {
                    return Err(vm.new_type_error(format!(
                        "Objects of type '{}' can not be used as binary registry values",
                        value.class().name()
                    )));
                }
                value.try_bytes_like(vm, |bytes| Some(bytes.to_vec()))
            }
        }
    }

    #[pyfunction]
    fn SetValueEx(
        key: HKEYArg,
        value_name: Option<String>,
        _reserved: PyObjectRef,
        r#type: u32,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let hkey = key.0;
        let value_name = value_name
            .map(WideCString::from_str)
            .transpose()
            .map_err(|e| e.to_pyexception(vm))?;
        let reg_value = py2reg(&value, r#type, vm)?;
        vm.audit("winreg.SetValue", || {
            (
                hkey as isize,
                audit_wide(value_name.as_deref(), vm),
                r#type as isize,
                value,
            )
        })?;
        let (ptr, len) = match &reg_value {
            Some(v) => (v.as_ptr(), v.len() as u32),
            None => (core::ptr::null(), 0),
        };
        let res =
            unsafe { host_winreg::set_value_ex(hkey, value_name.as_deref(), r#type, ptr, len) };
        if res != 0 {
            return Err(os_error_from_windows_code(vm, res as i32));
        }
        Ok(())
    }

    #[pyfunction]
    fn DisableReflectionKey(key: HKEYArg, vm: &VirtualMachine) -> PyResult<()> {
        vm.audit("winreg.DisableReflectionKey", || (key.0 as isize,))?;
        let res = host_winreg::disable_reflection_key(key.0);
        if res == 0 {
            Ok(())
        } else {
            Err(os_error_from_windows_code(vm, res as i32))
        }
    }

    #[pyfunction]
    fn EnableReflectionKey(key: HKEYArg, vm: &VirtualMachine) -> PyResult<()> {
        vm.audit("winreg.EnableReflectionKey", || (key.0 as isize,))?;
        let res = host_winreg::enable_reflection_key(key.0);
        if res == 0 {
            Ok(())
        } else {
            Err(os_error_from_windows_code(vm, res as i32))
        }
    }

    #[pyfunction]
    fn QueryReflectionKey(key: HKEYArg, vm: &VirtualMachine) -> PyResult<bool> {
        vm.audit("winreg.QueryReflectionKey", || (key.0 as isize,))?;
        let mut result: i32 = 0;
        let res = unsafe { host_winreg::query_reflection_key(key.0, &mut result) };
        if res == 0 {
            Ok(result != 0)
        } else {
            Err(os_error_from_windows_code(vm, res as i32))
        }
    }

    #[pyfunction]
    fn ExpandEnvironmentStrings(
        string: PyRef<PyStr>,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<PyStr>> {
        let string = WideCString::from_vec(string.as_wtf8().encode_wide().collect::<Vec<_>>())
            .map_err(|_| nul_char_error(vm))?;
        vm.audit("winreg.ExpandEnvironmentStrings", || {
            (audit_wide(Some(&string), vm),)
        })?;
        let expanded = host_winreg::expand_environment_strings_wide(&string)
            .map_err(|err| err.to_pyexception(vm))?;
        Ok(vm.ctx.new_str(Wtf8Buf::from_wide(&expanded)))
    }
}

#![allow(non_snake_case)]

use crate::{PyObjectRef, VirtualMachine};

pub(crate) fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let module = winreg::make_module(vm);

    macro_rules! add_constants {
        ($($name:ident),*$(,)?) => {
            extend_module!(vm, module, {
                $((stringify!($name)) => vm.new_pyobj(::winreg::enums::$name as usize)),*
            })
        };
    }

    add_constants!(
        HKEY_CLASSES_ROOT,
        HKEY_CURRENT_USER,
        HKEY_LOCAL_MACHINE,
        HKEY_USERS,
        HKEY_PERFORMANCE_DATA,
        HKEY_CURRENT_CONFIG,
        HKEY_DYN_DATA,
    );
    module
}

#[pymodule]
mod winreg {
    use crate::common::lock::{PyRwLock, PyRwLockReadGuard, PyRwLockWriteGuard};
    use crate::{
        builtins::PyStrRef, function::IntoPyException, PyObjectRef, PyRef, PyResult, PyValue,
        TryFromObject, VirtualMachine,
    };
    use ::winreg::{enums::RegType, RegKey, RegValue};
    use std::ffi::OsStr;
    use std::io;
    use winapi::shared::winerror;

    // access rights
    #[pyattr]
    pub use winapi::um::winnt::{
        KEY_ALL_ACCESS, KEY_CREATE_LINK, KEY_CREATE_SUB_KEY, KEY_ENUMERATE_SUB_KEYS, KEY_EXECUTE,
        KEY_NOTIFY, KEY_QUERY_VALUE, KEY_READ, KEY_SET_VALUE, KEY_WOW64_32KEY, KEY_WOW64_64KEY,
        KEY_WRITE,
    };
    // value types
    #[pyattr]
    pub use winapi::um::winnt::{
        REG_BINARY, REG_DWORD, REG_DWORD_BIG_ENDIAN, REG_DWORD_LITTLE_ENDIAN, REG_EXPAND_SZ,
        REG_FULL_RESOURCE_DESCRIPTOR, REG_LINK, REG_MULTI_SZ, REG_NONE, REG_QWORD,
        REG_QWORD_LITTLE_ENDIAN, REG_RESOURCE_LIST, REG_RESOURCE_REQUIREMENTS_LIST, REG_SZ,
    };

    #[pyattr]
    #[pyclass(module = "winreg", name = "HKEYType")]
    #[derive(Debug, PyValue)]
    struct PyHkey {
        key: PyRwLock<RegKey>,
    }
    type PyHkeyRef = PyRef<PyHkey>;

    // TODO: fix this
    unsafe impl Sync for PyHkey {}

    impl PyHkey {
        fn new(key: RegKey) -> Self {
            Self {
                key: PyRwLock::new(key),
            }
        }

        fn key(&self) -> PyRwLockReadGuard<'_, RegKey> {
            self.key.read()
        }

        fn key_mut(&self) -> PyRwLockWriteGuard<'_, RegKey> {
            self.key.write()
        }
    }

    #[pyimpl]
    impl PyHkey {
        #[pymethod]
        fn Close(&self) {
            let null_key = RegKey::predef(0 as ::winreg::HKEY);
            let key = std::mem::replace(&mut *self.key_mut(), null_key);
            drop(key);
        }
        #[pymethod]
        fn Detach(&self) -> usize {
            let null_key = RegKey::predef(0 as ::winreg::HKEY);
            let key = std::mem::replace(&mut *self.key_mut(), null_key);
            let handle = key.raw_handle();
            std::mem::forget(key);
            handle as usize
        }

        #[pymethod(magic)]
        fn bool(&self) -> bool {
            !self.key().raw_handle().is_null()
        }
        #[pymethod(magic)]
        fn enter(zelf: PyRef<Self>) -> PyRef<Self> {
            zelf
        }
        #[pymethod(magic)]
        fn exit(&self, _cls: PyObjectRef, _exc: PyObjectRef, _tb: PyObjectRef) {
            self.Close();
        }
    }

    enum Hkey {
        PyHkey(PyHkeyRef),
        Constant(::winreg::HKEY),
    }
    impl TryFromObject for Hkey {
        fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
            obj.downcast().map(Self::PyHkey).or_else(|o| {
                usize::try_from_object(vm, o).map(|i| Self::Constant(i as ::winreg::HKEY))
            })
        }
    }
    impl Hkey {
        fn with_key<R>(&self, f: impl FnOnce(&RegKey) -> R) -> R {
            match self {
                Self::PyHkey(py) => f(&py.key()),
                Self::Constant(hkey) => {
                    let k = RegKey::predef(*hkey);
                    let res = f(&k);
                    std::mem::forget(k);
                    res
                }
            }
        }
        fn into_key(self) -> RegKey {
            let k = match self {
                Self::PyHkey(py) => py.key().raw_handle(),
                Self::Constant(k) => k,
            };
            RegKey::predef(k)
        }
    }

    #[derive(FromArgs)]
    struct OpenKeyArgs {
        key: Hkey,
        sub_key: Option<PyStrRef>,
        #[pyarg(any, default = "0")]
        reserved: i32,
        #[pyarg(any, default = "::winreg::enums::KEY_READ")]
        access: u32,
    }

    #[pyfunction(name = "OpenKeyEx")]
    #[pyfunction]
    fn OpenKey(args: OpenKeyArgs, vm: &VirtualMachine) -> PyResult<PyHkey> {
        let OpenKeyArgs {
            key,
            sub_key,
            reserved,
            access,
        } = args;

        if reserved != 0 {
            // RegKey::open_subkey* doesn't have a reserved param, so this'll do
            return Err(vm.new_value_error("reserved param must be 0".to_owned()));
        }

        let sub_key = sub_key.as_ref().map_or("", |s| s.as_str());
        let key = key
            .with_key(|k| k.open_subkey_with_flags(sub_key, access))
            .map_err(|e| e.into_pyexception(vm))?;

        Ok(PyHkey::new(key))
    }

    #[pyfunction]
    fn QueryValue(key: Hkey, subkey: Option<PyStrRef>, vm: &VirtualMachine) -> PyResult<String> {
        let subkey = subkey.as_ref().map_or("", |s| s.as_str());
        key.with_key(|k| k.get_value(subkey))
            .map_err(|e| e.into_pyexception(vm))
    }

    #[pyfunction]
    fn QueryValueEx(
        key: Hkey,
        subkey: Option<PyStrRef>,
        vm: &VirtualMachine,
    ) -> PyResult<(PyObjectRef, usize)> {
        let subkey = subkey.as_ref().map_or("", |s| s.as_str());
        key.with_key(|k| k.get_raw_value(subkey))
            .map_err(|e| e.into_pyexception(vm))
            .and_then(|regval| {
                let ty = regval.vtype.clone() as usize;
                Ok((reg_to_py(regval, vm)?, ty))
            })
    }

    #[pyfunction]
    fn EnumKey(key: Hkey, index: u32, vm: &VirtualMachine) -> PyResult<String> {
        key.with_key(|k| k.enum_keys().nth(index as usize))
            .unwrap_or_else(|| {
                Err(io::Error::from_raw_os_error(
                    winerror::ERROR_NO_MORE_ITEMS as i32,
                ))
            })
            .map_err(|e| e.into_pyexception(vm))
    }

    #[pyfunction]
    fn EnumValue(
        key: Hkey,
        index: u32,
        vm: &VirtualMachine,
    ) -> PyResult<(String, PyObjectRef, usize)> {
        key.with_key(|k| k.enum_values().nth(index as usize))
            .unwrap_or_else(|| {
                Err(io::Error::from_raw_os_error(
                    winerror::ERROR_NO_MORE_ITEMS as i32,
                ))
            })
            .map_err(|e| e.into_pyexception(vm))
            .and_then(|(name, value)| {
                let ty = value.vtype.clone() as usize;
                Ok((name, reg_to_py(value, vm)?, ty))
            })
    }

    #[pyfunction]
    fn CloseKey(key: Hkey) {
        match key {
            Hkey::PyHkey(py) => py.Close(),
            Hkey::Constant(hkey) => drop(RegKey::predef(hkey)),
        }
    }

    #[pyfunction]
    fn CreateKey(key: Hkey, subkey: Option<PyStrRef>, vm: &VirtualMachine) -> PyResult<PyHkey> {
        let k = match subkey {
            Some(subkey) => {
                let (k, _disp) = key
                    .with_key(|k| k.create_subkey(subkey.as_str()))
                    .map_err(|e| e.into_pyexception(vm))?;
                k
            }
            None => key.into_key(),
        };
        Ok(PyHkey::new(k))
    }

    #[pyfunction]
    fn SetValue(
        key: Hkey,
        subkey: Option<PyStrRef>,
        typ: u32,
        value: PyStrRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        if typ != REG_SZ as u32 {
            return Err(vm.new_type_error("type must be winreg.REG_SZ".to_owned()));
        }
        let subkey = subkey.as_ref().map_or("", |s| s.as_str());
        key.with_key(|k| k.set_value(subkey, &OsStr::new(value.as_str())))
            .map_err(|e| e.into_pyexception(vm))
    }

    #[pyfunction]
    fn DeleteKey(key: Hkey, subkey: PyStrRef, vm: &VirtualMachine) -> PyResult<()> {
        key.with_key(|k| k.delete_subkey(subkey.as_str()))
            .map_err(|e| e.into_pyexception(vm))
    }

    fn reg_to_py(value: RegValue, vm: &VirtualMachine) -> PyResult {
        macro_rules! bytes_to_int {
            ($int:ident, $f:ident, $name:ident) => {{
                let i = if value.bytes.is_empty() {
                    Ok(0 as $int)
                } else {
                    (&*value.bytes).try_into().map($int::$f).map_err(|_| {
                        vm.new_value_error(format!("{} value is wrong length", stringify!(name)))
                    })
                };
                i.map(|i| vm.ctx.new_int(i).into())
            }};
        }
        let bytes_to_wide = |b: &[u8]| -> Option<&[u16]> {
            if b.len() % 2 == 0 {
                Some(unsafe { std::slice::from_raw_parts(b.as_ptr().cast(), b.len() / 2) })
            } else {
                None
            }
        };
        match value.vtype {
            RegType::REG_DWORD => bytes_to_int!(u32, from_ne_bytes, REG_DWORD),
            RegType::REG_DWORD_BIG_ENDIAN => {
                bytes_to_int!(u32, from_be_bytes, REG_DWORD_BIG_ENDIAN)
            }
            RegType::REG_QWORD => bytes_to_int!(u64, from_ne_bytes, REG_DWORD),
            // RegType::REG_QWORD_BIG_ENDIAN => bytes_to_int!(u64, from_be_bytes, REG_DWORD_BIG_ENDIAN),
            RegType::REG_SZ | RegType::REG_EXPAND_SZ => {
                let wide_slice = bytes_to_wide(&value.bytes).ok_or_else(|| {
                    vm.new_value_error("REG_SZ string doesn't have an even byte length".to_owned())
                })?;
                let nul_pos = wide_slice
                    .iter()
                    .position(|w| *w == 0)
                    .unwrap_or_else(|| wide_slice.len());
                let s = String::from_utf16_lossy(&wide_slice[..nul_pos]);
                Ok(vm.ctx.new_str(s).into())
            }
            RegType::REG_MULTI_SZ => {
                if value.bytes.is_empty() {
                    return Ok(vm.ctx.new_list(vec![]).into());
                }
                let wide_slice = bytes_to_wide(&value.bytes).ok_or_else(|| {
                    vm.new_value_error(
                        "REG_MULTI_SZ string doesn't have an even byte length".to_owned(),
                    )
                })?;
                let wide_slice = if let Some((0, rest)) = wide_slice.split_last() {
                    rest
                } else {
                    wide_slice
                };
                let strings = wide_slice
                    .split(|c| *c == 0)
                    .map(|s| vm.new_pyobj(String::from_utf16_lossy(s)))
                    .collect();
                Ok(vm.ctx.new_list(strings).into())
            }
            _ => {
                if value.bytes.is_empty() {
                    Ok(vm.ctx.none())
                } else {
                    Ok(vm.ctx.new_bytes(value.bytes).into())
                }
            }
        }
    }
}

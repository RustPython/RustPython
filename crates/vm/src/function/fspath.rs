use crate::{
    PyObjectRef, PyResult, TryFromObject, VirtualMachine,
    builtins::{PyBytes, PyBytesRef, PyStrRef},
    convert::{IntoPyException, ToPyObject},
    function::PyStr,
    protocol::PyBuffer,
};
use std::{borrow::Cow, ffi::OsStr, path::PathBuf};

#[derive(Clone)]
pub enum FsPath {
    Str(PyStrRef),
    Bytes(PyBytesRef),
}

impl FsPath {
    pub fn try_from_path_like(
        obj: PyObjectRef,
        check_for_nul: bool,
        vm: &VirtualMachine,
    ) -> PyResult<Self> {
        Self::try_from(
            obj,
            check_for_nul,
            "expected str, bytes or os.PathLike object",
            vm,
        )
    }

    // PyOS_FSPath in CPython
    pub fn try_from(
        obj: PyObjectRef,
        check_for_nul: bool,
        msg: &'static str,
        vm: &VirtualMachine,
    ) -> PyResult<Self> {
        let check_nul = |b: &[u8]| {
            if !check_for_nul || memchr::memchr(b'\0', b).is_none() {
                Ok(())
            } else {
                Err(crate::exceptions::cstring_error(vm))
            }
        };
        let match1 = |obj: PyObjectRef| {
            let pathlike = match_class!(match obj {
                s @ PyStr => {
                    check_nul(s.as_bytes())?;
                    Self::Str(s)
                }
                b @ PyBytes => {
                    check_nul(&b)?;
                    Self::Bytes(b)
                }
                obj => return Ok(Err(obj)),
            });
            Ok(Ok(pathlike))
        };
        let obj = match match1(obj)? {
            Ok(pathlike) => return Ok(pathlike),
            Err(obj) => obj,
        };
        let not_pathlike_error = || format!("{msg}, not {}", obj.class().name());
        let method = vm.get_method_or_type_error(
            obj.clone(),
            identifier!(vm, __fspath__),
            not_pathlike_error,
        )?;
        // If __fspath__ is explicitly set to None, treat it as if it doesn't have __fspath__
        if vm.is_none(&method) {
            return Err(vm.new_type_error(not_pathlike_error()));
        }
        let result = method.call((), vm)?;
        match1(result)?.map_err(|result| {
            vm.new_type_error(format!(
                "expected {}.__fspath__() to return str or bytes, not {}",
                obj.class().name(),
                result.class().name(),
            ))
        })
    }

    pub fn as_os_str(&self, vm: &VirtualMachine) -> PyResult<Cow<'_, OsStr>> {
        // TODO: FS encodings
        match self {
            Self::Str(s) => vm.fsencode(s),
            Self::Bytes(b) => Self::bytes_as_os_str(b.as_bytes(), vm).map(Cow::Borrowed),
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        // TODO: FS encodings
        match self {
            Self::Str(s) => s.as_bytes(),
            Self::Bytes(b) => b.as_bytes(),
        }
    }

    pub fn to_string_lossy(&self) -> Cow<'_, str> {
        match self {
            Self::Str(s) => s.to_string_lossy(),
            Self::Bytes(s) => String::from_utf8_lossy(s),
        }
    }

    pub fn to_path_buf(&self, vm: &VirtualMachine) -> PyResult<PathBuf> {
        let path = match self {
            Self::Str(s) => PathBuf::from(s.as_str()),
            Self::Bytes(b) => PathBuf::from(Self::bytes_as_os_str(b, vm)?),
        };
        Ok(path)
    }

    pub fn to_cstring(&self, vm: &VirtualMachine) -> PyResult<std::ffi::CString> {
        std::ffi::CString::new(self.as_bytes()).map_err(|e| e.into_pyexception(vm))
    }

    #[cfg(windows)]
    pub fn to_wide_cstring(&self, vm: &VirtualMachine) -> PyResult<widestring::WideCString> {
        widestring::WideCString::from_os_str(self.as_os_str(vm)?)
            .map_err(|err| err.into_pyexception(vm))
    }

    pub fn bytes_as_os_str<'a>(b: &'a [u8], vm: &VirtualMachine) -> PyResult<&'a std::ffi::OsStr> {
        rustpython_common::os::bytes_as_os_str(b)
            .map_err(|_| vm.new_unicode_decode_error("can't decode path for utf-8"))
    }
}

impl ToPyObject for FsPath {
    fn to_pyobject(self, _vm: &VirtualMachine) -> PyObjectRef {
        match self {
            Self::Str(s) => s.into(),
            Self::Bytes(b) => b.into(),
        }
    }
}

impl TryFromObject for FsPath {
    // PyUnicode_FSDecoder in CPython
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        let obj = match obj.try_to_value::<PyBuffer>(vm) {
            Ok(buffer) => {
                let mut bytes = vec![];
                buffer.append_to(&mut bytes);
                vm.ctx.new_bytes(bytes).into()
            }
            Err(_) => obj,
        };
        Self::try_from_path_like(obj, true, vm)
    }
}

use rustpython_common::crt_fd;

use crate::{
    PyObjectRef, PyResult, VirtualMachine,
    convert::{IntoPyException, ToPyException, ToPyObject, TryFromObject},
    function::FsPath,
};
use std::path::{Path, PathBuf};

// path_ without allow_fd in CPython
#[derive(Clone)]
pub struct OsPath {
    pub path: std::ffi::OsString,
    pub(super) mode: OutputMode,
}

#[derive(Debug, Copy, Clone)]
pub(super) enum OutputMode {
    String,
    Bytes,
}

impl OutputMode {
    pub(super) fn process_path(self, path: impl Into<PathBuf>, vm: &VirtualMachine) -> PyObjectRef {
        fn inner(mode: OutputMode, path: PathBuf, vm: &VirtualMachine) -> PyObjectRef {
            match mode {
                OutputMode::String => vm.fsdecode(path).into(),
                OutputMode::Bytes => vm
                    .ctx
                    .new_bytes(path.into_os_string().into_encoded_bytes())
                    .into(),
            }
        }
        inner(self, path.into(), vm)
    }
}

impl OsPath {
    pub fn new_str(path: impl Into<std::ffi::OsString>) -> Self {
        let path = path.into();
        Self {
            path,
            mode: OutputMode::String,
        }
    }

    pub(crate) fn from_fspath(fspath: FsPath, vm: &VirtualMachine) -> PyResult<Self> {
        let path = fspath.as_os_str(vm)?.into_owned();
        let mode = match fspath {
            FsPath::Str(_) => OutputMode::String,
            FsPath::Bytes(_) => OutputMode::Bytes,
        };
        Ok(Self { path, mode })
    }

    /// Convert an object to OsPath using the os.fspath-style error message.
    /// This is used by open() which should report "expected str, bytes or os.PathLike object, not"
    /// instead of "should be string, bytes or os.PathLike, not".
    pub(crate) fn try_from_fspath(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<Self> {
        let fspath = FsPath::try_from_path_like(obj, true, vm)?;
        Self::from_fspath(fspath, vm)
    }

    pub fn as_path(&self) -> &Path {
        Path::new(&self.path)
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.path.into_encoded_bytes()
    }

    pub fn to_string_lossy(&self) -> std::borrow::Cow<'_, str> {
        self.path.to_string_lossy()
    }

    pub fn into_cstring(self, vm: &VirtualMachine) -> PyResult<std::ffi::CString> {
        std::ffi::CString::new(self.into_bytes()).map_err(|err| err.to_pyexception(vm))
    }

    #[cfg(windows)]
    pub fn to_wide_cstring(&self, vm: &VirtualMachine) -> PyResult<widestring::WideCString> {
        widestring::WideCString::from_os_str(&self.path).map_err(|err| err.to_pyexception(vm))
    }

    pub fn filename(&self, vm: &VirtualMachine) -> PyObjectRef {
        self.mode.process_path(self.path.clone(), vm)
    }
}

impl AsRef<Path> for OsPath {
    fn as_ref(&self) -> &Path {
        self.as_path()
    }
}

impl TryFromObject for OsPath {
    // TODO: path_converter with allow_fd=0 in CPython
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        let fspath = FsPath::try_from(
            obj,
            true,
            "should be string, bytes, os.PathLike or integer",
            vm,
        )?;
        Self::from_fspath(fspath, vm)
    }
}

// path_t with allow_fd in CPython
#[derive(Clone)]
pub(crate) enum OsPathOrFd<'fd> {
    Path(OsPath),
    Fd(crt_fd::Borrowed<'fd>),
}

impl TryFromObject for OsPathOrFd<'_> {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        match obj.try_index_opt(vm) {
            Some(int) => {
                let fd = int?.try_to_primitive(vm)?;
                unsafe { crt_fd::Borrowed::try_borrow_raw(fd) }
                    .map(Self::Fd)
                    .map_err(|e| e.into_pyexception(vm))
            }
            None => obj.try_into_value(vm).map(Self::Path),
        }
    }
}

impl From<OsPath> for OsPathOrFd<'_> {
    fn from(path: OsPath) -> Self {
        Self::Path(path)
    }
}

impl OsPathOrFd<'_> {
    pub fn filename(&self, vm: &VirtualMachine) -> PyObjectRef {
        match self {
            Self::Path(path) => path.filename(vm),
            Self::Fd(fd) => fd.to_pyobject(vm),
        }
    }
}

impl crate::exceptions::OSErrorBuilder {
    #[must_use]
    pub(crate) fn with_filename<'a>(
        error: &std::io::Error,
        filename: impl Into<OsPathOrFd<'a>>,
        vm: &VirtualMachine,
    ) -> crate::builtins::PyBaseExceptionRef {
        // TODO: return type to PyRef<PyOSError>
        use crate::exceptions::ToOSErrorBuilder;
        let builder = error.to_os_error_builder(vm);
        let builder = builder.filename(filename.into().filename(vm));
        builder.build(vm).upcast()
    }
}

use crate::{
    builtins::PyBaseExceptionRef,
    convert::{ToPyException, TryFromObject},
    function::FsPath,
    object::AsObject,
    PyObjectRef, PyResult, VirtualMachine,
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
    pub(super) fn process_path(self, path: impl Into<PathBuf>, vm: &VirtualMachine) -> PyResult {
        fn inner(mode: OutputMode, path: PathBuf, vm: &VirtualMachine) -> PyResult {
            let path_as_string = |p: PathBuf| {
                p.into_os_string().into_string().map_err(|_| {
                    vm.new_unicode_decode_error(
                        "Can't convert OS path to valid UTF-8 string".into(),
                    )
                })
            };
            match mode {
                OutputMode::String => path_as_string(path).map(|s| vm.ctx.new_str(s).into()),
                OutputMode::Bytes => {
                    #[cfg(any(unix, target_os = "wasi"))]
                    {
                        use rustpython_common::os::ffi::OsStringExt;
                        Ok(vm.ctx.new_bytes(path.into_os_string().into_vec()).into())
                    }
                    #[cfg(windows)]
                    {
                        path_as_string(path).map(|s| vm.ctx.new_bytes(s.into_bytes()).into())
                    }
                }
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

    pub(crate) fn from_fspath(fspath: FsPath, vm: &VirtualMachine) -> PyResult<OsPath> {
        let path = fspath.as_os_str(vm)?.to_owned();
        let mode = match fspath {
            FsPath::Str(_) => OutputMode::String,
            FsPath::Bytes(_) => OutputMode::Bytes,
        };
        Ok(OsPath { path, mode })
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
    pub fn to_widecstring(&self, vm: &VirtualMachine) -> PyResult<widestring::WideCString> {
        widestring::WideCString::from_os_str(&self.path).map_err(|err| err.to_pyexception(vm))
    }

    pub fn filename(&self, vm: &VirtualMachine) -> PyResult {
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
        let fspath = FsPath::try_from(obj, true, vm)?;
        Self::from_fspath(fspath, vm)
    }
}

// path_t with allow_fd in CPython
#[derive(Clone)]
pub(crate) enum OsPathOrFd {
    Path(OsPath),
    Fd(i32),
}

impl TryFromObject for OsPathOrFd {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        let r = match obj.try_index_opt(vm) {
            Some(int) => Self::Fd(int?.try_to_primitive(vm)?),
            None => Self::Path(obj.try_into_value(vm)?),
        };
        Ok(r)
    }
}

impl From<OsPath> for OsPathOrFd {
    fn from(path: OsPath) -> Self {
        Self::Path(path)
    }
}

impl OsPathOrFd {
    pub fn filename(&self, vm: &VirtualMachine) -> PyObjectRef {
        match self {
            OsPathOrFd::Path(path) => path.filename(vm).unwrap_or_else(|_| vm.ctx.none()),
            OsPathOrFd::Fd(fd) => vm.ctx.new_int(*fd).into(),
        }
    }
}

// TODO: preserve the input `PyObjectRef` of filename and filename2 (Failing check `self.assertIs(err.filename, name, str(func)`)
pub struct IOErrorBuilder<'a> {
    error: &'a std::io::Error,
    filename: Option<OsPathOrFd>,
    filename2: Option<OsPathOrFd>,
}

impl<'a> IOErrorBuilder<'a> {
    pub fn new(error: &'a std::io::Error) -> Self {
        Self {
            error,
            filename: None,
            filename2: None,
        }
    }
    pub(crate) fn filename(mut self, filename: impl Into<OsPathOrFd>) -> Self {
        let filename = filename.into();
        self.filename.replace(filename);
        self
    }
    pub(crate) fn filename2(mut self, filename: impl Into<OsPathOrFd>) -> Self {
        let filename = filename.into();
        self.filename2.replace(filename);
        self
    }

    pub(crate) fn with_filename(
        error: &'a std::io::Error,
        filename: impl Into<OsPathOrFd>,
        vm: &VirtualMachine,
    ) -> PyBaseExceptionRef {
        let zelf = Self {
            error,
            filename: Some(filename.into()),
            filename2: None,
        };
        zelf.to_pyexception(vm)
    }
}

impl ToPyException for IOErrorBuilder<'_> {
    fn to_pyexception(&self, vm: &VirtualMachine) -> PyBaseExceptionRef {
        let excp = self.error.to_pyexception(vm);

        if let Some(filename) = &self.filename {
            excp.as_object()
                .set_attr("filename", filename.filename(vm), vm)
                .unwrap();
        }
        if let Some(filename2) = &self.filename2 {
            excp.as_object()
                .set_attr("filename2", filename2.filename(vm), vm)
                .unwrap();
        }
        excp
    }
}

use crate::{
    AsObject, PyObjectRef, PyResult, VirtualMachine,
    class::StaticType,
    convert::{IntoPyException, ToPyObject, TryFromObject},
    exceptions::OSErrorBuilder,
    function::FsPath,
    ospath::{OsPath, PathConverter},
};
use rustpython_host_env::crt_fd;

impl PathConverter {
    /// Convert str and bytes to paths first, then treat indexable objects as
    /// file descriptors before considering `__fspath__`.
    pub(crate) fn try_path_or_fd<'fd>(
        &self,
        obj: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<OsPathOrFd<'fd>> {
        let obj = match self.try_match_str_bytes(obj, vm)? {
            Ok(path) => return Ok(OsPathOrFd::Path(path)),
            Err(obj) => obj,
        };

        if let Some(int) = obj.try_index_opt(vm) {
            if obj
                .class()
                .is(crate::builtins::bool_::PyBool::static_type())
            {
                crate::stdlib::_warnings::warn(
                    vm.ctx.exceptions.runtime_warning,
                    "bool is used as a file descriptor".to_owned(),
                    1,
                    vm,
                )?;
            }
            let fd = int?.try_to_primitive(vm)?;
            return unsafe { crt_fd::Borrowed::try_borrow_raw(fd) }
                .map(OsPathOrFd::Fd)
                .map_err(|e| e.into_pyexception(vm));
        }

        self.try_pathlike(obj, true, vm).map(OsPathOrFd::Path)
    }
}

impl OsPath {
    pub(crate) fn from_fspath(fspath: FsPath, vm: &VirtualMachine) -> PyResult<Self> {
        let path = fspath.as_os_str(vm)?.into_owned();
        let origin = match fspath {
            FsPath::Str(s) => s.into(),
            FsPath::Bytes(b) => b.into(),
        };
        Ok(Self {
            path,
            origin: Some(origin),
        })
    }

    /// Convert an object to OsPath using the os.fspath-style error message.
    /// Used by open(), which should report "expected str, bytes or os.PathLike object, not"
    /// instead of "should be string, bytes or os.PathLike, not".
    pub(crate) fn try_from_fspath(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<Self> {
        let fspath = FsPath::try_from_path_like(obj, true, vm)?;
        Self::from_fspath(fspath, vm)
    }
}

#[derive(Clone)]
pub(crate) enum OsPathOrFd<'fd> {
    Path(OsPath),
    Fd(crt_fd::Borrowed<'fd>),
}

impl TryFromObject for OsPathOrFd<'_> {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        PathConverter::new().try_path_or_fd(obj, vm)
    }
}

impl From<OsPath> for OsPathOrFd<'_> {
    fn from(path: OsPath) -> Self {
        Self::Path(path)
    }
}

impl OsPathOrFd<'_> {
    pub(crate) fn filename(&self, vm: &VirtualMachine) -> PyObjectRef {
        match self {
            Self::Path(path) => path.filename(vm),
            Self::Fd(fd) => fd.as_raw().to_pyobject(vm),
        }
    }
}

impl OSErrorBuilder {
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

    /// Like `with_filename`, but strips winerror on Windows.
    /// Use for C runtime errors (open, fstat, etc.) that should produce
    /// `[Errno X]` format instead of `[WinError X]`.
    #[must_use]
    pub(crate) fn with_filename_from_errno<'a>(
        error: &std::io::Error,
        filename: impl Into<OsPathOrFd<'a>>,
        vm: &VirtualMachine,
    ) -> crate::builtins::PyBaseExceptionRef {
        use crate::exceptions::ToOSErrorBuilder;
        let builder = error.to_os_error_builder(vm);
        #[cfg(windows)]
        let builder = builder.without_winerror();
        let builder = builder.filename(filename.into().filename(vm));
        builder.build(vm).upcast()
    }
}

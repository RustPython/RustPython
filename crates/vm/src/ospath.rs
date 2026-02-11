use rustpython_common::crt_fd;

use crate::{
    AsObject, PyObjectRef, PyResult, VirtualMachine,
    builtins::{PyBytes, PyStr},
    class::StaticType,
    convert::{IntoPyException, ToPyException, ToPyObject, TryFromObject},
    function::FsPath,
};
use std::path::{Path, PathBuf};

/// path_converter
#[derive(Clone, Copy, Default)]
pub struct PathConverter {
    /// Function name for error messages (e.g., "rename")
    pub function_name: Option<&'static str>,
    /// Argument name for error messages (e.g., "src", "dst")
    pub argument_name: Option<&'static str>,
    /// If true, embedded null characters are allowed
    pub non_strict: bool,
}

impl PathConverter {
    pub const fn new() -> Self {
        Self {
            function_name: None,
            argument_name: None,
            non_strict: false,
        }
    }

    pub const fn function(mut self, name: &'static str) -> Self {
        self.function_name = Some(name);
        self
    }

    pub const fn argument(mut self, name: &'static str) -> Self {
        self.argument_name = Some(name);
        self
    }

    pub const fn non_strict(mut self) -> Self {
        self.non_strict = true;
        self
    }

    /// Generate error message prefix like "rename: "
    fn error_prefix(&self) -> String {
        match self.function_name {
            Some(func) => format!("{}: ", func),
            None => String::new(),
        }
    }

    /// Get argument name for error messages, defaults to "path"
    fn arg_name(&self) -> &'static str {
        self.argument_name.unwrap_or("path")
    }

    /// Format a type error message
    fn type_error_msg(&self, type_name: &str, allow_fd: bool) -> String {
        let expected = if allow_fd {
            "string, bytes, os.PathLike or integer"
        } else {
            "string, bytes or os.PathLike"
        };
        format!(
            "{}{} should be {}, not {}",
            self.error_prefix(),
            self.arg_name(),
            expected,
            type_name
        )
    }

    /// Convert to OsPathOrFd (path or file descriptor)
    pub(crate) fn try_path_or_fd<'fd>(
        &self,
        obj: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<OsPathOrFd<'fd>> {
        // Handle fd (before __fspath__ check, like CPython)
        if let Some(int) = obj.try_index_opt(vm) {
            // Warn if bool is used as a file descriptor
            if obj
                .class()
                .is(crate::builtins::bool_::PyBool::static_type())
            {
                crate::stdlib::warnings::warn(
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

        self.try_path_inner(obj, true, vm).map(OsPathOrFd::Path)
    }

    /// Convert to OsPath only (no fd support)
    fn try_path_inner(
        &self,
        obj: PyObjectRef,
        allow_fd: bool,
        vm: &VirtualMachine,
    ) -> PyResult<OsPath> {
        // Try direct str/bytes match
        let obj = match self.try_match_str_bytes(obj.clone(), vm)? {
            Ok(path) => return Ok(path),
            Err(obj) => obj,
        };

        // Call __fspath__
        let type_error_msg = || self.type_error_msg(&obj.class().name(), allow_fd);
        let method =
            vm.get_method_or_type_error(obj.clone(), identifier!(vm, __fspath__), type_error_msg)?;
        if vm.is_none(&method) {
            return Err(vm.new_type_error(type_error_msg()));
        }
        let result = method.call((), vm)?;

        // Match __fspath__ result
        self.try_match_str_bytes(result.clone(), vm)?.map_err(|_| {
            vm.new_type_error(format!(
                "{}expected {}.__fspath__() to return str or bytes, not {}",
                self.error_prefix(),
                obj.class().name(),
                result.class().name(),
            ))
        })
    }

    /// Try to match str or bytes, returns Err(obj) if neither
    fn try_match_str_bytes(
        &self,
        obj: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<Result<OsPath, PyObjectRef>> {
        let check_nul = |b: &[u8]| {
            if self.non_strict || memchr::memchr(b'\0', b).is_none() {
                Ok(())
            } else {
                Err(vm.new_value_error(format!(
                    "{}embedded null character in {}",
                    self.error_prefix(),
                    self.arg_name()
                )))
            }
        };

        match_class!(match obj {
            s @ PyStr => {
                check_nul(s.as_bytes())?;
                let path = vm.fsencode(&s)?.into_owned();
                Ok(Ok(OsPath {
                    path,
                    origin: Some(s.into()),
                }))
            }
            b @ PyBytes => {
                check_nul(&b)?;
                let path = FsPath::bytes_as_os_str(&b, vm)?.to_owned();
                Ok(Ok(OsPath {
                    path,
                    origin: Some(b.into()),
                }))
            }
            obj => Ok(Err(obj)),
        })
    }

    /// Convert to OsPath directly
    pub fn try_path(&self, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<OsPath> {
        self.try_path_inner(obj, false, vm)
    }
}

/// path_t output - the converted path
#[derive(Clone)]
pub struct OsPath {
    pub path: std::ffi::OsString,
    /// Original Python object for identity preservation in OSError
    pub(super) origin: Option<PyObjectRef>,
}

#[derive(Debug, Copy, Clone)]
pub enum OutputMode {
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
        Self { path, origin: None }
    }

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

    pub fn to_string_lossy(&self) -> alloc::borrow::Cow<'_, str> {
        self.path.to_string_lossy()
    }

    pub fn into_cstring(self, vm: &VirtualMachine) -> PyResult<alloc::ffi::CString> {
        alloc::ffi::CString::new(self.into_bytes()).map_err(|err| err.to_pyexception(vm))
    }

    #[cfg(windows)]
    pub fn to_wide_cstring(&self, vm: &VirtualMachine) -> PyResult<widestring::WideCString> {
        widestring::WideCString::from_os_str(&self.path).map_err(|err| err.to_pyexception(vm))
    }

    pub fn filename(&self, vm: &VirtualMachine) -> PyObjectRef {
        if let Some(ref origin) = self.origin {
            origin.clone()
        } else {
            // Default to string when no origin (e.g., from new_str)
            OutputMode::String.process_path(self.path.clone(), vm)
        }
    }

    /// Get the output mode based on origin type (bytes -> Bytes, otherwise -> String)
    pub fn mode(&self) -> OutputMode {
        match &self.origin {
            Some(obj) if obj.downcast_ref::<PyBytes>().is_some() => OutputMode::Bytes,
            _ => OutputMode::String,
        }
    }
}

impl AsRef<Path> for OsPath {
    fn as_ref(&self) -> &Path {
        self.as_path()
    }
}

impl TryFromObject for OsPath {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        PathConverter::new().try_path(obj, vm)
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
        PathConverter::new().try_path_or_fd(obj, vm)
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

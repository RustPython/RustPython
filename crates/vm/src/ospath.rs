use crate::{
    PyObjectRef, PyResult, VirtualMachine,
    builtins::{PyBytes, PyStr},
    convert::{ToPyException, TryFromObject},
    function::FsPath,
};
use core::hint::cold_path;
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
    #[must_use]
    pub const fn new() -> Self {
        Self {
            function_name: None,
            argument_name: None,
            non_strict: false,
        }
    }

    #[must_use]
    pub const fn function(mut self, name: &'static str) -> Self {
        self.function_name = Some(name);
        self
    }

    #[must_use]
    pub const fn argument(mut self, name: &'static str) -> Self {
        self.argument_name = Some(name);
        self
    }

    #[must_use]
    pub const fn non_strict(mut self) -> Self {
        self.non_strict = true;
        self
    }

    /// Generate error message prefix like "rename: "
    fn error_prefix(&self) -> String {
        match self.function_name {
            Some(func) => format!("{func}: "),
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

    /// Convert to OsPath only (no fd support)
    pub(crate) fn try_path_inner(
        &self,
        obj: PyObjectRef,
        allow_fd: bool,
        vm: &VirtualMachine,
    ) -> PyResult<OsPath> {
        // Try direct str/bytes match
        let obj = match self.try_match_str_bytes(obj, vm)? {
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
                cold_path();
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

    #[must_use]
    pub fn as_path(&self) -> &Path {
        Path::new(&self.path)
    }

    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.path.into_encoded_bytes()
    }

    #[must_use]
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
    #[must_use]
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

#[cfg(feature = "host_env")]
pub(crate) use crate::ospath_fd::OsPathOrFd;

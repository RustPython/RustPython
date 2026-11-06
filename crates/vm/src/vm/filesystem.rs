use super::{Settings, VirtualMachine};
use crate::{
    AsObject, Py, PyResult,
    builtins::{PyStr, PyStrInterned, PyStrRef, PyUtf8Str, PyUtf8StrInterned},
    codecs::FastCodec,
};
use alloc::borrow::Cow;
use core::sync::atomic::Ordering;
use std::{
    ffi::{OsStr, OsString},
    sync::OnceLock,
};

/// The filesystem codec belongs to the interpreter configuration, not LC_CTYPE.
/// Subinterpreters share this snapshot, but initialize their own codec registry.
pub(super) struct FilesystemCodec {
    encoding: String,
    error: Option<String>,
    canonical_name: OnceLock<String>,
    #[cfg(all(feature = "host_env", target_os = "linux", target_env = "gnu"))]
    locale: Result<rustpython_host_env::locale::LocaleSnapshot, String>,
}

impl FilesystemCodec {
    pub(super) fn new(settings: &Settings) -> Self {
        #[cfg(all(feature = "host_env", target_os = "linux", target_env = "gnu"))]
        let locale = rustpython_host_env::locale::LocaleSnapshot::capture()
            .map_err(|error| error.to_string());
        #[allow(unused_mut)]
        let mut error = None;
        let encoding = (|| {
            // CPython 3.15 enables UTF-8 mode by default. These platforms use
            // UTF-8 for filesystem paths independently of the UTF-8 mode flag.
            if settings.utf8_mode != 0
                || cfg!(any(
                    windows,
                    target_vendor = "apple",
                    target_os = "android",
                    target_os = "vxworks"
                ))
                || !cfg!(feature = "host_env")
            {
                return "utf-8".to_owned();
            }
            #[cfg(all(feature = "host_env", target_os = "linux", target_env = "gnu"))]
            match &locale {
                Ok(locale) => return locale.encoding().to_owned(),
                Err(message) => error = Some(message.clone()),
            }
            #[cfg(all(feature = "host_env", target_os = "linux", not(target_env = "gnu")))]
            if let Some(encoding) = rustpython_host_env::locale::nl_langinfo_codeset()
                .and_then(|name| String::from_utf8(name).ok())
                .filter(|name| !name.is_empty())
            {
                // Other Linux libc targets have no native bootstrap snapshot
                // yet. Keep their existing UTF-8 behavior for wider codecs.
                if FastCodec::classify(&encoding).is_some() {
                    return encoding;
                }
            }
            "utf-8".to_owned()
        })();
        // Native aliases must also be usable before encodings is imported.
        let encoding =
            FastCodec::classify(&encoding).map_or(encoding, |codec| codec.name().to_owned());
        Self {
            encoding,
            error,
            canonical_name: OnceLock::new(),
            #[cfg(all(feature = "host_env", target_os = "linux", target_env = "gnu"))]
            locale,
        }
    }

    fn encoding(&self) -> &str {
        self.canonical_name.get().unwrap_or(&self.encoding)
    }
}

impl VirtualMachine {
    pub fn fs_encoding(&self) -> &'static PyStrInterned {
        self.ctx.intern_str(self.state.config.filesystem.encoding())
    }

    pub fn fs_encode_errors(&self) -> &'static PyUtf8StrInterned {
        if cfg!(windows) {
            identifier_utf8!(self, surrogatepass)
        } else {
            identifier_utf8!(self, surrogateescape)
        }
    }

    pub(super) fn check_filesystem_config(&self) -> PyResult<()> {
        match &self.state.config.filesystem.error {
            Some(error) => Err(self.new_runtime_error(error.clone())),
            None => Ok(()),
        }
    }

    pub(super) fn init_filesystem_codec(&self) -> PyResult<()> {
        self.check_filesystem_config()?;
        let filesystem = &self.state.config.filesystem;
        // Do not share Python codec objects between interpreters. Even when a
        // parent has finalized the name, this interpreter needs its own lookup.
        let codec = self
            .state
            .codec_registry
            .lookup(filesystem.encoding(), self)?;
        let name: crate::PyRef<PyUtf8Str> = codec
            .as_tuple()
            .as_object()
            .get_attr("name", self)?
            .try_into_value(self)?;
        let _ = filesystem.canonical_name.set(name.as_str().to_owned());
        self.state
            .filesystem_codec_ready
            .store(true, Ordering::Release);
        Ok(())
    }

    pub fn fsdecode(&self, value: impl Into<OsString>) -> PyResult<PyStrRef> {
        self.check_filesystem_config()?;
        let value = value.into();
        let filesystem = &self.state.config.filesystem;
        let encoding = filesystem.encoding();
        if FastCodec::classify(encoding) == Some(FastCodec::Utf8)
            && let Some(value) = value.to_str()
        {
            return Ok(self.ctx.new_str(value));
        }
        let bytes = value.into_encoded_bytes();
        #[cfg(all(feature = "host_env", target_os = "linux", target_env = "gnu"))]
        if !self.state.filesystem_codec_ready.load(Ordering::Acquire)
            && FastCodec::classify(encoding).is_none()
        {
            let locale = filesystem
                .locale
                .as_ref()
                .map_err(|error| self.new_runtime_error(error.clone()))?;
            let value = locale
                .decode(&bytes)
                .map_err(|error| self.new_runtime_error(error.to_string()))?;
            return Ok(self.ctx.new_str(value));
        }
        self.state.codec_registry.decode_text(
            self.ctx.new_bytes(bytes).into(),
            encoding,
            Some(self.fs_encode_errors().to_owned()),
            self,
        )
    }

    pub fn fsencode<'a>(&self, value: &'a Py<PyStr>) -> PyResult<Cow<'a, OsStr>> {
        self.check_filesystem_config()?;
        let filesystem = &self.state.config.filesystem;
        let encoding = filesystem.encoding();
        if FastCodec::classify(encoding) == Some(FastCodec::Utf8)
            && (cfg!(windows) || value.is_utf8())
        {
            // XXX: this is sketchy on windows; it's not guaranteed that the
            //      OsStr encoding will always be compatible with WTF-8.
            return Ok(Cow::Borrowed(unsafe {
                OsStr::from_encoded_bytes_unchecked(value.as_bytes())
            }));
        }
        #[cfg(all(feature = "host_env", target_os = "linux", target_env = "gnu"))]
        if !self.state.filesystem_codec_ready.load(Ordering::Acquire)
            && FastCodec::classify(encoding).is_none()
        {
            let locale = filesystem
                .locale
                .as_ref()
                .map_err(|error| self.new_runtime_error(error.clone()))?;
            let bytes = locale.encode(value.as_wtf8()).map_err(|error| {
                use rustpython_host_env::locale::LocaleEncodeError;
                match error {
                    LocaleEncodeError::Native(error) => self.new_runtime_error(error.to_string()),
                    LocaleEncodeError::Character { index, reason } => self
                        .new_unicode_encode_error(
                            self.ctx.new_str(encoding),
                            value.to_owned(),
                            index,
                            index + 1,
                            self.ctx.new_str(reason),
                        ),
                }
            })?;
            return Ok(Cow::Owned(unsafe {
                OsString::from_encoded_bytes_unchecked(bytes)
            }));
        }
        let bytes = self
            .state
            .codec_registry
            .encode_text(
                value.to_owned(),
                encoding,
                Some(self.fs_encode_errors().to_owned()),
                self,
            )?
            .as_bytes()
            .to_vec();
        Ok(Cow::Owned(unsafe {
            OsString::from_encoded_bytes_unchecked(bytes)
        }))
    }
}

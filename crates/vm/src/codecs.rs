use rustpython_common::{
    borrow::BorrowedValue,
    encodings::{
        CodecContext, DecodeContext, DecodeErrorHandler, EncodeContext, EncodeErrorHandler,
        EncodeReplace, StrBuffer, StrSize, errors,
    },
    str::StrKind,
    wtf8::{CodePoint, Wtf8, Wtf8Buf},
};

use crate::common::lock::OnceCell;
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyResult, TryFromBorrowedObject,
    TryFromObject, VirtualMachine,
    builtins::{PyBaseExceptionRef, PyBytes, PyBytesRef, PyStr, PyStrRef, PyTuple, PyTupleRef},
    common::{ascii, lock::PyRwLock},
    convert::ToPyObject,
    function::{ArgBytesLike, PyMethodDef},
};
use alloc::borrow::Cow;
use core::ops::{self, Range};
use std::collections::HashMap;

pub struct CodecsRegistry {
    inner: PyRwLock<RegistryInner>,
}

struct RegistryInner {
    search_path: Vec<PyObjectRef>,
    search_cache: HashMap<String, PyCodec>,
    errors: HashMap<String, PyObjectRef>,
}

pub const DEFAULT_ENCODING: &str = "utf-8";

#[derive(Clone)]
#[repr(transparent)]
pub struct PyCodec(PyTupleRef);
impl PyCodec {
    #[inline]
    pub fn from_tuple(tuple: PyTupleRef) -> Result<Self, PyTupleRef> {
        if tuple.len() == 4 {
            Ok(Self(tuple))
        } else {
            Err(tuple)
        }
    }
    #[inline]
    pub fn into_tuple(self) -> PyTupleRef {
        self.0
    }
    #[inline]
    pub fn as_tuple(&self) -> &Py<PyTuple> {
        &self.0
    }

    #[inline]
    pub fn get_encode_func(&self) -> &PyObject {
        &self.0[0]
    }
    #[inline]
    pub fn get_decode_func(&self) -> &PyObject {
        &self.0[1]
    }

    pub fn is_text_codec(&self, vm: &VirtualMachine) -> PyResult<bool> {
        let is_text = vm.get_attribute_opt(self.0.clone().into(), "_is_text_encoding")?;
        is_text.map_or(Ok(true), |is_text| is_text.try_to_bool(vm))
    }

    pub fn encode(
        &self,
        obj: PyObjectRef,
        errors: Option<PyStrRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let args = match errors {
            Some(errors) => vec![obj, errors.into()],
            None => vec![obj],
        };
        let res = self.get_encode_func().call(args, vm)?;
        let res = res
            .downcast::<PyTuple>()
            .ok()
            .filter(|tuple| tuple.len() == 2)
            .ok_or_else(|| vm.new_type_error("encoder must return a tuple (object, integer)"))?;
        // we don't actually care about the integer
        Ok(res[0].clone())
    }

    pub fn decode(
        &self,
        obj: PyObjectRef,
        errors: Option<PyStrRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let args = match errors {
            Some(errors) => vec![obj, errors.into()],
            None => vec![obj],
        };
        let res = self.get_decode_func().call(args, vm)?;
        let res = res
            .downcast::<PyTuple>()
            .ok()
            .filter(|tuple| tuple.len() == 2)
            .ok_or_else(|| vm.new_type_error("decoder must return a tuple (object,integer)"))?;
        // we don't actually care about the integer
        Ok(res[0].clone())
    }

    pub fn get_incremental_encoder(
        &self,
        errors: Option<PyStrRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let args = match errors {
            Some(e) => vec![e.into()],
            None => vec![],
        };
        vm.call_method(self.0.as_object(), "incrementalencoder", args)
    }

    pub fn get_incremental_decoder(
        &self,
        errors: Option<PyStrRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let args = match errors {
            Some(e) => vec![e.into()],
            None => vec![],
        };
        vm.call_method(self.0.as_object(), "incrementaldecoder", args)
    }
}

impl TryFromObject for PyCodec {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        obj.downcast::<PyTuple>()
            .ok()
            .and_then(|tuple| Self::from_tuple(tuple).ok())
            .ok_or_else(|| vm.new_type_error("codec search functions must return 4-tuples"))
    }
}

impl ToPyObject for PyCodec {
    #[inline]
    fn to_pyobject(self, _vm: &VirtualMachine) -> PyObjectRef {
        self.0.into()
    }
}

impl CodecsRegistry {
    pub(crate) fn new(ctx: &Context) -> Self {
        ::rustpython_vm::common::static_cell! {
            static METHODS: Box<[PyMethodDef]>;
        }

        let methods = METHODS.get_or_init(|| {
            crate::define_methods![
                "strict_errors" => strict_errors as EMPTY,
                "ignore_errors" => ignore_errors as EMPTY,
                "replace_errors" => replace_errors as EMPTY,
                "xmlcharrefreplace_errors" => xmlcharrefreplace_errors as EMPTY,
                "backslashreplace_errors" => backslashreplace_errors as EMPTY,
                "namereplace_errors" => namereplace_errors as EMPTY,
                "surrogatepass_errors" => surrogatepass_errors as EMPTY,
                "surrogateescape_errors" => surrogateescape_errors as EMPTY
            ]
            .into_boxed_slice()
        });

        let errors = [
            ("strict", methods[0].build_function(ctx)),
            ("ignore", methods[1].build_function(ctx)),
            ("replace", methods[2].build_function(ctx)),
            ("xmlcharrefreplace", methods[3].build_function(ctx)),
            ("backslashreplace", methods[4].build_function(ctx)),
            ("namereplace", methods[5].build_function(ctx)),
            ("surrogatepass", methods[6].build_function(ctx)),
            ("surrogateescape", methods[7].build_function(ctx)),
        ];
        let errors = errors
            .into_iter()
            .map(|(name, f)| (name.to_owned(), f.into()))
            .collect();
        let inner = RegistryInner {
            search_path: Vec::new(),
            search_cache: HashMap::new(),
            errors,
        };
        Self {
            inner: PyRwLock::new(inner),
        }
    }

    pub fn register(&self, search_function: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        if !search_function.is_callable() {
            return Err(vm.new_type_error("argument must be callable"));
        }
        self.inner.write().search_path.push(search_function);
        Ok(())
    }

    pub fn unregister(&self, search_function: PyObjectRef) -> PyResult<()> {
        let mut inner = self.inner.write();
        // Do nothing if search_path is not created yet or was cleared.
        if inner.search_path.is_empty() {
            return Ok(());
        }
        for (i, item) in inner.search_path.iter().enumerate() {
            if item.get_id() == search_function.get_id() {
                if !inner.search_cache.is_empty() {
                    inner.search_cache.clear();
                }
                inner.search_path.remove(i);
                return Ok(());
            }
        }
        Ok(())
    }

    pub(crate) fn register_manual(&self, name: &str, codec: PyCodec) -> PyResult<()> {
        let name = normalize_encoding_name(name);
        self.inner
            .write()
            .search_cache
            .insert(name.into_owned(), codec);
        Ok(())
    }

    pub fn lookup(&self, encoding: &str, vm: &VirtualMachine) -> PyResult<PyCodec> {
        let encoding = normalize_encoding_name(encoding);
        let search_path = {
            let inner = self.inner.read();
            if let Some(codec) = inner.search_cache.get(encoding.as_ref()) {
                // hit cache
                return Ok(codec.clone());
            }
            inner.search_path.clone()
        };
        let encoding = PyStr::from(encoding.into_owned()).into_ref(&vm.ctx);
        for func in search_path {
            let res = func.call((encoding.clone(),), vm)?;
            let res: Option<PyCodec> = res.try_into_value(vm)?;
            if let Some(codec) = res {
                let mut inner = self.inner.write();
                // someone might have raced us to this, so use theirs
                let codec = inner
                    .search_cache
                    .entry(encoding.as_str().to_owned())
                    .or_insert(codec);
                return Ok(codec.clone());
            }
        }
        Err(vm.new_lookup_error(format!("unknown encoding: {encoding}")))
    }

    fn _lookup_text_encoding(
        &self,
        encoding: &str,
        generic_func: &str,
        vm: &VirtualMachine,
    ) -> PyResult<PyCodec> {
        let codec = self.lookup(encoding, vm)?;
        if codec.is_text_codec(vm)? {
            Ok(codec)
        } else {
            Err(vm.new_lookup_error(format!(
                "'{encoding}' is not a text encoding; use {generic_func} to handle arbitrary codecs"
            )))
        }
    }

    pub fn forget(&self, encoding: &str) -> Option<PyCodec> {
        let encoding = normalize_encoding_name(encoding);
        self.inner.write().search_cache.remove(encoding.as_ref())
    }

    pub fn encode(
        &self,
        obj: PyObjectRef,
        encoding: &str,
        errors: Option<PyStrRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let codec = self.lookup(encoding, vm)?;
        codec.encode(obj, errors, vm).inspect_err(|exc| {
            Self::add_codec_note(exc, "encoding", encoding, vm);
        })
    }

    pub fn decode(
        &self,
        obj: PyObjectRef,
        encoding: &str,
        errors: Option<PyStrRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let codec = self.lookup(encoding, vm)?;
        codec.decode(obj, errors, vm).inspect_err(|exc| {
            Self::add_codec_note(exc, "decoding", encoding, vm);
        })
    }

    pub fn encode_text(
        &self,
        obj: PyStrRef,
        encoding: &str,
        errors: Option<PyStrRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyBytesRef> {
        let codec = self._lookup_text_encoding(encoding, "codecs.encode()", vm)?;
        codec
            .encode(obj.into(), errors, vm)
            .inspect_err(|exc| {
                Self::add_codec_note(exc, "encoding", encoding, vm);
            })?
            .downcast()
            .map_err(|obj| {
                vm.new_type_error(format!(
                    "'{}' encoder returned '{}' instead of 'bytes'; use codecs.encode() to \
                     encode to arbitrary types",
                    encoding,
                    obj.class().name(),
                ))
            })
    }

    pub fn decode_text(
        &self,
        obj: PyObjectRef,
        encoding: &str,
        errors: Option<PyStrRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyStrRef> {
        let codec = self._lookup_text_encoding(encoding, "codecs.decode()", vm)?;
        codec
            .decode(obj, errors, vm)
            .inspect_err(|exc| {
                Self::add_codec_note(exc, "decoding", encoding, vm);
            })?
            .downcast()
            .map_err(|obj| {
                vm.new_type_error(format!(
                    "'{}' decoder returned '{}' instead of 'str'; use codecs.decode() to \
                 decode to arbitrary types",
                    encoding,
                    obj.class().name(),
                ))
            })
    }

    fn add_codec_note(
        exc: &crate::builtins::PyBaseExceptionRef,
        operation: &str,
        encoding: &str,
        vm: &VirtualMachine,
    ) {
        let note = format!("{operation} with '{encoding}' codec failed");
        let _ = vm.call_method(exc.as_object(), "add_note", (vm.ctx.new_str(note),));
    }

    pub fn register_error(&self, name: String, handler: PyObjectRef) -> Option<PyObjectRef> {
        self.inner.write().errors.insert(name, handler)
    }

    pub fn unregister_error(&self, name: &str, vm: &VirtualMachine) -> PyResult<bool> {
        const BUILTIN_ERROR_HANDLERS: &[&str] = &[
            "strict",
            "ignore",
            "replace",
            "xmlcharrefreplace",
            "backslashreplace",
            "namereplace",
            "surrogatepass",
            "surrogateescape",
        ];
        if BUILTIN_ERROR_HANDLERS.contains(&name) {
            return Err(vm.new_value_error(format!(
                "cannot un-register built-in error handler '{name}'"
            )));
        }
        Ok(self.inner.write().errors.remove(name).is_some())
    }

    pub fn lookup_error_opt(&self, name: &str) -> Option<PyObjectRef> {
        self.inner.read().errors.get(name).cloned()
    }

    pub fn lookup_error(&self, name: &str, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        self.lookup_error_opt(name)
            .ok_or_else(|| vm.new_lookup_error(format!("unknown error handler name '{name}'")))
    }
}

fn normalize_encoding_name(encoding: &str) -> Cow<'_, str> {
    // _Py_normalize_encoding: collapse non-alphanumeric/non-dot chars into
    // single underscore, strip non-ASCII, lowercase ASCII letters.
    let needs_transform = encoding
        .bytes()
        .any(|b| b.is_ascii_uppercase() || !b.is_ascii_alphanumeric() && b != b'.');
    if !needs_transform {
        return encoding.into();
    }
    let mut out = String::with_capacity(encoding.len());
    let mut punct = false;
    for c in encoding.chars() {
        if c.is_ascii_alphanumeric() || c == '.' {
            if punct && !out.is_empty() {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
            punct = false;
        } else {
            punct = true;
        }
    }
    out.into()
}

#[derive(Eq, PartialEq)]
enum StandardEncoding {
    Utf8,
    Utf16Be,
    Utf16Le,
    Utf32Be,
    Utf32Le,
}

impl StandardEncoding {
    #[cfg(target_endian = "little")]
    const UTF_16_NE: Self = Self::Utf16Le;
    #[cfg(target_endian = "big")]
    const UTF_16_NE: Self = Self::Utf16Be;

    #[cfg(target_endian = "little")]
    const UTF_32_NE: Self = Self::Utf32Le;
    #[cfg(target_endian = "big")]
    const UTF_32_NE: Self = Self::Utf32Be;

    fn parse(encoding: &str) -> Option<Self> {
        if let Some(encoding) = encoding.to_lowercase().strip_prefix("utf") {
            let encoding = encoding
                .strip_prefix(|c| ['-', '_'].contains(&c))
                .unwrap_or(encoding);
            if encoding == "8" {
                Some(Self::Utf8)
            } else if let Some(encoding) = encoding.strip_prefix("16") {
                if encoding.is_empty() {
                    return Some(Self::UTF_16_NE);
                }
                let encoding = encoding.strip_prefix(['-', '_']).unwrap_or(encoding);
                match encoding {
                    "be" => Some(Self::Utf16Be),
                    "le" => Some(Self::Utf16Le),
                    _ => None,
                }
            } else if let Some(encoding) = encoding.strip_prefix("32") {
                if encoding.is_empty() {
                    return Some(Self::UTF_32_NE);
                }
                let encoding = encoding.strip_prefix(['-', '_']).unwrap_or(encoding);
                match encoding {
                    "be" => Some(Self::Utf32Be),
                    "le" => Some(Self::Utf32Le),
                    _ => None,
                }
            } else {
                None
            }
        } else if encoding == "cp65001" {
            Some(Self::Utf8)
        } else {
            None
        }
    }
}

struct SurrogatePass;

impl<'a> EncodeErrorHandler<PyEncodeContext<'a>> for SurrogatePass {
    fn handle_encode_error(
        &self,
        ctx: &mut PyEncodeContext<'a>,
        range: Range<StrSize>,
        reason: Option<&str>,
    ) -> PyResult<(EncodeReplace<PyEncodeContext<'a>>, StrSize)> {
        let standard_encoding = StandardEncoding::parse(ctx.encoding)
            .ok_or_else(|| ctx.error_encoding(range.clone(), reason))?;
        let err_str = &ctx.full_data()[range.start.bytes..range.end.bytes];
        let num_chars = range.end.chars - range.start.chars;
        let mut out: Vec<u8> = Vec::with_capacity(num_chars * 4);
        for ch in err_str.code_points() {
            let c = ch.to_u32();
            let 0xd800..=0xdfff = c else {
                // Not a surrogate, fail with original exception
                return Err(ctx.error_encoding(range, reason));
            };
            match standard_encoding {
                StandardEncoding::Utf8 => out.extend(ch.encode_wtf8(&mut [0; 4]).as_bytes()),
                StandardEncoding::Utf16Le => out.extend((c as u16).to_le_bytes()),
                StandardEncoding::Utf16Be => out.extend((c as u16).to_be_bytes()),
                StandardEncoding::Utf32Le => out.extend(c.to_le_bytes()),
                StandardEncoding::Utf32Be => out.extend(c.to_be_bytes()),
            }
        }
        Ok((EncodeReplace::Bytes(ctx.bytes(out)), range.end))
    }
}

impl<'a> DecodeErrorHandler<PyDecodeContext<'a>> for SurrogatePass {
    fn handle_decode_error(
        &self,
        ctx: &mut PyDecodeContext<'a>,
        byte_range: Range<usize>,
        reason: Option<&str>,
    ) -> PyResult<(PyStrRef, usize)> {
        let standard_encoding = StandardEncoding::parse(ctx.encoding)
            .ok_or_else(|| ctx.error_decoding(byte_range.clone(), reason))?;

        let s = ctx.full_data();
        debug_assert!(byte_range.start <= 0.max(s.len() - 1));
        debug_assert!(byte_range.end >= 1.min(s.len()));
        debug_assert!(byte_range.end <= s.len());

        // Try decoding a single surrogate character. If there are more,
        // let the codec call us again.
        let p = &s[byte_range.start..];

        fn slice<const N: usize>(p: &[u8]) -> Option<[u8; N]> {
            p.first_chunk().copied()
        }

        let c = match standard_encoding {
            StandardEncoding::Utf8 => {
                // it's a three-byte code
                slice::<3>(p)
                    .filter(|&[a, b, c]| {
                        (u32::from(a) & 0xf0) == 0xe0
                            && (u32::from(b) & 0xc0) == 0x80
                            && (u32::from(c) & 0xc0) == 0x80
                    })
                    .map(|[a, b, c]| {
                        ((u32::from(a) & 0x0f) << 12)
                            + ((u32::from(b) & 0x3f) << 6)
                            + (u32::from(c) & 0x3f)
                    })
            }
            StandardEncoding::Utf16Le => slice(p).map(u16::from_le_bytes).map(u32::from),
            StandardEncoding::Utf16Be => slice(p).map(u16::from_be_bytes).map(u32::from),
            StandardEncoding::Utf32Le => slice(p).map(u32::from_le_bytes),
            StandardEncoding::Utf32Be => slice(p).map(u32::from_be_bytes),
        };
        let byte_length = match standard_encoding {
            StandardEncoding::Utf8 => 3,
            StandardEncoding::Utf16Be | StandardEncoding::Utf16Le => 2,
            StandardEncoding::Utf32Be | StandardEncoding::Utf32Le => 4,
        };

        // !Py_UNICODE_IS_SURROGATE
        let c = c
            .and_then(CodePoint::from_u32)
            .filter(|c| matches!(c.to_u32(), 0xd800..=0xdfff))
            .ok_or_else(|| ctx.error_decoding(byte_range.clone(), reason))?;

        Ok((ctx.string(c.into()), byte_range.start + byte_length))
    }
}

pub struct PyEncodeContext<'a> {
    vm: &'a VirtualMachine,
    encoding: &'a str,
    data: &'a Py<PyStr>,
    pos: StrSize,
    exception: OnceCell<PyBaseExceptionRef>,
}

impl<'a> PyEncodeContext<'a> {
    pub fn new(encoding: &'a str, data: &'a Py<PyStr>, vm: &'a VirtualMachine) -> Self {
        Self {
            vm,
            encoding,
            data,
            pos: StrSize::default(),
            exception: OnceCell::new(),
        }
    }
}

impl CodecContext for PyEncodeContext<'_> {
    type Error = PyBaseExceptionRef;
    type StrBuf = PyStrRef;
    type BytesBuf = PyBytesRef;

    fn string(&self, s: Wtf8Buf) -> Self::StrBuf {
        self.vm.ctx.new_str(s)
    }

    fn bytes(&self, b: Vec<u8>) -> Self::BytesBuf {
        self.vm.ctx.new_bytes(b)
    }
}
impl EncodeContext for PyEncodeContext<'_> {
    fn full_data(&self) -> &Wtf8 {
        self.data.as_wtf8()
    }

    fn data_len(&self) -> StrSize {
        StrSize {
            bytes: self.data.byte_len(),
            chars: self.data.char_len(),
        }
    }

    fn remaining_data(&self) -> &Wtf8 {
        &self.full_data()[self.pos.bytes..]
    }

    fn position(&self) -> StrSize {
        self.pos
    }

    fn restart_from(&mut self, pos: StrSize) -> Result<(), Self::Error> {
        if pos.chars > self.data.char_len() {
            return Err(self.vm.new_index_error(format!(
                "position {} from error handler out of bounds",
                pos.chars
            )));
        }
        assert!(
            self.data.as_wtf8().is_code_point_boundary(pos.bytes),
            "invalid pos {pos:?} for {:?}",
            self.data.as_wtf8()
        );
        self.pos = pos;
        Ok(())
    }

    fn error_encoding(&self, range: Range<StrSize>, reason: Option<&str>) -> Self::Error {
        let vm = self.vm;
        match self.exception.get() {
            Some(exc) => {
                match update_unicode_error_attrs(
                    exc.as_object(),
                    range.start.chars,
                    range.end.chars,
                    reason,
                    vm,
                ) {
                    Ok(()) => exc.clone(),
                    Err(e) => e,
                }
            }
            None => self
                .exception
                .get_or_init(|| {
                    let reason = reason.expect(
                        "should only ever pass reason: None if an exception is already set",
                    );
                    vm.new_unicode_encode_error_real(
                        vm.ctx.new_str(self.encoding),
                        self.data.to_owned(),
                        range.start.chars,
                        range.end.chars,
                        vm.ctx.new_str(reason),
                    )
                })
                .clone(),
        }
    }
}

pub struct PyDecodeContext<'a> {
    vm: &'a VirtualMachine,
    encoding: &'a str,
    data: PyDecodeData<'a>,
    orig_bytes: Option<&'a Py<PyBytes>>,
    pos: usize,
    exception: OnceCell<PyBaseExceptionRef>,
}
enum PyDecodeData<'a> {
    Original(BorrowedValue<'a, [u8]>),
    Modified(PyBytesRef),
}
impl ops::Deref for PyDecodeData<'_> {
    type Target = [u8];
    fn deref(&self) -> &Self::Target {
        match self {
            PyDecodeData::Original(data) => data,
            PyDecodeData::Modified(data) => data,
        }
    }
}

impl<'a> PyDecodeContext<'a> {
    pub fn new(encoding: &'a str, data: &'a ArgBytesLike, vm: &'a VirtualMachine) -> Self {
        Self {
            vm,
            encoding,
            data: PyDecodeData::Original(data.borrow_buf()),
            orig_bytes: data.as_object().downcast_ref(),
            pos: 0,
            exception: OnceCell::new(),
        }
    }
}

impl CodecContext for PyDecodeContext<'_> {
    type Error = PyBaseExceptionRef;
    type StrBuf = PyStrRef;
    type BytesBuf = PyBytesRef;

    fn string(&self, s: Wtf8Buf) -> Self::StrBuf {
        self.vm.ctx.new_str(s)
    }

    fn bytes(&self, b: Vec<u8>) -> Self::BytesBuf {
        self.vm.ctx.new_bytes(b)
    }
}
impl DecodeContext for PyDecodeContext<'_> {
    fn full_data(&self) -> &[u8] {
        &self.data
    }

    fn remaining_data(&self) -> &[u8] {
        &self.data[self.pos..]
    }

    fn position(&self) -> usize {
        self.pos
    }

    fn advance(&mut self, by: usize) {
        self.pos += by;
    }

    fn restart_from(&mut self, pos: usize) -> Result<(), Self::Error> {
        if pos > self.data.len() {
            return Err(self
                .vm
                .new_index_error(format!("position {pos} from error handler out of bounds",)));
        }
        self.pos = pos;
        Ok(())
    }

    fn error_decoding(&self, byte_range: Range<usize>, reason: Option<&str>) -> Self::Error {
        let vm = self.vm;

        match self.exception.get() {
            Some(exc) => {
                match update_unicode_error_attrs(
                    exc.as_object(),
                    byte_range.start,
                    byte_range.end,
                    reason,
                    vm,
                ) {
                    Ok(()) => exc.clone(),
                    Err(e) => e,
                }
            }
            None => self
                .exception
                .get_or_init(|| {
                    let reason = reason.expect(
                        "should only ever pass reason: None if an exception is already set",
                    );
                    let data = if let Some(bytes) = self.orig_bytes {
                        bytes.to_owned()
                    } else {
                        vm.ctx.new_bytes(self.data.to_vec())
                    };
                    vm.new_unicode_decode_error_real(
                        vm.ctx.new_str(self.encoding),
                        data,
                        byte_range.start,
                        byte_range.end,
                        vm.ctx.new_str(reason),
                    )
                })
                .clone(),
        }
    }
}

#[derive(strum_macros::EnumString)]
#[strum(serialize_all = "lowercase")]
enum StandardError {
    Strict,
    Ignore,
    Replace,
    XmlCharRefReplace,
    BackslashReplace,
    SurrogatePass,
    SurrogateEscape,
}

impl<'a> EncodeErrorHandler<PyEncodeContext<'a>> for StandardError {
    fn handle_encode_error(
        &self,
        ctx: &mut PyEncodeContext<'a>,
        range: Range<StrSize>,
        reason: Option<&str>,
    ) -> PyResult<(EncodeReplace<PyEncodeContext<'a>>, StrSize)> {
        use StandardError::*;
        // use errors::*;
        match self {
            Strict => errors::Strict.handle_encode_error(ctx, range, reason),
            Ignore => errors::Ignore.handle_encode_error(ctx, range, reason),
            Replace => errors::Replace.handle_encode_error(ctx, range, reason),
            XmlCharRefReplace => errors::XmlCharRefReplace.handle_encode_error(ctx, range, reason),
            BackslashReplace => errors::BackslashReplace.handle_encode_error(ctx, range, reason),
            SurrogatePass => SurrogatePass.handle_encode_error(ctx, range, reason),
            SurrogateEscape => errors::SurrogateEscape.handle_encode_error(ctx, range, reason),
        }
    }
}

impl<'a> DecodeErrorHandler<PyDecodeContext<'a>> for StandardError {
    fn handle_decode_error(
        &self,
        ctx: &mut PyDecodeContext<'a>,
        byte_range: Range<usize>,
        reason: Option<&str>,
    ) -> PyResult<(PyStrRef, usize)> {
        use StandardError::*;
        match self {
            Strict => errors::Strict.handle_decode_error(ctx, byte_range, reason),
            Ignore => errors::Ignore.handle_decode_error(ctx, byte_range, reason),
            Replace => errors::Replace.handle_decode_error(ctx, byte_range, reason),
            XmlCharRefReplace => Err(ctx
                .vm
                .new_type_error("don't know how to handle UnicodeDecodeError in error callback")),
            BackslashReplace => {
                errors::BackslashReplace.handle_decode_error(ctx, byte_range, reason)
            }
            SurrogatePass => self::SurrogatePass.handle_decode_error(ctx, byte_range, reason),
            SurrogateEscape => errors::SurrogateEscape.handle_decode_error(ctx, byte_range, reason),
        }
    }
}

pub struct ErrorsHandler<'a> {
    errors: &'a Py<PyStr>,
    resolved: OnceCell<ResolvedError>,
}
enum ResolvedError {
    Standard(StandardError),
    Handler(PyObjectRef),
}

impl<'a> ErrorsHandler<'a> {
    #[inline]
    pub fn new(errors: Option<&'a Py<PyStr>>, vm: &VirtualMachine) -> Self {
        match errors {
            Some(errors) => Self {
                errors,
                resolved: OnceCell::new(),
            },
            None => Self {
                errors: identifier!(vm, strict).as_ref(),
                resolved: OnceCell::from(ResolvedError::Standard(StandardError::Strict)),
            },
        }
    }
    #[inline]
    fn resolve(&self, vm: &VirtualMachine) -> PyResult<&ResolvedError> {
        if let Some(val) = self.resolved.get() {
            return Ok(val);
        }
        let val = if let Ok(standard) = self.errors.as_str().parse() {
            ResolvedError::Standard(standard)
        } else {
            vm.state
                .codec_registry
                .lookup_error(self.errors.as_str(), vm)
                .map(ResolvedError::Handler)?
        };
        let _ = self.resolved.set(val);
        Ok(self.resolved.get().unwrap())
    }
}
impl StrBuffer for PyStrRef {
    fn is_compatible_with(&self, kind: StrKind) -> bool {
        self.kind() <= kind
    }
}
impl<'a> EncodeErrorHandler<PyEncodeContext<'a>> for ErrorsHandler<'_> {
    fn handle_encode_error(
        &self,
        ctx: &mut PyEncodeContext<'a>,
        range: Range<StrSize>,
        reason: Option<&str>,
    ) -> PyResult<(EncodeReplace<PyEncodeContext<'a>>, StrSize)> {
        let vm = ctx.vm;
        let handler = match self.resolve(vm)? {
            ResolvedError::Standard(standard) => {
                return standard.handle_encode_error(ctx, range, reason);
            }
            ResolvedError::Handler(handler) => handler,
        };
        let encode_exc = ctx.error_encoding(range.clone(), reason);
        let res = handler.call((encode_exc.clone(),), vm)?;
        let tuple_err =
            || vm.new_type_error("encoding error handler must return (str/bytes, int) tuple");
        let (replace, restart) = match res.downcast_ref::<PyTuple>().map(|tup| tup.as_slice()) {
            Some([replace, restart]) => (replace.clone(), restart),
            _ => return Err(tuple_err()),
        };
        let replace = match_class!(match replace {
            s @ PyStr => EncodeReplace::Str(s),
            b @ PyBytes => EncodeReplace::Bytes(b),
            _ => return Err(tuple_err()),
        });
        let restart = isize::try_from_borrowed_object(vm, restart).map_err(|_| tuple_err())?;
        let restart = if restart < 0 {
            // will still be out of bounds if it underflows ¯\_(ツ)_/¯
            ctx.data.char_len().wrapping_sub(restart.unsigned_abs())
        } else {
            restart as usize
        };
        let restart = if restart == range.end.chars {
            range.end
        } else {
            StrSize {
                chars: restart,
                bytes: ctx
                    .data
                    .as_wtf8()
                    .code_point_indices()
                    .nth(restart)
                    .map_or(ctx.data.byte_len(), |(i, _)| i),
            }
        };
        Ok((replace, restart))
    }
}
impl<'a> DecodeErrorHandler<PyDecodeContext<'a>> for ErrorsHandler<'_> {
    fn handle_decode_error(
        &self,
        ctx: &mut PyDecodeContext<'a>,
        byte_range: Range<usize>,
        reason: Option<&str>,
    ) -> PyResult<(PyStrRef, usize)> {
        let vm = ctx.vm;
        let handler = match self.resolve(vm)? {
            ResolvedError::Standard(standard) => {
                return standard.handle_decode_error(ctx, byte_range, reason);
            }
            ResolvedError::Handler(handler) => handler,
        };
        let decode_exc = ctx.error_decoding(byte_range.clone(), reason);
        let data_bytes: PyObjectRef = decode_exc.as_object().get_attr("object", vm)?;
        let res = handler.call((decode_exc.clone(),), vm)?;
        let new_data = decode_exc.as_object().get_attr("object", vm)?;
        if !new_data.is(&data_bytes) {
            let new_data: PyBytesRef = new_data
                .downcast()
                .map_err(|_| vm.new_type_error("object attribute must be bytes"))?;
            ctx.data = PyDecodeData::Modified(new_data);
        }
        let data = &*ctx.data;
        let tuple_err = || vm.new_type_error("decoding error handler must return (str, int) tuple");
        match res.downcast_ref::<PyTuple>().map(|tup| tup.as_slice()) {
            Some([replace, restart]) => {
                let replace = replace
                    .downcast_ref::<PyStr>()
                    .ok_or_else(tuple_err)?
                    .to_owned();
                let restart =
                    isize::try_from_borrowed_object(vm, restart).map_err(|_| tuple_err())?;
                let restart = if restart < 0 {
                    // will still be out of bounds if it underflows ¯\_(ツ)_/¯
                    data.len().wrapping_sub(restart.unsigned_abs())
                } else {
                    restart as usize
                };
                Ok((replace, restart))
            }
            _ => Err(tuple_err()),
        }
    }
}

fn call_native_encode_error<E>(
    handler: E,
    err: PyObjectRef,
    vm: &VirtualMachine,
) -> PyResult<(PyObjectRef, usize)>
where
    for<'a> E: EncodeErrorHandler<PyEncodeContext<'a>>,
{
    // let err = err.
    let range = extract_unicode_error_range(&err, vm)?;
    let s = PyStrRef::try_from_object(vm, err.get_attr("object", vm)?)?;
    let s_encoding = PyStrRef::try_from_object(vm, err.get_attr("encoding", vm)?)?;
    let mut ctx = PyEncodeContext {
        vm,
        encoding: s_encoding.as_str(),
        data: &s,
        pos: StrSize::default(),
        exception: OnceCell::from(err.downcast().unwrap()),
    };
    let mut iter = s.as_wtf8().code_point_indices();
    let start = StrSize {
        chars: range.start,
        bytes: iter.nth(range.start).unwrap().0,
    };
    let end = StrSize {
        chars: range.end,
        bytes: if let Some(n) = range.len().checked_sub(1) {
            iter.nth(n).map_or(s.byte_len(), |(i, _)| i)
        } else {
            start.bytes
        },
    };
    let (replace, restart) = handler.handle_encode_error(&mut ctx, start..end, None)?;
    let replace = match replace {
        EncodeReplace::Str(s) => s.into(),
        EncodeReplace::Bytes(b) => b.into(),
    };
    Ok((replace, restart.chars))
}

fn call_native_decode_error<E>(
    handler: E,
    err: PyObjectRef,
    vm: &VirtualMachine,
) -> PyResult<(PyObjectRef, usize)>
where
    for<'a> E: DecodeErrorHandler<PyDecodeContext<'a>>,
{
    let range = extract_unicode_error_range(&err, vm)?;
    let s = ArgBytesLike::try_from_object(vm, err.get_attr("object", vm)?)?;
    let s_encoding = PyStrRef::try_from_object(vm, err.get_attr("encoding", vm)?)?;
    let mut ctx = PyDecodeContext {
        vm,
        encoding: s_encoding.as_str(),
        data: PyDecodeData::Original(s.borrow_buf()),
        orig_bytes: s.as_object().downcast_ref(),
        pos: 0,
        exception: OnceCell::from(err.downcast().unwrap()),
    };
    let (replace, restart) = handler.handle_decode_error(&mut ctx, range, None)?;
    Ok((replace.into(), restart))
}

// this is a hack, for now
fn call_native_translate_error<E>(
    handler: E,
    err: PyObjectRef,
    vm: &VirtualMachine,
) -> PyResult<(PyObjectRef, usize)>
where
    for<'a> E: EncodeErrorHandler<PyEncodeContext<'a>>,
{
    // let err = err.
    let range = extract_unicode_error_range(&err, vm)?;
    let s = PyStrRef::try_from_object(vm, err.get_attr("object", vm)?)?;
    let mut ctx = PyEncodeContext {
        vm,
        encoding: "",
        data: &s,
        pos: StrSize::default(),
        exception: OnceCell::from(err.downcast().unwrap()),
    };
    let mut iter = s.as_wtf8().code_point_indices();
    let start = StrSize {
        chars: range.start,
        bytes: iter.nth(range.start).unwrap().0,
    };
    let end = StrSize {
        chars: range.end,
        bytes: if let Some(n) = range.len().checked_sub(1) {
            iter.nth(n).map_or(s.byte_len(), |(i, _)| i)
        } else {
            start.bytes
        },
    };
    let (replace, restart) = handler.handle_encode_error(&mut ctx, start..end, None)?;
    let replace = match replace {
        EncodeReplace::Str(s) => s.into(),
        EncodeReplace::Bytes(b) => b.into(),
    };
    Ok((replace, restart.chars))
}

// TODO: exceptions with custom payloads
fn extract_unicode_error_range(err: &PyObject, vm: &VirtualMachine) -> PyResult<Range<usize>> {
    let start = err.get_attr("start", vm)?;
    let start = start.try_into_value(vm)?;
    let end = err.get_attr("end", vm)?;
    let end = end.try_into_value(vm)?;
    Ok(Range { start, end })
}

fn update_unicode_error_attrs(
    err: &PyObject,
    start: usize,
    end: usize,
    reason: Option<&str>,
    vm: &VirtualMachine,
) -> PyResult<()> {
    err.set_attr("start", start.to_pyobject(vm), vm)?;
    err.set_attr("end", end.to_pyobject(vm), vm)?;
    if let Some(reason) = reason {
        err.set_attr("reason", reason.to_pyobject(vm), vm)?;
    }
    Ok(())
}

#[inline]
fn is_encode_err(err: &PyObject, vm: &VirtualMachine) -> bool {
    err.fast_isinstance(vm.ctx.exceptions.unicode_encode_error)
}
#[inline]
fn is_decode_err(err: &PyObject, vm: &VirtualMachine) -> bool {
    err.fast_isinstance(vm.ctx.exceptions.unicode_decode_error)
}
#[inline]
fn is_translate_err(err: &PyObject, vm: &VirtualMachine) -> bool {
    err.fast_isinstance(vm.ctx.exceptions.unicode_translate_error)
}

fn bad_err_type(err: PyObjectRef, vm: &VirtualMachine) -> PyBaseExceptionRef {
    vm.new_type_error(format!(
        "don't know how to handle {} in error callback",
        err.class().name()
    ))
}

fn strict_errors(err: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    let err = err
        .downcast()
        .unwrap_or_else(|_| vm.new_type_error("codec must pass exception instance"));
    Err(err)
}

fn ignore_errors(err: PyObjectRef, vm: &VirtualMachine) -> PyResult<(PyObjectRef, usize)> {
    if is_encode_err(&err, vm) || is_decode_err(&err, vm) || is_translate_err(&err, vm) {
        let range = extract_unicode_error_range(&err, vm)?;
        Ok((vm.ctx.new_str(ascii!("")).into(), range.end))
    } else {
        Err(bad_err_type(err, vm))
    }
}

fn replace_errors(err: PyObjectRef, vm: &VirtualMachine) -> PyResult<(PyObjectRef, usize)> {
    if is_encode_err(&err, vm) {
        call_native_encode_error(errors::Replace, err, vm)
    } else if is_decode_err(&err, vm) {
        call_native_decode_error(errors::Replace, err, vm)
    } else if is_translate_err(&err, vm) {
        // char::REPLACEMENT_CHARACTER as a str
        let replacement_char = "\u{FFFD}";
        let range = extract_unicode_error_range(&err, vm)?;
        let replace = replacement_char.repeat(range.end - range.start);
        Ok((replace.to_pyobject(vm), range.end))
    } else {
        Err(bad_err_type(err, vm))
    }
}

fn xmlcharrefreplace_errors(
    err: PyObjectRef,
    vm: &VirtualMachine,
) -> PyResult<(PyObjectRef, usize)> {
    if is_encode_err(&err, vm) {
        call_native_encode_error(errors::XmlCharRefReplace, err, vm)
    } else {
        Err(bad_err_type(err, vm))
    }
}

fn backslashreplace_errors(
    err: PyObjectRef,
    vm: &VirtualMachine,
) -> PyResult<(PyObjectRef, usize)> {
    if is_decode_err(&err, vm) {
        call_native_decode_error(errors::BackslashReplace, err, vm)
    } else if is_encode_err(&err, vm) {
        call_native_encode_error(errors::BackslashReplace, err, vm)
    } else if is_translate_err(&err, vm) {
        call_native_translate_error(errors::BackslashReplace, err, vm)
    } else {
        Err(bad_err_type(err, vm))
    }
}

fn namereplace_errors(err: PyObjectRef, vm: &VirtualMachine) -> PyResult<(PyObjectRef, usize)> {
    if is_encode_err(&err, vm) {
        call_native_encode_error(errors::NameReplace, err, vm)
    } else {
        Err(bad_err_type(err, vm))
    }
}

fn surrogatepass_errors(err: PyObjectRef, vm: &VirtualMachine) -> PyResult<(PyObjectRef, usize)> {
    if is_encode_err(&err, vm) {
        call_native_encode_error(SurrogatePass, err, vm)
    } else if is_decode_err(&err, vm) {
        call_native_decode_error(SurrogatePass, err, vm)
    } else {
        Err(bad_err_type(err, vm))
    }
}

fn surrogateescape_errors(err: PyObjectRef, vm: &VirtualMachine) -> PyResult<(PyObjectRef, usize)> {
    if is_encode_err(&err, vm) {
        call_native_encode_error(errors::SurrogateEscape, err, vm)
    } else if is_decode_err(&err, vm) {
        call_native_decode_error(errors::SurrogateEscape, err, vm)
    } else {
        Err(bad_err_type(err, vm))
    }
}

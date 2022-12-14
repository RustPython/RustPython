use crate::{
    builtins::{PyBaseExceptionRef, PyBytesRef, PyStr, PyStrRef, PyTuple, PyTupleRef},
    common::{ascii, lock::PyRwLock},
    convert::ToPyObject,
    AsObject, Context, PyObject, PyObjectRef, PyPayload, PyResult, TryFromObject, VirtualMachine,
};
use std::{borrow::Cow, collections::HashMap, fmt::Write, ops::Range};

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
            Ok(PyCodec(tuple))
        } else {
            Err(tuple)
        }
    }
    #[inline]
    pub fn into_tuple(self) -> PyTupleRef {
        self.0
    }
    #[inline]
    pub fn as_tuple(&self) -> &PyTupleRef {
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
        let res = vm.invoke(self.get_encode_func(), args)?;
        let res = res
            .downcast::<PyTuple>()
            .ok()
            .filter(|tuple| tuple.len() == 2)
            .ok_or_else(|| {
                vm.new_type_error("encoder must return a tuple (object, integer)".to_owned())
            })?;
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
        let res = vm.invoke(self.get_decode_func(), args)?;
        let res = res
            .downcast::<PyTuple>()
            .ok()
            .filter(|tuple| tuple.len() == 2)
            .ok_or_else(|| {
                vm.new_type_error("decoder must return a tuple (object,integer)".to_owned())
            })?;
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
            .and_then(|tuple| PyCodec::from_tuple(tuple).ok())
            .ok_or_else(|| {
                vm.new_type_error("codec search functions must return 4-tuples".to_owned())
            })
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
        let errors = [
            ("strict", ctx.new_function("strict_errors", strict_errors)),
            ("ignore", ctx.new_function("ignore_errors", ignore_errors)),
            (
                "replace",
                ctx.new_function("replace_errors", replace_errors),
            ),
            (
                "xmlcharrefreplace",
                ctx.new_function("xmlcharrefreplace_errors", xmlcharrefreplace_errors),
            ),
            (
                "backslashreplace",
                ctx.new_function("backslashreplace_errors", backslashreplace_errors),
            ),
            (
                "namereplace",
                ctx.new_function("namereplace_errors", namereplace_errors),
            ),
            (
                "surrogatepass",
                ctx.new_function("surrogatepass_errors", surrogatepass_errors),
            ),
            (
                "surrogateescape",
                ctx.new_function("surrogateescape_errors", surrogateescape_errors),
            ),
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
        CodecsRegistry {
            inner: PyRwLock::new(inner),
        }
    }

    pub fn register(&self, search_function: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        if !vm.is_callable(&search_function) {
            return Err(vm.new_type_error("argument must be callable".to_owned()));
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
        self.inner
            .write()
            .search_cache
            .insert(name.to_owned(), codec);
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
        let encoding = PyStr::from(encoding.into_owned()).into_ref(vm);
        for func in search_path {
            let res = vm.invoke(&func, (encoding.clone(),))?;
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
        codec.encode(obj, errors, vm)
    }

    pub fn decode(
        &self,
        obj: PyObjectRef,
        encoding: &str,
        errors: Option<PyStrRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let codec = self.lookup(encoding, vm)?;
        codec.decode(obj, errors, vm)
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
            .encode(obj.into(), errors, vm)?
            .downcast()
            .map_err(|obj| {
                vm.new_type_error(format!(
                    "'{}' encoder returned '{}' instead of 'bytes'; use codecs.encode() to \
                     encode arbitrary types",
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
        codec.decode(obj, errors, vm)?.downcast().map_err(|obj| {
            vm.new_type_error(format!(
                "'{}' decoder returned '{}' instead of 'str'; use codecs.decode() \
                 to encode arbitrary types",
                encoding,
                obj.class().name(),
            ))
        })
    }

    pub fn register_error(&self, name: String, handler: PyObjectRef) -> Option<PyObjectRef> {
        self.inner.write().errors.insert(name, handler)
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
    if let Some(i) = encoding.find(|c: char| c == ' ' || c.is_ascii_uppercase()) {
        let mut out = encoding.as_bytes().to_owned();
        for byte in &mut out[i..] {
            if *byte == b' ' {
                *byte = b'-';
            } else {
                byte.make_ascii_lowercase();
            }
        }
        String::from_utf8(out).unwrap().into()
    } else {
        encoding.into()
    }
}

// TODO: exceptions with custom payloads
fn extract_unicode_error_range(err: &PyObject, vm: &VirtualMachine) -> PyResult<Range<usize>> {
    let start = err.to_owned().get_attr("start", vm)?;
    let start = start.try_into_value(vm)?;
    let end = err.to_owned().get_attr("end", vm)?;
    let end = end.try_into_value(vm)?;
    Ok(Range { start, end })
}

#[inline]
fn is_decode_err(err: &PyObject, vm: &VirtualMachine) -> bool {
    err.fast_isinstance(vm.ctx.exceptions.unicode_decode_error)
}
#[inline]
fn is_encode_ish_err(err: &PyObject, vm: &VirtualMachine) -> bool {
    err.fast_isinstance(vm.ctx.exceptions.unicode_encode_error)
        || err.fast_isinstance(vm.ctx.exceptions.unicode_translate_error)
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
        .unwrap_or_else(|_| vm.new_type_error("codec must pass exception instance".to_owned()));
    Err(err)
}

fn ignore_errors(err: PyObjectRef, vm: &VirtualMachine) -> PyResult<(PyObjectRef, usize)> {
    if is_encode_ish_err(&err, vm) || is_decode_err(&err, vm) {
        let range = extract_unicode_error_range(&err, vm)?;
        Ok((vm.ctx.new_str(ascii!("")).into(), range.end))
    } else {
        Err(bad_err_type(err, vm))
    }
}

fn replace_errors(err: PyObjectRef, vm: &VirtualMachine) -> PyResult<(String, usize)> {
    // char::REPLACEMENT_CHARACTER as a str
    let replacement_char = "\u{FFFD}";
    let replace = if err.fast_isinstance(vm.ctx.exceptions.unicode_encode_error) {
        "?"
    } else if err.fast_isinstance(vm.ctx.exceptions.unicode_decode_error) {
        let range = extract_unicode_error_range(&err, vm)?;
        return Ok((replacement_char.to_owned(), range.end));
    } else if err.fast_isinstance(vm.ctx.exceptions.unicode_translate_error) {
        replacement_char
    } else {
        return Err(bad_err_type(err, vm));
    };
    let range = extract_unicode_error_range(&err, vm)?;
    let replace = replace.repeat(range.end - range.start);
    Ok((replace, range.end))
}

fn xmlcharrefreplace_errors(err: PyObjectRef, vm: &VirtualMachine) -> PyResult<(String, usize)> {
    if !is_encode_ish_err(&err, vm) {
        return Err(bad_err_type(err, vm));
    }
    let range = extract_unicode_error_range(&err, vm)?;
    let s = PyStrRef::try_from_object(vm, err.get_attr("object", vm)?)?;
    let s_after_start = crate::common::str::try_get_chars(s.as_str(), range.start..).unwrap_or("");
    let num_chars = range.len();
    // capacity rough guess; assuming that the codepoints are 3 digits in decimal + the &#;
    let mut out = String::with_capacity(num_chars * 6);
    for c in s_after_start.chars().take(num_chars) {
        write!(out, "&#{};", c as u32).unwrap()
    }
    Ok((out, range.end))
}

fn backslashreplace_errors(err: PyObjectRef, vm: &VirtualMachine) -> PyResult<(String, usize)> {
    if is_decode_err(&err, vm) {
        let range = extract_unicode_error_range(&err, vm)?;
        let b = PyBytesRef::try_from_object(vm, err.get_attr("object", vm)?)?;
        let mut replace = String::with_capacity(4 * range.len());
        for &c in &b[range.clone()] {
            write!(replace, "\\x{c:02x}").unwrap();
        }
        return Ok((replace, range.end));
    } else if !is_encode_ish_err(&err, vm) {
        return Err(bad_err_type(err, vm));
    }
    let range = extract_unicode_error_range(&err, vm)?;
    let s = PyStrRef::try_from_object(vm, err.get_attr("object", vm)?)?;
    let s_after_start = crate::common::str::try_get_chars(s.as_str(), range.start..).unwrap_or("");
    let num_chars = range.len();
    // minimum 4 output bytes per char: \xNN
    let mut out = String::with_capacity(num_chars * 4);
    for c in s_after_start.chars().take(num_chars) {
        let c = c as u32;
        if c >= 0x10000 {
            write!(out, "\\U{c:08x}").unwrap();
        } else if c >= 0x100 {
            write!(out, "\\u{c:04x}").unwrap();
        } else {
            write!(out, "\\x{c:02x}").unwrap();
        }
    }
    Ok((out, range.end))
}

fn namereplace_errors(err: PyObjectRef, vm: &VirtualMachine) -> PyResult<(String, usize)> {
    if err.fast_isinstance(vm.ctx.exceptions.unicode_encode_error) {
        let range = extract_unicode_error_range(&err, vm)?;
        let s = PyStrRef::try_from_object(vm, err.get_attr("object", vm)?)?;
        let s_after_start =
            crate::common::str::try_get_chars(s.as_str(), range.start..).unwrap_or("");
        let num_chars = range.len();
        let mut out = String::with_capacity(num_chars * 4);
        for c in s_after_start.chars().take(num_chars) {
            let c_u32 = c as u32;
            if let Some(c_name) = unicode_names2::name(c) {
                write!(out, "\\N{{{c_name}}}").unwrap();
            } else if c_u32 >= 0x10000 {
                write!(out, "\\U{c_u32:08x}").unwrap();
            } else if c_u32 >= 0x100 {
                write!(out, "\\u{c_u32:04x}").unwrap();
            } else {
                write!(out, "\\x{c_u32:02x}").unwrap();
            }
        }
        Ok((out, range.end))
    } else {
        Err(bad_err_type(err, vm))
    }
}

#[derive(Eq, PartialEq)]
enum StandardEncoding {
    Utf8,
    Utf16Be,
    Utf16Le,
    Utf32Be,
    Utf32Le,
    Unknown,
}

fn get_standard_encoding(encoding: &str) -> (usize, StandardEncoding) {
    if let Some(encoding) = encoding.to_lowercase().strip_prefix("utf") {
        let mut byte_length: usize = 0;
        let mut standard_encoding = StandardEncoding::Unknown;
        let encoding = encoding
            .strip_prefix(|c| ['-', '_'].contains(&c))
            .unwrap_or(encoding);
        if encoding == "8" {
            byte_length = 3;
            standard_encoding = StandardEncoding::Utf8;
        } else if let Some(encoding) = encoding.strip_prefix("16") {
            byte_length = 2;
            if encoding.is_empty() {
                if cfg!(target_endian = "little") {
                    standard_encoding = StandardEncoding::Utf16Le;
                } else if cfg!(target_endian = "big") {
                    standard_encoding = StandardEncoding::Utf16Be;
                }
                if standard_encoding != StandardEncoding::Unknown {
                    return (byte_length, standard_encoding);
                }
            }
            let encoding = encoding
                .strip_prefix(|c| ['-', '_'].contains(&c))
                .unwrap_or(encoding);
            standard_encoding = match encoding {
                "be" => StandardEncoding::Utf16Be,
                "le" => StandardEncoding::Utf16Le,
                _ => StandardEncoding::Unknown,
            }
        } else if let Some(encoding) = encoding.strip_prefix("32") {
            byte_length = 4;
            if encoding.is_empty() {
                if cfg!(target_endian = "little") {
                    standard_encoding = StandardEncoding::Utf32Le;
                } else if cfg!(target_endian = "big") {
                    standard_encoding = StandardEncoding::Utf32Be;
                }
                if standard_encoding != StandardEncoding::Unknown {
                    return (byte_length, standard_encoding);
                }
            }
            let encoding = encoding
                .strip_prefix(|c| ['-', '_'].contains(&c))
                .unwrap_or(encoding);
            standard_encoding = match encoding {
                "be" => StandardEncoding::Utf32Be,
                "le" => StandardEncoding::Utf32Le,
                _ => StandardEncoding::Unknown,
            }
        }
        return (byte_length, standard_encoding);
    } else if encoding == "CP_UTF8" {
        return (3, StandardEncoding::Utf8);
    }
    (0, StandardEncoding::Unknown)
}

fn surrogatepass_errors(err: PyObjectRef, vm: &VirtualMachine) -> PyResult<(PyObjectRef, usize)> {
    if err.fast_isinstance(vm.ctx.exceptions.unicode_encode_error) {
        let range = extract_unicode_error_range(&err, vm)?;
        let s = PyStrRef::try_from_object(vm, err.get_attr("object", vm)?)?;
        let s_encoding = PyStrRef::try_from_object(vm, err.get_attr("encoding", vm)?)?;
        let (_, standard_encoding) = get_standard_encoding(s_encoding.as_str());
        if let StandardEncoding::Unknown = standard_encoding {
            // Not supported, fail with original exception
            return Err(err.downcast().unwrap());
        }
        let s_after_start =
            crate::common::str::try_get_chars(s.as_str(), range.start..).unwrap_or("");
        let num_chars = range.len();
        let mut out: Vec<u8> = Vec::with_capacity(num_chars * 4);
        for c in s_after_start.chars().take(num_chars).map(|x| x as u32) {
            if !(0xd800..=0xdfff).contains(&c) {
                // Not a surrogate, fail with original exception
                return Err(err.downcast().unwrap());
            }
            match standard_encoding {
                StandardEncoding::Utf8 => {
                    out.push((0xe0 | (c >> 12)) as u8);
                    out.push((0x80 | ((c >> 6) & 0x3f)) as u8);
                    out.push((0x80 | (c & 0x3f)) as u8);
                }
                StandardEncoding::Utf16Le => {
                    out.push(c as u8);
                    out.push((c >> 8) as u8);
                }
                StandardEncoding::Utf16Be => {
                    out.push((c >> 8) as u8);
                    out.push(c as u8);
                }
                StandardEncoding::Utf32Le => {
                    out.push(c as u8);
                    out.push((c >> 8) as u8);
                    out.push((c >> 16) as u8);
                    out.push((c >> 24) as u8);
                }
                StandardEncoding::Utf32Be => {
                    out.push((c >> 24) as u8);
                    out.push((c >> 16) as u8);
                    out.push((c >> 8) as u8);
                    out.push(c as u8);
                }
                StandardEncoding::Unknown => {
                    unreachable!("NOTE: RUSTPYTHON, should've bailed out earlier")
                }
            }
        }
        Ok((vm.ctx.new_bytes(out).into(), range.end))
    } else if is_decode_err(&err, vm) {
        let range = extract_unicode_error_range(&err, vm)?;
        let s = PyBytesRef::try_from_object(vm, err.get_attr("object", vm)?)?;
        let s_encoding = PyStrRef::try_from_object(vm, err.get_attr("encoding", vm)?)?;
        let (byte_length, standard_encoding) = get_standard_encoding(s_encoding.as_str());
        if let StandardEncoding::Unknown = standard_encoding {
            // Not supported, fail with original exception
            return Err(err.downcast().unwrap());
        }
        let mut c: u32 = 0;
        // Try decoding a single surrogate character. If there are more,
        // let the codec call us again.
        let p = &s.as_bytes()[range.start..];
        if p.len() - range.start >= byte_length {
            match standard_encoding {
                StandardEncoding::Utf8 => {
                    if (p[0] as u32 & 0xf0) == 0xe0
                        && (p[1] as u32 & 0xc0) == 0x80
                        && (p[2] as u32 & 0xc0) == 0x80
                    {
                        // it's a three-byte code
                        c = ((p[0] as u32 & 0x0f) << 12)
                            + ((p[1] as u32 & 0x3f) << 6)
                            + (p[2] as u32 & 0x3f);
                    }
                }
                StandardEncoding::Utf16Le => {
                    c = (p[1] as u32) << 8 | p[0] as u32;
                }
                StandardEncoding::Utf16Be => {
                    c = (p[0] as u32) << 8 | p[1] as u32;
                }
                StandardEncoding::Utf32Le => {
                    c = ((p[3] as u32) << 24)
                        | ((p[2] as u32) << 16)
                        | ((p[1] as u32) << 8)
                        | p[0] as u32;
                }
                StandardEncoding::Utf32Be => {
                    c = ((p[0] as u32) << 24)
                        | ((p[1] as u32) << 16)
                        | ((p[2] as u32) << 8)
                        | p[3] as u32;
                }
                StandardEncoding::Unknown => {
                    unreachable!("NOTE: RUSTPYTHON, should've bailed out earlier")
                }
            }
        }
        // !Py_UNICODE_IS_SURROGATE
        if !(0xd800..=0xdfff).contains(&c) {
            // Not a surrogate, fail with original exception
            return Err(err.downcast().unwrap());
        }

        Ok((
            vm.new_pyobj(format!("\\x{c:x?}")),
            range.start + byte_length,
        ))
    } else {
        Err(bad_err_type(err, vm))
    }
}

fn surrogateescape_errors(err: PyObjectRef, vm: &VirtualMachine) -> PyResult<(PyObjectRef, usize)> {
    if err.fast_isinstance(vm.ctx.exceptions.unicode_encode_error) {
        let range = extract_unicode_error_range(&err, vm)?;
        let object = PyStrRef::try_from_object(vm, err.get_attr("object", vm)?)?;
        let s_after_start =
            crate::common::str::try_get_chars(object.as_str(), range.start..).unwrap_or("");
        let mut out: Vec<u8> = Vec::with_capacity(range.len());
        for ch in s_after_start.chars().take(range.len()) {
            let ch = ch as u32;
            if !(0xdc80..=0xdcff).contains(&ch) {
                // Not a UTF-8b surrogate, fail with original exception
                return Err(err.downcast().unwrap());
            }
            out.push((ch - 0xdc00) as u8);
        }
        let out = vm.ctx.new_bytes(out);
        Ok((out.into(), range.end))
    } else if is_decode_err(&err, vm) {
        let range = extract_unicode_error_range(&err, vm)?;
        let object = err.get_attr("object", vm)?;
        let object = PyBytesRef::try_from_object(vm, object)?;
        let p = &object.as_bytes()[range.clone()];
        let mut consumed = 0;
        let mut replace = String::with_capacity(4 * range.len());
        while consumed < 4 && consumed < range.len() {
            let c = p[consumed] as u32;
            // Refuse to escape ASCII bytes
            if c < 128 {
                break;
            }
            write!(replace, "#{}", 0xdc00 + c).unwrap();
            consumed += 1;
        }
        if consumed == 0 {
            return Err(err.downcast().unwrap());
        }
        Ok((vm.new_pyobj(replace), range.start + consumed))
    } else {
        Err(bad_err_type(err, vm))
    }
}

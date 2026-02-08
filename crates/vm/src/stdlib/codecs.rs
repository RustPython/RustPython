// spell-checker: ignore unencodable pused

pub(crate) use _codecs::module_def;

use crate::common::static_cell::StaticCell;

#[pymodule(with(#[cfg(windows)] _codecs_windows))]
mod _codecs {
    use crate::codecs::{ErrorsHandler, PyDecodeContext, PyEncodeContext};
    use crate::common::encodings;
    use crate::common::wtf8::Wtf8Buf;
    use crate::{
        AsObject, PyObjectRef, PyResult, VirtualMachine,
        builtins::{PyStrRef, PyUtf8StrRef},
        codecs,
        exceptions::cstring_error,
        function::{ArgBytesLike, FuncArgs},
    };

    #[pyfunction]
    fn register(search_function: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        vm.state.codec_registry.register(search_function, vm)
    }

    #[pyfunction]
    fn unregister(search_function: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        vm.state.codec_registry.unregister(search_function)
    }

    #[pyfunction]
    fn lookup(encoding: PyUtf8StrRef, vm: &VirtualMachine) -> PyResult {
        if encoding.as_str().contains('\0') {
            return Err(cstring_error(vm));
        }
        vm.state
            .codec_registry
            .lookup(encoding.as_str(), vm)
            .map(|codec| codec.into_tuple().into())
    }

    #[derive(FromArgs)]
    struct CodeArgs {
        obj: PyObjectRef,
        #[pyarg(any, optional)]
        encoding: Option<PyStrRef>,
        #[pyarg(any, optional)]
        errors: Option<PyStrRef>,
    }

    #[pyfunction]
    fn encode(args: CodeArgs, vm: &VirtualMachine) -> PyResult {
        let encoding = args
            .encoding
            .as_ref()
            .map_or(codecs::DEFAULT_ENCODING, |s| s.as_str());
        vm.state
            .codec_registry
            .encode(args.obj, encoding, args.errors, vm)
    }

    #[pyfunction]
    fn decode(args: CodeArgs, vm: &VirtualMachine) -> PyResult {
        let encoding = args
            .encoding
            .as_ref()
            .map_or(codecs::DEFAULT_ENCODING, |s| s.as_str());
        vm.state
            .codec_registry
            .decode(args.obj, encoding, args.errors, vm)
    }

    #[pyfunction]
    fn _forget_codec(encoding: PyStrRef, vm: &VirtualMachine) {
        vm.state.codec_registry.forget(encoding.as_str());
    }

    #[pyfunction]
    fn register_error(name: PyStrRef, handler: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        if !handler.is_callable() {
            return Err(vm.new_type_error("handler must be callable".to_owned()));
        }
        vm.state
            .codec_registry
            .register_error(name.as_str().to_owned(), handler);
        Ok(())
    }

    #[pyfunction]
    fn lookup_error(name: PyStrRef, vm: &VirtualMachine) -> PyResult {
        if name.as_wtf8().as_bytes().contains(&0) {
            return Err(cstring_error(vm));
        }
        if !name.as_wtf8().is_utf8() {
            return Err(vm.new_unicode_encode_error(
                "'utf-8' codec can't encode character: surrogates not allowed".to_owned(),
            ));
        }
        vm.state.codec_registry.lookup_error(name.as_str(), vm)
    }

    #[pyfunction]
    fn _unregister_error(errors: PyStrRef, vm: &VirtualMachine) -> PyResult<bool> {
        if errors.as_wtf8().as_bytes().contains(&0) {
            return Err(cstring_error(vm));
        }
        if !errors.as_wtf8().is_utf8() {
            return Err(vm.new_unicode_encode_error(
                "'utf-8' codec can't encode character: surrogates not allowed".to_owned(),
            ));
        }
        vm.state
            .codec_registry
            .unregister_error(errors.as_str(), vm)
    }

    type EncodeResult = PyResult<(Vec<u8>, usize)>;

    #[derive(FromArgs)]
    struct EncodeArgs {
        #[pyarg(positional)]
        s: PyStrRef,
        #[pyarg(positional, optional)]
        errors: Option<PyStrRef>,
    }

    impl EncodeArgs {
        #[inline]
        fn encode<'a, F>(&'a self, name: &'a str, encode: F, vm: &'a VirtualMachine) -> EncodeResult
        where
            F: FnOnce(PyEncodeContext<'a>, &ErrorsHandler<'a>) -> PyResult<Vec<u8>>,
        {
            let ctx = PyEncodeContext::new(name, &self.s, vm);
            let errors = ErrorsHandler::new(self.errors.as_deref(), vm);
            let encoded = encode(ctx, &errors)?;
            Ok((encoded, self.s.char_len()))
        }
    }

    type DecodeResult = PyResult<(Wtf8Buf, usize)>;

    #[derive(FromArgs)]
    struct DecodeArgs {
        #[pyarg(positional)]
        data: ArgBytesLike,
        #[pyarg(positional, optional)]
        errors: Option<PyStrRef>,
        #[pyarg(positional, default = false)]
        final_decode: bool,
    }

    impl DecodeArgs {
        #[inline]
        fn decode<'a, F>(&'a self, name: &'a str, decode: F, vm: &'a VirtualMachine) -> DecodeResult
        where
            F: FnOnce(PyDecodeContext<'a>, &ErrorsHandler<'a>, bool) -> DecodeResult,
        {
            let ctx = PyDecodeContext::new(name, &self.data, vm);
            let errors = ErrorsHandler::new(self.errors.as_deref(), vm);
            decode(ctx, &errors, self.final_decode)
        }
    }

    #[derive(FromArgs)]
    struct DecodeArgsNoFinal {
        #[pyarg(positional)]
        data: ArgBytesLike,
        #[pyarg(positional, optional)]
        errors: Option<PyStrRef>,
    }

    impl DecodeArgsNoFinal {
        #[inline]
        fn decode<'a, F>(&'a self, name: &'a str, decode: F, vm: &'a VirtualMachine) -> DecodeResult
        where
            F: FnOnce(PyDecodeContext<'a>, &ErrorsHandler<'a>) -> DecodeResult,
        {
            let ctx = PyDecodeContext::new(name, &self.data, vm);
            let errors = ErrorsHandler::new(self.errors.as_deref(), vm);
            decode(ctx, &errors)
        }
    }

    macro_rules! do_codec {
        ($module:ident :: $func:ident, $args: expr, $vm:expr) => {{
            use encodings::$module as codec;
            $args.$func(codec::ENCODING_NAME, codec::$func, $vm)
        }};
    }

    #[pyfunction]
    fn utf_8_encode(args: EncodeArgs, vm: &VirtualMachine) -> EncodeResult {
        if args.s.is_utf8()
            || args
                .errors
                .as_ref()
                .is_some_and(|s| s.is(identifier!(vm, surrogatepass)))
        {
            return Ok((args.s.as_bytes().to_vec(), args.s.byte_len()));
        }
        do_codec!(utf8::encode, args, vm)
    }

    #[pyfunction]
    fn utf_8_decode(args: DecodeArgs, vm: &VirtualMachine) -> DecodeResult {
        do_codec!(utf8::decode, args, vm)
    }

    #[pyfunction]
    fn latin_1_encode(args: EncodeArgs, vm: &VirtualMachine) -> EncodeResult {
        if args.s.isascii() {
            return Ok((args.s.as_bytes().to_vec(), args.s.byte_len()));
        }
        do_codec!(latin_1::encode, args, vm)
    }

    #[pyfunction]
    fn latin_1_decode(args: DecodeArgsNoFinal, vm: &VirtualMachine) -> DecodeResult {
        do_codec!(latin_1::decode, args, vm)
    }

    #[pyfunction]
    fn ascii_encode(args: EncodeArgs, vm: &VirtualMachine) -> EncodeResult {
        if args.s.isascii() {
            return Ok((args.s.as_bytes().to_vec(), args.s.byte_len()));
        }
        do_codec!(ascii::encode, args, vm)
    }

    #[pyfunction]
    fn ascii_decode(args: DecodeArgsNoFinal, vm: &VirtualMachine) -> DecodeResult {
        do_codec!(ascii::decode, args, vm)
    }

    // TODO: implement these codecs in Rust!

    macro_rules! delegate_pycodecs {
        ($name:ident, $args:ident, $vm:ident) => {{
            rustpython_common::static_cell!(
                static FUNC: PyObjectRef;
            );
            super::delegate_pycodecs(&FUNC, stringify!($name), $args, $vm)
        }};
    }

    #[pyfunction]
    fn readbuffer_encode(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        delegate_pycodecs!(readbuffer_encode, args, vm)
    }
    #[pyfunction]
    fn escape_encode(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        delegate_pycodecs!(escape_encode, args, vm)
    }
    #[pyfunction]
    fn escape_decode(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        delegate_pycodecs!(escape_decode, args, vm)
    }
    #[pyfunction]
    fn unicode_escape_encode(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        delegate_pycodecs!(unicode_escape_encode, args, vm)
    }
    #[pyfunction]
    fn unicode_escape_decode(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        delegate_pycodecs!(unicode_escape_decode, args, vm)
    }
    #[pyfunction]
    fn raw_unicode_escape_encode(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        delegate_pycodecs!(raw_unicode_escape_encode, args, vm)
    }
    #[pyfunction]
    fn raw_unicode_escape_decode(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        delegate_pycodecs!(raw_unicode_escape_decode, args, vm)
    }
    #[pyfunction]
    fn utf_7_encode(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        delegate_pycodecs!(utf_7_encode, args, vm)
    }
    #[pyfunction]
    fn utf_7_decode(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        delegate_pycodecs!(utf_7_decode, args, vm)
    }
    #[pyfunction]
    fn utf_16_encode(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        delegate_pycodecs!(utf_16_encode, args, vm)
    }
    #[pyfunction]
    fn utf_16_decode(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        delegate_pycodecs!(utf_16_decode, args, vm)
    }
    #[pyfunction]
    fn charmap_encode(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        delegate_pycodecs!(charmap_encode, args, vm)
    }
    #[pyfunction]
    fn charmap_decode(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        delegate_pycodecs!(charmap_decode, args, vm)
    }
    #[pyfunction]
    fn charmap_build(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        delegate_pycodecs!(charmap_build, args, vm)
    }
    #[pyfunction]
    fn utf_16_le_encode(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        delegate_pycodecs!(utf_16_le_encode, args, vm)
    }
    #[pyfunction]
    fn utf_16_le_decode(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        delegate_pycodecs!(utf_16_le_decode, args, vm)
    }
    #[pyfunction]
    fn utf_16_be_encode(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        delegate_pycodecs!(utf_16_be_encode, args, vm)
    }
    #[pyfunction]
    fn utf_16_be_decode(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        delegate_pycodecs!(utf_16_be_decode, args, vm)
    }
    #[pyfunction]
    fn utf_16_ex_decode(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        delegate_pycodecs!(utf_16_ex_decode, args, vm)
    }
    #[pyfunction]
    fn utf_32_ex_decode(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        delegate_pycodecs!(utf_32_ex_decode, args, vm)
    }
    #[pyfunction]
    fn utf_32_encode(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        delegate_pycodecs!(utf_32_encode, args, vm)
    }
    #[pyfunction]
    fn utf_32_decode(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        delegate_pycodecs!(utf_32_decode, args, vm)
    }
    #[pyfunction]
    fn utf_32_le_encode(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        delegate_pycodecs!(utf_32_le_encode, args, vm)
    }
    #[pyfunction]
    fn utf_32_le_decode(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        delegate_pycodecs!(utf_32_le_decode, args, vm)
    }
    #[pyfunction]
    fn utf_32_be_encode(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        delegate_pycodecs!(utf_32_be_encode, args, vm)
    }
    #[pyfunction]
    fn utf_32_be_decode(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        delegate_pycodecs!(utf_32_be_decode, args, vm)
    }
}

#[inline]
fn delegate_pycodecs(
    cell: &'static StaticCell<crate::PyObjectRef>,
    name: &'static str,
    args: crate::function::FuncArgs,
    vm: &crate::VirtualMachine,
) -> crate::PyResult {
    let f = cell.get_or_try_init(|| {
        let module = vm.import("_pycodecs", 0)?;
        module.get_attr(name, vm)
    })?;
    f.call(args, vm)
}

#[cfg(windows)]
#[pymodule(sub)]
mod _codecs_windows {
    use crate::{PyResult, VirtualMachine};
    use crate::{builtins::PyStrRef, function::ArgBytesLike};

    #[derive(FromArgs)]
    struct MbcsEncodeArgs {
        #[pyarg(positional)]
        s: PyStrRef,
        #[pyarg(positional, optional)]
        errors: Option<PyStrRef>,
    }

    #[pyfunction]
    fn mbcs_encode(args: MbcsEncodeArgs, vm: &VirtualMachine) -> PyResult<(Vec<u8>, usize)> {
        use crate::common::windows::ToWideString;
        use windows_sys::Win32::Globalization::{
            CP_ACP, WC_NO_BEST_FIT_CHARS, WideCharToMultiByte,
        };

        let errors = args.errors.as_ref().map(|s| s.as_str()).unwrap_or("strict");
        let s = match args.s.to_str() {
            Some(s) => s,
            None => {
                // String contains surrogates - not encodable with mbcs
                return Err(vm.new_unicode_encode_error(
                    "'mbcs' codec can't encode character: surrogates not allowed".to_string(),
                ));
            }
        };
        let char_len = args.s.char_len();

        if s.is_empty() {
            return Ok((Vec::new(), char_len));
        }

        // Convert UTF-8 string to UTF-16
        let wide: Vec<u16> = std::ffi::OsStr::new(s).to_wide();

        // Get the required buffer size
        let size = unsafe {
            WideCharToMultiByte(
                CP_ACP,
                WC_NO_BEST_FIT_CHARS,
                wide.as_ptr(),
                wide.len() as i32,
                core::ptr::null_mut(),
                0,
                core::ptr::null(),
                core::ptr::null_mut(),
            )
        };

        if size == 0 {
            let err = std::io::Error::last_os_error();
            return Err(vm.new_os_error(format!("mbcs_encode failed: {}", err)));
        }

        let mut buffer = vec![0u8; size as usize];
        let mut used_default_char: i32 = 0;

        let result = unsafe {
            WideCharToMultiByte(
                CP_ACP,
                WC_NO_BEST_FIT_CHARS,
                wide.as_ptr(),
                wide.len() as i32,
                buffer.as_mut_ptr().cast(),
                size,
                core::ptr::null(),
                if errors == "strict" {
                    &mut used_default_char
                } else {
                    core::ptr::null_mut()
                },
            )
        };

        if result == 0 {
            let err = std::io::Error::last_os_error();
            return Err(vm.new_os_error(format!("mbcs_encode failed: {err}")));
        }

        if errors == "strict" && used_default_char != 0 {
            return Err(vm.new_unicode_encode_error(
                "'mbcs' codec can't encode characters: invalid character",
            ));
        }

        buffer.truncate(result as usize);
        Ok((buffer, char_len))
    }

    #[derive(FromArgs)]
    struct MbcsDecodeArgs {
        #[pyarg(positional)]
        data: ArgBytesLike,
        #[pyarg(positional, optional)]
        errors: Option<PyStrRef>,
        #[pyarg(positional, default = false)]
        #[allow(dead_code)]
        r#final: bool,
    }

    #[pyfunction]
    fn mbcs_decode(args: MbcsDecodeArgs, vm: &VirtualMachine) -> PyResult<(String, usize)> {
        use windows_sys::Win32::Globalization::{
            CP_ACP, MB_ERR_INVALID_CHARS, MultiByteToWideChar,
        };

        let _errors = args.errors.as_ref().map(|s| s.as_str()).unwrap_or("strict");
        let data = args.data.borrow_buf();
        let len = data.len();

        if data.is_empty() {
            return Ok((String::new(), 0));
        }

        // Get the required buffer size for UTF-16
        let size = unsafe {
            MultiByteToWideChar(
                CP_ACP,
                MB_ERR_INVALID_CHARS,
                data.as_ptr().cast(),
                len as i32,
                core::ptr::null_mut(),
                0,
            )
        };

        if size == 0 {
            // Try without MB_ERR_INVALID_CHARS for non-strict mode (replacement behavior)
            let size = unsafe {
                MultiByteToWideChar(
                    CP_ACP,
                    0,
                    data.as_ptr().cast(),
                    len as i32,
                    core::ptr::null_mut(),
                    0,
                )
            };
            if size == 0 {
                let err = std::io::Error::last_os_error();
                return Err(vm.new_os_error(format!("mbcs_decode failed: {}", err)));
            }

            let mut buffer = vec![0u16; size as usize];
            let result = unsafe {
                MultiByteToWideChar(
                    CP_ACP,
                    0,
                    data.as_ptr().cast(),
                    len as i32,
                    buffer.as_mut_ptr(),
                    size,
                )
            };
            if result == 0 {
                let err = std::io::Error::last_os_error();
                return Err(vm.new_os_error(format!("mbcs_decode failed: {}", err)));
            }
            buffer.truncate(result as usize);
            let s = String::from_utf16(&buffer)
                .map_err(|e| vm.new_unicode_decode_error(format!("mbcs_decode failed: {}", e)))?;
            return Ok((s, len));
        }

        // Strict mode succeeded - no invalid characters
        let mut buffer = vec![0u16; size as usize];
        let result = unsafe {
            MultiByteToWideChar(
                CP_ACP,
                MB_ERR_INVALID_CHARS,
                data.as_ptr().cast(),
                len as i32,
                buffer.as_mut_ptr(),
                size,
            )
        };
        if result == 0 {
            let err = std::io::Error::last_os_error();
            return Err(vm.new_os_error(format!("mbcs_decode failed: {}", err)));
        }
        buffer.truncate(result as usize);
        let s = String::from_utf16(&buffer)
            .map_err(|e| vm.new_unicode_decode_error(format!("mbcs_decode failed: {}", e)))?;

        Ok((s, len))
    }

    #[derive(FromArgs)]
    struct OemEncodeArgs {
        #[pyarg(positional)]
        s: PyStrRef,
        #[pyarg(positional, optional)]
        errors: Option<PyStrRef>,
    }

    #[pyfunction]
    fn oem_encode(args: OemEncodeArgs, vm: &VirtualMachine) -> PyResult<(Vec<u8>, usize)> {
        use crate::common::windows::ToWideString;
        use windows_sys::Win32::Globalization::{
            CP_OEMCP, WC_NO_BEST_FIT_CHARS, WideCharToMultiByte,
        };

        let errors = args.errors.as_ref().map(|s| s.as_str()).unwrap_or("strict");
        let s = match args.s.to_str() {
            Some(s) => s,
            None => {
                // String contains surrogates - not encodable with oem
                return Err(vm.new_unicode_encode_error(
                    "'oem' codec can't encode character: surrogates not allowed".to_string(),
                ));
            }
        };
        let char_len = args.s.char_len();

        if s.is_empty() {
            return Ok((Vec::new(), char_len));
        }

        // Convert UTF-8 string to UTF-16
        let wide: Vec<u16> = std::ffi::OsStr::new(s).to_wide();

        // Get the required buffer size
        let size = unsafe {
            WideCharToMultiByte(
                CP_OEMCP,
                WC_NO_BEST_FIT_CHARS,
                wide.as_ptr(),
                wide.len() as i32,
                core::ptr::null_mut(),
                0,
                core::ptr::null(),
                core::ptr::null_mut(),
            )
        };

        if size == 0 {
            let err = std::io::Error::last_os_error();
            return Err(vm.new_os_error(format!("oem_encode failed: {}", err)));
        }

        let mut buffer = vec![0u8; size as usize];
        let mut used_default_char: i32 = 0;

        let result = unsafe {
            WideCharToMultiByte(
                CP_OEMCP,
                WC_NO_BEST_FIT_CHARS,
                wide.as_ptr(),
                wide.len() as i32,
                buffer.as_mut_ptr().cast(),
                size,
                core::ptr::null(),
                if errors == "strict" {
                    &mut used_default_char
                } else {
                    core::ptr::null_mut()
                },
            )
        };

        if result == 0 {
            let err = std::io::Error::last_os_error();
            return Err(vm.new_os_error(format!("oem_encode failed: {err}")));
        }

        if errors == "strict" && used_default_char != 0 {
            return Err(vm.new_unicode_encode_error(
                "'oem' codec can't encode characters: invalid character",
            ));
        }

        buffer.truncate(result as usize);
        Ok((buffer, char_len))
    }

    #[derive(FromArgs)]
    struct OemDecodeArgs {
        #[pyarg(positional)]
        data: ArgBytesLike,
        #[pyarg(positional, optional)]
        errors: Option<PyStrRef>,
        #[pyarg(positional, default = false)]
        #[allow(dead_code)]
        r#final: bool,
    }

    #[pyfunction]
    fn oem_decode(args: OemDecodeArgs, vm: &VirtualMachine) -> PyResult<(String, usize)> {
        use windows_sys::Win32::Globalization::{
            CP_OEMCP, MB_ERR_INVALID_CHARS, MultiByteToWideChar,
        };

        let _errors = args.errors.as_ref().map(|s| s.as_str()).unwrap_or("strict");
        let data = args.data.borrow_buf();
        let len = data.len();

        if data.is_empty() {
            return Ok((String::new(), 0));
        }

        // Get the required buffer size for UTF-16
        let size = unsafe {
            MultiByteToWideChar(
                CP_OEMCP,
                MB_ERR_INVALID_CHARS,
                data.as_ptr().cast(),
                len as i32,
                core::ptr::null_mut(),
                0,
            )
        };

        if size == 0 {
            // Try without MB_ERR_INVALID_CHARS for non-strict mode (replacement behavior)
            let size = unsafe {
                MultiByteToWideChar(
                    CP_OEMCP,
                    0,
                    data.as_ptr().cast(),
                    len as i32,
                    core::ptr::null_mut(),
                    0,
                )
            };
            if size == 0 {
                let err = std::io::Error::last_os_error();
                return Err(vm.new_os_error(format!("oem_decode failed: {}", err)));
            }

            let mut buffer = vec![0u16; size as usize];
            let result = unsafe {
                MultiByteToWideChar(
                    CP_OEMCP,
                    0,
                    data.as_ptr().cast(),
                    len as i32,
                    buffer.as_mut_ptr(),
                    size,
                )
            };
            if result == 0 {
                let err = std::io::Error::last_os_error();
                return Err(vm.new_os_error(format!("oem_decode failed: {}", err)));
            }
            buffer.truncate(result as usize);
            let s = String::from_utf16(&buffer)
                .map_err(|e| vm.new_unicode_decode_error(format!("oem_decode failed: {}", e)))?;
            return Ok((s, len));
        }

        // Strict mode succeeded - no invalid characters
        let mut buffer = vec![0u16; size as usize];
        let result = unsafe {
            MultiByteToWideChar(
                CP_OEMCP,
                MB_ERR_INVALID_CHARS,
                data.as_ptr().cast(),
                len as i32,
                buffer.as_mut_ptr(),
                size,
            )
        };
        if result == 0 {
            let err = std::io::Error::last_os_error();
            return Err(vm.new_os_error(format!("oem_decode failed: {}", err)));
        }
        buffer.truncate(result as usize);
        let s = String::from_utf16(&buffer)
            .map_err(|e| vm.new_unicode_decode_error(format!("oem_decode failed: {}", e)))?;

        Ok((s, len))
    }

    #[derive(FromArgs)]
    struct CodePageEncodeArgs {
        #[pyarg(positional)]
        code_page: i32,
        #[pyarg(positional)]
        s: PyStrRef,
        #[pyarg(positional, optional)]
        errors: Option<PyStrRef>,
    }

    fn code_page_encoding_name(code_page: u32) -> String {
        match code_page {
            0 => "mbcs".to_string(),
            cp => format!("cp{cp}"),
        }
    }

    /// Get WideCharToMultiByte flags for encoding.
    /// Matches encode_code_page_flags() in CPython.
    fn encode_code_page_flags(code_page: u32, errors: &str) -> u32 {
        use windows_sys::Win32::Globalization::{WC_ERR_INVALID_CHARS, WC_NO_BEST_FIT_CHARS};
        if code_page == 65001 {
            // CP_UTF8
            WC_ERR_INVALID_CHARS
        } else if code_page == 65000 {
            // CP_UTF7 only supports flags=0
            0
        } else if errors == "replace" {
            0
        } else {
            WC_NO_BEST_FIT_CHARS
        }
    }

    /// Try to encode the entire wide string at once (fast/strict path).
    /// Returns Ok(Some(bytes)) on success, Ok(None) if there are unencodable chars,
    /// or Err on OS error.
    fn try_encode_code_page_strict(
        code_page: u32,
        wide: &[u16],
        vm: &VirtualMachine,
    ) -> PyResult<Option<Vec<u8>>> {
        use windows_sys::Win32::Globalization::WideCharToMultiByte;

        let flags = encode_code_page_flags(code_page, "strict");

        let use_default_char = code_page != 65001 && code_page != 65000;
        let mut used_default_char: i32 = 0;
        let pused = if use_default_char {
            &mut used_default_char as *mut i32
        } else {
            core::ptr::null_mut()
        };

        let size = unsafe {
            WideCharToMultiByte(
                code_page,
                flags,
                wide.as_ptr(),
                wide.len() as i32,
                core::ptr::null_mut(),
                0,
                core::ptr::null(),
                pused,
            )
        };

        if size <= 0 {
            let err_code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            if err_code == 1113 {
                // ERROR_NO_UNICODE_TRANSLATION
                return Ok(None);
            }
            let err = std::io::Error::last_os_error();
            return Err(vm.new_os_error(format!("code_page_encode: {err}")));
        }

        if use_default_char && used_default_char != 0 {
            return Ok(None);
        }

        let mut buffer = vec![0u8; size as usize];
        used_default_char = 0;
        let pused = if use_default_char {
            &mut used_default_char as *mut i32
        } else {
            core::ptr::null_mut()
        };

        let result = unsafe {
            WideCharToMultiByte(
                code_page,
                flags,
                wide.as_ptr(),
                wide.len() as i32,
                buffer.as_mut_ptr().cast(),
                size,
                core::ptr::null(),
                pused,
            )
        };

        if result <= 0 {
            let err_code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            if err_code == 1113 {
                return Ok(None);
            }
            let err = std::io::Error::last_os_error();
            return Err(vm.new_os_error(format!("code_page_encode: {err}")));
        }

        if use_default_char && used_default_char != 0 {
            return Ok(None);
        }

        buffer.truncate(result as usize);
        Ok(Some(buffer))
    }

    /// Encode character by character with error handling.
    fn encode_code_page_errors(
        code_page: u32,
        s: &PyStrRef,
        errors: &str,
        encoding_name: &str,
        vm: &VirtualMachine,
    ) -> PyResult<(Vec<u8>, usize)> {
        use crate::builtins::{PyBytes, PyStr, PyTuple};
        use windows_sys::Win32::Globalization::WideCharToMultiByte;

        let char_len = s.char_len();
        let flags = encode_code_page_flags(code_page, errors);
        let use_default_char = code_page != 65001 && code_page != 65000;
        let encoding_str = vm.ctx.new_str(encoding_name);
        let reason_str = vm.ctx.new_str("invalid character");

        // For strict mode, find the first unencodable character and raise
        if errors == "strict" {
            // Find the failing position by trying each character
            let mut fail_pos = 0;
            for cp in s.as_wtf8().code_points() {
                let ch = cp.to_u32();
                if (0xD800..=0xDFFF).contains(&ch) {
                    break;
                }
                let mut wchars = [0u16; 2];
                let wchar_len = if ch < 0x10000 {
                    wchars[0] = ch as u16;
                    1
                } else {
                    wchars[0] = ((ch - 0x10000) >> 10) as u16 + 0xD800;
                    wchars[1] = ((ch - 0x10000) & 0x3FF) as u16 + 0xDC00;
                    2
                };
                let mut used_default_char: i32 = 0;
                let pused = if use_default_char {
                    &mut used_default_char as *mut i32
                } else {
                    core::ptr::null_mut()
                };
                let outsize = unsafe {
                    WideCharToMultiByte(
                        code_page,
                        flags,
                        wchars.as_ptr(),
                        wchar_len,
                        core::ptr::null_mut(),
                        0,
                        core::ptr::null(),
                        pused,
                    )
                };
                if outsize <= 0 || (use_default_char && used_default_char != 0) {
                    break;
                }
                fail_pos += 1;
            }
            return Err(vm.new_unicode_encode_error_real(
                encoding_str,
                s.clone(),
                fail_pos,
                fail_pos + 1,
                reason_str,
            ));
        }

        let error_handler = vm.state.codec_registry.lookup_error(errors, vm)?;
        let mut output = Vec::new();

        // Collect code points for random access
        let code_points: Vec<u32> = s.as_wtf8().code_points().map(|cp| cp.to_u32()).collect();

        let mut pos = 0usize;
        while pos < code_points.len() {
            let ch = code_points[pos];

            // Convert code point to UTF-16
            let mut wchars = [0u16; 2];
            let wchar_len;
            let is_surrogate = (0xD800..=0xDFFF).contains(&ch);

            if is_surrogate {
                wchar_len = 0; // Can't encode surrogates normally
            } else if ch < 0x10000 {
                wchars[0] = ch as u16;
                wchar_len = 1;
            } else {
                wchars[0] = ((ch - 0x10000) >> 10) as u16 + 0xD800;
                wchars[1] = ((ch - 0x10000) & 0x3FF) as u16 + 0xDC00;
                wchar_len = 2;
            }

            if !is_surrogate {
                let mut used_default_char: i32 = 0;
                let pused = if use_default_char {
                    &mut used_default_char as *mut i32
                } else {
                    core::ptr::null_mut()
                };

                let mut buf = [0u8; 8];
                let outsize = unsafe {
                    WideCharToMultiByte(
                        code_page,
                        flags,
                        wchars.as_ptr(),
                        wchar_len,
                        buf.as_mut_ptr().cast(),
                        buf.len() as i32,
                        core::ptr::null(),
                        pused,
                    )
                };

                if outsize > 0 && (!use_default_char || used_default_char == 0) {
                    output.extend_from_slice(&buf[..outsize as usize]);
                    pos += 1;
                    continue;
                }
            }

            // Character can't be encoded - call error handler
            let exc = vm.new_unicode_encode_error_real(
                encoding_str.clone(),
                s.clone(),
                pos,
                pos + 1,
                reason_str.clone(),
            );

            let res = error_handler.call((exc,), vm)?;
            let tuple_err =
                || vm.new_type_error("encoding error handler must return (str/bytes, int) tuple");
            let tuple: &PyTuple = res.downcast_ref().ok_or_else(&tuple_err)?;
            let tuple_slice = tuple.as_slice();
            if tuple_slice.len() != 2 {
                return Err(tuple_err());
            }

            let replacement = &tuple_slice[0];
            let new_pos_obj = tuple_slice[1].clone();

            if let Some(bytes) = replacement.downcast_ref::<PyBytes>() {
                output.extend_from_slice(bytes);
            } else if let Some(rep_str) = replacement.downcast_ref::<PyStr>() {
                // Replacement string - try to encode each character
                for rcp in rep_str.as_wtf8().code_points() {
                    let rch = rcp.to_u32();
                    if rch > 127 {
                        return Err(vm.new_unicode_encode_error_real(
                            encoding_str.clone(),
                            s.clone(),
                            pos,
                            pos + 1,
                            vm.ctx
                                .new_str("unable to encode error handler result to ASCII"),
                        ));
                    }
                    output.push(rch as u8);
                }
            } else {
                return Err(tuple_err());
            }

            let new_pos: isize = new_pos_obj.try_into_value(vm).map_err(|_| tuple_err())?;
            pos = if new_pos < 0 {
                (code_points.len() as isize + new_pos).max(0) as usize
            } else {
                new_pos as usize
            };
        }

        Ok((output, char_len))
    }

    #[pyfunction]
    fn code_page_encode(
        args: CodePageEncodeArgs,
        vm: &VirtualMachine,
    ) -> PyResult<(Vec<u8>, usize)> {
        use crate::common::windows::ToWideString;

        if args.code_page < 0 {
            return Err(vm.new_value_error("invalid code page number".to_owned()));
        }
        let errors = args.errors.as_ref().map(|s| s.as_str()).unwrap_or("strict");
        let code_page = args.code_page as u32;
        let char_len = args.s.char_len();

        if char_len == 0 {
            return Ok((Vec::new(), 0));
        }

        let encoding_name = code_page_encoding_name(code_page);

        // Fast path: try encoding the whole string at once (only if no surrogates)
        if let Some(str_data) = args.s.to_str() {
            let wide: Vec<u16> = std::ffi::OsStr::new(str_data).to_wide();
            if let Some(result) = try_encode_code_page_strict(code_page, &wide, vm)? {
                return Ok((result, char_len));
            }
        }

        // Slow path: character by character with error handling
        encode_code_page_errors(code_page, &args.s, errors, &encoding_name, vm)
    }

    #[derive(FromArgs)]
    struct CodePageDecodeArgs {
        #[pyarg(positional)]
        code_page: i32,
        #[pyarg(positional)]
        data: ArgBytesLike,
        #[pyarg(positional, optional)]
        errors: Option<PyStrRef>,
        #[pyarg(positional, default = false)]
        r#final: bool,
    }

    /// Try to decode the entire buffer with strict flags (fast path).
    /// Returns Ok(Some(wide_chars)) on success, Ok(None) on decode error,
    /// or Err on OS error.
    fn try_decode_code_page_strict(
        code_page: u32,
        data: &[u8],
        vm: &VirtualMachine,
    ) -> PyResult<Option<Vec<u16>>> {
        use windows_sys::Win32::Globalization::{MB_ERR_INVALID_CHARS, MultiByteToWideChar};

        let mut flags = MB_ERR_INVALID_CHARS;

        loop {
            let size = unsafe {
                MultiByteToWideChar(
                    code_page,
                    flags,
                    data.as_ptr().cast(),
                    data.len() as i32,
                    core::ptr::null_mut(),
                    0,
                )
            };
            if size > 0 {
                let mut buffer = vec![0u16; size as usize];
                let result = unsafe {
                    MultiByteToWideChar(
                        code_page,
                        flags,
                        data.as_ptr().cast(),
                        data.len() as i32,
                        buffer.as_mut_ptr(),
                        size,
                    )
                };
                if result > 0 {
                    buffer.truncate(result as usize);
                    return Ok(Some(buffer));
                }
            }

            let err_code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            // ERROR_INVALID_FLAGS = 1004
            if flags != 0 && err_code == 1004 {
                flags = 0;
                continue;
            }
            // ERROR_NO_UNICODE_TRANSLATION = 1113
            if err_code == 1113 {
                return Ok(None);
            }
            let err = std::io::Error::last_os_error();
            return Err(vm.new_os_error(format!("code_page_decode: {err}")));
        }
    }

    /// Decode byte by byte with error handling (slow path).
    fn decode_code_page_errors(
        code_page: u32,
        data: &[u8],
        errors: &str,
        is_final: bool,
        encoding_name: &str,
        vm: &VirtualMachine,
    ) -> PyResult<(PyStrRef, usize)> {
        use crate::builtins::PyTuple;
        use crate::common::wtf8::Wtf8Buf;
        use windows_sys::Win32::Globalization::{MB_ERR_INVALID_CHARS, MultiByteToWideChar};

        let len = data.len();
        let encoding_str = vm.ctx.new_str(encoding_name);
        let reason_str = vm
            .ctx
            .new_str("No mapping for the Unicode character exists in the target code page.");

        // For strict+final, find the failing position and raise
        if errors == "strict" && is_final {
            // Find the exact failing byte position by trying byte by byte
            let mut fail_pos = 0;
            let mut flags_s: u32 = MB_ERR_INVALID_CHARS;
            let mut buf = [0u16; 2];
            while fail_pos < len {
                let mut in_size = 1;
                let mut found = false;
                while in_size <= 4 && fail_pos + in_size <= len {
                    let outsize = unsafe {
                        MultiByteToWideChar(
                            code_page,
                            flags_s,
                            data[fail_pos..].as_ptr().cast(),
                            in_size as i32,
                            buf.as_mut_ptr(),
                            2,
                        )
                    };
                    if outsize > 0 {
                        fail_pos += in_size;
                        found = true;
                        break;
                    }
                    let err_code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
                    if err_code == 1004 && flags_s != 0 {
                        flags_s = 0;
                        continue;
                    }
                    in_size += 1;
                }
                if !found {
                    break;
                }
            }
            let object = vm.ctx.new_bytes(data.to_vec());
            return Err(vm.new_unicode_decode_error_real(
                encoding_str,
                object,
                fail_pos,
                fail_pos + 1,
                reason_str,
            ));
        }

        let error_handler = if errors != "strict"
            && errors != "ignore"
            && errors != "replace"
            && errors != "backslashreplace"
            && errors != "surrogateescape"
        {
            Some(vm.state.codec_registry.lookup_error(errors, vm)?)
        } else {
            None
        };

        let mut wide_buf: Vec<u16> = Vec::new();
        let mut pos = 0usize;
        let mut flags: u32 = MB_ERR_INVALID_CHARS;

        while pos < len {
            // Try to decode with increasing byte counts (1, 2, 3, 4)
            let mut in_size = 1;
            let mut outsize;
            let mut buffer = [0u16; 2];

            loop {
                outsize = unsafe {
                    MultiByteToWideChar(
                        code_page,
                        flags,
                        data[pos..].as_ptr().cast(),
                        in_size as i32,
                        buffer.as_mut_ptr(),
                        2,
                    )
                };
                if outsize > 0 {
                    break;
                }
                let err_code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
                if err_code == 1004 && flags != 0 {
                    // ERROR_INVALID_FLAGS - retry with flags=0
                    flags = 0;
                    continue;
                }
                if err_code != 1113 && err_code != 122 {
                    // Not ERROR_NO_UNICODE_TRANSLATION and not ERROR_INSUFFICIENT_BUFFER
                    let err = std::io::Error::last_os_error();
                    return Err(vm.new_os_error(format!("code_page_decode: {err}")));
                }
                in_size += 1;
                if in_size > 4 || pos + in_size > len {
                    break;
                }
            }

            if outsize <= 0 {
                // Can't decode this byte sequence
                if pos + in_size >= len && !is_final {
                    // Incomplete sequence at end, not final - stop here
                    break;
                }

                // Handle the error based on error mode
                match errors {
                    "ignore" => {
                        pos += 1;
                    }
                    "replace" => {
                        wide_buf.push(0xFFFD);
                        pos += 1;
                    }
                    "backslashreplace" => {
                        let byte = data[pos];
                        for ch in format!("\\x{byte:02x}").encode_utf16() {
                            wide_buf.push(ch);
                        }
                        pos += 1;
                    }
                    "surrogateescape" => {
                        let byte = data[pos];
                        wide_buf.push(0xDC00 + byte as u16);
                        pos += 1;
                    }
                    "strict" => {
                        let object = vm.ctx.new_bytes(data.to_vec());
                        return Err(vm.new_unicode_decode_error_real(
                            encoding_str,
                            object,
                            pos,
                            pos + 1,
                            reason_str,
                        ));
                    }
                    _ => {
                        // Custom error handler
                        let object = vm.ctx.new_bytes(data.to_vec());
                        let exc = vm.new_unicode_decode_error_real(
                            encoding_str.clone(),
                            object,
                            pos,
                            pos + 1,
                            reason_str.clone(),
                        );
                        let handler = error_handler.as_ref().unwrap();
                        let res = handler.call((exc,), vm)?;
                        let tuple_err = || {
                            vm.new_type_error("decoding error handler must return (str, int) tuple")
                        };
                        let tuple: &PyTuple = res.downcast_ref().ok_or_else(&tuple_err)?;
                        let tuple_slice = tuple.as_slice();
                        if tuple_slice.len() != 2 {
                            return Err(tuple_err());
                        }

                        let replacement: PyStrRef = tuple_slice[0]
                            .clone()
                            .try_into_value(vm)
                            .map_err(|_| tuple_err())?;
                        let new_pos: isize = tuple_slice[1]
                            .clone()
                            .try_into_value(vm)
                            .map_err(|_| tuple_err())?;

                        for cp in replacement.as_wtf8().code_points() {
                            let u = cp.to_u32();
                            if u < 0x10000 {
                                wide_buf.push(u as u16);
                            } else {
                                wide_buf.push(((u - 0x10000) >> 10) as u16 + 0xD800);
                                wide_buf.push(((u - 0x10000) & 0x3FF) as u16 + 0xDC00);
                            }
                        }

                        pos = if new_pos < 0 {
                            (len as isize + new_pos).max(0) as usize
                        } else {
                            new_pos as usize
                        };
                    }
                }
            } else {
                // Successfully decoded
                wide_buf.extend_from_slice(&buffer[..outsize as usize]);
                pos += in_size;
            }
        }

        let s = Wtf8Buf::from_wide(&wide_buf);
        Ok((vm.ctx.new_str(s), pos))
    }

    #[pyfunction]
    fn code_page_decode(
        args: CodePageDecodeArgs,
        vm: &VirtualMachine,
    ) -> PyResult<(PyStrRef, usize)> {
        use crate::common::wtf8::Wtf8Buf;

        if args.code_page < 0 {
            return Err(vm.new_value_error("invalid code page number".to_owned()));
        }
        let errors = args.errors.as_ref().map(|s| s.as_str()).unwrap_or("strict");
        let code_page = args.code_page as u32;
        let data = args.data.borrow_buf();
        let is_final = args.r#final;

        if data.is_empty() {
            return Ok((vm.ctx.empty_str.to_owned(), 0));
        }

        let encoding_name = code_page_encoding_name(code_page);

        // Fast path: try to decode the whole buffer with strict flags
        match try_decode_code_page_strict(code_page, &data, vm)? {
            Some(wide) => {
                let s = Wtf8Buf::from_wide(&wide);
                return Ok((vm.ctx.new_str(s), data.len()));
            }
            None => {
                // Decode error - fall through to slow path
            }
        }

        // Slow path: byte by byte with error handling
        decode_code_page_errors(code_page, &data, errors, is_final, &encoding_name, vm)
    }
}

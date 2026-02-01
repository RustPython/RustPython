pub(crate) use _codecs::module_def;

#[pymodule]
mod _codecs {
    use crate::codecs::{ErrorsHandler, PyDecodeContext, PyEncodeContext};
    use crate::common::encodings;
    use crate::common::wtf8::Wtf8Buf;
    use crate::{
        AsObject, PyObjectRef, PyResult, VirtualMachine,
        builtins::{PyStrRef, PyUtf8StrRef},
        codecs,
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
        vm.state.codec_registry.lookup_error(name.as_str(), vm)
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

    use crate::common::static_cell::StaticCell;
    #[inline]
    fn delegate_pycodecs(
        cell: &'static StaticCell<PyObjectRef>,
        name: &'static str,
        args: FuncArgs,
        vm: &VirtualMachine,
    ) -> PyResult {
        let f = cell.get_or_try_init(|| {
            let module = vm.import("_pycodecs", 0)?;
            module.get_attr(name, vm)
        })?;
        f.call(args, vm)
    }
    macro_rules! delegate_pycodecs {
        ($name:ident, $args:ident, $vm:ident) => {{
            rustpython_common::static_cell!(
                static FUNC: PyObjectRef;
            );
            delegate_pycodecs(&FUNC, stringify!($name), $args, $vm)
        }};
    }

    #[cfg(windows)]
    #[derive(FromArgs)]
    struct MbcsEncodeArgs {
        #[pyarg(positional)]
        s: PyStrRef,
        #[pyarg(positional, optional)]
        errors: Option<PyStrRef>,
    }

    #[cfg(windows)]
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
                std::ptr::null_mut(),
                0,
                core::ptr::null(),
                std::ptr::null_mut(),
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
                    std::ptr::null_mut()
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

    #[cfg(not(windows))]
    #[pyfunction]
    fn mbcs_encode(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        delegate_pycodecs!(mbcs_encode, args, vm)
    }

    #[cfg(windows)]
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

    #[cfg(windows)]
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
                std::ptr::null_mut(),
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
                    std::ptr::null_mut(),
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

    #[cfg(not(windows))]
    #[pyfunction]
    fn mbcs_decode(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        delegate_pycodecs!(mbcs_decode, args, vm)
    }

    #[cfg(windows)]
    #[derive(FromArgs)]
    struct OemEncodeArgs {
        #[pyarg(positional)]
        s: PyStrRef,
        #[pyarg(positional, optional)]
        errors: Option<PyStrRef>,
    }

    #[cfg(windows)]
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
                std::ptr::null_mut(),
                0,
                core::ptr::null(),
                std::ptr::null_mut(),
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
                    std::ptr::null_mut()
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

    #[cfg(not(windows))]
    #[pyfunction]
    fn oem_encode(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        delegate_pycodecs!(oem_encode, args, vm)
    }

    #[cfg(windows)]
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

    #[cfg(windows)]
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
                std::ptr::null_mut(),
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
                    std::ptr::null_mut(),
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

    #[cfg(not(windows))]
    #[pyfunction]
    fn oem_decode(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        delegate_pycodecs!(oem_decode, args, vm)
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

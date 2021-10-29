pub(crate) use _codecs::make_module;

#[pymodule]
mod _codecs {
    use crate::common::encodings;
    use crate::{
        builtins::{PyBaseExceptionRef, PyBytes, PyBytesRef, PyStr, PyStrRef, PyTuple},
        codecs,
        function::{ArgBytesLike, FuncArgs},
        IdProtocol, PyObject, PyObjectRef, PyResult, TryFromBorrowedObject, VirtualMachine,
    };
    use std::ops::Range;

    #[pyfunction]
    fn register(search_function: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        vm.state.codec_registry.register(search_function, vm)
    }

    #[pyfunction]
    fn lookup(encoding: PyStrRef, vm: &VirtualMachine) -> PyResult {
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
    fn register_error(name: PyStrRef, handler: PyObjectRef, vm: &VirtualMachine) {
        vm.state
            .codec_registry
            .register_error(name.as_str().to_owned(), handler);
    }

    #[pyfunction]
    fn lookup_error(name: PyStrRef, vm: &VirtualMachine) -> PyResult {
        vm.state.codec_registry.lookup_error(name.as_str(), vm)
    }

    struct ErrorsHandler<'a> {
        vm: &'a VirtualMachine,
        encoding: &'a str,
        errors: Option<PyStrRef>,
        handler: once_cell::unsync::OnceCell<PyObjectRef>,
    }
    impl<'a> ErrorsHandler<'a> {
        #[inline]
        fn new(encoding: &'a str, errors: Option<PyStrRef>, vm: &'a VirtualMachine) -> Self {
            ErrorsHandler {
                vm,
                encoding,
                errors,
                handler: Default::default(),
            }
        }
        #[inline]
        fn handler_func(&self) -> PyResult<&PyObject> {
            let vm = self.vm;
            Ok(self.handler.get_or_try_init(|| {
                let errors = self.errors.as_ref().map_or("strict", |s| s.as_str());
                vm.state.codec_registry.lookup_error(errors, vm)
            })?)
        }
    }
    impl encodings::StrBuffer for PyStrRef {
        fn is_ascii(&self) -> bool {
            PyStr::is_ascii(self)
        }
    }
    impl<'vm> encodings::ErrorHandler for ErrorsHandler<'vm> {
        type Error = PyBaseExceptionRef;
        type StrBuf = PyStrRef;
        type BytesBuf = PyBytesRef;

        fn handle_encode_error(
            &self,
            data: &str,
            char_range: Range<usize>,
            reason: &str,
        ) -> PyResult<(encodings::EncodeReplace<PyStrRef, PyBytesRef>, usize)> {
            let vm = self.vm;
            let data_str = vm.ctx.new_str(data).into();
            let encode_exc = vm.new_exception(
                vm.ctx.exceptions.unicode_encode_error.clone(),
                vec![
                    vm.ctx.new_str(self.encoding).into(),
                    data_str,
                    vm.ctx.new_int(char_range.start).into(),
                    vm.ctx.new_int(char_range.end).into(),
                    vm.ctx.new_str(reason).into(),
                ],
            );
            let res = vm.invoke(self.handler_func()?, (encode_exc,))?;
            let tuple_err = || {
                vm.new_type_error(
                    "encoding error handler must return (str/bytes, int) tuple".to_owned(),
                )
            };
            let (replace, restart) = match res.payload::<PyTuple>().map(|tup| tup.as_slice()) {
                Some([replace, restart]) => (replace.clone(), restart),
                _ => return Err(tuple_err()),
            };
            let replace = match_class!(match replace {
                s @ PyStr => encodings::EncodeReplace::Str(s),
                b @ PyBytes => encodings::EncodeReplace::Bytes(b),
                _ => return Err(tuple_err()),
            });
            let restart = isize::try_from_borrowed_object(vm, restart).map_err(|_| tuple_err())?;
            let restart = if restart < 0 {
                // will still be out of bounds if it underflows ¯\_(ツ)_/¯
                data.len().wrapping_sub(restart.unsigned_abs())
            } else {
                restart as usize
            };
            Ok((replace, restart))
        }

        fn handle_decode_error(
            &self,
            data: &[u8],
            byte_range: Range<usize>,
            reason: &str,
        ) -> PyResult<(PyStrRef, Option<PyBytesRef>, usize)> {
            let vm = self.vm;
            let data_bytes: PyObjectRef = vm.ctx.new_bytes(data.to_vec()).into();
            let decode_exc = vm.new_exception(
                vm.ctx.exceptions.unicode_decode_error.clone(),
                vec![
                    vm.ctx.new_str(self.encoding).into(),
                    data_bytes.clone(),
                    vm.ctx.new_int(byte_range.start).into(),
                    vm.ctx.new_int(byte_range.end).into(),
                    vm.ctx.new_str(reason).into(),
                ],
            );
            let res = vm.invoke(self.handler_func()?, (decode_exc.clone(),))?;
            let new_data = decode_exc
                .get_arg(1)
                .ok_or_else(|| vm.new_type_error("object attribute not set".to_owned()))?;
            let new_data = if new_data.is(&data_bytes) {
                None
            } else {
                let new_data: PyBytesRef = new_data
                    .downcast()
                    .map_err(|_| vm.new_type_error("object attribute must be bytes".to_owned()))?;
                Some(new_data)
            };
            let data = new_data.as_ref().map_or(data, |s| s.as_ref());
            let tuple_err = || {
                vm.new_type_error("decoding error handler must return (str, int) tuple".to_owned())
            };
            match res.payload::<PyTuple>().map(|tup| tup.as_slice()) {
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
                    Ok((replace, new_data, restart))
                }
                _ => Err(tuple_err()),
            }
        }

        fn error_oob_restart(&self, i: usize) -> PyBaseExceptionRef {
            self.vm
                .new_index_error(format!("position {} from error handler out of bounds", i))
        }

        fn error_encoding(
            &self,
            data: &str,
            char_range: Range<usize>,
            reason: &str,
        ) -> Self::Error {
            let vm = self.vm;
            vm.new_exception(
                vm.ctx.exceptions.unicode_encode_error.clone(),
                vec![
                    vm.ctx.new_str(self.encoding).into(),
                    vm.ctx.new_str(data).into(),
                    vm.ctx.new_int(char_range.start).into(),
                    vm.ctx.new_int(char_range.end).into(),
                    vm.ctx.new_str(reason).into(),
                ],
            )
        }
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
        fn encode<'a, F>(self, name: &'a str, encode: F, vm: &'a VirtualMachine) -> EncodeResult
        where
            F: FnOnce(&str, &ErrorsHandler<'a>) -> PyResult<Vec<u8>>,
        {
            let errors = ErrorsHandler::new(name, self.errors, vm);
            let encoded = encode(self.s.as_str(), &errors)?;
            Ok((encoded, self.s.char_len()))
        }
    }

    type DecodeResult = PyResult<(String, usize)>;

    #[derive(FromArgs)]
    struct DecodeArgs {
        #[pyarg(positional)]
        data: ArgBytesLike,
        #[pyarg(positional, optional)]
        errors: Option<PyStrRef>,
        #[pyarg(positional, default = "false")]
        final_decode: bool,
    }

    impl DecodeArgs {
        #[inline]
        fn decode<'a, F>(self, name: &'a str, decode: F, vm: &'a VirtualMachine) -> DecodeResult
        where
            F: FnOnce(&[u8], &ErrorsHandler<'a>, bool) -> DecodeResult,
        {
            let data = self.data.borrow_buf();
            let errors = ErrorsHandler::new(name, self.errors, vm);
            decode(&data, &errors, self.final_decode)
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
        fn decode<'a, F>(self, name: &'a str, decode: F, vm: &'a VirtualMachine) -> DecodeResult
        where
            F: FnOnce(&[u8], &ErrorsHandler<'a>) -> DecodeResult,
        {
            let data = self.data.borrow_buf();
            let errors = ErrorsHandler::new(name, self.errors, vm);
            decode(&data, &errors)
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
        do_codec!(utf8::encode, args, vm)
    }

    #[pyfunction]
    fn utf_8_decode(args: DecodeArgs, vm: &VirtualMachine) -> DecodeResult {
        do_codec!(utf8::decode, args, vm)
    }

    #[pyfunction]
    fn latin_1_encode(args: EncodeArgs, vm: &VirtualMachine) -> EncodeResult {
        if args.s.is_ascii() {
            return Ok((args.s.as_str().as_bytes().to_vec(), args.s.byte_len()));
        }
        do_codec!(latin_1::encode, args, vm)
    }

    #[pyfunction]
    fn latin_1_decode(args: DecodeArgsNoFinal, vm: &VirtualMachine) -> DecodeResult {
        do_codec!(latin_1::decode, args, vm)
    }

    #[pyfunction]
    fn ascii_encode(args: EncodeArgs, vm: &VirtualMachine) -> EncodeResult {
        if args.s.is_ascii() {
            return Ok((args.s.as_str().as_bytes().to_vec(), args.s.byte_len()));
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
        name: &str,
        args: FuncArgs,
        vm: &VirtualMachine,
    ) -> PyResult {
        let f = cell.get_or_try_init(|| {
            let module = vm.import("_pycodecs", None, 0)?;
            module.get_attr(name, vm)
        })?;
        vm.invoke(f, args)
    }
    macro_rules! delegate_pycodecs {
        ($name:ident, $args:ident, $vm:ident) => {{
            rustpython_common::static_cell!(
                static FUNC: PyObjectRef;
            );
            delegate_pycodecs(&FUNC, stringify!($name), $args, $vm)
        }};
    }

    #[pyfunction]
    fn mbcs_encode(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        delegate_pycodecs!(mbcs_encode, args, vm)
    }
    #[pyfunction]
    fn mbcs_decode(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        delegate_pycodecs!(mbcs_decode, args, vm)
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
    // TODO: utf-32 functions
}

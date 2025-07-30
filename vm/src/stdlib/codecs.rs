pub(crate) use _codecs::make_module;

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
    fn register_error(name: PyStrRef, handler: PyObjectRef, vm: &VirtualMachine) {
        vm.state
            .codec_registry
            .register_error(name.as_str().to_owned(), handler);
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
        if args.s.is_ascii() {
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
        if args.s.is_ascii() {
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

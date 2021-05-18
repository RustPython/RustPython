pub(crate) use _codecs::make_module;

#[pymodule]
mod _codecs {
    use std::ops::Range;

    use crate::builtins::{PyBytesRef, PyStr, PyStrRef, PyTuple};
    use crate::byteslike::PyBytesLike;
    use crate::codecs;
    use crate::common::encodings::{self, utf8};
    use crate::exceptions::PyBaseExceptionRef;
    use crate::function::{FuncArgs, OptionalArg, OptionalOption};
    use crate::VirtualMachine;
    use crate::{IdProtocol, PyObjectRef, PyResult, TryFromObject};

    #[pyfunction]
    fn register(search_function: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        vm.state.codec_registry.register(search_function, vm)
    }

    #[pyfunction]
    fn lookup(encoding: PyStrRef, vm: &VirtualMachine) -> PyResult {
        vm.state
            .codec_registry
            .lookup(encoding.as_str(), vm)
            .map(|codec| codec.into_tuple().into_object())
    }

    #[pyfunction]
    fn encode(
        obj: PyObjectRef,
        encoding: OptionalOption<PyStrRef>,
        errors: OptionalOption<PyStrRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let encoding = encoding.flatten();
        let encoding = encoding
            .as_ref()
            .map_or(codecs::DEFAULT_ENCODING, |s| s.as_str());
        vm.state
            .codec_registry
            .encode(obj, encoding, errors.flatten(), vm)
    }

    #[pyfunction]
    fn decode(
        obj: PyObjectRef,
        encoding: OptionalOption<PyStrRef>,
        errors: OptionalOption<PyStrRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let encoding = encoding.flatten();
        let encoding = encoding
            .as_ref()
            .map_or(codecs::DEFAULT_ENCODING, |s| s.as_str());
        vm.state
            .codec_registry
            .decode(obj, encoding, errors.flatten(), vm)
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
        errors: Option<&'a PyStrRef>,
        handler: once_cell::unsync::OnceCell<PyObjectRef>,
    }
    impl<'a> ErrorsHandler<'a> {
        fn new(encoding: &'a str, errors: Option<&'a PyStrRef>, vm: &'a VirtualMachine) -> Self {
            ErrorsHandler {
                vm,
                encoding,
                errors,
                handler: Default::default(),
            }
        }
        fn handler_func(&self) -> PyResult<&PyObjectRef> {
            let vm = self.vm;
            self.handler.get_or_try_init(|| {
                vm.state
                    .codec_registry
                    .lookup_error(self.errors.map_or("strict", |s| s.as_ref()), vm)
            })
        }
    }
    impl<'vm> encodings::ErrorHandler for ErrorsHandler<'vm> {
        type Error = PyBaseExceptionRef;
        type StrBuf = PyStrRef;
        type BytesBuf = PyBytesRef;

        fn handle_encode_error(
            &self,
            _byte_range: Range<usize>,
            _reason: &str,
        ) -> PyResult<(encodings::EncodeReplace<PyStrRef, PyBytesRef>, usize)> {
            // we don't use common::encodings to encode anything yet, so this can't
            // get called until we do
            todo!()
        }

        fn handle_decode_error(
            &self,
            data: &[u8],
            byte_range: Range<usize>,
            reason: &str,
        ) -> PyResult<(PyStrRef, Option<PyBytesRef>, usize)> {
            let vm = self.vm;
            let data_bytes = vm.ctx.new_bytes(data.to_vec());
            let decode_exc = vm.new_exception(
                vm.ctx.exceptions.unicode_decode_error.clone(),
                vec![
                    vm.ctx.new_str(self.encoding),
                    data_bytes.clone(),
                    vm.ctx.new_int(byte_range.start),
                    vm.ctx.new_int(byte_range.end),
                    vm.ctx.new_str(reason),
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
                        .clone();
                    let restart =
                        isize::try_from_object(vm, restart.clone()).map_err(|_| tuple_err())?;
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
    }

    #[pyfunction]
    fn utf_8_encode(s: PyStrRef, _errors: OptionalArg<PyStrRef>) -> (Vec<u8>, usize) {
        (s.as_str().as_bytes().to_vec(), s.char_len())
    }

    #[pyfunction]
    fn utf_8_decode(
        data: PyBytesLike,
        errors: OptionalArg<PyStrRef>,
        final_decode: OptionalArg<bool>,
        vm: &VirtualMachine,
    ) -> PyResult<(String, usize)> {
        let errors = ErrorsHandler::new("utf-8", errors.as_ref().into_option(), vm);
        data.with_ref(|data| utf8::decode(data, &errors, final_decode.unwrap_or(false)))
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
            vm.get_attribute(module, name)
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
    fn latin_1_encode(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        delegate_pycodecs!(latin_1_encode, args, vm)
    }
    #[pyfunction]
    fn latin_1_decode(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        delegate_pycodecs!(latin_1_decode, args, vm)
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
    fn ascii_encode(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        delegate_pycodecs!(ascii_encode, args, vm)
    }
    #[pyfunction]
    fn ascii_decode(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        delegate_pycodecs!(ascii_decode, args, vm)
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

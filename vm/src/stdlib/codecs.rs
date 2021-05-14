pub(crate) use _codecs::make_module;

#[pymodule]
mod _codecs {
    use crate::builtins::PyStrRef;
    use crate::codecs;
    use crate::common::borrow::BorrowValue;
    use crate::function::{FuncArgs, OptionalArg, OptionalOption};
    use crate::pyobject::{PyObjectRef, PyResult};
    use crate::VirtualMachine;

    #[pyfunction]
    fn register(search_function: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        vm.state.codec_registry.register(search_function, vm)
    }

    #[pyfunction]
    fn lookup(encoding: PyStrRef, vm: &VirtualMachine) -> PyResult {
        vm.state
            .codec_registry
            .lookup(encoding.borrow_value(), vm)
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
            .map_or(codecs::DEFAULT_ENCODING, |s| s.borrow_value());
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
            .map_or(codecs::DEFAULT_ENCODING, |s| s.borrow_value());
        vm.state
            .codec_registry
            .decode(obj, encoding, errors.flatten(), vm)
    }

    #[pyfunction]
    fn _forget_codec(encoding: PyStrRef, vm: &VirtualMachine) {
        vm.state.codec_registry.forget(encoding.borrow_value());
    }

    #[pyfunction]
    fn register_error(name: PyStrRef, handler: PyObjectRef, vm: &VirtualMachine) {
        vm.state
            .codec_registry
            .register_error(name.borrow_value().to_owned(), handler);
    }

    #[pyfunction]
    fn lookup_error(name: PyStrRef, vm: &VirtualMachine) -> PyResult {
        vm.state
            .codec_registry
            .lookup_error(name.borrow_value(), vm)
    }

    #[pyfunction]
    fn utf_8_encode(s: PyStrRef, _errors: OptionalArg<PyStrRef>) -> (Vec<u8>, usize) {
        (s.borrow_value().as_bytes().to_vec(), s.char_len())
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
    fn utf_8_decode(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        delegate_pycodecs!(utf_8_decode, args, vm)
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

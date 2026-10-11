// spell-checker:ignore hexlify unhexlify uuencodes rlecode rledecode

pub(super) use decl::crc32;
pub(crate) use decl::module_def;

use rustpython_common::binascii::{Base64DecodeError, Error};
use rustpython_vm::{VirtualMachine, builtins::PyBaseExceptionRef};

#[pymodule(name = "binascii")]
mod decl {
    use super::new_binascii_error;
    use crate::vm::{
        AsObject, Py, PyObjectRef, PyRef, PyResult, VirtualMachine,
        builtins::{PyBytes, PyBytesRef, PyDict, PyModule, PyTypeRef},
        function::{
            ArgAsciiBuffer, ArgBytesLike, ArgIndex, ArgIntoBool, ItemDoc, OptionalArg, PyMethodDef,
            PyMethodFlags,
        },
    };
    use rustpython_common::binascii;

    pub(crate) fn module_exec(vm: &VirtualMachine, module: &Py<PyModule>) -> PyResult<()> {
        __module_exec(vm, module);
        module.set_native_state(vm.ctx.new_dict().into());
        const DECODE: &PyMethodDef = &PyMethodDef::new_const(
            "a2b_base64",
            a2b_base64,
            PyMethodFlags::empty(),
            ItemDoc::static_text(
                "a2b_base64($module, data, /, *, strict_mode=<unrepresentable>, padded=True, alphabet=BASE64_ALPHABET, ignorechars=<unrepresentable>, canonical=False)\n--\n\nDecode a line of base64 data.",
            ),
        );
        let function = DECODE.build_bound_function(&vm.ctx, module.to_owned().into());
        function
            .as_object()
            .set_attr("__module__", vm.ctx.new_str("binascii"), vm)?;
        module.dict().set_item("a2b_base64", function.into(), vm)?;
        Ok(())
    }

    fn reverse_table(
        module: &Py<PyModule>,
        alphabet: &PyBytesRef,
        vm: &VirtualMachine,
    ) -> PyResult<[u8; 256]> {
        let bytes: &[u8; 64] = alphabet
            .as_bytes()
            .try_into()
            .map_err(|_| vm.new_value_error("alphabet must have length 64"))?;
        let cache = module
            .native_state()
            .ok_or_else(|| vm.new_runtime_error("Missing binascii state"))?
            .downcast::<PyDict>()
            .map_err(|_| vm.new_runtime_error("Broken binascii cache"))?;
        // The actual bytes object is the key: subclass hash/equality callbacks
        // are part of CPython's cache lookup and insertion semantics.
        if let Some(value) = cache.get_item_opt(alphabet.as_object(), vm)? {
            return value
                .downcast_ref::<PyBytes>()
                .and_then(|value| value.as_bytes().try_into().ok())
                .ok_or_else(|| vm.new_runtime_error("Broken binascii cache"));
        }
        let table = binascii::base64_reverse_table(bytes);
        cache.set_item(
            alphabet.as_object(),
            vm.ctx.new_bytes(table.to_vec()).into(),
            vm,
        )?;
        Ok(table)
    }

    #[pyattr(name = "Error", once)]
    pub(super) fn error_type(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.new_exception_type(
            "binascii",
            "Error",
            Some(vec![vm.ctx.exceptions.value_error.to_owned()]),
        )
    }

    #[pyattr(name = "Incomplete", once)]
    fn incomplete_type(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.new_exception_type("binascii", "Incomplete", None)
    }

    #[pyfunction(name = "b2a_hex")]
    #[pyfunction]
    fn hexlify(
        data: ArgBytesLike,
        sep: OptionalArg<ArgAsciiBuffer>,
        bytes_per_sep: OptionalArg<isize>,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<u8>> {
        let sep = match sep {
            OptionalArg::Present(sep) => sep.with_ref(|sep| {
                let [sep] = sep else {
                    return Err(vm.new_value_error("sep must be length 1."));
                };
                if !sep.is_ascii() {
                    return Err(vm.new_value_error("sep must be ASCII."));
                }
                Ok(Some(*sep))
            })?,
            OptionalArg::Missing => None,
        };
        let bytes_per_sep = bytes_per_sep.unwrap_or(1);
        Ok(data.with_ref(|bytes| binascii::hexlify(bytes, sep, bytes_per_sep)))
    }

    #[pyfunction(name = "a2b_hex")]
    #[pyfunction]
    fn unhexlify(hexstr: ArgAsciiBuffer, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        hexstr
            .with_ref(binascii::unhexlify)
            .map_err(|e| new_binascii_error(e, vm))
    }

    #[derive(FromArgs)]
    struct Crc32Args {
        #[pyarg(positional)]
        data: ArgBytesLike,
        #[pyarg(positional, default = 0)]
        crc: ArgIndex,
    }

    pub(crate) fn crc32(data: ArgBytesLike, crc: ArgIndex) -> u32 {
        let crc = crc.into_int_ref().as_u32_mask();
        data.with_ref(|bytes| binascii::crc32(bytes, crc))
    }

    #[pyfunction(name = "crc32")]
    fn crc32_py(args: Crc32Args) -> u32 {
        let Crc32Args { data, crc } = args;
        crc32(data, crc)
    }

    #[pyfunction]
    pub(crate) fn crc_hqx(data: ArgBytesLike, crc: ArgIndex) -> u32 {
        data.with_ref(|bytes| binascii::crc_hqx(bytes, crc.into_int_ref().as_u32_mask()))
    }

    #[pyattr(name = "BASE64_ALPHABET")]
    fn base64_alphabet(vm: &VirtualMachine) -> PyBytesRef {
        vm.ctx.new_bytes(binascii::BASE64_ALPHABET.to_vec())
    }

    #[pyattr(name = "URLSAFE_BASE64_ALPHABET")]
    fn urlsafe_base64_alphabet(vm: &VirtualMachine) -> PyBytesRef {
        vm.ctx
            .new_bytes(b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_".to_vec())
    }

    #[derive(FromArgs)]
    struct A2bBase64Args {
        #[pyarg(positional)]
        data: ArgAsciiBuffer,
        #[pyarg(named, optional)]
        strict_mode: OptionalArg<ArgIntoBool>,
        #[pyarg(named, default = ArgIntoBool::TRUE, py_default = "True")]
        padded: ArgIntoBool,
        #[pyarg(named, optional, py_default = "BASE64_ALPHABET")]
        alphabet: OptionalArg<PyBytesRef>,
        #[pyarg(named, optional)]
        ignorechars: OptionalArg<ArgBytesLike>,
        #[pyarg(named, optional, py_default = "False")]
        canonical: OptionalArg<PyObjectRef>,
    }

    fn a2b_base64(
        module: PyRef<PyModule>,
        args: A2bBase64Args,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<u8>> {
        let A2bBase64Args {
            data,
            strict_mode,
            padded,
            alphabet,
            ignorechars,
            canonical,
        } = args;
        let strict_mode = match strict_mode {
            OptionalArg::Present(value) => value.into_bool(),
            OptionalArg::Missing => matches!(&ignorechars, OptionalArg::Present(_)),
        };
        let result = (|| {
            // Convert the last Clinic argument while both acquired buffers are alive.
            let canonical = match canonical {
                OptionalArg::Present(value) => value.try_to_bool(vm)?,
                OptionalArg::Missing => false,
            };
            const DEFAULT_TABLE: [u8; 256] =
                binascii::base64_reverse_table(binascii::BASE64_ALPHABET);
            let table = match &alphabet {
                OptionalArg::Present(value) => reverse_table(&module, value, vm)?,
                OptionalArg::Missing => DEFAULT_TABLE,
            };
            let mut ignored = [false; 256];
            if strict_mode && let OptionalArg::Present(value) = &ignorechars {
                value.with_ref(|bytes| {
                    for &symbol in bytes {
                        ignored[symbol as usize] = true;
                    }
                });
            }
            data.with_ref(|bytes| {
                binascii::a2b_base64(
                    bytes,
                    binascii::Base64DecodeOptions {
                        strict_mode,
                        padded: padded.into_bool(),
                        table: &table,
                        ignored: &ignored,
                        canonical,
                    },
                )
            })
            .map_err(|e| new_binascii_error(e, vm))
        })();
        // Match Argument Clinic cleanup order, including failures in __bool__.
        drop(data);
        drop(ignorechars);
        result
    }

    #[derive(FromArgs)]
    struct B2aBase64Args {
        #[pyarg(positional)]
        data: ArgBytesLike,
        #[pyarg(named, default = ArgIntoBool::TRUE, py_default = "True")]
        padded: ArgIntoBool,
        #[pyarg(named, default = 0)]
        wrapcol: usize,
        #[pyarg(named, default = ArgIntoBool::TRUE, py_default = "True")]
        newline: ArgIntoBool,
        #[pyarg(named, optional, py_default = "BASE64_ALPHABET")]
        alphabet: OptionalArg<ArgBytesLike>,
    }

    #[pyfunction]
    fn b2a_base64(args: B2aBase64Args, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        let B2aBase64Args {
            data,
            padded,
            wrapcol,
            newline,
            alphabet,
        } = args;
        let copy_alphabet = |bytes: &[u8]| -> PyResult<[u8; 64]> {
            bytes
                .try_into()
                .map_err(|_| vm.new_value_error("alphabet must have length 64"))
        };
        let alphabet_bytes = match &alphabet {
            OptionalArg::Present(value) => value.with_ref(copy_alphabet),
            OptionalArg::Missing => Ok(*binascii::BASE64_ALPHABET),
        };
        let result = alphabet_bytes.and_then(|alphabet_bytes| {
            data.with_ref(|bytes| {
                binascii::b2a_base64(
                    bytes,
                    padded.into_bool(),
                    wrapcol,
                    newline.into_bool(),
                    &alphabet_bytes,
                )
            })
            .map_err(|e| new_binascii_error(e, vm))
        });
        drop(data);
        drop(alphabet);
        result
    }

    #[derive(FromArgs)]
    struct A2bQpArgs {
        #[pyarg(any)]
        data: ArgAsciiBuffer,
        #[pyarg(any, default)]
        header: bool,
    }

    #[pyfunction]
    fn a2b_qp(args: A2bQpArgs) -> Vec<u8> {
        let A2bQpArgs { data, header } = args;
        data.with_ref(|buffer| binascii::a2b_qp(buffer, header))
    }

    #[derive(FromArgs)]
    struct B2aQpArgs {
        #[pyarg(any)]
        data: ArgBytesLike,
        #[pyarg(any, default)]
        quotetabs: bool,
        #[pyarg(any, default = true)]
        istext: bool,
        #[pyarg(any, default)]
        header: bool,
    }

    #[pyfunction]
    fn b2a_qp(args: B2aQpArgs) -> Vec<u8> {
        let B2aQpArgs {
            data,
            quotetabs,
            istext,
            header,
        } = args;
        data.with_ref(|buf| binascii::b2a_qp(buf, quotetabs, istext, header))
    }

    #[pyfunction]
    fn a2b_uu(data: ArgAsciiBuffer, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        data.with_ref(binascii::a2b_uu)
            .map_err(|e| new_binascii_error(e, vm))
    }

    #[derive(FromArgs)]
    struct BacktickArg {
        #[pyarg(named, default)]
        backtick: bool,
    }

    #[pyfunction]
    fn b2a_uu(
        data: ArgBytesLike,
        BacktickArg { backtick }: BacktickArg,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<u8>> {
        data.with_ref(|b| binascii::b2a_uu(b, backtick))
            .map_err(|e| new_binascii_error(e, vm))
    }
}

/// Builds the `binascii.Error` a transform failure maps to.
fn new_binascii_error(error: Error, vm: &VirtualMachine) -> PyBaseExceptionRef {
    let message = match error {
        Error::OddLengthString => "Odd-length string".to_owned(),
        Error::NonHexadecimalDigit => "Non-hexadecimal digit found".to_owned(),
        Error::MissingLengthByte => "Missing length byte".to_owned(),
        Error::IllegalChar => "Illegal char".to_owned(),
        Error::TrailingGarbage => "Trailing garbage".to_owned(),
        Error::TooLong => "At most 45 bytes at once".to_owned(),
        Error::Base64(e) => base64_message(e),
        Error::TooMuchBase64Data => "Too much data for base64".to_owned(),
        Error::Memory => return vm.no_memory_error(),
    };
    vm.new_exception_msg(decl::error_type(vm), message.into())
}

fn base64_message(error: Base64DecodeError) -> String {
    match error {
        Base64DecodeError::LeadingPaddingNotAllowed => "Leading padding not allowed".to_owned(),
        Base64DecodeError::ExcessPaddingNotAllowed => "Excess padding not allowed".to_owned(),
        Base64DecodeError::OnlyBase64DataAllowed => "Only base64 data is allowed".to_owned(),
        Base64DecodeError::ExcessDataAfterPadding => "Excess data after padding".to_owned(),
        Base64DecodeError::DiscontinuousPaddingNotAllowed => {
            "Discontinuous padding not allowed".to_owned()
        }
        Base64DecodeError::InvalidLastSymbol { index } => format!(
            "Invalid base64-encoded string: number of data characters ({index}) cannot be 1 more than a multiple of 4"
        ),
        Base64DecodeError::IncorrectPadding => "Incorrect padding".to_owned(),
        Base64DecodeError::PaddingNotAllowed => "Padding not allowed".to_owned(),
        Base64DecodeError::NonZeroPaddingBits => "Non-zero padding bits".to_owned(),
    }
}

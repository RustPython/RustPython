// spell-checker:ignore hexlify unhexlify uuencodes rlecode rledecode ABCDEFGHIJKLMNPQRSTUVXYZ abcdefhijklmpqr

mod base_n;

pub(super) use decl::crc32;
pub(crate) use decl::module_def;

use rustpython_common::binascii::{Base64DecodeError, Error};
use rustpython_vm::{VirtualMachine, builtins::PyBaseExceptionRef};

#[pymodule(name = "binascii")]
mod decl {
    use super::{base_n, new_binascii_error};
    use crate::vm::{
        PyObjectRef, PyResult, VirtualMachine,
        builtins::{PyBytesRef, PyTypeRef},
        function::{ArgAsciiBuffer, ArgBytesLike, ArgIndex, ArgIntoBool, OptionalArg},
    };
    use rustpython_common::binascii;

    #[pyattr(name = "BASE64_ALPHABET")]
    fn base64_alphabet(vm: &VirtualMachine) -> PyBytesRef {
        vm.ctx
            .new_bytes(b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/".to_vec())
    }

    #[pyattr(name = "URLSAFE_BASE64_ALPHABET")]
    fn urlsafe_base64_alphabet(vm: &VirtualMachine) -> PyBytesRef {
        vm.ctx
            .new_bytes(b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_".to_vec())
    }

    #[pyattr(name = "CRYPT_ALPHABET")]
    fn crypt_alphabet(vm: &VirtualMachine) -> PyBytesRef {
        vm.ctx
            .new_bytes(b"./0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz".to_vec())
    }

    #[pyattr(name = "UU_ALPHABET")]
    fn uu_alphabet(vm: &VirtualMachine) -> PyBytesRef {
        vm.ctx.new_bytes(
            b" !\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_".to_vec(),
        )
    }

    #[pyattr(name = "BINHEX_ALPHABET")]
    fn binhex_alphabet(vm: &VirtualMachine) -> PyBytesRef {
        vm.ctx.new_bytes(
            b"!\"#$%&'()*+,-012345689@ABCDEFGHIJKLMNPQRSTUVXYZ[`abcdefhijklmpqr".to_vec(),
        )
    }

    #[pyattr(name = "BASE85_ALPHABET")]
    fn base85_alphabet(vm: &VirtualMachine) -> PyBytesRef {
        vm.ctx.new_bytes(b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz!#$%&()*+-;<=>?@^_`{|}~".to_vec())
    }

    #[pyattr(name = "ASCII85_ALPHABET")]
    fn ascii85_alphabet(vm: &VirtualMachine) -> PyBytesRef {
        vm.ctx.new_bytes(b"!\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghijklmnopqrstu".to_vec())
    }

    #[pyattr(name = "Z85_ALPHABET")]
    fn z85_alphabet(vm: &VirtualMachine) -> PyBytesRef {
        vm.ctx.new_bytes(b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ.-:+=^!/*?&<>()[]{}@%$#".to_vec())
    }

    #[pyattr(name = "BASE32_ALPHABET")]
    fn base32_alphabet(vm: &VirtualMachine) -> PyBytesRef {
        vm.ctx
            .new_bytes(b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567".to_vec())
    }

    #[pyattr(name = "BASE32HEX_ALPHABET")]
    fn base32hex_alphabet(vm: &VirtualMachine) -> PyBytesRef {
        vm.ctx
            .new_bytes(b"0123456789ABCDEFGHIJKLMNOPQRSTUV".to_vec())
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

    #[derive(FromArgs)]
    struct HexlifyArgs {
        #[pyarg(any)]
        data: ArgBytesLike,
        #[pyarg(any, optional)]
        sep: OptionalArg<ArgAsciiBuffer>,
        #[pyarg(any, optional)]
        bytes_per_sep: OptionalArg<isize>,
    }

    #[pyfunction(name = "b2a_hex")]
    #[pyfunction]
    fn hexlify(args: HexlifyArgs, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        let HexlifyArgs {
            data,
            sep,
            bytes_per_sep,
        } = args;
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

    #[derive(FromArgs)]
    struct UnhexlifyArgs {
        #[pyarg(positional)]
        hexstr: ArgAsciiBuffer,
        #[pyarg(named, optional, py_default = "b''")]
        ignorechars: OptionalArg<ArgBytesLike>,
    }

    #[pyfunction(name = "a2b_hex")]
    #[pyfunction]
    fn unhexlify(args: UnhexlifyArgs, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        let ignorechars = ignored_argument(&args.ignorechars);
        let result = args
            .hexstr
            .with_ref(|data| base_n::decode_hex(data, &ignorechars));
        drop(args.hexstr);
        drop(args.ignorechars);
        base_n_result(result, vm)
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

    fn base_n_result(
        result: Result<Vec<u8>, base_n::Error>,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<u8>> {
        result.map_err(|error| match error {
            base_n::Error::InvalidAlphabet(size) => {
                vm.new_value_error(format!("alphabet must have length {size}"))
            }
            base_n::Error::Codec(message) => vm.new_exception_msg(error_type(vm), message.into()),
            base_n::Error::Memory => vm.no_memory_error(),
        })
    }

    fn encoder_alphabet(
        argument: &OptionalArg<ArgBytesLike>,
        default: &[u8],
    ) -> Result<Vec<u8>, base_n::Error> {
        match argument {
            OptionalArg::Present(value) => {
                value.with_ref(|bytes| base_n::copy_alphabet(bytes, default.len()))
            }
            OptionalArg::Missing => base_n::copy_alphabet(default, default.len()),
        }
    }

    fn decoder_alphabet(
        argument: &OptionalArg<PyBytesRef>,
        default: &[u8],
    ) -> Result<Vec<u8>, base_n::Error> {
        match argument {
            OptionalArg::Present(value) => base_n::copy_alphabet(value.as_bytes(), default.len()),
            OptionalArg::Missing => base_n::copy_alphabet(default, default.len()),
        }
    }

    fn ignored_argument(argument: &OptionalArg<ArgBytesLike>) -> [bool; 256] {
        match argument {
            OptionalArg::Present(value) => value.with_ref(base_n::ignored),
            OptionalArg::Missing => [false; 256],
        }
    }

    fn canonical_argument(
        argument: &OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<bool> {
        match argument {
            OptionalArg::Present(value) => value.try_to_bool(vm),
            OptionalArg::Missing => Ok(false),
        }
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

    #[pyfunction]
    fn a2b_base64(args: A2bBase64Args, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
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
            // This is the last Clinic conversion. Keep acquired exports alive
            // and reach ordered cleanup even when __bool__ raises.
            let canonical = canonical_argument(&canonical, vm)?;
            let result = decoder_alphabet(&alphabet, base_n::BASE64).and_then(|alphabet_bytes| {
                let ignore = if strict_mode {
                    ignored_argument(&ignorechars)
                } else {
                    [false; 256]
                };
                data.with_ref(|data| {
                    base_n::decode_bits(
                        data,
                        &alphabet_bytes,
                        &ignore,
                        base_n::BitDecodeOptions {
                            bits: 6,
                            padded: padded.into_bool(),
                            strict: strict_mode,
                            canonical,
                        },
                    )
                })
            });
            base_n_result(result, vm)
        })();
        // Match Argument Clinic cleanup: release data before auxiliary exports.
        drop(data);
        drop(ignorechars);
        result
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
        let result = encoder_alphabet(&alphabet, base_n::BASE64).and_then(|alphabet_bytes| {
            data.with_ref(|data| {
                base_n::encode_bits(
                    data,
                    6,
                    padded.into_bool(),
                    wrapcol,
                    newline.into_bool(),
                    &alphabet_bytes,
                )
            })
        });
        drop(data);
        drop(alphabet);
        base_n_result(result, vm)
    }

    #[derive(FromArgs)]
    struct B2aBase32Args {
        #[pyarg(positional)]
        data: ArgBytesLike,
        #[pyarg(named, default = ArgIntoBool::TRUE, py_default = "True")]
        padded: ArgIntoBool,
        #[pyarg(named, default = 0)]
        wrapcol: usize,
        #[pyarg(named, optional, py_default = "BASE32_ALPHABET")]
        alphabet: OptionalArg<ArgBytesLike>,
    }

    #[derive(FromArgs)]
    struct A2bBase32Args {
        #[pyarg(positional)]
        data: ArgAsciiBuffer,
        #[pyarg(named, default = ArgIntoBool::TRUE, py_default = "True")]
        padded: ArgIntoBool,
        #[pyarg(named, optional, py_default = "BASE32_ALPHABET")]
        alphabet: OptionalArg<PyBytesRef>,
        #[pyarg(named, optional, py_default = "b''")]
        ignorechars: OptionalArg<ArgBytesLike>,
        #[pyarg(named, optional, py_default = "False")]
        canonical: OptionalArg<PyObjectRef>,
    }

    #[pyfunction]
    fn a2b_base32(args: A2bBase32Args, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        let A2bBase32Args {
            data,
            padded,
            alphabet,
            ignorechars,
            canonical,
        } = args;
        let strict_mode = true;
        let result = (|| {
            // This is the last Clinic conversion. Keep acquired exports alive
            // and reach ordered cleanup even when __bool__ raises.
            let canonical = canonical_argument(&canonical, vm)?;
            let result = decoder_alphabet(&alphabet, base_n::BASE32).and_then(|alphabet_bytes| {
                let ignore = if strict_mode {
                    ignored_argument(&ignorechars)
                } else {
                    [false; 256]
                };
                data.with_ref(|data| {
                    base_n::decode_bits(
                        data,
                        &alphabet_bytes,
                        &ignore,
                        base_n::BitDecodeOptions {
                            bits: 5,
                            padded: padded.into_bool(),
                            strict: strict_mode,
                            canonical,
                        },
                    )
                })
            });
            base_n_result(result, vm)
        })();
        // Match Argument Clinic cleanup: release data before auxiliary exports.
        drop(data);
        drop(ignorechars);
        result
    }

    #[pyfunction]
    fn b2a_base32(args: B2aBase32Args, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        let B2aBase32Args {
            data,
            padded,
            wrapcol,
            alphabet,
        } = args;
        let result = encoder_alphabet(&alphabet, base_n::BASE32).and_then(|alphabet_bytes| {
            data.with_ref(|data| {
                base_n::encode_bits(data, 5, padded.into_bool(), wrapcol, false, &alphabet_bytes)
            })
        });
        drop(data);
        drop(alphabet);
        base_n_result(result, vm)
    }

    #[derive(FromArgs)]
    struct B2aBase85Args {
        #[pyarg(positional)]
        data: ArgBytesLike,
        #[pyarg(named, default = ArgIntoBool::FALSE, py_default = "False")]
        pad: ArgIntoBool,
        #[pyarg(named, default = 0)]
        wrapcol: usize,
        #[pyarg(named, optional, py_default = "BASE85_ALPHABET")]
        alphabet: OptionalArg<ArgBytesLike>,
    }

    #[derive(FromArgs)]
    struct A2bBase85Args {
        #[pyarg(positional)]
        data: ArgAsciiBuffer,
        #[pyarg(named, optional, py_default = "BASE85_ALPHABET")]
        alphabet: OptionalArg<PyBytesRef>,
        #[pyarg(named, optional, py_default = "b''")]
        ignorechars: OptionalArg<ArgBytesLike>,
        #[pyarg(named, optional, py_default = "False")]
        canonical: OptionalArg<PyObjectRef>,
    }

    #[pyfunction]
    fn a2b_base85(args: A2bBase85Args, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        let A2bBase85Args {
            data,
            alphabet,
            ignorechars,
            canonical,
        } = args;
        let result = (|| {
            // This is the last Clinic conversion. Keep acquired exports alive
            // and reach ordered cleanup even when __bool__ raises.
            let canonical = canonical_argument(&canonical, vm)?;
            let result = decoder_alphabet(&alphabet, base_n::BASE85).and_then(|alphabet_bytes| {
                let ignore = ignored_argument(&ignorechars);
                data.with_ref(|data| {
                    base_n::decode85(
                        data,
                        &alphabet_bytes,
                        &ignore,
                        canonical,
                        false,
                        false,
                        false,
                    )
                })
            });
            base_n_result(result, vm)
        })();
        drop(data);
        drop(ignorechars);
        result
    }

    #[pyfunction]
    fn b2a_base85(args: B2aBase85Args, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        let B2aBase85Args {
            data,
            pad,
            wrapcol,
            alphabet,
        } = args;
        let result = encoder_alphabet(&alphabet, base_n::BASE85).and_then(|alphabet_bytes| {
            data.with_ref(|data| {
                base_n::encode85(
                    data,
                    pad.into_bool(),
                    wrapcol,
                    &alphabet_bytes,
                    false,
                    false,
                    false,
                )
            })
        });
        drop(data);
        drop(alphabet);
        base_n_result(result, vm)
    }

    #[derive(FromArgs)]
    struct B2aAscii85Args {
        #[pyarg(positional)]
        data: ArgBytesLike,
        #[pyarg(named, default = ArgIntoBool::FALSE, py_default = "False")]
        foldspaces: ArgIntoBool,
        #[pyarg(named, default = 0)]
        wrapcol: usize,
        #[pyarg(named, default = ArgIntoBool::FALSE, py_default = "False")]
        pad: ArgIntoBool,
        #[pyarg(named, default = ArgIntoBool::FALSE, py_default = "False")]
        adobe: ArgIntoBool,
    }

    #[derive(FromArgs)]
    struct A2bAscii85Args {
        #[pyarg(positional)]
        data: ArgAsciiBuffer,
        #[pyarg(named, default = ArgIntoBool::FALSE, py_default = "False")]
        foldspaces: ArgIntoBool,
        #[pyarg(named, default = ArgIntoBool::FALSE, py_default = "False")]
        adobe: ArgIntoBool,
        #[pyarg(named, optional, py_default = "b''")]
        ignorechars: OptionalArg<ArgBytesLike>,
        #[pyarg(named, optional, py_default = "False")]
        canonical: OptionalArg<PyObjectRef>,
    }

    #[pyfunction]
    fn a2b_ascii85(args: A2bAscii85Args, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        let A2bAscii85Args {
            data,
            foldspaces,
            adobe,
            ignorechars,
            canonical,
        } = args;
        let result = (|| {
            // This is the last Clinic conversion. Keep acquired exports alive
            // and reach ordered cleanup even when __bool__ raises.
            let canonical = canonical_argument(&canonical, vm)?;
            let ignore = ignored_argument(&ignorechars);
            let result = data.with_ref(|data| {
                base_n::decode85(
                    data,
                    base_n::ASCII85,
                    &ignore,
                    canonical,
                    true,
                    foldspaces.into_bool(),
                    adobe.into_bool(),
                )
            });
            base_n_result(result, vm)
        })();
        drop(data);
        drop(ignorechars);
        result
    }

    #[pyfunction]
    fn b2a_ascii85(args: B2aAscii85Args, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        let B2aAscii85Args {
            data,
            foldspaces,
            wrapcol,
            pad,
            adobe,
        } = args;
        let result = data.with_ref(|data| {
            base_n::encode85(
                data,
                pad.into_bool(),
                wrapcol,
                base_n::ASCII85,
                true,
                foldspaces.into_bool(),
                adobe.into_bool(),
            )
        });
        base_n_result(result, vm)
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
    }
}

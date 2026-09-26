// spell-checker:ignore hexlify unhexlify uuencodes rlecode rledecode

pub(super) use decl::crc32;
pub(crate) use decl::module_def;

use rustpython_common::binascii::{Base64DecodeError, Error};
use rustpython_vm::{VirtualMachine, builtins::PyBaseExceptionRef};

#[pymodule(name = "binascii")]
mod decl {
    use super::new_binascii_error;
    use crate::vm::{
        PyResult, VirtualMachine,
        builtins::PyTypeRef,
        function::{ArgAsciiBuffer, ArgBytesLike, ArgIndex, OptionalArg},
    };
    use rustpython_common::binascii;

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

    #[derive(FromArgs)]
    struct NewlineArg {
        #[pyarg(named, default = true)]
        newline: bool,
    }

    #[derive(FromArgs)]
    struct A2bBase64Args {
        #[pyarg(positional)]
        data: ArgAsciiBuffer,
        #[pyarg(named, default)]
        strict_mode: bool,
    }

    #[pyfunction]
    fn a2b_base64(args: A2bBase64Args, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        let A2bBase64Args { data, strict_mode } = args;
        data.with_ref(|b| binascii::a2b_base64(b, strict_mode))
            .map_err(|e| new_binascii_error(e, vm))
    }

    #[pyfunction]
    fn b2a_base64(data: ArgBytesLike, NewlineArg { newline }: NewlineArg) -> Vec<u8> {
        data.with_ref(|bytes| binascii::b2a_base64(bytes, newline))
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

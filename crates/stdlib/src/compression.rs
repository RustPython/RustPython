//! internal shared module for compression libraries

use crate::vm::function::{ArgBytesLike, OptionalArg, PySsize};

#[derive(FromArgs)]
pub(crate) struct DecompressArgs {
    #[pyarg(positional)]
    data: ArgBytesLike,
    // Omitted max_length is 0, which means unlimited for this call.
    #[pyarg(any, optional, py_default = "0")]
    max_length: OptionalArg<PySsize>,
}

impl DecompressArgs {
    pub(crate) fn data(&self) -> crate::common::borrow::BorrowedValue<'_, [u8]> {
        self.data.borrow_buf()
    }
    pub(crate) fn raw_max_length(&self) -> Option<isize> {
        self.max_length.into_option()
    }
}

#[derive(FromArgs)]
pub(crate) struct DecompressorArgs {
    #[pyarg(any)]
    data: ArgBytesLike,
    // Missing max_length is unlimited, shown as -1.
    #[pyarg(any, optional, py_default = "-1")]
    max_length: OptionalArg<PySsize>,
}

impl DecompressorArgs {
    pub(crate) fn data(&self) -> crate::common::borrow::BorrowedValue<'_, [u8]> {
        self.data.borrow_buf()
    }

    pub(crate) fn max_length(&self) -> Option<usize> {
        self.max_length
            .into_option()
            .and_then(|value| usize::try_from(value).ok())
    }
}

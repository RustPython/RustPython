//! internal shared module for compression libraries

use crate::vm::function::{ArgBytesLike, ArgSize, OptionalArg};

#[derive(FromArgs)]
pub(crate) struct DecompressArgs {
    #[pyarg(positional)]
    data: ArgBytesLike,
    #[pyarg(any, optional)]
    max_length: OptionalArg<ArgSize>,
}

impl DecompressArgs {
    pub(crate) fn data(&self) -> crate::common::borrow::BorrowedValue<'_, [u8]> {
        self.data.borrow_buf()
    }
    pub(crate) fn raw_max_length(&self) -> Option<isize> {
        self.max_length.into_option().map(|ArgSize { value }| value)
    }

    // negative is None
    pub(crate) fn max_length(&self) -> Option<usize> {
        self.max_length
            .into_option()
            .and_then(|ArgSize { value }| usize::try_from(value).ok())
    }
}

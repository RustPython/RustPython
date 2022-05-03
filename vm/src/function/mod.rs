mod argument;
mod arithmetic;
mod buffer;
mod builtin;
mod either;
mod number;
mod protocol;

pub use argument::{
    ArgumentError, FromArgOptional, FromArgs, FuncArgs, IntoFuncArgs, KwArgs, OptionalArg,
    OptionalOption, PosArgs,
};
pub use arithmetic::{PyArithmeticValue, PyComparisonValue};
pub use buffer::{ArgAsciiBuffer, ArgBytesLike, ArgMemoryBuffer, ArgStrOrBytesLike};
pub use builtin::{IntoPyNativeFunc, OwnedParam, PyNativeFunc, RefParam};
pub use either::Either;
pub use number::{ArgIntoBool, ArgIntoComplex, ArgIntoFloat};
pub use protocol::{ArgCallable, ArgIterable, ArgMapping, ArgSequence};

use crate::{builtins::PyStr, convert::TryFromBorrowedObject, PyObject, PyResult, VirtualMachine};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ArgByteOrder {
    Big,
    Little,
}

impl TryFromBorrowedObject for ArgByteOrder {
    fn try_from_borrowed_object(vm: &VirtualMachine, obj: &PyObject) -> PyResult<Self> {
        obj.try_value_with(
            |s: &PyStr| match s.as_str() {
                "big" => Ok(Self::Big),
                "little" => Ok(Self::Little),
                _ => {
                    Err(vm.new_value_error("byteorder must be either 'little' or 'big'".to_owned()))
                }
            },
            vm,
        )
    }
}

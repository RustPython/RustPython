mod argument;
mod arithmetic;
mod buffer;
mod builtin;
mod either;
mod getset;
mod number;
mod protocol;

pub use argument::{
    ArgumentError, FromArgOptional, FromArgs, FuncArgs, IntoFuncArgs, KwArgs, OptionalArg,
    OptionalOption, PosArgs,
};
pub use arithmetic::{PyArithmeticValue, PyComparisonValue};
pub use buffer::{ArgAsciiBuffer, ArgBytesLike, ArgMemoryBuffer, ArgStrOrBytesLike};
pub(self) use builtin::{BorrowedParam, OwnedParam, RefParam};
pub use builtin::{IntoPyNativeFunc, PyNativeFunc};
pub use either::Either;
pub use getset::PySetterValue;
pub(super) use getset::{IntoPyGetterFunc, IntoPySetterFunc, PyGetterFunc, PySetterFunc};
pub use number::{ArgIndex, ArgIntoBool, ArgIntoComplex, ArgIntoFloat, ArgPrimitiveIndex, ArgSize};
pub use protocol::{ArgCallable, ArgIterable, ArgMapping, ArgSequence};

use crate::{builtins::PyStr, convert::TryFromBorrowedObject, PyObject, PyResult, VirtualMachine};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ArgByteOrder {
    Big,
    Little,
}

impl<'a> TryFromBorrowedObject<'a> for ArgByteOrder {
    fn try_from_borrowed_object(vm: &VirtualMachine, obj: &'a PyObject) -> PyResult<Self> {
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

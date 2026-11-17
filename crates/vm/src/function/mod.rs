mod argument;
mod arithmetic;
mod buffer;
mod builtin;
mod doctext;
mod either;
mod fspath;
mod getset;
mod legacy_args;
pub(crate) mod method;
mod number;
mod protocol;
mod signature;
mod time;

pub use argument::{
    ArgsMetadata, ArgumentError, BorrowedArgs, Callee, FromArgOptional, FromArgs,
    FromBorrowedArgOptional, FuncArgs, IntoFuncArgs, KwArgs, KwArgsMap, NameArgs, NameChanges,
    NameCoordinates, NameExcInfo, NameFields, NameIntegers, NameIterables, NameKeywords,
    NameKwargs, NameKwds, NameKws, NameObjects, NameObjs, NameOthers, NameValues, OptionalArg,
    OptionalArgDefault, OptionalOption, PosArgs, PositionalIterable, UnpackKeywords,
};
pub(crate) use argument::{
    arity_message, given_by_name_and_position_message, unexpected_keyword_message,
};
pub use arithmetic::{PyArithmeticValue, PyComparisonValue};
pub use buffer::{
    ArgAsciiBuffer, ArgBytesLike, ArgBytesOrStr, ArgContiguousBytesLike, ArgMemoryBuffer,
    ArgStrOrBytesLike,
};
pub use builtin::{IntoPyNativeFn, PyNativeFn, ReceiverFunction, static_func, static_raw_func};
pub use doctext::{ItemDoc, db_doc, plain_doc};
pub use either::Either;
pub use fspath::FsPath;
pub use getset::PySetterValue;
pub(super) use getset::{IntoPyGetterFunc, IntoPySetterFunc, PyGetterFunc, PySetterFunc};
pub use legacy_args::{ArgSlots, ArgSpec};
pub use method::{HeapMethodDef, PyMethodDef, PyMethodFlags};
pub(crate) use number::int_as_ssize;
pub use number::{
    ArgCInt, ArgIndex, ArgIntoBool, ArgIntoComplex, ArgIntoFloat, ArgSliceIndex, ArgStrictInt,
    PySize, PySsize,
};
pub use protocol::{ArgCallable, ArgIterable, ArgMapping, ArgSequence};
pub use signature::{
    DefaultRepr, Param, ParamKind, SigArg, choose_class_params, has_signature, internal_doc_bytes,
    internal_doc_len, real_signature, signature_from_flags, signature_prefix_bytes,
    signature_prefix_len,
};
pub use time::TimeoutSeconds;

use crate::{
    Py, PyObject, PyObjectRef, PyResult, VirtualMachine,
    builtins::{PyStr, PyUtf8Str, PyUtf8StrRef},
    convert::{TryFromBorrowedObject, TryFromObject},
    exceptions::nul_char_error,
};
use builtin::{BorrowedParam, OwnedParam, RefParam};

/// A `str` argument whose conversion raises `ValueError` when it holds a
/// null character.
#[derive(Debug, Clone)]
pub struct ArgNulFreeStr(PyUtf8StrRef);

impl ArgNulFreeStr {
    #[inline]
    #[must_use]
    pub fn into_utf8_ref(self) -> PyUtf8StrRef {
        self.0
    }

    fn checked(s: PyUtf8StrRef, vm: &VirtualMachine) -> PyResult<Self> {
        if s.as_str().contains('\0') {
            return Err(nul_char_error(vm));
        }
        Ok(Self(s))
    }
}

impl core::ops::Deref for ArgNulFreeStr {
    type Target = Py<PyUtf8Str>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl TryFromObject for ArgNulFreeStr {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        Self::checked(PyUtf8StrRef::try_from_object(vm, obj)?, vm)
    }

    fn try_from_argument(vm: &VirtualMachine, obj: PyObjectRef) -> Result<Self, ArgumentError> {
        Ok(Self::checked(
            PyUtf8StrRef::try_from_argument(vm, obj)?,
            vm,
        )?)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ArgByteOrder {
    Big,
    Little,
}

impl ArgByteOrder {
    #[must_use]
    pub const fn py_default(&self) -> DefaultRepr {
        match self {
            Self::Big => DefaultRepr::Str("big"),
            Self::Little => DefaultRepr::Str("little"),
        }
    }
}

impl<'a> TryFromBorrowedObject<'a> for ArgByteOrder {
    fn try_from_borrowed_object(vm: &VirtualMachine, obj: &'a PyObject) -> PyResult<Self> {
        obj.try_value_with(
            |s: &Py<PyStr>| match s.as_bytes() {
                b"big" => Ok(Self::Big),
                b"little" => Ok(Self::Little),
                _ => Err(vm.new_value_error("byteorder must be either 'little' or 'big'")),
            },
            vm,
        )
    }
}

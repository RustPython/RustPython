mod argument;
mod arithmetic;
mod buffer;
mod builtin;
mod number;
mod protocol;

pub use argument::{
    ArgumentError, FromArgOptional, FromArgs, FuncArgs, IntoFuncArgs, KwArgs, OptionalArg,
    OptionalOption, PosArgs,
};
pub use arithmetic::{PyArithmeticValue, PyComparisonValue};
pub use buffer::{ArgAsciiBuffer, ArgBytesLike, ArgMemoryBuffer, ArgStrOrBytesLike};
pub use builtin::{IntoPyNativeFunc, OwnedParam, PyNativeFunc, RefParam};
pub use number::{ArgIntoBool, ArgIntoComplex, ArgIntoFloat};
pub use protocol::{ArgCallable, ArgIterable, ArgMapping, ArgSequence};

use crate::{
    builtins::PyTupleRef, convert::TryFromObject, PyObject, PyObjectRef, PyResult, VirtualMachine,
};

/// Tests that the predicate is True on a single value, or if the value is a tuple a tuple, then
/// test that any of the values contained within the tuples satisfies the predicate. Type parameter
/// T specifies the type that is expected, if the input value is not of that type or a tuple of
/// values of that type, then a TypeError is raised.
pub fn single_or_tuple_any<T, F, M>(
    obj: PyObjectRef,
    predicate: &F,
    message: &M,
    vm: &VirtualMachine,
) -> PyResult<bool>
where
    T: TryFromObject,
    F: Fn(&T) -> PyResult<bool>,
    M: Fn(&PyObject) -> String,
{
    match T::try_from_object(vm, obj.clone()) {
        Ok(single) => (predicate)(&single),
        Err(_) => {
            let tuple = PyTupleRef::try_from_object(vm, obj.clone())
                .map_err(|_| vm.new_type_error((message)(&obj)))?;
            for obj in tuple.as_slice().iter() {
                if single_or_tuple_any(obj.clone(), predicate, message, vm)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
    }
}

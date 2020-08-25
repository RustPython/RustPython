use std::collections::HashMap;
use std::mem;
use std::ops::RangeInclusive;

use indexmap::IndexMap;
use result_like::impl_option_like;

use crate::exceptions::PyBaseExceptionRef;
use crate::obj::objtuple::PyTupleRef;
use crate::obj::objtype::{isinstance, PyClassRef};
use crate::pyobject::{
    BorrowValue, IntoPyResult, PyObjectRef, PyRef, PyResult, PyThreadingConstraint, PyValue,
    TryFromObject, TypeProtocol,
};
use crate::vm::VirtualMachine;

use self::OptionalArg::*;

/// The `PyFuncArgs` struct is one of the most used structs then creating
/// a rust function that can be called from python. It holds both positional
/// arguments, as well as keyword arguments passed to the function.
#[derive(Debug, Default, Clone)]
pub struct PyFuncArgs {
    pub args: Vec<PyObjectRef>,
    // sorted map, according to https://www.python.org/dev/peps/pep-0468/
    pub kwargs: IndexMap<String, PyObjectRef>,
}

/// Conversion from vector of python objects to function arguments.
impl From<Vec<PyObjectRef>> for PyFuncArgs {
    fn from(args: Vec<PyObjectRef>) -> Self {
        PyFuncArgs {
            args,
            kwargs: IndexMap::new(),
        }
    }
}

impl From<PyObjectRef> for PyFuncArgs {
    fn from(arg: PyObjectRef) -> Self {
        PyFuncArgs {
            args: vec![arg],
            kwargs: IndexMap::new(),
        }
    }
}

impl From<(Args, KwArgs)> for PyFuncArgs {
    fn from(arg: (Args, KwArgs)) -> Self {
        let Args(args) = arg.0;
        let KwArgs(kwargs) = arg.1;
        PyFuncArgs {
            args,
            kwargs: kwargs.into_iter().collect(),
        }
    }
}
impl From<(&Args, &KwArgs)> for PyFuncArgs {
    fn from(arg: (&Args, &KwArgs)) -> Self {
        let Args(args) = arg.0;
        let KwArgs(kwargs) = arg.1;
        PyFuncArgs {
            args: args.clone(),
            kwargs: kwargs.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        }
    }
}

impl From<KwArgs> for PyFuncArgs {
    fn from(kwargs: KwArgs) -> Self {
        PyFuncArgs {
            args: Vec::new(),
            kwargs: kwargs.into_iter().collect(),
        }
    }
}

impl FromArgs for PyFuncArgs {
    fn from_args(_vm: &VirtualMachine, args: &mut PyFuncArgs) -> Result<Self, ArgumentError> {
        Ok(mem::take(args))
    }
}

impl PyFuncArgs {
    pub fn new(mut args: Vec<PyObjectRef>, kwarg_names: Vec<String>) -> PyFuncArgs {
        // last `kwarg_names.len()` elements of args in order of appearance in the call signature
        let kwarg_values = args.drain((args.len() - kwarg_names.len())..);

        let mut kwargs = IndexMap::new();
        for (name, value) in kwarg_names.iter().zip(kwarg_values) {
            kwargs.insert(name.clone(), value);
        }
        PyFuncArgs { args, kwargs }
    }

    pub fn insert(&self, item: PyObjectRef) -> PyFuncArgs {
        let mut args = PyFuncArgs {
            args: self.args.clone(),
            kwargs: self.kwargs.clone(),
        };
        args.args.insert(0, item);
        args
    }

    pub fn shift(&mut self) -> PyObjectRef {
        self.args.remove(0)
    }

    pub fn get_kwarg(&self, key: &str, default: PyObjectRef) -> PyObjectRef {
        self.kwargs
            .get(key)
            .cloned()
            .unwrap_or_else(|| default.clone())
    }

    pub fn get_optional_kwarg(&self, key: &str) -> Option<PyObjectRef> {
        self.kwargs.get(key).cloned()
    }

    pub fn get_optional_kwarg_with_type(
        &self,
        key: &str,
        ty: PyClassRef,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>> {
        match self.get_optional_kwarg(key) {
            Some(kwarg) => {
                if isinstance(&kwarg, &ty) {
                    Ok(Some(kwarg))
                } else {
                    let expected_ty_name = &ty.name;
                    let actual_ty_name = &kwarg.class().name;
                    Err(vm.new_type_error(format!(
                        "argument of type {} is required for named parameter `{}` (got: {})",
                        expected_ty_name, key, actual_ty_name
                    )))
                }
            }
            None => Ok(None),
        }
    }

    pub fn take_positional(&mut self) -> Option<PyObjectRef> {
        if self.args.is_empty() {
            None
        } else {
            Some(self.args.remove(0))
        }
    }

    pub fn take_positional_keyword(&mut self, name: &str) -> Option<PyObjectRef> {
        self.take_positional().or_else(|| self.take_keyword(name))
    }

    pub fn take_keyword(&mut self, name: &str) -> Option<PyObjectRef> {
        self.kwargs.swap_remove(name)
    }

    pub fn remaining_keywords<'a>(
        &'a mut self,
    ) -> impl Iterator<Item = (String, PyObjectRef)> + 'a {
        self.kwargs.drain(..)
    }

    /// Binds these arguments to their respective values.
    ///
    /// If there is an insufficient number of arguments, there are leftover
    /// arguments after performing the binding, or if an argument is not of
    /// the expected type, a TypeError is raised.
    ///
    /// If the given `FromArgs` includes any conversions, exceptions raised
    /// during the conversion will halt the binding and return the error.
    pub fn bind<T: FromArgs>(mut self, vm: &VirtualMachine) -> PyResult<T> {
        let given_args = self.args.len();
        let bound = T::from_args(vm, &mut self).map_err(|e| match e {
            ArgumentError::TooFewArgs => vm.new_type_error(format!(
                "Expected at least {} arguments ({} given)",
                T::arity().start(),
                given_args,
            )),
            ArgumentError::TooManyArgs => vm.new_type_error(format!(
                "Expected at most {} arguments ({} given)",
                T::arity().end(),
                given_args,
            )),
            ArgumentError::InvalidKeywordArgument(name) => {
                vm.new_type_error(format!("{} is an invalid keyword argument", name))
            }
            ArgumentError::RequiredKeywordArgument(name) => {
                vm.new_type_error(format!("Required keyqord only argument {}", name))
            }
            ArgumentError::Exception(ex) => ex,
        })?;

        if !self.args.is_empty() {
            Err(vm.new_type_error(format!(
                "Expected at most {} arguments ({} given)",
                T::arity().end(),
                given_args,
            )))
        } else if !self.kwargs.is_empty() {
            Err(vm.new_type_error(format!(
                "Unexpected keyword argument {}",
                self.kwargs.keys().next().unwrap()
            )))
        } else {
            Ok(bound)
        }
    }
}

/// An error encountered while binding arguments to the parameters of a Python
/// function call.
pub enum ArgumentError {
    /// The call provided fewer positional arguments than the function requires.
    TooFewArgs,
    /// The call provided more positional arguments than the function accepts.
    TooManyArgs,
    /// The function doesn't accept a keyword argument with the given name.
    InvalidKeywordArgument(String),
    /// The function require a keyword argument with the given name, but one wasn't provided
    RequiredKeywordArgument(String),
    /// An exception was raised while binding arguments to the function
    /// parameters.
    Exception(PyBaseExceptionRef),
}

impl From<PyBaseExceptionRef> for ArgumentError {
    fn from(ex: PyBaseExceptionRef) -> Self {
        ArgumentError::Exception(ex)
    }
}

/// Implemented by any type that can be accepted as a parameter to a built-in
/// function.
///
pub trait FromArgs: Sized {
    /// The range of positional arguments permitted by the function signature.
    ///
    /// Returns an empty range if not applicable.
    fn arity() -> RangeInclusive<usize> {
        0..=0
    }

    /// Extracts this item from the next argument(s).
    fn from_args(vm: &VirtualMachine, args: &mut PyFuncArgs) -> Result<Self, ArgumentError>;
}

/// A map of keyword arguments to their values.
///
/// A built-in function with a `KwArgs` parameter is analagous to a Python
/// function with `**kwargs`. All remaining keyword arguments are extracted
/// (and hence the function will permit an arbitrary number of them).
///
/// `KwArgs` optionally accepts a generic type parameter to allow type checks
/// or conversions of each argument.
///
/// Note:
///
/// KwArgs is only for functions that accept arbitrary keyword arguments. For
/// functions that accept only *specific* named arguments, a rust struct with
/// an appropriate FromArgs implementation must be created.
pub struct KwArgs<T = PyObjectRef>(HashMap<String, T>);

impl<T> KwArgs<T> {
    pub fn new(map: HashMap<String, T>) -> Self {
        KwArgs(map)
    }

    pub fn pop_kwarg(&mut self, name: &str) -> Option<T> {
        self.0.remove(name)
    }
}
impl<T> From<HashMap<String, T>> for KwArgs<T> {
    fn from(map: HashMap<String, T>) -> Self {
        KwArgs(map)
    }
}
impl<T> Default for KwArgs<T> {
    fn default() -> Self {
        KwArgs(HashMap::new())
    }
}

impl<T> FromArgs for KwArgs<T>
where
    T: TryFromObject,
{
    fn from_args(vm: &VirtualMachine, args: &mut PyFuncArgs) -> Result<Self, ArgumentError> {
        let mut kwargs = HashMap::new();
        for (name, value) in args.remaining_keywords() {
            kwargs.insert(name, T::try_from_object(vm, value)?);
        }
        Ok(KwArgs(kwargs))
    }
}

impl<T> IntoIterator for KwArgs<T> {
    type Item = (String, T);
    type IntoIter = std::collections::hash_map::IntoIter<String, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

/// A list of positional argument values.
///
/// A built-in function with a `Args` parameter is analogous to a Python
/// function with `*args`. All remaining positional arguments are extracted
/// (and hence the function will permit an arbitrary number of them).
///
/// `Args` optionally accepts a generic type parameter to allow type checks
/// or conversions of each argument.
#[derive(Clone)]
pub struct Args<T = PyObjectRef>(Vec<T>);

impl<T> Args<T> {
    pub fn new(args: Vec<T>) -> Self {
        Args(args)
    }

    pub fn into_vec(self) -> Vec<T> {
        self.0
    }
}
impl<T> From<Vec<T>> for Args<T> {
    fn from(v: Vec<T>) -> Self {
        Args(v)
    }
}

impl<T> AsRef<[T]> for Args<T> {
    fn as_ref(&self) -> &[T] {
        &self.0
    }
}

impl<T: PyValue> Args<PyRef<T>> {
    pub fn into_tuple(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx
            .new_tuple(self.0.into_iter().map(PyRef::into_object).collect())
    }
}

impl<T> FromArgs for Args<T>
where
    T: TryFromObject,
{
    fn from_args(vm: &VirtualMachine, args: &mut PyFuncArgs) -> Result<Self, ArgumentError> {
        let mut varargs = Vec::new();
        while let Some(value) = args.take_positional() {
            varargs.push(T::try_from_object(vm, value)?);
        }
        Ok(Args(varargs))
    }
}

impl<T> IntoIterator for Args<T> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<T> FromArgs for T
where
    T: TryFromObject,
{
    fn arity() -> RangeInclusive<usize> {
        1..=1
    }

    fn from_args(vm: &VirtualMachine, args: &mut PyFuncArgs) -> Result<Self, ArgumentError> {
        if let Some(value) = args.take_positional() {
            Ok(T::try_from_object(vm, value)?)
        } else {
            Err(ArgumentError::TooFewArgs)
        }
    }
}

/// An argument that may or may not be provided by the caller.
///
/// This style of argument is not possible in pure Python.
#[derive(Debug, is_macro::Is)]
pub enum OptionalArg<T = PyObjectRef> {
    Present(T),
    Missing,
}

impl_option_like!(OptionalArg, Present, Missing);

pub type OptionalOption<T> = OptionalArg<Option<T>>;

impl<T> OptionalOption<T> {
    #[inline]
    pub fn flatten(self) -> Option<T> {
        match self {
            Present(Some(value)) => Some(value),
            _ => None,
        }
    }
}

impl<T> FromArgs for OptionalArg<T>
where
    T: TryFromObject,
{
    fn arity() -> RangeInclusive<usize> {
        0..=1
    }

    fn from_args(vm: &VirtualMachine, args: &mut PyFuncArgs) -> Result<Self, ArgumentError> {
        if let Some(value) = args.take_positional() {
            Ok(Present(T::try_from_object(vm, value)?))
        } else {
            Ok(Missing)
        }
    }
}

// For functions that accept no arguments. Implemented explicitly instead of via
// macro below to avoid unused warnings.
impl FromArgs for () {
    fn from_args(_vm: &VirtualMachine, _args: &mut PyFuncArgs) -> Result<Self, ArgumentError> {
        Ok(())
    }
}

// A tuple of types that each implement `FromArgs` represents a sequence of
// arguments that can be bound and passed to a built-in function.
//
// Technically, a tuple can contain tuples, which can contain tuples, and so on,
// so this actually represents a tree of values to be bound from arguments, but
// in practice this is only used for the top-level parameters.
macro_rules! tuple_from_py_func_args {
    ($($T:ident),+) => {
        impl<$($T),+> FromArgs for ($($T,)+)
        where
            $($T: FromArgs),+
        {
            fn arity() -> RangeInclusive<usize> {
                let mut min = 0;
                let mut max = 0;
                $(
                    let (start, end) = $T::arity().into_inner();
                    min += start;
                    max += end;
                )+
                min..=max
            }

            fn from_args(vm: &VirtualMachine, args: &mut PyFuncArgs) -> Result<Self, ArgumentError> {
                Ok(($($T::from_args(vm, args)?,)+))
            }
        }
    };
}

// Implement `FromArgs` for up to 5-tuples, allowing built-in functions to bind
// up to 5 top-level parameters (note that `Args`, `KwArgs`, nested tuples, etc.
// count as 1, so this should actually be more than enough).
tuple_from_py_func_args!(A);
tuple_from_py_func_args!(A, B);
tuple_from_py_func_args!(A, B, C);
tuple_from_py_func_args!(A, B, C, D);
tuple_from_py_func_args!(A, B, C, D, E);
tuple_from_py_func_args!(A, B, C, D, E, F);

/// A built-in Python function.
pub type PyNativeFunc = Box<py_dyn_fn!(dyn Fn(&VirtualMachine, PyFuncArgs) -> PyResult)>;

/// Implemented by types that are or can generate built-in functions.
///
/// For example, any function that:
///
/// - Accepts a sequence of types that implement `FromArgs`, followed by a
///   `&VirtualMachine`
/// - Returns some type that implements `IntoPyObject`
///
/// will generate a `PyNativeFunc` that performs the appropriate type and arity
/// checking, any requested conversions, and then if successful call the function
/// with the bound values.
///
/// A bare `PyNativeFunc` also implements this trait, allowing the above to be
/// done manually, for rare situations that don't fit into this model.
pub trait IntoPyNativeFunc<T, R, VM>: Sized + PyThreadingConstraint + 'static {
    fn call(&self, vm: &VirtualMachine, args: PyFuncArgs) -> PyResult;
    fn into_func(self) -> PyNativeFunc {
        Box::new(move |vm: &VirtualMachine, args| self.call(vm, args))
    }
}

impl<F> IntoPyNativeFunc<PyFuncArgs, PyResult, VirtualMachine> for F
where
    F: Fn(&VirtualMachine, PyFuncArgs) -> PyResult + PyThreadingConstraint + 'static,
{
    fn call(&self, vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
        (self)(vm, args)
    }
}

pub struct OwnedParam<T>(std::marker::PhantomData<T>);
pub struct RefParam<T>(std::marker::PhantomData<T>);

// This is the "magic" that allows rust functions of varying signatures to
// generate native python functions.
//
// Note that this could be done without a macro - it is simply to avoid repetition.
macro_rules! into_py_native_func_tuple {
    ($(($n:tt, $T:ident)),*) => {
        impl<F, $($T,)* R> IntoPyNativeFunc<($(OwnedParam<$T>,)*), R, VirtualMachine> for F
        where
            F: Fn($($T,)* &VirtualMachine) -> R + PyThreadingConstraint + 'static,
            $($T: FromArgs,)*
            R: IntoPyResult,
        {
            fn call(&self, vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
                let ($($n,)*) = args.bind::<($($T,)*)>(vm)?;

                (self)($($n,)* vm).into_pyresult(vm)
            }
        }

        impl<F, S, $($T,)* R> IntoPyNativeFunc<(RefParam<S>, $(OwnedParam<$T>,)*), R, VirtualMachine> for F
        where
            F: Fn(&S, $($T,)* &VirtualMachine) -> R + PyThreadingConstraint + 'static,
            S: PyValue,
            $($T: FromArgs,)*
            R: IntoPyResult,
        {
            fn call(&self, vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
                let (zelf, $($n,)*) = args.bind::<(PyRef<S>, $($T,)*)>(vm)?;

                (self)(&zelf, $($n,)* vm).into_pyresult(vm)
            }
        }

        impl<F, $($T,)* R> IntoPyNativeFunc<($(OwnedParam<$T>,)*), R, ()> for F
        where
            F: Fn($($T,)*) -> R + PyThreadingConstraint + 'static,
            $($T: FromArgs,)*
            R: IntoPyResult,
        {
            fn call(&self, vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
                let ($($n,)*) = args.bind::<($($T,)*)>(vm)?;

                (self)($($n,)*).into_pyresult(vm)
            }
        }

        impl<F, S, $($T,)* R> IntoPyNativeFunc<(RefParam<S>, $(OwnedParam<$T>,)*), R, ()> for F
        where
            F: Fn(&S, $($T,)*) -> R + PyThreadingConstraint + 'static,
            S: PyValue,
            $($T: FromArgs,)*
            R: IntoPyResult,
        {
            fn call(&self, vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
                let (zelf, $($n,)*) = args.bind::<(PyRef<S>, $($T,)*)>(vm)?;

                (self)(&zelf, $($n,)*).into_pyresult(vm)
            }
        }
    };
}

into_py_native_func_tuple!();
into_py_native_func_tuple!((v1, T1));
into_py_native_func_tuple!((v1, T1), (v2, T2));
into_py_native_func_tuple!((v1, T1), (v2, T2), (v3, T3));
into_py_native_func_tuple!((v1, T1), (v2, T2), (v3, T3), (v4, T4));
into_py_native_func_tuple!((v1, T1), (v2, T2), (v3, T3), (v4, T4), (v5, T5));

/// Tests that the predicate is True on a single value, or if the value is a tuple a tuple, then
/// test that any of the values contained within the tuples satisfies the predicate. Type parameter
/// T specifies the type that is expected, if the input value is not of that type or a tuple of
/// values of that type, then a TypeError is raised.
pub fn single_or_tuple_any<T, F, M>(
    obj: PyObjectRef,
    predicate: F,
    message: M,
    vm: &VirtualMachine,
) -> PyResult<bool>
where
    T: TryFromObject,
    F: Fn(&T) -> PyResult<bool>,
    M: Fn(&PyObjectRef) -> String,
{
    // TODO: figure out some way to have recursive calls without... this
    struct Checker<T, F, M>
    where
        F: Fn(&T) -> PyResult<bool>,
        M: Fn(&PyObjectRef) -> String,
    {
        predicate: F,
        message: M,
        t: std::marker::PhantomData<T>,
    }
    impl<T, F, M> Checker<T, F, M>
    where
        T: TryFromObject,
        F: Fn(&T) -> PyResult<bool>,
        M: Fn(&PyObjectRef) -> String,
    {
        fn check(&self, obj: &PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
            match T::try_from_object(vm, obj.clone()) {
                Ok(single) => (self.predicate)(&single),
                Err(_) => {
                    let tuple = PyTupleRef::try_from_object(vm, obj.clone())
                        .map_err(|_| vm.new_type_error((self.message)(&obj)))?;
                    for obj in tuple.borrow_value().iter() {
                        if self.check(&obj, vm)? {
                            return Ok(true);
                        }
                    }
                    Ok(false)
                }
            }
        }
    }
    let checker = Checker {
        predicate,
        message,
        t: std::marker::PhantomData,
    };
    checker.check(&obj, vm)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_intonativefunc_noalloc() {
        let check_zst = |f: PyNativeFunc| assert_eq!(std::mem::size_of_val(f.as_ref()), 0);
        fn py_func(_b: bool, _vm: &crate::VirtualMachine) -> i32 {
            1
        }
        check_zst(py_func.into_func());
        let empty_closure = || "foo".to_owned();
        check_zst(empty_closure.into_func());
    }
}

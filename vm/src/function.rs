use std::collections::HashMap;
use std::mem;
use std::ops::RangeInclusive;

use indexmap::IndexMap;
use result_like::impl_option_like;
use smallbox::{smallbox, space::S1, SmallBox};

use crate::exceptions::PyBaseExceptionRef;
use crate::obj::objtuple::PyTupleRef;
use crate::obj::objtype::{isinstance, PyClassRef};
use crate::pyobject::{
    IntoPyObject, PyObjectRef, PyRef, PyResult, PyValue, TryFromObject, TypeProtocol,
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
                    let expected_ty_name = vm.to_pystr(&ty)?;
                    let actual_ty_name = vm.to_pystr(&kwarg.class())?;
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
        let bound = match T::from_args(vm, &mut self) {
            Ok(args) => args,
            Err(ArgumentError::TooFewArgs) => {
                return Err(vm.new_type_error(format!(
                    "Expected at least {} arguments ({} given)",
                    T::arity().start(),
                    given_args,
                )));
            }
            Err(ArgumentError::TooManyArgs) => {
                return Err(vm.new_type_error(format!(
                    "Expected at most {} arguments ({} given)",
                    T::arity().end(),
                    given_args,
                )));
            }
            Err(ArgumentError::InvalidKeywordArgument(name)) => {
                return Err(vm.new_type_error(format!("{} is an invalid keyword argument", name)));
            }
            Err(ArgumentError::RequiredKeywordArgument(name)) => {
                return Err(vm.new_type_error(format!("Required keyqord only argument {}", name)));
            }
            Err(ArgumentError::Exception(ex)) => {
                return Err(ex);
            }
        };

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
    pub fn flat_option(self) -> Option<T> {
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

/// A container that can hold a `dyn Fn*` trait object, but doesn't allocate if it's only a fn() pointer
pub type FunctionBox<T> = SmallBox<T, S1>;

/// A built-in Python function.
pub type PyNativeFunc =
    FunctionBox<dyn Fn(&VirtualMachine, PyFuncArgs) -> PyResult + 'static + Send + Sync>;

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
pub trait IntoPyNativeFunc<T, R, VM> {
    fn into_func(self) -> PyNativeFunc;
}

impl<F> IntoPyNativeFunc<PyFuncArgs, PyResult, VirtualMachine> for F
where
    F: Fn(&VirtualMachine, PyFuncArgs) -> PyResult + 'static + Send + Sync,
{
    fn into_func(self) -> PyNativeFunc {
        smallbox!(self)
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
            F: Fn($($T,)* &VirtualMachine) -> R + 'static + Send + Sync,
            $($T: FromArgs,)*
            R: IntoPyObject,
        {
            fn into_func(self) -> PyNativeFunc {
                smallbox!(move |vm: &VirtualMachine, args: PyFuncArgs| {
                    let ($($n,)*) = args.bind::<($($T,)*)>(vm)?;

                    (self)($($n,)* vm).into_pyobject(vm)
                })
            }
        }

        impl<F, S, $($T,)* R> IntoPyNativeFunc<(RefParam<S>, $(OwnedParam<$T>,)*), R, VirtualMachine> for F
        where
            F: Fn(&S, $($T,)* &VirtualMachine) -> R + 'static  + Send + Sync,
            S: PyValue,
            $($T: FromArgs,)*
            R: IntoPyObject,
        {
            fn into_func(self) -> PyNativeFunc {
                smallbox!(move |vm: &VirtualMachine, args: PyFuncArgs| {
                    let (zelf, $($n,)*) = args.bind::<(PyRef<S>, $($T,)*)>(vm)?;

                    (self)(&zelf, $($n,)* vm).into_pyobject(vm)
                })
            }
        }

        impl<F, $($T,)* R> IntoPyNativeFunc<($(OwnedParam<$T>,)*), R, ()> for F
        where
            F: Fn($($T,)*) -> R + 'static  + Send + Sync,
            $($T: FromArgs,)*
            R: IntoPyObject,
        {
            fn into_func(self) -> PyNativeFunc {
                IntoPyNativeFunc::into_func(move |$($n,)* _vm: &VirtualMachine| (self)($($n,)*))
            }
        }

        impl<F, S, $($T,)* R> IntoPyNativeFunc<(RefParam<S>, $(OwnedParam<$T>,)*), R, ()> for F
        where
            F: Fn(&S, $($T,)*) -> R + 'static  + Send + Sync,
            S: PyValue,
            $($T: FromArgs,)*
            R: IntoPyObject,
        {
            fn into_func(self) -> PyNativeFunc {
                IntoPyNativeFunc::into_func(move |zelf: &S, $($n,)* _vm: &VirtualMachine| (self)(zelf, $($n,)*))
            }
        }
    };
}

into_py_native_func_tuple!();
into_py_native_func_tuple!((a, A));
into_py_native_func_tuple!((a, A), (b, B));
into_py_native_func_tuple!((a, A), (b, B), (c, C));
into_py_native_func_tuple!((a, A), (b, B), (c, C), (d, D));
into_py_native_func_tuple!((a, A), (b, B), (c, C), (d, D), (e, E));

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
                    for obj in tuple.as_slice().iter() {
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
    #[test]
    fn test_functionbox_noalloc() {
        fn py_func(_b: bool, _vm: &crate::VirtualMachine) -> i32 {
            1
        }
        let f = super::IntoPyNativeFunc::into_func(py_func);
        assert!(!f.is_heap());
    }
}

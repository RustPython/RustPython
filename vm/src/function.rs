use self::OptionalArg::*;
use crate::builtins::pytype::PyTypeRef;
use crate::builtins::tuple::PyTupleRef;
use crate::exceptions::PyBaseExceptionRef;
use crate::pyobject::{
    BorrowValue, IntoPyObject, IntoPyResult, PyObjectRef, PyRef, PyResult, PyThreadingConstraint,
    PyValue, TryFromObject, TypeProtocol,
};
use crate::vm::VirtualMachine;
use indexmap::IndexMap;
use itertools::Itertools;
use result_like::impl_option_like;
use std::marker::PhantomData;
use std::ops::RangeInclusive;

pub trait IntoFuncArgs: Sized {
    fn into_args(self, vm: &VirtualMachine) -> FuncArgs;
    fn into_method_args(self, obj: PyObjectRef, vm: &VirtualMachine) -> FuncArgs {
        let mut args = self.into_args(vm);
        args.prepend_arg(obj);
        args
    }
}

impl<T> IntoFuncArgs for T
where
    T: Into<FuncArgs>,
{
    fn into_args(self, _vm: &VirtualMachine) -> FuncArgs {
        self.into()
    }
}

// A tuple of values that each implement `IntoPyObject` represents a sequence of
// arguments that can be bound and passed to a built-in function.
macro_rules! into_func_args_from_tuple {
    ($(($n:tt, $T:ident)),*) => {
        impl<$($T,)*> IntoFuncArgs for ($($T,)*)
        where
            $($T: IntoPyObject,)*
        {
            #[inline]
            fn into_args(self, vm: &VirtualMachine) -> FuncArgs {
                let ($($n,)*) = self;
                vec![$($n.into_pyobject(vm),)*].into()
            }

            #[inline]
            fn into_method_args(self, obj: PyObjectRef, vm: &VirtualMachine) -> FuncArgs {
                let ($($n,)*) = self;
                vec![obj, $($n.into_pyobject(vm),)*].into()
            }
        }
    };
}

into_func_args_from_tuple!((v1, T1));
into_func_args_from_tuple!((v1, T1), (v2, T2));
into_func_args_from_tuple!((v1, T1), (v2, T2), (v3, T3));
into_func_args_from_tuple!((v1, T1), (v2, T2), (v3, T3), (v4, T4));
into_func_args_from_tuple!((v1, T1), (v2, T2), (v3, T3), (v4, T4), (v5, T5));

/// The `FuncArgs` struct is one of the most used structs then creating
/// a rust function that can be called from python. It holds both positional
/// arguments, as well as keyword arguments passed to the function.
#[derive(Debug, Default, Clone)]
pub struct FuncArgs {
    pub args: Vec<PyObjectRef>,
    // sorted map, according to https://www.python.org/dev/peps/pep-0468/
    pub kwargs: IndexMap<String, PyObjectRef>,
}

/// Conversion from vector of python objects to function arguments.
impl<A> From<A> for FuncArgs
where
    A: Into<Args>,
{
    fn from(args: A) -> Self {
        FuncArgs {
            args: args.into().into_vec(),
            kwargs: IndexMap::new(),
        }
    }
}

impl From<KwArgs> for FuncArgs {
    fn from(kwargs: KwArgs) -> Self {
        FuncArgs {
            args: Vec::new(),
            kwargs: kwargs.0,
        }
    }
}

impl FromArgs for FuncArgs {
    fn from_args(_vm: &VirtualMachine, args: &mut FuncArgs) -> Result<Self, ArgumentError> {
        Ok(std::mem::take(args))
    }
}

impl FuncArgs {
    pub fn new<A, K>(args: A, kwargs: K) -> Self
    where
        A: Into<Args>,
        K: Into<KwArgs>,
    {
        let Args(args) = args.into();
        let KwArgs(kwargs) = kwargs.into();
        Self { args, kwargs }
    }

    pub fn with_kwargs_names<A, KW>(mut args: A, kwarg_names: KW) -> Self
    where
        A: ExactSizeIterator<Item = PyObjectRef>,
        KW: ExactSizeIterator<Item = String>,
    {
        // last `kwarg_names.len()` elements of args in order of appearance in the call signature
        let total_argc = args.len();
        let kwargc = kwarg_names.len();
        let posargc = total_argc - kwargc;

        let posargs = args.by_ref().take(posargc).collect();

        let kwargs = kwarg_names.zip_eq(args).collect::<IndexMap<_, _>>();

        FuncArgs {
            args: posargs,
            kwargs,
        }
    }

    pub fn prepend_arg(&mut self, item: PyObjectRef) {
        self.args.reserve_exact(1);
        self.args.insert(0, item)
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
        ty: PyTypeRef,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>> {
        match self.get_optional_kwarg(key) {
            Some(kwarg) => {
                if kwarg.isinstance(&ty) {
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

    pub fn remaining_keywords(&mut self) -> impl Iterator<Item = (String, PyObjectRef)> + '_ {
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
        let bound = T::from_args(vm, &mut self)
            .map_err(|e| e.into_exception(T::arity(), given_args, vm))?;

        if !self.args.is_empty() {
            Err(vm.new_type_error(format!(
                "Expected at most {} arguments ({} given)",
                T::arity().end(),
                given_args,
            )))
        } else if let Some(err) = self.check_kwargs_empty(vm) {
            Err(err)
        } else {
            Ok(bound)
        }
    }

    pub fn check_kwargs_empty(&self, vm: &VirtualMachine) -> Option<PyBaseExceptionRef> {
        if let Some(k) = self.kwargs.keys().next() {
            Some(vm.new_type_error(format!("Unexpected keyword argument {}", k)))
        } else {
            None
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

impl ArgumentError {
    fn into_exception(
        self,
        arity: RangeInclusive<usize>,
        num_given: usize,
        vm: &VirtualMachine,
    ) -> PyBaseExceptionRef {
        match self {
            ArgumentError::TooFewArgs => vm.new_type_error(format!(
                "Expected at least {} arguments ({} given)",
                arity.start(),
                num_given
            )),
            ArgumentError::TooManyArgs => vm.new_type_error(format!(
                "Expected at most {} arguments ({} given)",
                arity.end(),
                num_given
            )),
            ArgumentError::InvalidKeywordArgument(name) => {
                vm.new_type_error(format!("{} is an invalid keyword argument", name))
            }
            ArgumentError::RequiredKeywordArgument(name) => {
                vm.new_type_error(format!("Required keyqord only argument {}", name))
            }
            ArgumentError::Exception(ex) => ex,
        }
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
    fn from_args(vm: &VirtualMachine, args: &mut FuncArgs) -> Result<Self, ArgumentError>;
}

pub trait FromArgOptional {
    type Inner: TryFromObject;
    fn from_inner(x: Self::Inner) -> Self;
}
impl<T: TryFromObject> FromArgOptional for OptionalArg<T> {
    type Inner = T;
    fn from_inner(x: T) -> Self {
        Self::Present(x)
    }
}
impl<T: TryFromObject> FromArgOptional for T {
    type Inner = Self;
    fn from_inner(x: Self) -> Self {
        x
    }
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
#[derive(Clone)]
pub struct KwArgs<T = PyObjectRef>(IndexMap<String, T>);

impl<T> KwArgs<T> {
    pub fn new(map: IndexMap<String, T>) -> Self {
        KwArgs(map)
    }

    pub fn pop_kwarg(&mut self, name: &str) -> Option<T> {
        self.0.remove(name)
    }
}
impl<T> std::iter::FromIterator<(String, T)> for KwArgs<T> {
    fn from_iter<I: IntoIterator<Item = (String, T)>>(iter: I) -> Self {
        KwArgs(iter.into_iter().collect())
    }
}
impl<T> Default for KwArgs<T> {
    fn default() -> Self {
        KwArgs(IndexMap::new())
    }
}

impl<T> FromArgs for KwArgs<T>
where
    T: TryFromObject,
{
    fn from_args(vm: &VirtualMachine, args: &mut FuncArgs) -> Result<Self, ArgumentError> {
        let mut kwargs = IndexMap::new();
        for (name, value) in args.remaining_keywords() {
            kwargs.insert(name, T::try_from_object(vm, value)?);
        }
        Ok(KwArgs(kwargs))
    }
}

impl<T> IntoIterator for KwArgs<T> {
    type Item = (String, T);
    type IntoIter = indexmap::map::IntoIter<String, T>;

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

    pub fn iter(&self) -> std::slice::Iter<T> {
        self.0.iter()
    }
}

impl<T> From<Vec<T>> for Args<T> {
    fn from(v: Vec<T>) -> Self {
        Args(v)
    }
}

impl From<()> for Args<PyObjectRef> {
    fn from(_args: ()) -> Self {
        Args(Vec::new())
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
    fn from_args(vm: &VirtualMachine, args: &mut FuncArgs) -> Result<Self, ArgumentError> {
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

    fn from_args(vm: &VirtualMachine, args: &mut FuncArgs) -> Result<Self, ArgumentError> {
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

impl OptionalArg<PyObjectRef> {
    pub fn unwrap_or_none(self, vm: &VirtualMachine) -> PyObjectRef {
        self.unwrap_or_else(|| vm.ctx.none())
    }
}

pub type OptionalOption<T = PyObjectRef> = OptionalArg<Option<T>>;

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

    fn from_args(vm: &VirtualMachine, args: &mut FuncArgs) -> Result<Self, ArgumentError> {
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
    fn from_args(_vm: &VirtualMachine, _args: &mut FuncArgs) -> Result<Self, ArgumentError> {
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

            fn from_args(vm: &VirtualMachine, args: &mut FuncArgs) -> Result<Self, ArgumentError> {
                Ok(($($T::from_args(vm, args)?,)+))
            }
        }
    };
}

// Implement `FromArgs` for up to 7-tuples, allowing built-in functions to bind
// up to 7 top-level parameters (note that `Args`, `KwArgs`, nested tuples, etc.
// count as 1, so this should actually be more than enough).
tuple_from_py_func_args!(A);
tuple_from_py_func_args!(A, B);
tuple_from_py_func_args!(A, B, C);
tuple_from_py_func_args!(A, B, C, D);
tuple_from_py_func_args!(A, B, C, D, E);
tuple_from_py_func_args!(A, B, C, D, E, F);
tuple_from_py_func_args!(A, B, C, D, E, F, G);
tuple_from_py_func_args!(A, B, C, D, E, F, G, H);

/// A built-in Python function.
pub type PyNativeFunc = Box<py_dyn_fn!(dyn Fn(&VirtualMachine, FuncArgs) -> PyResult)>;

/// Implemented by types that are or can generate built-in functions.
///
/// This trait is implemented by any function that matches the pattern:
///
/// ```rust,ignore
/// Fn([&self,] [T where T: FromArgs, ...] [, vm: &VirtualMachine])
/// ```
///
/// For example, anything from `Fn()` to `Fn(vm: &VirtualMachine) -> u32` to
/// `Fn(PyIntRef, PyIntRef) -> String` to
/// `Fn(&self, PyStrRef, FooOptions, vm: &VirtualMachine) -> PyResult<PyInt>`
/// is `IntoPyNativeFunc`. If you do want a really general function signature, e.g.
/// to forward the args to another function, you can define a function like
/// `Fn(FuncArgs [, &VirtualMachine]) -> ...`
///
/// Note that the `Kind` type parameter is meaningless and should be considered
/// an implementation detail; if you need to use `IntoPyNativeFunc` as a trait bound
/// just pass an unconstrained generic type, e.g.
/// `fn foo<F, FKind>(f: F) where F: IntoPyNativeFunc<FKind>`
pub trait IntoPyNativeFunc<Kind>: Sized + PyThreadingConstraint + 'static {
    fn call(&self, vm: &VirtualMachine, args: FuncArgs) -> PyResult;
    /// `IntoPyNativeFunc::into_func()` generates a PyNativeFunc that performs the
    /// appropriate type and arity checking, any requested conversions, and then if
    /// successful calls the function with the extracted parameters.
    fn into_func(self) -> PyNativeFunc {
        Box::new(move |vm: &VirtualMachine, args| self.call(vm, args))
    }
}

// TODO: once higher-rank trait bounds are stabilized, remove the `Kind` type
// parameter and impl for F where F: for<T, R, VM> PyNativeFuncInternal<T, R, VM>
impl<F, T, R, VM> IntoPyNativeFunc<(T, R, VM)> for F
where
    F: PyNativeFuncInternal<T, R, VM>,
{
    fn call(&self, vm: &VirtualMachine, args: FuncArgs) -> PyResult {
        self.call_(vm, args)
    }
}

mod sealed {
    use super::*;
    pub trait PyNativeFuncInternal<T, R, VM>: Sized + PyThreadingConstraint + 'static {
        fn call_(&self, vm: &VirtualMachine, args: FuncArgs) -> PyResult;
    }
}
use sealed::PyNativeFuncInternal;

#[doc(hidden)]
pub struct OwnedParam<T>(PhantomData<T>);
#[doc(hidden)]
pub struct RefParam<T>(PhantomData<T>);

// This is the "magic" that allows rust functions of varying signatures to
// generate native python functions.
//
// Note that this could be done without a macro - it is simply to avoid repetition.
macro_rules! into_py_native_func_tuple {
    ($(($n:tt, $T:ident)),*) => {
        impl<F, $($T,)* R> PyNativeFuncInternal<($(OwnedParam<$T>,)*), R, VirtualMachine> for F
        where
            F: Fn($($T,)* &VirtualMachine) -> R + PyThreadingConstraint + 'static,
            $($T: FromArgs,)*
            R: IntoPyResult,
        {
            fn call_(&self, vm: &VirtualMachine, args: FuncArgs) -> PyResult {
                let ($($n,)*) = args.bind::<($($T,)*)>(vm)?;

                (self)($($n,)* vm).into_pyresult(vm)
            }
        }

        impl<F, S, $($T,)* R> PyNativeFuncInternal<(RefParam<S>, $(OwnedParam<$T>,)*), R, VirtualMachine> for F
        where
            F: Fn(&S, $($T,)* &VirtualMachine) -> R + PyThreadingConstraint + 'static,
            S: PyValue,
            $($T: FromArgs,)*
            R: IntoPyResult,
        {
            fn call_(&self, vm: &VirtualMachine, args: FuncArgs) -> PyResult {
                let (zelf, $($n,)*) = args.bind::<(PyRef<S>, $($T,)*)>(vm)?;

                (self)(&zelf, $($n,)* vm).into_pyresult(vm)
            }
        }

        impl<F, $($T,)* R> PyNativeFuncInternal<($(OwnedParam<$T>,)*), R, ()> for F
        where
            F: Fn($($T,)*) -> R + PyThreadingConstraint + 'static,
            $($T: FromArgs,)*
            R: IntoPyResult,
        {
            fn call_(&self, vm: &VirtualMachine, args: FuncArgs) -> PyResult {
                let ($($n,)*) = args.bind::<($($T,)*)>(vm)?;

                (self)($($n,)*).into_pyresult(vm)
            }
        }

        impl<F, S, $($T,)* R> PyNativeFuncInternal<(RefParam<S>, $(OwnedParam<$T>,)*), R, ()> for F
        where
            F: Fn(&S, $($T,)*) -> R + PyThreadingConstraint + 'static,
            S: PyValue,
            $($T: FromArgs,)*
            R: IntoPyResult,
        {
            fn call_(&self, vm: &VirtualMachine, args: FuncArgs) -> PyResult {
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
into_py_native_func_tuple!((v1, T1), (v2, T2), (v3, T3), (v4, T4), (v5, T5), (v6, T6));
into_py_native_func_tuple!(
    (v1, T1),
    (v2, T2),
    (v3, T3),
    (v4, T4),
    (v5, T5),
    (v6, T6),
    (v7, T7)
);

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
    M: Fn(&PyObjectRef) -> String,
{
    match T::try_from_object(vm, obj.clone()) {
        Ok(single) => (predicate)(&single),
        Err(_) => {
            let tuple = PyTupleRef::try_from_object(vm, obj.clone())
                .map_err(|_| vm.new_type_error((message)(&obj)))?;
            for obj in tuple.borrow_value().iter() {
                if single_or_tuple_any(obj.clone(), predicate, message, vm)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
    }
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

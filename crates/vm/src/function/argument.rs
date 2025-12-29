use crate::{
    AsObject, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject, VirtualMachine,
    builtins::{PyBaseExceptionRef, PyTupleRef, PyTypeRef},
    convert::ToPyObject,
    object::{Traverse, TraverseFn},
};
use core::ops::RangeInclusive;
use indexmap::IndexMap;
use itertools::Itertools;

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

// A tuple of values that each implement `ToPyObject` represents a sequence of
// arguments that can be bound and passed to a built-in function.
macro_rules! into_func_args_from_tuple {
    ($(($n:tt, $T:ident)),*) => {
        impl<$($T,)*> IntoFuncArgs for ($($T,)*)
        where
            $($T: ToPyObject,)*
        {
            #[inline]
            fn into_args(self, vm: &VirtualMachine) -> FuncArgs {
                let ($($n,)*) = self;
                PosArgs::new(vec![$($n.to_pyobject(vm),)*]).into()
            }

            #[inline]
            fn into_method_args(self, obj: PyObjectRef, vm: &VirtualMachine) -> FuncArgs {
                let ($($n,)*) = self;
                PosArgs::new(vec![obj, $($n.to_pyobject(vm),)*]).into()
            }
        }
    };
}

into_func_args_from_tuple!((v1, T1));
into_func_args_from_tuple!((v1, T1), (v2, T2));
into_func_args_from_tuple!((v1, T1), (v2, T2), (v3, T3));
into_func_args_from_tuple!((v1, T1), (v2, T2), (v3, T3), (v4, T4));
into_func_args_from_tuple!((v1, T1), (v2, T2), (v3, T3), (v4, T4), (v5, T5));
into_func_args_from_tuple!((v1, T1), (v2, T2), (v3, T3), (v4, T4), (v5, T5), (v6, T6));
// We currently allows only 6 unnamed positional arguments.
// Please use `#[derive(FromArgs)]` and a struct for more complex argument parsing.
// The number of limitation came from:
// https://rust-lang.github.io/rust-clippy/master/index.html#too_many_arguments

/// The `FuncArgs` struct is one of the most used structs then creating
/// a rust function that can be called from python. It holds both positional
/// arguments, as well as keyword arguments passed to the function.
#[derive(Debug, Default, Clone, Traverse)]
pub struct FuncArgs {
    pub args: Vec<PyObjectRef>,
    // sorted map, according to https://www.python.org/dev/peps/pep-0468/
    pub kwargs: IndexMap<String, PyObjectRef>,
}

unsafe impl Traverse for IndexMap<String, PyObjectRef> {
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        self.values().for_each(|v| v.traverse(tracer_fn));
    }
}

/// Conversion from vector of python objects to function arguments.
impl<A> From<A> for FuncArgs
where
    A: Into<PosArgs>,
{
    fn from(args: A) -> Self {
        Self {
            args: args.into().into_vec(),
            kwargs: IndexMap::new(),
        }
    }
}

impl From<KwArgs> for FuncArgs {
    fn from(kwargs: KwArgs) -> Self {
        Self {
            args: Vec::new(),
            kwargs: kwargs.0,
        }
    }
}

impl FromArgs for FuncArgs {
    fn from_args(_vm: &VirtualMachine, args: &mut FuncArgs) -> Result<Self, ArgumentError> {
        Ok(core::mem::take(args))
    }
}

impl FuncArgs {
    pub fn new<A, K>(args: A, kwargs: K) -> Self
    where
        A: Into<PosArgs>,
        K: Into<KwArgs>,
    {
        let PosArgs(args) = args.into();
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
        let kwarg_count = kwarg_names.len();
        let pos_arg_count = total_argc - kwarg_count;

        let pos_args = args.by_ref().take(pos_arg_count).collect();

        let kwargs = kwarg_names.zip_eq(args).collect::<IndexMap<_, _>>();

        Self {
            args: pos_args,
            kwargs,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.args.is_empty() && self.kwargs.is_empty()
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
                if kwarg.fast_isinstance(&ty) {
                    Ok(Some(kwarg))
                } else {
                    let expected_ty_name = &ty.name();
                    let kwarg_class = kwarg.class();
                    let actual_ty_name = &kwarg_class.name();
                    Err(vm.new_type_error(format!(
                        "argument of type {expected_ty_name} is required for named parameter `{key}` (got: {actual_ty_name})"
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
                "expected at most {} arguments, got {}",
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
        self.kwargs
            .keys()
            .next()
            .map(|k| vm.new_type_error(format!("Unexpected keyword argument {k}")))
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
        Self::Exception(ex)
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
            Self::TooFewArgs => vm.new_type_error(format!(
                "expected at least {} arguments, got {}",
                arity.start(),
                num_given
            )),
            Self::TooManyArgs => vm.new_type_error(format!(
                "expected at most {} arguments, got {}",
                arity.end(),
                num_given
            )),
            Self::InvalidKeywordArgument(name) => {
                vm.new_type_error(format!("{name} is an invalid keyword argument"))
            }
            Self::RequiredKeywordArgument(name) => {
                vm.new_type_error(format!("Required keyword only argument {name}"))
            }
            Self::Exception(ex) => ex,
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
/// A built-in function with a `KwArgs` parameter is analogous to a Python
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

unsafe impl<T> Traverse for KwArgs<T>
where
    T: Traverse,
{
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        self.0.iter().map(|(_, v)| v.traverse(tracer_fn)).count();
    }
}

impl<T> KwArgs<T> {
    pub const fn new(map: IndexMap<String, T>) -> Self {
        Self(map)
    }

    pub fn pop_kwarg(&mut self, name: &str) -> Option<T> {
        self.0.swap_remove(name)
    }

    pub fn is_empty(self) -> bool {
        self.0.is_empty()
    }
}

impl<T> FromIterator<(String, T)> for KwArgs<T> {
    fn from_iter<I: IntoIterator<Item = (String, T)>>(iter: I) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl<T> Default for KwArgs<T> {
    fn default() -> Self {
        Self(IndexMap::new())
    }
}

impl<T> FromArgs for KwArgs<T>
where
    T: TryFromObject,
{
    fn from_args(vm: &VirtualMachine, args: &mut FuncArgs) -> Result<Self, ArgumentError> {
        let mut kwargs = IndexMap::new();
        for (name, value) in args.remaining_keywords() {
            kwargs.insert(name, value.try_into_value(vm)?);
        }
        Ok(Self(kwargs))
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
/// A built-in function with a `PosArgs` parameter is analogous to a Python
/// function with `*args`. All remaining positional arguments are extracted
/// (and hence the function will permit an arbitrary number of them).
///
/// `PosArgs` optionally accepts a generic type parameter to allow type checks
/// or conversions of each argument.
#[derive(Clone)]
pub struct PosArgs<T = PyObjectRef>(Vec<T>);

unsafe impl<T> Traverse for PosArgs<T>
where
    T: Traverse,
{
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        self.0.traverse(tracer_fn)
    }
}

impl<T> PosArgs<T> {
    pub const fn new(args: Vec<T>) -> Self {
        Self(args)
    }

    pub fn into_vec(self) -> Vec<T> {
        self.0
    }

    pub fn iter(&self) -> core::slice::Iter<'_, T> {
        self.0.iter()
    }
}

impl<T> From<Vec<T>> for PosArgs<T> {
    fn from(v: Vec<T>) -> Self {
        Self(v)
    }
}

impl From<()> for PosArgs<PyObjectRef> {
    fn from(_args: ()) -> Self {
        Self(Vec::new())
    }
}

impl<T> AsRef<[T]> for PosArgs<T> {
    fn as_ref(&self) -> &[T] {
        &self.0
    }
}

impl<T: PyPayload> PosArgs<PyRef<T>> {
    pub fn into_tuple(self, vm: &VirtualMachine) -> PyTupleRef {
        vm.ctx
            .new_tuple(self.0.into_iter().map(Into::into).collect())
    }
}

impl<T> FromArgs for PosArgs<T>
where
    T: TryFromObject,
{
    fn from_args(vm: &VirtualMachine, args: &mut FuncArgs) -> Result<Self, ArgumentError> {
        let mut varargs = Vec::new();
        while let Some(value) = args.take_positional() {
            varargs.push(value.try_into_value(vm)?);
        }
        Ok(Self(varargs))
    }
}

impl<T> IntoIterator for PosArgs<T> {
    type Item = T;
    type IntoIter = alloc::vec::IntoIter<T>;

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
        let value = args.take_positional().ok_or(ArgumentError::TooFewArgs)?;
        Ok(value.try_into_value(vm)?)
    }
}

/// An argument that may or may not be provided by the caller.
///
/// This style of argument is not possible in pure Python.
#[derive(Debug, result_like::OptionLike, is_macro::Is)]
pub enum OptionalArg<T = PyObjectRef> {
    Present(T),
    Missing,
}

unsafe impl<T> Traverse for OptionalArg<T>
where
    T: Traverse,
{
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        match self {
            Self::Present(o) => o.traverse(tracer_fn),
            Self::Missing => (),
        }
    }
}

impl OptionalArg<PyObjectRef> {
    pub fn unwrap_or_none(self, vm: &VirtualMachine) -> PyObjectRef {
        self.unwrap_or_else(|| vm.ctx.none())
    }
}

pub type OptionalOption<T = PyObjectRef> = OptionalArg<Option<T>>;

impl<T> OptionalOption<T> {
    #[inline]
    pub fn flatten(self) -> Option<T> {
        self.into_option().flatten()
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
        let r = if let Some(value) = args.take_positional() {
            Self::Present(value.try_into_value(vm)?)
        } else {
            Self::Missing
        };
        Ok(r)
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
// up to 7 top-level parameters (note that `PosArgs`, `KwArgs`, nested tuples, etc.
// count as 1, so this should actually be more than enough).
tuple_from_py_func_args!(A);
tuple_from_py_func_args!(A, B);
tuple_from_py_func_args!(A, B, C);
tuple_from_py_func_args!(A, B, C, D);
tuple_from_py_func_args!(A, B, C, D, E);
tuple_from_py_func_args!(A, B, C, D, E, F);
tuple_from_py_func_args!(A, B, C, D, E, F, G);
tuple_from_py_func_args!(A, B, C, D, E, F, G, H);

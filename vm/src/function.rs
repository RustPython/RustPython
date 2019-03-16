use std::collections::HashMap;
use std::iter;
use std::ops::RangeInclusive;

use crate::obj::objtype;
use crate::pyobject::{IntoPyObject, PyObjectRef, PyResult, TryFromObject, TypeProtocol};
use crate::vm::VirtualMachine;

use self::OptionalArg::*;

/// The `PyFuncArgs` struct is one of the most used structs then creating
/// a rust function that can be called from python. It holds both positional
/// arguments, as well as keyword arguments passed to the function.
#[derive(Debug, Default, Clone)]
pub struct PyFuncArgs {
    pub args: Vec<PyObjectRef>,
    pub kwargs: Vec<(String, PyObjectRef)>,
}

/// Conversion from vector of python objects to function arguments.
impl From<Vec<PyObjectRef>> for PyFuncArgs {
    fn from(args: Vec<PyObjectRef>) -> Self {
        PyFuncArgs {
            args: args,
            kwargs: vec![],
        }
    }
}

impl From<PyObjectRef> for PyFuncArgs {
    fn from(arg: PyObjectRef) -> Self {
        PyFuncArgs {
            args: vec![arg],
            kwargs: vec![],
        }
    }
}

impl PyFuncArgs {
    pub fn new(mut args: Vec<PyObjectRef>, kwarg_names: Vec<String>) -> PyFuncArgs {
        let mut kwargs = vec![];
        for name in kwarg_names.iter().rev() {
            kwargs.push((name.clone(), args.pop().unwrap()));
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
        for (arg_name, arg_value) in self.kwargs.iter() {
            if arg_name == key {
                return arg_value.clone();
            }
        }
        default.clone()
    }

    pub fn get_optional_kwarg(&self, key: &str) -> Option<PyObjectRef> {
        for (arg_name, arg_value) in self.kwargs.iter() {
            if arg_name == key {
                return Some(arg_value.clone());
            }
        }
        None
    }

    pub fn get_optional_kwarg_with_type(
        &self,
        key: &str,
        ty: PyObjectRef,
        vm: &mut VirtualMachine,
    ) -> Result<Option<PyObjectRef>, PyObjectRef> {
        match self.get_optional_kwarg(key) {
            Some(kwarg) => {
                if objtype::isinstance(&kwarg, &ty) {
                    Ok(Some(kwarg))
                } else {
                    let expected_ty_name = vm.to_pystr(&ty)?;
                    let actual_ty_name = vm.to_pystr(&kwarg.typ())?;
                    Err(vm.new_type_error(format!(
                        "argument of type {} is required for named parameter `{}` (got: {})",
                        expected_ty_name, key, actual_ty_name
                    )))
                }
            }
            None => Ok(None),
        }
    }

    /// Serializes these arguments into an iterator starting with the positional
    /// arguments followed by keyword arguments.
    fn into_iter(self) -> impl Iterator<Item = PyArg> {
        self.args.into_iter().map(PyArg::Positional).chain(
            self.kwargs
                .into_iter()
                .map(|(name, value)| PyArg::Keyword(name, value)),
        )
    }

    /// Binds these arguments to their respective values.
    ///
    /// If there is an insufficient number of arguments, there are leftover
    /// arguments after performing the binding, or if an argument is not of
    /// the expected type, a TypeError is raised.
    ///
    /// If the given `FromArgs` includes any conversions, exceptions raised
    /// during the conversion will halt the binding and return the error.
    fn bind<T: FromArgs>(self, vm: &mut VirtualMachine) -> PyResult<T> {
        let given_args = self.args.len();
        let mut args = self.into_iter().peekable();
        let bound = match T::from_args(vm, &mut args) {
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
            Err(ArgumentError::Exception(ex)) => {
                return Err(ex);
            }
        };

        match args.next() {
            None => Ok(bound),
            Some(PyArg::Positional(_)) => Err(vm.new_type_error(format!(
                "Expected at most {} arguments ({} given)",
                T::arity().end(),
                given_args,
            ))),
            Some(PyArg::Keyword(name, _)) => {
                Err(vm.new_type_error(format!("Unexpected keyword argument {}", name)))
            }
        }
    }
}

pub enum PyArg {
    Positional(PyObjectRef),
    Keyword(String, PyObjectRef),
}

/// An error encountered while binding arguments to the parameters of a Python
/// function call.
pub enum ArgumentError {
    /// The call provided fewer positional arguments than the function requires.
    TooFewArgs,
    /// The call provided more positional arguments than the function accepts.
    TooManyArgs,
    /// An exception was raised while binding arguments to the function
    /// parameters.
    Exception(PyObjectRef),
}

impl From<PyObjectRef> for ArgumentError {
    fn from(ex: PyObjectRef) -> Self {
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
    fn from_args<I>(
        vm: &mut VirtualMachine,
        args: &mut iter::Peekable<I>,
    ) -> Result<Self, ArgumentError>
    where
        I: Iterator<Item = PyArg>;
}
/// A map of keyword arguments to their values.
///
/// A built-in function with a `KwArgs` parameter is analagous to a Python
/// function with `*kwargs`. All remaining keyword arguments are extracted
/// (and hence the function will permit an arbitrary number of them).
///
/// `KwArgs` optionally accepts a generic type parameter to allow type checks
/// or conversions of each argument.
pub struct KwArgs<T = PyObjectRef>(HashMap<String, T>);

impl<T> FromArgs for KwArgs<T>
where
    T: TryFromObject,
{
    fn from_args<I>(
        vm: &mut VirtualMachine,
        args: &mut iter::Peekable<I>,
    ) -> Result<Self, ArgumentError>
    where
        I: Iterator<Item = PyArg>,
    {
        let mut kwargs = HashMap::new();
        loop {
            match args.next() {
                Some(PyArg::Keyword(name, value)) => {
                    kwargs.insert(name, T::try_from_object(vm, value)?);
                }
                Some(PyArg::Positional(_)) => {
                    return Err(ArgumentError::TooManyArgs);
                }
                None => {
                    return Ok(KwArgs(kwargs));
                }
            }
        }
    }
}

/// A list of positional argument values.
///
/// A built-in function with a `Args` parameter is analagous to a Python
/// function with `*args`. All remaining positional arguments are extracted
/// (and hence the function will permit an arbitrary number of them).
///
/// `Args` optionally accepts a generic type parameter to allow type checks
/// or conversions of each argument.
pub struct Args<T>(Vec<T>);

impl<T> FromArgs for Args<T>
where
    T: TryFromObject,
{
    fn from_args<I>(
        vm: &mut VirtualMachine,
        args: &mut iter::Peekable<I>,
    ) -> Result<Self, ArgumentError>
    where
        I: Iterator<Item = PyArg>,
    {
        let mut varargs = Vec::new();
        while let Some(PyArg::Positional(value)) = args.next() {
            varargs.push(T::try_from_object(vm, value)?);
        }
        Ok(Args(varargs))
    }
}

impl<T> FromArgs for T
where
    T: TryFromObject,
{
    fn arity() -> RangeInclusive<usize> {
        1..=1
    }

    fn from_args<I>(
        vm: &mut VirtualMachine,
        args: &mut iter::Peekable<I>,
    ) -> Result<Self, ArgumentError>
    where
        I: Iterator<Item = PyArg>,
    {
        if let Some(PyArg::Positional(value)) = args.next() {
            Ok(T::try_from_object(vm, value)?)
        } else {
            Err(ArgumentError::TooFewArgs)
        }
    }
}

/// An argument that may or may not be provided by the caller.
///
/// This style of argument is not possible in pure Python.
pub enum OptionalArg<T> {
    Present(T),
    Missing,
}

impl<T> OptionalArg<T> {
    pub fn into_option(self) -> Option<T> {
        match self {
            Present(value) => Some(value),
            Missing => None,
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

    fn from_args<I>(
        vm: &mut VirtualMachine,
        args: &mut iter::Peekable<I>,
    ) -> Result<Self, ArgumentError>
    where
        I: Iterator<Item = PyArg>,
    {
        Ok(if let Some(PyArg::Positional(_)) = args.peek() {
            let value = if let Some(PyArg::Positional(value)) = args.next() {
                value
            } else {
                unreachable!()
            };
            Present(T::try_from_object(vm, value)?)
        } else {
            Missing
        })
    }
}

// For functions that accept no arguments. Implemented explicitly instead of via
// macro below to avoid unused warnings.
impl FromArgs for () {
    fn from_args<I>(
        _vm: &mut VirtualMachine,
        _args: &mut iter::Peekable<I>,
    ) -> Result<Self, ArgumentError>
    where
        I: Iterator<Item = PyArg>,
    {
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

            fn from_args<I>(
                vm: &mut VirtualMachine,
                args: &mut iter::Peekable<I>
            ) -> Result<Self, ArgumentError>
            where
                I: Iterator<Item = PyArg>
            {
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

/// A built-in Python function.
pub type PyNativeFunc = Box<dyn Fn(&mut VirtualMachine, PyFuncArgs) -> PyResult + 'static>;

/// Implemented by types that are or can generate built-in functions.
///
/// For example, any function that:
///
/// - Accepts a sequence of types that implement `FromArgs`, followed by a
///   `&mut VirtualMachine`
/// - Returns some type that implements `IntoPyObject`
///
/// will generate a `PyNativeFunc` that performs the appropriate type and arity
/// checking, any requested conversions, and then if successful call the function
/// with the bound values.
///
/// A bare `PyNativeFunc` also implements this trait, allowing the above to be
/// done manually, for rare situations that don't fit into this model.
pub trait IntoPyNativeFunc<T, R> {
    fn into_func(self) -> PyNativeFunc;
}

impl<F> IntoPyNativeFunc<PyFuncArgs, PyResult> for F
where
    F: Fn(&mut VirtualMachine, PyFuncArgs) -> PyResult + 'static,
{
    fn into_func(self) -> PyNativeFunc {
        Box::new(self)
    }
}

impl IntoPyNativeFunc<PyFuncArgs, PyResult> for PyNativeFunc {
    fn into_func(self) -> PyNativeFunc {
        self
    }
}

// This is the "magic" that allows rust functions of varying signatures to
// generate native python functions.
//
// Note that this could be done without a macro - it is simply to avoid repetition.
macro_rules! into_py_native_func_tuple {
    ($(($n:tt, $T:ident)),*) => {
        impl<F, $($T,)* R> IntoPyNativeFunc<($($T,)*), R> for F
        where
            F: Fn($($T,)* &mut VirtualMachine) -> R + 'static,
            $($T: FromArgs,)*
            ($($T,)*): FromArgs,
            R: IntoPyObject,
        {
            fn into_func(self) -> PyNativeFunc {
                Box::new(move |vm, args| {
                    let ($($n,)*) = args.bind::<($($T,)*)>(vm)?;

                    (self)($($n,)* vm).into_pyobject(vm)
                })
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

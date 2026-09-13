use crate::{
    AsObject, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject, VirtualMachine,
    builtins::{PyBaseExceptionRef, PyTupleRef, PyTypeRef},
    common::wtf8::{Wtf8, Wtf8Buf},
    convert::ToPyObject,
    object::{Traverse, TraverseFn},
};
use core::ops::{Deref, DerefMut, RangeInclusive};
use indexmap::IndexMap;
use itertools::Itertools;
use std::hash::DefaultHasher;

pub trait IntoFuncArgs: Sized {
    fn into_args(self, vm: &VirtualMachine) -> FuncArgs;
    fn into_method_args(self, obj: PyObjectRef, vm: &VirtualMachine) -> FuncArgs {
        let mut args = self.into_args(vm);
        // Build the final vec once instead of prepending (realloc + memmove).
        let mut with_obj = Vec::with_capacity(args.args.len() + 1);
        with_obj.push(obj);
        with_obj.append(&mut args.args);
        args.args = with_obj;
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

/// The `FuncArgs` struct is one of the most used structs when creating
/// a rust function that can be called from python. It holds both positional
/// arguments, as well as keyword arguments passed to the function.
#[derive(Debug, Default, Clone, Traverse)]
pub struct FuncArgs {
    pub args: Vec<PyObjectRef>,
    // sorted map, according to https://www.python.org/dev/peps/pep-0468/
    pub kwargs: KwArgs,
}

/// Conversion from vector of python objects to function arguments.
impl<A> From<A> for FuncArgs
where
    A: Into<PosArgs>,
{
    fn from(args: A) -> Self {
        Self {
            args: args.into().into_vec(),
            ..Default::default()
        }
    }
}

impl From<KwArgs> for FuncArgs {
    fn from(kwargs: KwArgs) -> Self {
        Self {
            kwargs,
            ..Default::default()
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
        Self {
            args,
            kwargs: kwargs.into(),
        }
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

        let kwargs = kwarg_names.zip_eq(args).collect();

        Self {
            args: pos_args,
            kwargs,
        }
    }

    /// Create FuncArgs from a vectorcall-style argument slice (PEP 590).
    /// `args[..nargs]` are positional, and if `kwnames` is provided,
    /// the last `kwnames.len()` entries in `args[nargs..]` are keyword values.
    /// Convert borrowed vectorcall args to FuncArgs (clones all values).
    #[must_use]
    pub fn from_vectorcall(
        args: &[PyObjectRef],
        nargs: usize,
        kwnames: Option<&[PyObjectRef]>,
    ) -> Self {
        debug_assert!(nargs <= args.len());
        debug_assert!(kwnames.is_none_or(|kw| nargs + kw.len() <= args.len()));

        let pos_args = args[..nargs].to_vec();

        let kwargs = kwnames.map_or_else(KwArgs::default, |names| {
            names
                .iter()
                .zip(&args[nargs..nargs + names.len()])
                .map(|(name, val)| {
                    // `PyStr`, not `PyUtf8Str`: a surrogate key is a valid str and
                    // must survive as WTF-8 rather than panic.
                    let key = name
                        .downcast_ref::<crate::builtins::PyStr>()
                        .expect("kwnames must be strings")
                        .as_wtf8()
                        .to_owned();
                    (key, val.clone())
                })
                .collect()
        });

        Self {
            args: pos_args,
            kwargs,
        }
    }

    /// Convert owned vectorcall args to FuncArgs (moves values, no clone).
    #[must_use]
    pub fn from_vectorcall_owned(
        mut args: Vec<PyObjectRef>,
        nargs: usize,
        kwnames: Option<&[PyObjectRef]>,
    ) -> Self {
        debug_assert!(nargs <= args.len());
        debug_assert!(kwnames.is_none_or(|kw| nargs + kw.len() <= args.len()));
        let kwargs = kwnames.map_or_else(KwArgs::default, |names| {
            let kw_count = names.len();
            names
                .iter()
                .zip(args.drain(nargs..nargs + kw_count))
                .map(|(name, val)| {
                    let key = name
                        .downcast_ref::<crate::builtins::PyStr>()
                        .expect("kwnames must be strings")
                        .as_wtf8()
                        .to_owned();
                    (key, val)
                })
                .collect()
        });

        args.truncate(nargs);
        Self { args, kwargs }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.args.is_empty() && self.kwargs.is_empty()
    }

    pub fn prepend_arg(&mut self, item: PyObjectRef) {
        // reserve (not reserve_exact): incoming vectors are usually built with
        // exact capacity, so exact growth would realloc on every prepend.
        self.args.reserve(1);
        self.args.insert(0, item)
    }

    pub fn shift(&mut self) -> PyObjectRef {
        self.args.remove(0)
    }

    #[must_use]
    pub fn get_kwarg(&self, key: &str, default: PyObjectRef) -> PyObjectRef {
        self.kwargs
            .get(key)
            .cloned()
            .unwrap_or_else(|| default.clone())
    }

    #[must_use]
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

    pub fn remaining_keywords(&mut self) -> impl Iterator<Item = (Wtf8Buf, PyObjectRef)> + '_ {
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
    pub fn bind<T: FromArgs>(self, vm: &VirtualMachine) -> PyResult<T> {
        self.bind_for(vm, Callee::default())
    }

    /// Binds these arguments the way [`bind`](Self::bind) does, for a call whose
    /// function a failure can describe.
    pub fn bind_for<T: FromArgs>(
        mut self,
        vm: &VirtualMachine,
        callee: impl Into<Callee>,
    ) -> PyResult<T> {
        let callee = callee.into();
        // A message describes the parameters the function declares, and the
        // instance a method is called on is not one of them.
        let instance = callee.instance_args();
        let arity = T::arity();
        let arity = arity.start().saturating_sub(instance)..=arity.end().saturating_sub(instance);
        let num_given = self.args.len().saturating_sub(instance);

        let bound = T::from_args(vm, &mut self)
            .map_err(|e| e.into_exception(&arity, num_given, callee, vm))?;

        if !self.args.is_empty() {
            Err(ArgumentError::TooManyArgs.into_exception(&arity, num_given, callee, vm))
        } else if let Some(err) = self.check_kwargs_empty_for(vm, callee) {
            Err(err)
        } else {
            Ok(bound)
        }
    }

    pub fn check_kwargs_empty(&self, vm: &VirtualMachine) -> Option<PyBaseExceptionRef> {
        self.check_kwargs_empty_for(vm, Callee::default())
    }

    /// The same as [`check_kwargs_empty`](Self::check_kwargs_empty), for a call
    /// whose function the message can name.
    pub fn check_kwargs_empty_for(
        &self,
        vm: &VirtualMachine,
        callee: impl Into<Callee>,
    ) -> Option<PyBaseExceptionRef> {
        let callee = callee.into();
        self.kwargs
            .keys()
            .next()
            .map(|k| callee.unexpected_keyword(&k.to_string(), vm))
    }
}

/// What a message says about the function whose arguments are being bound.
///
/// A binding that happens somewhere the name isn't known leaves it off, the way
/// `_PyArg_Parser.fname` is NULL. A message describes the parameters the
/// function declares, so a method's instance argument counts as neither an
/// expected parameter nor a given argument, the way `descrobject.c` reports
/// `nargs - 1`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Callee {
    name: Option<&'static str>,
    instance_arg: bool,
}

impl From<&'static str> for Callee {
    fn from(name: &'static str) -> Self {
        Self::named(name)
    }
}

impl Callee {
    /// A function a message can name.
    #[must_use]
    pub const fn named(name: &'static str) -> Self {
        Self {
            name: Some(name),
            instance_arg: false,
        }
    }

    /// The type a slot builds or initializes, named the way a message names it.
    #[must_use]
    pub fn for_type(class: &crate::Py<crate::builtins::PyType>) -> Self {
        Self::named(class.slots.name)
    }

    /// The same, for the type a slot was written for.
    #[must_use]
    pub fn of<T: crate::PyPayload>(vm: &VirtualMachine) -> Self {
        Self::for_type(T::class(&vm.ctx))
    }

    /// Marks a call whose leading argument fills the method's instance parameter.
    #[must_use]
    pub const fn with_instance_arg(mut self, instance_arg: bool) -> Self {
        self.instance_arg = instance_arg;
        self
    }

    /// How many leading arguments answer for the instance rather than for a
    /// parameter the method declares.
    const fn instance_args(self) -> usize {
        self.instance_arg as usize
    }

    /// _PyArg_CheckPositional
    fn wrong_arity(
        self,
        arity: &RangeInclusive<usize>,
        too_few: bool,
        num_given: usize,
        vm: &VirtualMachine,
    ) -> PyBaseExceptionRef {
        vm.new_type_error(arity_message(self.name, arity, too_few, num_given))
    }

    /// The branch of _PyArg_UnpackKeywords that names a keyword it didn't expect.
    fn unexpected_keyword(self, keyword: &str, vm: &VirtualMachine) -> PyBaseExceptionRef {
        vm.new_type_error(unexpected_keyword_message(self.name, keyword))
    }

    /// The branch of _PyArg_UnpackKeywords that names a parameter it didn't get.
    fn missing_argument(
        self,
        keyword: &str,
        pos: usize,
        vm: &VirtualMachine,
    ) -> PyBaseExceptionRef {
        vm.new_type_error(missing_argument_message(self.name, keyword, pos))
    }
}

/// The name a message uses. `tp_name` carries the module along with the type,
/// and a message names only the type itself.
fn short_name(name: &str) -> &str {
    name.rsplit_once('.').map_or(name, |(_, name)| name)
}

/// The name a message opens with and the parentheses that follow it, given what
/// to call a function whose name isn't known.
fn call_form<'a>(name: Option<&'a str>, unnamed: &'a str) -> (&'a str, &'static str) {
    match name.map(short_name) {
        Some(name) => (name, "()"),
        None => (unnamed, ""),
    }
}

/// _PyArg_CheckPositional
pub(crate) fn arity_message(
    name: Option<&str>,
    arity: &RangeInclusive<usize>,
    too_few: bool,
    num_given: usize,
) -> String {
    let (limit, bound) = if too_few {
        (*arity.start(), "at least ")
    } else {
        (*arity.end(), "at most ")
    };
    let bound = if arity.start() == arity.end() {
        ""
    } else {
        bound
    };
    let plural = if limit == 1 { "" } else { "s" };
    let name = name
        .map(short_name)
        .map_or_else(String::new, |name| format!("{name} "));
    format!("{name}expected {bound}{limit} argument{plural}, got {num_given}")
}

/// The branch of _PyArg_UnpackKeywords that names a keyword it didn't expect.
pub(crate) fn unexpected_keyword_message(name: Option<&str>, keyword: &str) -> String {
    let (name, parens) = call_form(name, "this function");
    format!("{name}{parens} got an unexpected keyword argument '{keyword}'")
}

/// The branch of _PyArg_UnpackKeywords that names a parameter it didn't get.
pub(crate) fn missing_argument_message(name: Option<&str>, keyword: &str, pos: usize) -> String {
    let (name, parens) = call_form(name, "function");
    format!("{name}{parens} missing required argument '{keyword}' (pos {pos})")
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
    /// The function requires an argument for the named parameter, at the given
    /// 1-based position, but the call didn't pass one.
    MissingRequiredArgument { name: String, pos: usize },
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
        arity: &RangeInclusive<usize>,
        num_given: usize,
        callee: Callee,
        vm: &VirtualMachine,
    ) -> PyBaseExceptionRef {
        match self {
            Self::TooFewArgs => callee.wrong_arity(arity, true, num_given, vm),
            Self::TooManyArgs => callee.wrong_arity(arity, false, num_given, vm),
            Self::InvalidKeywordArgument(name) => callee.unexpected_keyword(&name, vm),
            Self::MissingRequiredArgument { name, pos } => callee.missing_argument(&name, pos, vm),
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
    #[must_use]
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
// Keys are stored as `Wtf8Buf`, not `String`, so that a lone-surrogate keyword
// name coming through `f(**d)` is preserved instead of being rejected (see
// issue #8228). `PyStr` is WTF-8 backed, and CPython only requires that a
// keyword key be a `str`, not that it be valid UTF-8.
#[derive(Clone, Debug)]
pub struct KwArgs<T = PyObjectRef>(KwArgsMap<T>);

/// The map behind [`KwArgs`].
///
/// The hasher is zero-sized rather than the randomly seeded default: a
/// `KwArgs` is built for every call, including the far more common
/// keyword-less one, and seeding reads a thread-local. Keyword names come
/// from the program text, so per-process hash randomization buys nothing.
pub type KwArgsMap<T> = IndexMap<Wtf8Buf, T, core::hash::BuildHasherDefault<DefaultHasher>>;

impl<T> Default for KwArgs<T> {
    fn default() -> Self {
        Self(KwArgsMap::default())
    }
}

impl<T> Deref for KwArgs<T> {
    type Target = KwArgsMap<T>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for KwArgs<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

unsafe impl<T> Traverse for KwArgs<T>
where
    T: Traverse,
{
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        self.values().for_each(|v| v.traverse(tracer_fn));
    }
}

impl<T> KwArgs<T> {
    #[must_use]
    pub const fn new(map: KwArgsMap<T>) -> Self {
        Self(map)
    }

    // `String` keys accepted `&str` lookups for free via `Borrow<str>`; `Wtf8Buf`
    // borrows only as `Wtf8`, so these inherent methods restore the `&str` interface
    // via the zero-cost `Wtf8::new` cast, keeping every call site unchanged.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&T> {
        self.0.get(Wtf8::new(name))
    }

    #[must_use]
    pub fn contains_key(&self, name: &str) -> bool {
        self.0.contains_key(Wtf8::new(name))
    }

    pub fn swap_remove(&mut self, name: &str) -> Option<T> {
        self.0.swap_remove(Wtf8::new(name))
    }

    pub fn shift_remove(&mut self, name: &str) -> Option<T> {
        self.0.shift_remove(Wtf8::new(name))
    }

    pub fn pop_kwarg(&mut self, name: &str) -> Option<T> {
        self.swap_remove(name)
    }
}

// Accept any key that converts into `Wtf8Buf` (notably `String`), so existing
// call sites that build kwargs from string literals keep compiling unchanged.
impl<K: Into<Wtf8Buf>, T> FromIterator<(K, T)> for KwArgs<T> {
    fn from_iter<I: IntoIterator<Item = (K, T)>>(iter: I) -> Self {
        Self(iter.into_iter().map(|(k, v)| (k.into(), v)).collect())
    }
}

impl<'a, T> IntoIterator for &'a KwArgs<T> {
    type Item = (&'a Wtf8Buf, &'a T);
    type IntoIter = indexmap::map::Iter<'a, Wtf8Buf, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl<T> IntoIterator for KwArgs<T> {
    type Item = (Wtf8Buf, T);
    type IntoIter = indexmap::map::IntoIter<Wtf8Buf, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<T> FromArgs for KwArgs<T>
where
    T: TryFromObject,
{
    fn from_args(vm: &VirtualMachine, args: &mut FuncArgs) -> Result<Self, ArgumentError> {
        let mut kwargs = KwArgsMap::default();
        for (name, value) in args.remaining_keywords() {
            kwargs.insert(name, value.try_into_value(vm)?);
        }
        Ok(Self(kwargs))
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
    #[must_use]
    pub const fn new(args: Vec<T>) -> Self {
        Self(args)
    }

    #[must_use]
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

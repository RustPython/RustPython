use super::signature::{Param, ParamKind, params_take_no_keywords};
use crate::{
    AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject, VirtualMachine,
    builtins::{PyBaseExceptionRef, PyTupleRef, PyType},
    common::wtf8::{Wtf8, Wtf8Buf},
    convert::ToPyObject,
    object::{Traverse, TraverseFn},
};
use alloc::borrow::Cow;
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

impl<Name: ArgName> From<KwArgs<PyObjectRef, Name>> for FuncArgs {
    fn from(kwargs: KwArgs<PyObjectRef, Name>) -> Self {
        Self {
            kwargs: KwArgs::new(kwargs.0),
            ..Default::default()
        }
    }
}

impl FromArgs for FuncArgs {
    const PARAMS: Option<&'static [Param]> =
        Some(&[Param::var_positional("args"), Param::var_keyword("kwargs")]);

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
        let PosArgs(args, _) = args.into();
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
    pub fn get_kwarg(&self, key: &str, default: &PyObject) -> PyObjectRef {
        self.kwargs
            .get(key)
            .cloned()
            .unwrap_or_else(|| default.to_owned())
    }

    #[must_use]
    pub fn get_optional_kwarg(&self, key: &str) -> Option<PyObjectRef> {
        self.kwargs.get(key).cloned()
    }

    pub fn get_optional_kwarg_with_type(
        &self,
        key: &str,
        ty: &Py<PyType>,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>> {
        match self.get_optional_kwarg(key) {
            Some(kwarg) => {
                if kwarg.fast_isinstance(ty) {
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
        let nargs = self.args.len();
        let num_given = nargs.saturating_sub(instance);

        if let Some(err) = T::check_before_convert(&self, &arity, num_given, instance, callee, vm) {
            return Err(err);
        }

        let bound = match T::from_args(vm, &mut self) {
            Ok(bound) => bound,
            Err(e) => {
                // take_positional has just removed the failed argument, so N is
                // nargs - remaining, minus the instance.
                let pos = nargs
                    .saturating_sub(self.args.len())
                    .saturating_sub(instance);
                return Err(e.into_exception(&arity, num_given, pos, callee, vm));
            }
        };

        if !self.args.is_empty() {
            Err(ArgumentError::TooManyArgs.into_exception(&arity, num_given, 0, callee, vm))
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

    fn given_by_name_and_position(
        self,
        keyword: &str,
        pos: usize,
        vm: &VirtualMachine,
    ) -> PyBaseExceptionRef {
        vm.new_type_error(given_by_name_and_position_message(self.name, keyword, pos))
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

    /// _PyArg_NoKeywords
    fn no_keywords(self, vm: &VirtualMachine) -> PyBaseExceptionRef {
        vm.new_type_error(no_keywords_message(self.name))
    }

    /// _PyArg_BadArgument
    fn bad_argument(
        self,
        displayname: &str,
        expected: &'static Py<PyType>,
        or_none: bool,
        found: &PyObject,
        vm: &VirtualMachine,
    ) -> PyBaseExceptionRef {
        let expected_ty = expected.slot_name();
        let expected = if or_none {
            Cow::Owned(format!("{expected_ty} or None"))
        } else {
            Cow::Borrowed(&*expected_ty)
        };
        let class = found.class();
        let found_ty = class.slot_name();
        let found_name = if vm.is_none(found) {
            "None"
        } else {
            &*found_ty
        };
        vm.new_type_error(bad_argument_message(
            self.name,
            displayname,
            &expected,
            found_name,
        ))
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

/// _PyArg_NoKeywords
pub(crate) fn no_keywords_message(name: Option<&str>) -> String {
    let (name, parens) = call_form(name, "this function");
    format!("{name}{parens} takes no keyword arguments")
}

/// get_displayname
fn get_displayname(name: Option<&str>, pos: usize, meth_o: bool) -> Cow<'static, str> {
    if let Some(name) = name {
        Cow::Owned(format!("argument '{name}'"))
    } else if meth_o {
        Cow::Borrowed("argument")
    } else {
        Cow::Owned(format!("argument {pos}"))
    }
}

/// _PyArg_BadArgument
pub(crate) fn bad_argument_message(
    name: Option<&str>,
    displayname: &str,
    expected: &str,
    found: &str,
) -> String {
    let (name, parens) = call_form(name, "this function");
    format!("{name}{parens} {displayname} must be {expected}, not {found}")
}

pub(crate) fn given_by_name_and_position_message(
    name: Option<&str>,
    keyword: &str,
    pos: usize,
) -> String {
    let (name, parens) = call_form(name, "function");
    format!("argument for {name}{parens} given by name ('{keyword}') and position ({pos})")
}

/// Compile-time counts for a binding whose entire parameter tree is known.
struct BindingShape {
    fixed: usize,
    positional: usize,
    required_positional: usize,
    required_positional_only: usize,
    has_keywords: bool,
    var_positional: bool,
    var_keyword: bool,
    required_keyword_only: bool,
    anonymous_positional: bool,
}

impl BindingShape {
    const fn validated(params: Option<&[Param]>, skip: usize) -> Option<Self> {
        const fn known(params: Option<&[Param]>) -> bool {
            let Some(params) = params else {
                return false;
            };
            let mut i = 0;
            while i < params.len() {
                if let ParamKind::Flatten(inner) = params[i].kind
                    && !known(inner)
                {
                    return false;
                }
                i += 1;
            }
            true
        }

        if !known(params) {
            return None;
        }
        assert!(
            Self::from_params(params, 0).is_some(),
            "FromArgs binding parameters must describe one Clinic signature; use an explicit custom binding override for a staged parser",
        );
        Self::from_params(params, skip)
    }

    const fn arity(&self) -> RangeInclusive<usize> {
        self.required_positional..=if self.var_positional {
            usize::MAX
        } else {
            self.positional
        }
    }

    const fn from_params(params: Option<&[Param]>, mut skip: usize) -> Option<Self> {
        let Some(params) = params else {
            return None;
        };
        let mut shape = Self {
            fixed: 0,
            positional: 0,
            required_positional: 0,
            required_positional_only: 0,
            has_keywords: false,
            var_positional: false,
            var_keyword: false,
            required_keyword_only: false,
            anonymous_positional: false,
        };
        if shape.count(params, &mut skip) && skip == 0 {
            Some(shape)
        } else {
            None
        }
    }

    const fn count(&mut self, params: &[Param], skip: &mut usize) -> bool {
        let mut i = 0;
        while i < params.len() {
            let param = &params[i];
            if self.var_keyword && !matches!(param.kind, ParamKind::Flatten(_)) {
                return false;
            }
            match param.kind {
                ParamKind::Flatten(Some(inner)) => {
                    if !self.count(inner, skip) {
                        return false;
                    }
                }
                ParamKind::Flatten(None) => return false,
                ParamKind::PositionalOnly if *skip > 0 => *skip -= 1,
                ParamKind::PositionalOnly | ParamKind::PositionalOrKeyword => {
                    // Other tuple compositions are valid Rust binders, but do
                    // not describe the ordered layout of one Clinic parser.
                    if self.var_positional
                        || self.fixed > self.positional
                        || (matches!(param.kind, ParamKind::PositionalOnly) && self.has_keywords)
                    {
                        return false;
                    }
                    self.fixed += 1;
                    self.positional += 1;
                    if param.default.is_none() {
                        self.required_positional = self.positional;
                    }
                    if matches!(param.kind, ParamKind::PositionalOnly) {
                        self.anonymous_positional |= param.name.is_empty();
                        if param.default.is_none() {
                            self.required_positional_only = self.positional;
                        }
                    } else {
                        self.has_keywords = true;
                    }
                }
                ParamKind::KeywordOnly => {
                    self.fixed += 1;
                    self.has_keywords = true;
                    self.required_keyword_only |= param.default.is_none();
                }
                ParamKind::VarPositional => {
                    if self.var_positional || self.fixed > self.positional {
                        return false;
                    }
                    self.var_positional = true;
                }
                ParamKind::VarKeyword => {
                    self.has_keywords = true;
                    self.var_keyword = true;
                }
            }
            i += 1;
        }
        true
    }

    fn check(
        &self,
        params: &[Param],
        args: &FuncArgs,
        arity: &RangeInclusive<usize>,
        instance: usize,
        callee: Callee,
        vm: &VirtualMachine,
    ) -> Option<PyBaseExceptionRef> {
        let nargs = args.args.len().saturating_sub(instance);
        let nkwargs = args.kwargs.len();
        let binding_arity = self.arity();
        if !self.has_keywords {
            if nkwargs > 0 {
                return Some(callee.no_keywords(vm));
            }
            return Some(callee.wrong_arity(
                &binding_arity,
                nargs < self.required_positional,
                nargs,
                vm,
            ));
        }

        // Bare positional Rust arguments use _PyArg_CheckPositional, including
        // an outer argument followed by a separately bound options struct.
        if self.anonymous_positional
            && (nargs > *binding_arity.end() || nargs < self.required_positional_only)
        {
            return Some(callee.wrong_arity(
                &binding_arity,
                nargs < self.required_positional,
                nargs,
                vm,
            ));
        }
        let (name, parens) = call_form(callee.name, "function");
        if !self.anonymous_positional
            && !self.var_positional
            && !self.var_keyword
            && nargs + nkwargs > self.fixed
        {
            return Some(vm.new_type_error(format!(
                "{name}{parens} takes at most {} {}argument{} ({} given)",
                self.fixed,
                if nargs == 0 { "keyword " } else { "" },
                if self.fixed == 1 { "" } else { "s" },
                nargs + nkwargs,
            )));
        }
        if !self.var_positional && nargs > self.positional {
            return Some(vm.new_type_error(if self.positional == 0 {
                format!("{name}{parens} takes no positional arguments")
            } else {
                format!(
                    "{name}{parens} takes {} {} positional argument{} ({nargs} given)",
                    if self.required_positional == self.positional {
                        "exactly"
                    } else {
                        "at most"
                    },
                    self.positional,
                    if self.positional == 1 { "" } else { "s" },
                )
            }));
        }
        if nargs < self.required_positional_only {
            return Some(vm.new_type_error(format!(
                "{name}{parens} takes {} {} positional argument{} ({nargs} given)",
                if self.var_positional || self.required_positional_only < self.positional {
                    "at least"
                } else {
                    "exactly"
                },
                self.required_positional_only,
                if self.required_positional_only == 1 {
                    ""
                } else {
                    "s"
                },
            )));
        }

        // Without positional or required arguments, only keyword names need
        // checking. Avoid walking optional fields on calls such as list.sort.
        if nargs != 0 || self.required_positional != 0 || self.required_keyword_only {
            let mut matched = BindingMatch {
                positional: 0,
                position: 0,
                skip: instance,
                duplicate: None,
            };
            if let Err(missing) = matched.walk(params, nargs, &args.kwargs) {
                return Some(missing.into_exception(arity, nargs, 0, callee, vm));
            }
            if let Some((name, pos)) = matched.duplicate {
                return Some(callee.given_by_name_and_position(name, pos, vm));
            }
        }
        if !self.var_keyword {
            for name in args.kwargs.keys() {
                if !binding_accepts_keyword(params, name) {
                    return Some(callee.unexpected_keyword(&name.to_string(), vm));
                }
            }
        }
        None
    }
}

struct BindingMatch<'a> {
    positional: usize,
    position: usize,
    skip: usize,
    duplicate: Option<(&'a str, usize)>,
}

impl<'a> BindingMatch<'a> {
    fn walk(
        &mut self,
        params: &'a [Param],
        nargs: usize,
        kwargs: &KwArgs,
    ) -> Result<(), ArgumentError> {
        for param in params {
            match param.kind {
                ParamKind::Flatten(Some(inner)) => self.walk(inner, nargs, kwargs)?,
                ParamKind::PositionalOnly if self.skip > 0 => self.skip -= 1,
                ParamKind::PositionalOnly => {
                    self.positional += 1;
                    self.position += 1;
                }
                ParamKind::PositionalOrKeyword | ParamKind::KeywordOnly => {
                    self.position += 1;
                    let positional = matches!(param.kind, ParamKind::PositionalOrKeyword);
                    if positional {
                        self.positional += 1;
                    }
                    let by_position = positional && self.positional <= nargs;
                    if by_position {
                        if self.duplicate.is_none() && kwargs.contains_key(param.name) {
                            self.duplicate = Some((param.name, self.position));
                        }
                    } else if param.default.is_none() && !kwargs.contains_key(param.name) {
                        return Err(ArgumentError::MissingRequiredArgument {
                            name: param.name.to_owned(),
                            pos: self.position,
                        });
                    }
                }
                ParamKind::VarPositional | ParamKind::VarKeyword => {}
                ParamKind::Flatten(None) => unreachable!("binding shape was checked"),
            }
        }
        Ok(())
    }
}

fn binding_accepts_keyword(params: &[Param], name: &Wtf8) -> bool {
    params.iter().any(|param| match param.kind {
        ParamKind::Flatten(Some(inner)) => binding_accepts_keyword(inner, name),
        ParamKind::PositionalOrKeyword | ParamKind::KeywordOnly => Wtf8::new(param.name) == name,
        ParamKind::VarKeyword => true,
        _ => false,
    })
}

/// Walk `params` in declaration order looking for the first required argument
/// that is missing. `Err(None)` ends the walk without a result.
fn missing_from_params(
    params: &[Param],
    nargs: usize,
    kwargs: &KwArgs,
    pos: &mut usize,
) -> Result<(), Option<ArgumentError>> {
    for param in params {
        match param.kind {
            ParamKind::Flatten(Some(inner)) => {
                missing_from_params(inner, nargs, kwargs, pos)?;
            }
            ParamKind::Flatten(None) => *pos += 1,
            ParamKind::PositionalOnly => {
                *pos += 1;
                if *pos > nargs && param.default.is_none() {
                    return Err(Some(ArgumentError::TooFewArgs));
                }
            }
            ParamKind::PositionalOrKeyword => {
                *pos += 1;
                if *pos > nargs && param.default.is_none() && !kwargs.contains_key(param.name) {
                    return Err(Some(ArgumentError::MissingRequiredArgument {
                        name: param.name.to_owned(),
                        pos: *pos,
                    }));
                }
            }
            ParamKind::VarPositional => {}
            ParamKind::KeywordOnly => {
                *pos += 1;
                if param.default.is_none() && !kwargs.contains_key(param.name) {
                    return Err(Some(ArgumentError::MissingRequiredArgument {
                        name: param.name.to_owned(),
                        pos: *pos,
                    }));
                }
            }
            ParamKind::VarKeyword => {}
        }
    }
    Ok(())
}

/// Whether `params` includes a required keyword-only parameter, including
/// nested `Flatten(Some(_))` lists.
#[must_use]
pub const fn params_have_required_keyword_only(params: Option<&'static [Param]>) -> bool {
    let Some(params) = params else {
        return false;
    };
    let mut i = 0;
    while i < params.len() {
        match params[i].kind {
            ParamKind::KeywordOnly if params[i].default.is_none() => return true,
            ParamKind::Flatten(inner) if params_have_required_keyword_only(inner) => {
                return true;
            }
            _ => {}
        }
        i += 1;
    }
    false
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
    /// The argument is not the expected type. `_PyArg_BadArgument`.
    /// `name` is `None` for a positional-only parameter.
    BadArgument {
        expected: &'static Py<PyType>,
        or_none: bool,
        found: PyObjectRef,
        name: Option<&'static str>,
    },
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
    /// Attach the parameter name for get_displayname (`argument 'name'`).
    #[must_use]
    pub fn with_argument_name(self, name: &'static str) -> Self {
        match self {
            Self::BadArgument {
                expected,
                or_none,
                found,
                ..
            } => Self::BadArgument {
                expected,
                or_none,
                found,
                name: Some(name),
            },
            other => other,
        }
    }

    /// Mark `{expected}` as `X or None`.
    #[must_use]
    pub(crate) fn with_or_none(self) -> Self {
        match self {
            Self::BadArgument {
                expected,
                found,
                name,
                ..
            } => Self::BadArgument {
                expected,
                or_none: true,
                found,
                name,
            },
            other => other,
        }
    }

    fn into_exception(
        self,
        arity: &RangeInclusive<usize>,
        num_given: usize,
        pos: usize,
        callee: Callee,
        vm: &VirtualMachine,
    ) -> PyBaseExceptionRef {
        match self {
            Self::TooFewArgs => callee.wrong_arity(arity, true, num_given, vm),
            Self::TooManyArgs => callee.wrong_arity(arity, false, num_given, vm),
            Self::InvalidKeywordArgument(name) => callee.unexpected_keyword(&name, vm),
            Self::MissingRequiredArgument { name, pos } => callee.missing_argument(&name, pos, vm),
            Self::BadArgument {
                expected,
                or_none,
                found,
                name,
            } => {
                if callee.name.is_none() || (name.is_none() && pos == 0) {
                    vm.new_downcast_type_error(expected, &found)
                } else {
                    let meth_o = *arity.start() == 1 && *arity.end() == 1;
                    let displayname = get_displayname(name, pos, meth_o);
                    callee.bad_argument(&displayname, expected, or_none, &found, vm)
                }
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
    /// By default this is computed from the binding parameters. An explicit
    /// custom parser without binding metadata has no positional upper bound.
    #[must_use]
    fn arity() -> RangeInclusive<usize> {
        match const { BindingShape::validated(Self::BINDING_PARAMS, 0) } {
            Some(shape) => shape.arity(),
            None => 0..=usize::MAX,
        }
    }

    /// Parameters this type contributes to a text signature.
    ///
    /// `None`: the argument is one positional-only parameter, named by the
    /// function argument. `Some(&[])`: the argument contributes nothing.
    /// `Some(params)`: the type supplies those parameters and the argument name
    /// is ignored.
    const PARAMS: Option<&'static [Param]> = None;

    /// Parameters validated with Clinic rules before any argument conversion.
    /// Both manual implementations and derives use `PARAMS` by default; its
    /// unnamed scalar is represented here by one positional-only slot.
    ///
    /// Override this when the accepted arguments differ from the displayed
    /// signature. `None` explicitly delegates validation to a custom parser,
    /// retaining the conservative checks below. Such exceptions should explain
    /// why CPython needs different validation order or diagnostics.
    const BINDING_PARAMS: Option<&'static [Param]> = match Self::PARAMS {
        Some(params) => Some(params),
        None => Some(&[Param::positional_only("")]),
    };

    /// True when every parameter is positional-only or `*args`.
    const TAKES_NO_KEYWORDS: bool = match Self::PARAMS {
        Some(params) => params_take_no_keywords(params),
        None => false,
    };

    /// True when a required keyword-only parameter can be missing even after
    /// every required positional argument is given. Evaluated from [`PARAMS`]
    /// unless an implementor overrides it.
    const HAS_REQUIRED_KEYWORD_ONLY: bool = params_have_required_keyword_only(Self::PARAMS);

    /// Walk this binding looking for the first required argument that is
    /// missing. `Ok` continues; `Err(Some(_))` is that error; `Err(None)`
    /// ends the walk without a result (unknown shape).
    ///
    /// `skip` leading positional-only slots are the method instance: they do
    /// not consume a caller-supplied argument or advance `pos`.
    fn missing_from(
        nargs: usize,
        kwargs: &KwArgs,
        pos: &mut usize,
        skip: &mut usize,
    ) -> Result<(), Option<ArgumentError>> {
        if let Some(params) = Self::PARAMS {
            missing_from_params(params, nargs, kwargs, pos)
        } else {
            let arity = Self::arity();
            if *arity.start() == 1 && *arity.end() == 1 {
                if *skip > 0 {
                    *skip -= 1;
                    Ok(())
                } else {
                    *pos += 1;
                    if *pos > nargs {
                        Err(Some(ArgumentError::TooFewArgs))
                    } else {
                        Ok(())
                    }
                }
            } else {
                Err(None)
            }
        }
    }

    /// Validate a known binding shape before converting any argument, following
    /// `_PyArg_UnpackKeywords` error ordering. Explicit custom parsers keep the
    /// positional-count and no-keyword checks, but own named validation.
    /// Invalid known parameter ordering is rejected at compile time.
    ///
    /// The missing-argument walk is skipped when every required positional
    /// argument is present and the type has no required keyword-only parameter.
    #[inline]
    fn check_before_convert(
        args: &FuncArgs,
        arity: &RangeInclusive<usize>,
        num_given: usize,
        instance: usize,
        callee: Callee,
        vm: &VirtualMachine,
    ) -> Option<PyBaseExceptionRef> {
        let shape = if instance == 0 {
            const { BindingShape::validated(Self::BINDING_PARAMS, 0) }
        } else {
            const { BindingShape::validated(Self::BINDING_PARAMS, 1) }
        };
        if let Some(shape) = shape {
            if args.kwargs.is_empty()
                && num_given >= shape.required_positional
                && (shape.var_positional || num_given <= shape.positional)
                && !shape.required_keyword_only
            {
                return None;
            }
            return shape.check(
                Self::BINDING_PARAMS.unwrap(),
                args,
                arity,
                instance,
                callee,
                vm,
            );
        }

        // A function that takes no keywords refuses them before counting its
        // arguments.
        if Self::TAKES_NO_KEYWORDS && !args.kwargs.is_empty() {
            return Some(callee.no_keywords(vm));
        }
        if num_given > *arity.end() {
            return Some(
                ArgumentError::TooManyArgs.into_exception(arity, num_given, 0, callee, vm),
            );
        }
        // No keyword can fill a missing parameter of such a function, so too few
        // arguments are also reported before any is converted.
        if Self::TAKES_NO_KEYWORDS && num_given < *arity.start() {
            return Some(ArgumentError::TooFewArgs.into_exception(arity, num_given, 0, callee, vm));
        }
        None
    }

    /// Extracts this item from the next argument(s).
    fn from_args(vm: &VirtualMachine, args: &mut FuncArgs) -> Result<Self, ArgumentError>;
}

pub trait FromArgOptional {
    type Inner: TryFromObject;
    fn from_inner(x: Self::Inner) -> Self;
}

/// Signature default of a field marked `optional`.
///
/// Only [`Option`] (`None`) and [`OptionalArg`] (`<unrepresentable>`) are valid.
pub trait OptionalArgDefault {
    const PY_DEFAULT: super::signature::DefaultRepr;
}

impl<T> OptionalArgDefault for Option<T> {
    const PY_DEFAULT: super::signature::DefaultRepr = super::signature::DefaultRepr::None;
}

impl<T> OptionalArgDefault for OptionalArg<T> {
    const PY_DEFAULT: super::signature::DefaultRepr =
        super::signature::DefaultRepr::Unrepresentable;
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
pub trait ArgName {
    const NAME: &'static str;
}

macro_rules! arg_name {
    ($($ty:ident = $name:literal),* $(,)?) => {$(
        #[derive(Clone, Copy, Debug)]
        pub struct $ty;
        impl ArgName for $ty {
            const NAME: &'static str = $name;
        }
    )*};
}

arg_name! {
    NameArgs = "args",
    NameKwargs = "kwargs",
    NameOthers = "others",
    NameCoordinates = "coordinates",
    NameIntegers = "integers",
    NameChanges = "changes",
    NameExcInfo = "exc_info",
    NameKwds = "kwds",
    NameKws = "kws",
    NameObjs = "objs",
    NameObjects = "objects",
    NameIterables = "iterables",
    NameKeywords = "keywords",
    NameFields = "fields",
    NameValues = "values",
}

#[derive(Clone, Debug)]
pub struct KwArgs<T = PyObjectRef, Name: ArgName = NameKwargs>(
    KwArgsMap<T>,
    core::marker::PhantomData<Name>,
);

/// The map behind [`KwArgs`].
///
/// The hasher is zero-sized rather than the randomly seeded default: a
/// `KwArgs` is built for every call, including the far more common
/// keyword-less one, and seeding reads a thread-local. Keyword names come
/// from the program text, so per-process hash randomization buys nothing.
pub type KwArgsMap<T> = IndexMap<Wtf8Buf, T, core::hash::BuildHasherDefault<DefaultHasher>>;

impl<T> Default for KwArgs<T> {
    fn default() -> Self {
        Self(KwArgsMap::default(), core::marker::PhantomData)
    }
}

impl<T, Name: ArgName> Deref for KwArgs<T, Name> {
    type Target = KwArgsMap<T>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T, Name: ArgName> DerefMut for KwArgs<T, Name> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

unsafe impl<T, Name: ArgName> Traverse for KwArgs<T, Name>
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
        Self(map, core::marker::PhantomData)
    }
}

impl<T, Name: ArgName> KwArgs<T, Name> {
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

    #[must_use]
    pub fn into_default(self) -> KwArgs<T> {
        KwArgs(self.0, core::marker::PhantomData)
    }
}

// Accept any key that converts into `Wtf8Buf` (notably `String`), so existing
// call sites that build kwargs from string literals keep compiling unchanged.
impl<K: Into<Wtf8Buf>, T> FromIterator<(K, T)> for KwArgs<T> {
    fn from_iter<I: IntoIterator<Item = (K, T)>>(iter: I) -> Self {
        Self(
            iter.into_iter().map(|(k, v)| (k.into(), v)).collect(),
            core::marker::PhantomData,
        )
    }
}

impl<'a, T, Name: ArgName> IntoIterator for &'a KwArgs<T, Name> {
    type Item = (&'a Wtf8Buf, &'a T);
    type IntoIter = indexmap::map::Iter<'a, Wtf8Buf, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl<T, Name: ArgName> IntoIterator for KwArgs<T, Name> {
    type Item = (Wtf8Buf, T);
    type IntoIter = indexmap::map::IntoIter<Wtf8Buf, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<T, Name: ArgName> FromArgs for KwArgs<T, Name>
where
    T: TryFromObject,
{
    const PARAMS: Option<&'static [Param]> = Some(&[Param::var_keyword(Name::NAME)]);

    fn arity() -> RangeInclusive<usize> {
        0..=0
    }

    fn from_args(vm: &VirtualMachine, args: &mut FuncArgs) -> Result<Self, ArgumentError> {
        let mut kwargs = KwArgsMap::default();
        for (name, value) in args.remaining_keywords() {
            kwargs.insert(name, value.try_into_value(vm)?);
        }
        Ok(Self(kwargs, core::marker::PhantomData))
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
pub struct PosArgs<T = PyObjectRef, Name: ArgName = NameArgs>(
    Vec<T>,
    core::marker::PhantomData<Name>,
);

unsafe impl<T, Name: ArgName> Traverse for PosArgs<T, Name>
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
        Self(args, core::marker::PhantomData)
    }
}

impl<T, Name: ArgName> PosArgs<T, Name> {
    #[must_use]
    pub const fn named(args: Vec<T>) -> Self {
        Self(args, core::marker::PhantomData)
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
        Self(v, core::marker::PhantomData)
    }
}

impl From<()> for PosArgs<PyObjectRef> {
    fn from(_args: ()) -> Self {
        Self(Vec::new(), core::marker::PhantomData)
    }
}

impl<T, Name: ArgName> AsRef<[T]> for PosArgs<T, Name> {
    fn as_ref(&self) -> &[T] {
        &self.0
    }
}

impl<T: PyPayload, Name: ArgName> PosArgs<PyRef<T>, Name> {
    pub fn into_tuple(self, vm: &VirtualMachine) -> PyTupleRef {
        vm.ctx
            .new_tuple(self.0.into_iter().map(Into::into).collect())
    }
}

impl<T, Name: ArgName> FromArgs for PosArgs<T, Name>
where
    T: TryFromObject,
{
    const PARAMS: Option<&'static [Param]> = Some(&[Param::var_positional(Name::NAME)]);

    fn from_args(vm: &VirtualMachine, args: &mut FuncArgs) -> Result<Self, ArgumentError> {
        let mut varargs = Vec::new();
        while let Some(value) = args.take_positional() {
            varargs.push(value.try_into_value(vm)?);
        }
        Ok(Self(varargs, core::marker::PhantomData))
    }
}

impl<T, Name: ArgName> IntoIterator for PosArgs<T, Name> {
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
        T::try_from_argument(vm, value)
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

/// One positional-only iterable, defaulting to an empty tuple.
#[derive(FromArgs)]
pub struct PositionalIterable {
    #[pyarg(positional, default, py_default = "()")]
    pub iterable: OptionalArg<PyObjectRef>,
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
    const PARAMS: Option<&'static [Param]> = Some(&[Param {
        name: "",
        kind: super::signature::ParamKind::PositionalOnly,
        default: Some(super::signature::DefaultRepr::Unrepresentable),
    }]);

    fn arity() -> RangeInclusive<usize> {
        0..=1
    }

    fn from_args(vm: &VirtualMachine, args: &mut FuncArgs) -> Result<Self, ArgumentError> {
        let r = if let Some(value) = args.take_positional() {
            Self::Present(T::try_from_argument(vm, value)?)
        } else {
            Self::Missing
        };
        Ok(r)
    }
}

// For functions that accept no arguments. Implemented explicitly instead of via
// macro below to avoid unused warnings.
impl FromArgs for () {
    const PARAMS: Option<&'static [Param]> = Some(&[]);

    fn arity() -> RangeInclusive<usize> {
        0..=0
    }

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
//
// A pyfunction with one FromArgs struct is bound as `(T,)`. Use T's own
// checks so a `legacy` struct's no-op is not wrapped in the combined
// missing-argument walk. Longer tuples use the default checks.
macro_rules! tuple_check_before_convert {
    ($T:ident) => {
        fn check_before_convert(
            args: &FuncArgs,
            arity: &RangeInclusive<usize>,
            num_given: usize,
            instance: usize,
            callee: Callee,
            vm: &VirtualMachine,
        ) -> Option<PyBaseExceptionRef> {
            $T::check_before_convert(args, arity, num_given, instance, callee, vm)
        }
    };
    ($($T:ident),+) => {};
}

macro_rules! tuple_from_py_func_args {
    ($($T:ident),+) => {
        impl<$($T),+> FromArgs for ($($T,)+)
        where
            $($T: FromArgs),+
        {
            const PARAMS: Option<&'static [Param]> = {
                if true $(&& $T::PARAMS.is_some())* {
                    Some(&[$(Param::flatten($T::PARAMS)),*])
                } else {
                    None
                }
            };

            const BINDING_PARAMS: Option<&'static [Param]> = {
                if true $(&& $T::BINDING_PARAMS.is_some())* {
                    Some(&[$(Param::flatten($T::BINDING_PARAMS)),*])
                } else {
                    None
                }
            };

            const HAS_REQUIRED_KEYWORD_ONLY: bool = false $(|| $T::HAS_REQUIRED_KEYWORD_ONLY)*;

            fn arity() -> RangeInclusive<usize> {
                let mut min = 0;
                let mut max = 0;
                $(
                    let (start, end) = $T::arity().into_inner();
                    // Both custom required bounds and unbounded upper bounds
                    // can reach `usize::MAX`; neither composition may overflow.
                    min = usize::saturating_add(min, start);
                    max = usize::saturating_add(max, end);
                )+
                min..=max
            }

            fn missing_from(
                nargs: usize,
                kwargs: &KwArgs,
                pos: &mut usize,
                skip: &mut usize,
            ) -> Result<(), Option<ArgumentError>> {
                $($T::missing_from(nargs, kwargs, pos, skip)?;)+
                Ok(())
            }

            tuple_check_before_convert!($($T),+);

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

#[cfg(test)]
mod arity_composition_tests {
    use super::*;

    struct Bounds<const MIN: usize, const MAX: usize>;

    impl<const MIN: usize, const MAX: usize> FromArgs for Bounds<MIN, MAX> {
        // This fixture supplies bounds rather than a fixed parameter list.
        const BINDING_PARAMS: Option<&'static [Param]> = None;

        fn arity() -> RangeInclusive<usize> {
            MIN..=MAX
        }

        fn from_args(_vm: &VirtualMachine, _args: &mut FuncArgs) -> Result<Self, ArgumentError> {
            unreachable!("metadata-only fixture; no interpreter is constructed")
        }
    }

    #[allow(dead_code)]
    #[derive(FromArgs)]
    struct Mixed {
        #[pyarg(flatten)]
        prefix: Bounds<2, 3>,
        #[pyarg(positional)]
        required: usize,
        #[pyarg(positional, optional)]
        optional: OptionalArg<usize>,
        #[pyarg(positional, default = 0)]
        defaulted: usize,
    }

    #[allow(dead_code)]
    #[derive(FromArgs)]
    struct Nested(#[pyarg(flatten)] (Bounds<1, 2>, Bounds<0, 1>));

    #[allow(dead_code)]
    #[derive(FromArgs)]
    struct OptionalOnly {
        #[pyarg(flatten)]
        prefix: Bounds<0, 2>,
        #[pyarg(positional, optional)]
        optional: OptionalArg<usize>,
    }

    #[allow(dead_code)]
    #[derive(FromArgs)]
    struct Phantoms {
        _phantom: core::marker::PhantomData<()>,
        #[pyarg(flatten)]
        _phantom_flattened: core::marker::PhantomData<()>,
        #[pyarg(positional)]
        required: usize,
    }

    #[allow(dead_code)]
    #[derive(FromArgs)]
    struct Saturated {
        #[pyarg(flatten)]
        prefix: Bounds<{ usize::MAX }, { usize::MAX }>,
        #[pyarg(positional)]
        required: usize,
    }

    #[test]
    fn derived_bounds_include_required_flattened_arguments() {
        assert_eq!(Mixed::arity(), 3..=6);
        assert_eq!(Nested::arity(), 1..=3);
        assert_eq!(Phantoms::arity(), 1..=1);
    }

    #[test]
    fn optional_only_flattening_preserves_zero_minimum() {
        assert_eq!(OptionalOnly::arity(), 0..=3);
        assert_eq!(<(Bounds<0, 2>, Bounds<0, 3>)>::arity(), 0..=5);
        assert_eq!(
            <(Bounds<2, 3>, (Bounds<1, 2>, Bounds<0, 1>))>::arity(),
            3..=6
        );
    }

    #[test]
    fn both_tuple_and_derived_bounds_saturate() {
        assert_eq!(
            <(Bounds<1, 1>, Bounds<0, { usize::MAX }>)>::arity(),
            1..=usize::MAX
        );
        assert_eq!(Saturated::arity(), usize::MAX..=usize::MAX);
        assert_eq!(
            <(Bounds<{ usize::MAX }, { usize::MAX }>, Bounds<1, 1>)>::arity(),
            usize::MAX..=usize::MAX,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{ArgumentError, FromArgs, KwArgs};
    use super::{BindingShape, Callee, FuncArgs, PosArgs};
    use crate::{PyObjectRef, VirtualMachine};

    #[test]
    fn duplicate_argument_messages_distinguish_unnamed_callees() {
        assert_eq!(
            super::given_by_name_and_position_message(None, "dialect", 1),
            "argument for function given by name ('dialect') and position (1)",
        );
        assert_eq!(
            super::given_by_name_and_position_message(Some("example"), "dialect", 1),
            "argument for example() given by name ('dialect') and position (1)",
        );
    }

    #[allow(dead_code)]
    #[derive(FromArgs)]
    struct NamedRequired {
        #[pyarg(any)]
        first: i32,
        #[pyarg(named)]
        second: i32,
    }

    #[allow(dead_code)]
    #[derive(FromArgs)]
    struct ClinicNamed {
        #[pyarg(any)]
        first: i32,
        #[pyarg(any)]
        second: i32,
        #[pyarg(named, default)]
        optional: i32,
    }

    #[allow(dead_code)]
    #[derive(FromArgs)]
    struct FlattenedClinic {
        #[pyarg(flatten)]
        args: (PyObjectRef, ClinicNamed),
    }

    #[allow(dead_code)]
    #[derive(FromArgs)]
    struct VariadicNamed {
        #[pyarg(flatten)]
        args: PosArgs,
        #[pyarg(named)]
        required: i32,
    }

    #[allow(dead_code)]
    #[derive(FromArgs)]
    struct ExtraKeywords {
        #[pyarg(any)]
        first: i32,
        #[pyarg(flatten)]
        rest: KwArgs,
    }

    #[allow(dead_code)]
    #[derive(FromArgs)]
    #[from_args(legacy, fname = "example")]
    struct LegacyNamed {
        #[pyarg(any)]
        first: i32,
        #[pyarg(any)]
        second: i32,
    }

    #[allow(dead_code)]
    #[derive(FromArgs)]
    struct FlattenedLegacy {
        #[pyarg(flatten)]
        args: (PyObjectRef, LegacyNamed),
    }

    struct DisplayOnly(FuncArgs);

    impl FromArgs for DisplayOnly {
        // An explicit custom parser, unlike the default manual implementation.
        const BINDING_PARAMS: Option<&'static [super::Param]> = None;

        const PARAMS: Option<&'static [super::Param]> =
            Some(&[super::Param::positional_or_keyword("display")]);

        fn from_args(vm: &VirtualMachine, args: &mut FuncArgs) -> Result<Self, ArgumentError> {
            FuncArgs::from_args(vm, args).map(Self)
        }
    }

    fn binding_error<T: FromArgs>(
        nargs: usize,
        names: &[&str],
        receiver: bool,
        vm: &VirtualMachine,
    ) -> String {
        let args = FuncArgs::new(
            vec![vm.ctx.none(); nargs],
            names
                .iter()
                .map(|&name| (name, vm.ctx.none()))
                .collect::<KwArgs>(),
        );
        let callee = Callee::named("example").with_instance_arg(receiver);
        match args.bind_for::<T>(vm, callee) {
            Err(error) => error.args().as_slice()[0].try_to_value(vm).unwrap(),
            Ok(_) => panic!("binding unexpectedly succeeded"),
        }
    }

    #[test]
    fn clinic_checks_shape_before_converting_and_missing_before_duplicates() {
        crate::Interpreter::without_stdlib(Default::default()).enter(|vm| {
            assert_eq!(
                binding_error::<ClinicNamed>(1, &["first"], false, vm),
                "example() missing required argument 'second' (pos 2)",
            );
            assert_eq!(
                binding_error::<ClinicNamed>(2, &["unknown"], false, vm),
                "example() got an unexpected keyword argument 'unknown'",
            );
            assert_eq!(
                binding_error::<ClinicNamed>(3, &[], false, vm),
                "example() takes exactly 2 positional arguments (3 given)",
            );
            assert_eq!(
                binding_error::<ClinicNamed>(2, &["optional", "unknown"], false, vm),
                "example() takes at most 3 arguments (4 given)",
            );
            assert_eq!(
                binding_error::<ClinicNamed>(
                    0,
                    &["first", "second", "optional", "unknown"],
                    false,
                    vm
                ),
                "example() takes at most 3 keyword arguments (4 given)",
            );
        });
    }

    #[test]
    fn clinic_tuple_and_flatten_validation_skip_the_receiver() {
        const { assert!(<(PyObjectRef, ClinicNamed)>::PARAMS.is_none()) };
        crate::Interpreter::without_stdlib(Default::default()).enter(|vm| {
            assert_eq!(
                binding_error::<(PyObjectRef, ClinicNamed)>(2, &["first"], true, vm),
                "example() missing required argument 'second' (pos 2)",
            );
            assert_eq!(
                binding_error::<FlattenedClinic>(3, &["first"], true, vm),
                "argument for example() given by name ('first') and position (1)",
            );
        });
    }

    #[test]
    fn clinic_variadic_arguments_keep_named_validation() {
        crate::Interpreter::without_stdlib(Default::default()).enter(|vm| {
            assert_eq!(
                binding_error::<VariadicNamed>(5, &[], false, vm),
                "example() missing required argument 'required' (pos 1)",
            );
            assert_eq!(
                binding_error::<ExtraKeywords>(1, &["first", "extra"], false, vm),
                "argument for example() given by name ('first') and position (1)",
            );
        });
    }

    #[test]
    fn clinic_validation_preserves_successful_keyword_bindings() {
        crate::Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let args = FuncArgs::new(
                vec![vm.ctx.none()],
                [("first", 1), ("second", 2), ("optional", 3)]
                    .into_iter()
                    .map(|(name, value)| (name, vm.ctx.new_int(value).into()))
                    .collect::<KwArgs>(),
            );
            let bound = args
                .bind_for::<FlattenedClinic>(vm, Callee::named("example").with_instance_arg(true))
                .unwrap_or_else(|_| panic!("known keywords should bind"));
            assert_eq!(
                (
                    bound.args.1.first,
                    bound.args.1.second,
                    bound.args.1.optional
                ),
                (1, 2, 3),
            );

            let args = FuncArgs::new(
                vec![vm.ctx.new_int(42).into()],
                core::iter::once(("extra", vm.ctx.none())).collect::<KwArgs>(),
            );
            let bound = args
                .bind_for::<ExtraKeywords>(vm, "example")
                .unwrap_or_else(|_| panic!("extra keywords should be retained"));
            assert_eq!(bound.first, 42);
            assert!(bound.rest.contains_key("extra"));
        });
    }

    #[test]
    fn legacy_binding_remains_opaque_and_converts_first() {
        const { assert!(LegacyNamed::BINDING_PARAMS.is_none()) };
        const { assert!(<(PyObjectRef, LegacyNamed)>::BINDING_PARAMS.is_none()) };
        assert!(BindingShape::from_params(Some(&[super::Param::flatten(None)]), 0).is_none());
        assert!(BindingShape::from_params(<(PosArgs, i32)>::BINDING_PARAMS, 0).is_none());
        assert!(BindingShape::from_params(<(KwArgs, ClinicNamed)>::BINDING_PARAMS, 0).is_none());
        assert!(BindingShape::from_params(<(ClinicNamed, i32)>::BINDING_PARAMS, 0).is_none());
        assert!(BindingShape::from_params(<(ClinicNamed, PosArgs)>::BINDING_PARAMS, 0).is_none());
        crate::Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let message = binding_error::<(LegacyNamed,)>(1, &[], false, vm);
            assert!(message.contains("NoneType"), "{message}");
            assert!(!message.contains("missing required"), "{message}");
        });
    }

    #[test]
    fn legacy_conversion_order_survives_receivers_and_flattened_composition() {
        crate::Interpreter::without_stdlib(Default::default()).enter(|vm| {
            for message in [
                binding_error::<(PyObjectRef, LegacyNamed)>(2, &[], true, vm),
                binding_error::<FlattenedLegacy>(2, &[], true, vm),
                binding_error::<(LegacyNamed, ())>(1, &["first"], false, vm),
            ] {
                assert!(message.contains("NoneType"), "{message}");
                assert!(!message.contains("missing required"), "{message}");
                assert!(!message.contains("given by name"), "{message}");
            }
        });
    }

    #[test]
    fn explicitly_custom_signatures_do_not_opt_in_to_clinic_validation() {
        const { assert!(DisplayOnly::BINDING_PARAMS.is_none()) };
        crate::Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let bound = FuncArgs::default()
                .bind_for::<DisplayOnly>(vm, "example")
                .unwrap_or_else(|_| panic!("a display signature must not require arguments"));
            assert!(bound.0.args.is_empty());
        });
    }

    struct ManualNamed;

    impl FromArgs for ManualNamed {
        const PARAMS: Option<&'static [super::Param]> = ClinicNamed::PARAMS;

        fn from_args(_: &VirtualMachine, _: &mut FuncArgs) -> Result<Self, ArgumentError> {
            panic!("invalid calls must be rejected before the manual parser runs")
        }
    }

    struct ManualPositional;

    impl FromArgs for ManualPositional {
        const PARAMS: Option<&'static [super::Param]> =
            Some(&[super::Param::positional_only("value")]);

        fn from_args(_: &VirtualMachine, _: &mut FuncArgs) -> Result<Self, ArgumentError> {
            panic!("invalid calls must be rejected before the manual parser runs")
        }
    }

    #[test]
    fn manual_parsers_use_clinic_validation_and_shape_bounds_by_default() {
        assert_eq!(ManualNamed::arity(), 2..=2);
        assert_eq!(ManualPositional::arity(), 1..=1);
        crate::Interpreter::without_stdlib(Default::default()).enter(|vm| {
            assert_eq!(
                binding_error::<ManualNamed>(1, &["first"], false, vm),
                "example() missing required argument 'second' (pos 2)",
            );
            assert_eq!(
                binding_error::<ManualNamed>(2, &["unknown"], false, vm),
                "example() got an unexpected keyword argument 'unknown'",
            );
            assert_eq!(
                binding_error::<ManualPositional>(0, &[], false, vm),
                "example expected 1 argument, got 0",
            );
            assert_eq!(
                binding_error::<(PyObjectRef, ManualPositional)>(3, &[], true, vm),
                "example expected 1 argument, got 2",
            );
        });
    }

    #[test]
    fn a_receiver_does_not_restore_clinic_validation_for_an_explicit_custom_parser() {
        const { assert!(<(PyObjectRef, DisplayOnly)>::BINDING_PARAMS.is_none()) };
        crate::Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let (_, bound) = FuncArgs::from(vec![vm.ctx.none()])
                .bind_for::<(PyObjectRef, DisplayOnly)>(
                    vm,
                    Callee::named("example").with_instance_arg(true),
                )
                .unwrap_or_else(|_| {
                    panic!("the custom parser does not require display parameters")
                });
            assert!(bound.0.args.is_empty());
        });
    }

    #[test]
    #[should_panic(expected = "must describe one Clinic signature")]
    fn malformed_known_compositions_cannot_silently_disable_clinic_validation() {
        BindingShape::validated(<(PosArgs, i32)>::BINDING_PARAMS, 0);
    }

    fn missing<T: FromArgs>(nargs: usize, skip: usize) -> Result<(), Option<ArgumentError>> {
        let kwargs = KwArgs::default();
        let mut pos = 0;
        let mut skip = skip;
        T::missing_from(nargs, &kwargs, &mut pos, &mut skip)
    }

    #[test]
    fn tuple_of_required_positionals_reports_too_few_args() {
        assert!(matches!(
            missing::<(i32, i32)>(1, 0),
            Err(Some(ArgumentError::TooFewArgs))
        ));
        assert!(matches!(missing::<(i32, i32)>(2, 0), Ok(())));
    }

    #[test]
    fn method_instance_is_skipped_without_advancing_pos() {
        assert!(matches!(
            missing::<(i32, i32, i32)>(1, 1),
            Err(Some(ArgumentError::TooFewArgs))
        ));
        assert!(matches!(missing::<(i32, i32, i32)>(2, 1), Ok(())));
    }

    #[test]
    fn required_keyword_only_is_visible_at_compile_time() {
        const { assert!(NamedRequired::HAS_REQUIRED_KEYWORD_ONLY) };
        const { assert!(<(i32, NamedRequired)>::HAS_REQUIRED_KEYWORD_ONLY) };
        const { assert!(!<(i32, i32)>::HAS_REQUIRED_KEYWORD_ONLY) };
        match missing::<NamedRequired>(1, 0) {
            Err(Some(ArgumentError::MissingRequiredArgument { name, pos })) => {
                assert_eq!(name, "second");
                assert_eq!(pos, 2);
            }
            _ => panic!("expected a missing required keyword-only argument"),
        }
    }
}

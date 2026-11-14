use super::{BorrowedArgs, Callee, FromArgs, FuncArgs, SigArg};
use crate::{
    Py, PyObject, PyPayload, PyRef, PyResult, VirtualMachine,
    convert::{ToPyResult, TryFromObject},
    object::PyThreadingConstraint,
};
use core::marker::PhantomData;

/// A built-in Python function.
// PyCFunction in CPython
pub trait PyNativeFn:
    Fn(&VirtualMachine, Option<&PyObject>, FuncArgs, Callee) -> PyResult
    + PyThreadingConstraint
    + 'static
{
}

impl<F> PyNativeFn for F where
    F: Fn(&VirtualMachine, Option<&PyObject>, FuncArgs, Callee) -> PyResult
        + PyThreadingConstraint
        + 'static
{
}

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
/// is `IntoPyNativeFn`. If you do want a really general function signature, e.g.
/// to forward the args to another function, you can define a function like
/// `Fn(FuncArgs [, &VirtualMachine]) -> ...`
///
/// Note that the `Kind` type parameter is meaningless and should be considered
/// an implementation detail; if you need to use `IntoPyNativeFn` as a trait bound
/// just pass an unconstrained generic type, e.g.
/// `fn foo<F, FKind>(f: F) where F: IntoPyNativeFn<FKind>`
pub trait IntoPyNativeFn<Kind>: Sized + PyThreadingConstraint + 'static {
    /// Binding metadata of each argument, from which the calling convention
    /// is inferred. A `&self` receiver is the `$self` marker.
    const ARGS: &'static [SigArg] = &[SigArg::from_arg::<FuncArgs>("args")];

    fn call(
        &self,
        vm: &VirtualMachine,
        receiver: Option<&PyObject>,
        args: FuncArgs,
        callee: Callee,
    ) -> PyResult;

    /// `IntoPyNativeFn::into_func()` generates a PyNativeFn that performs the
    /// appropriate type and arity checking, any requested conversions, and then if
    /// successful calls the function with the extracted parameters.
    fn into_func(self) -> impl PyNativeFn {
        into_func(self)
    }
}

const fn into_func<F: IntoPyNativeFn<Kind>, Kind>(f: F) -> impl PyNativeFn {
    move |vm: &VirtualMachine, receiver, args, callee| f.call(vm, receiver, args, callee)
}

const fn zst_ref_out_of_thin_air<T: 'static>(x: T) -> &'static T {
    // if T is zero-sized, there's no issue forgetting it - even if it does have a Drop impl, it
    // would never get called anyway if we consider this semantically a Box::leak(Box::new(x))-type
    // operation. if T isn't zero-sized, we don't have to worry about it because we'll fail to compile.
    core::mem::forget(x);
    const {
        assert!(
            core::mem::size_of::<T>() == 0,
            "can't use a non-zero-sized type here"
        );
        // SAFETY: we just confirmed that T is zero-sized, so we can
        //         pull a value of it out of thin air.
        unsafe { core::ptr::NonNull::<T>::dangling().as_ref() }
    }
}

/// Get the STATIC_FUNC of the passed function. The same
/// requirements of zero-sized-ness apply, see that documentation for details.
///
/// Equivalent to [`IntoPyNativeFn::into_func()`], but usable in a const context. This is only
/// valid if the function is zero-sized, i.e. that `std::mem::size_of::<F>() == 0`. If you call
/// this function with a non-zero-sized function, it will raise a compile error.
#[inline(always)]
pub const fn static_func<Kind, F: IntoPyNativeFn<Kind>>(f: F) -> &'static dyn PyNativeFn {
    zst_ref_out_of_thin_air(into_func(f))
}

#[inline(always)]
pub const fn static_raw_func<F: PyNativeFn>(f: F) -> &'static dyn PyNativeFn {
    zst_ref_out_of_thin_air(f)
}

// TODO: once higher-rank trait bounds are stabilized, remove the `Kind` type
// parameter and impl for F where F: for<T, R, VM> PyNativeFnInternal<T, R, VM>
impl<F, T, R, VM> IntoPyNativeFn<(T, R, VM)> for F
where
    F: PyNativeFnInternal<T, R, VM>,
{
    const ARGS: &'static [SigArg] = F::ARGS;

    #[inline(always)]
    fn call(
        &self,
        vm: &VirtualMachine,
        receiver: Option<&PyObject>,
        args: FuncArgs,
        callee: Callee,
    ) -> PyResult {
        self.call_(vm, receiver, args, callee)
    }
}

mod sealed {
    use super::*;
    pub trait PyNativeFnInternal<T, R, VM>: Sized + PyThreadingConstraint + 'static {
        const ARGS: &'static [SigArg];
        fn call_(
            &self,
            vm: &VirtualMachine,
            receiver: Option<&PyObject>,
            args: FuncArgs,
            callee: Callee,
        ) -> PyResult;
    }
}
use sealed::PyNativeFnInternal;

#[doc(hidden)]
pub struct OwnedParam<T>(PhantomData<T>);

#[doc(hidden)]
pub struct BorrowedParam<T>(PhantomData<T>);

#[doc(hidden)]
pub struct RefParam<T>(PhantomData<T>);

#[doc(hidden)]
pub struct BorrowedArgsParam<T>(PhantomData<T>);

impl<F, T, R> PyNativeFnInternal<BorrowedArgsParam<T>, R, VirtualMachine> for F
where
    T: BorrowedArgs,
    F: Fn(T, &VirtualMachine) -> R
        + for<'a> Fn(T::Bound<'a>, &VirtualMachine) -> R
        + PyThreadingConstraint
        + 'static,
    R: ToPyResult,
{
    const ARGS: &'static [SigArg] = &[SigArg::from_arg::<T>("")];

    fn call_(
        &self,
        vm: &VirtualMachine,
        _receiver: Option<&PyObject>,
        args: FuncArgs,
        callee: Callee,
    ) -> PyResult {
        // The original owners outlive conversion, the body, and return-value
        // conversion, including every error path. No keepalive copy is needed.
        let result = T::bind(&args, vm, callee).and_then(|bound| (self)(bound, vm).to_pyresult(vm));
        args.drop_reverse();
        result
    }
}

/// Common typed adapter for an out-of-band native receiver.
/// `METHOD` selects argument metadata and legacy validation at compile time;
/// it does not add a receiver-kind tag or branch to native call dispatch.
#[doc(hidden)]
pub struct ReceiverFunction<F, const METHOD: bool>(pub F);

#[doc(hidden)]
pub struct ReceiverFunctionKind<T, R, VM>(PhantomData<(T, R, VM)>);

// A method validates its complete argument list before handing it to a
// custom parser. Keep that outer check even with one remaining Rust argument:
// `(T,)` alone delegates its check to T, which legacy parsers can override.
struct MethodArgs<T>(T);

impl<T: FromArgs> FromArgs for MethodArgs<T> {
    const PARAMS: Option<&'static [super::Param]> = T::PARAMS;
    const BINDING_PARAMS: Option<&'static [super::Param]> = T::BINDING_PARAMS;
    const FORCE_UNPACK_KEYWORDS: bool = T::FORCE_UNPACK_KEYWORDS;
    const TAKES_NO_KEYWORDS: bool = T::TAKES_NO_KEYWORDS;

    fn arity() -> core::ops::RangeInclusive<usize> {
        T::arity()
    }

    fn from_args(vm: &VirtualMachine, args: &mut FuncArgs) -> Result<Self, super::ArgumentError> {
        T::from_args(vm, args).map(Self)
    }
}

impl<F, const METHOD: bool> ReceiverFunction<F, METHOD> {
    // Method convention inference excludes the receiver marker; function
    // convention inference receives only the Python arguments.
    const fn args(args: &'static [SigArg]) -> &'static [SigArg] {
        if METHOD { args } else { args.split_at(1).1 }
    }

    // Preserve the method's outer validation without imposing it on a
    // function's single legacy/custom parser. The policy is not stored.
    #[inline(always)]
    fn bind_args<T: FromArgs>(args: FuncArgs, vm: &VirtualMachine, callee: Callee) -> PyResult<T> {
        if METHOD {
            args.bind_for::<MethodArgs<T>>(vm, callee)
                .map(|MethodArgs(args)| args)
        } else {
            args.bind_for::<T>(vm, callee)
        }
    }
}

#[inline]
fn native_receiver<'a>(
    receiver: Option<&'a PyObject>,
    callee: Callee,
    vm: &VirtualMachine,
) -> PyResult<&'a PyObject> {
    receiver.ok_or_else(|| callee.missing_receiver(vm))
}

// This is the "magic" that allows rust functions of varying signatures to
// generate native python functions.
//
// Note that this could be done without a macro - it is simply to avoid repetition.
macro_rules! into_py_native_fn_tuple {
    ($(($n:tt, $T:ident)),*) => {
        impl<F, $($T,)* R> PyNativeFnInternal<($(OwnedParam<$T>,)*), R, VirtualMachine> for F
        where
            F: Fn($($T,)* &VirtualMachine) -> R + PyThreadingConstraint + 'static,
            $($T: FromArgs,)*
            R: ToPyResult,
        {
            const ARGS: &'static [SigArg] = &[$(SigArg::from_arg::<$T>("")),*];

            fn call_(
                &self,
                vm: &VirtualMachine,
                _receiver: Option<&PyObject>,
                args: FuncArgs,
                callee: Callee,
            ) -> PyResult {
                let ($($n,)*) = args.bind_for::<($($T,)*)>(vm, callee)?;

                (self)($($n,)* vm).to_pyresult(vm)
            }
        }

        impl<F, S, $($T,)* R> PyNativeFnInternal<(BorrowedParam<S>, $(OwnedParam<$T>,)*), R, VirtualMachine> for F
        where
            F: Fn(&Py<S>, $($T,)* &VirtualMachine) -> R + PyThreadingConstraint + 'static,
            S: PyPayload,
            $($T: FromArgs,)*
            R: ToPyResult,
        {
            const ARGS: &'static [SigArg] =
                &[SigArg::marker("$self") $(, SigArg::from_arg::<$T>(""))*];

            fn call_(
                &self,
                vm: &VirtualMachine,
                _receiver: Option<&PyObject>,
                args: FuncArgs,
                callee: Callee,
            ) -> PyResult {
                let (zelf, $($n,)*) = args.bind_for::<(PyRef<S>, $($T,)*)>(vm, callee)?;

                (self)(&zelf, $($n,)* vm).to_pyresult(vm)
            }
        }

        impl<F, S, $($T,)* R> PyNativeFnInternal<(RefParam<S>, $(OwnedParam<$T>,)*), R, VirtualMachine> for F
        where
            F: Fn(&S, $($T,)* &VirtualMachine) -> R + PyThreadingConstraint + 'static,
            S: PyPayload,
            $($T: FromArgs,)*
            R: ToPyResult,
        {
            const ARGS: &'static [SigArg] =
                &[SigArg::marker("$self") $(, SigArg::from_arg::<$T>(""))*];

            fn call_(
                &self,
                vm: &VirtualMachine,
                _receiver: Option<&PyObject>,
                args: FuncArgs,
                callee: Callee,
            ) -> PyResult {
                let (zelf, $($n,)*) = args.bind_for::<(PyRef<S>, $($T,)*)>(vm, callee)?;

                (self)(&zelf, $($n,)* vm).to_pyresult(vm)
            }
        }

        impl<F, $($T,)* R> PyNativeFnInternal<($(OwnedParam<$T>,)*), R, ()> for F
        where
            F: Fn($($T,)*) -> R + PyThreadingConstraint + 'static,
            $($T: FromArgs,)*
            R: ToPyResult,
        {
            const ARGS: &'static [SigArg] = &[$(SigArg::from_arg::<$T>("")),*];

            fn call_(
                &self,
                vm: &VirtualMachine,
                _receiver: Option<&PyObject>,
                args: FuncArgs,
                callee: Callee,
            ) -> PyResult {
                let ($($n,)*) = args.bind_for::<($($T,)*)>(vm, callee)?;

                (self)($($n,)*).to_pyresult(vm)
            }
        }

        impl<F, S, $($T,)* R> PyNativeFnInternal<(BorrowedParam<S>, $(OwnedParam<$T>,)*), R, ()> for F
        where
            F: Fn(&Py<S>, $($T,)*) -> R + PyThreadingConstraint + 'static,
            S: PyPayload,
            $($T: FromArgs,)*
            R: ToPyResult,
        {
            const ARGS: &'static [SigArg] =
                &[SigArg::marker("$self") $(, SigArg::from_arg::<$T>(""))*];

            fn call_(
                &self,
                vm: &VirtualMachine,
                _receiver: Option<&PyObject>,
                args: FuncArgs,
                callee: Callee,
            ) -> PyResult {
                let (zelf, $($n,)*) = args.bind_for::<(PyRef<S>, $($T,)*)>(vm, callee)?;

                (self)(&zelf, $($n,)*).to_pyresult(vm)
            }
        }

        impl<F, S, $($T,)* R> PyNativeFnInternal<(RefParam<S>, $(OwnedParam<$T>,)*), R, ()> for F
        where
            F: Fn(&S, $($T,)*) -> R + PyThreadingConstraint + 'static,
            S: PyPayload,
            $($T: FromArgs,)*
            R: ToPyResult,
        {
            const ARGS: &'static [SigArg] =
                &[SigArg::marker("$self") $(, SigArg::from_arg::<$T>(""))*];

            fn call_(
                &self,
                vm: &VirtualMachine,
                _receiver: Option<&PyObject>,
                args: FuncArgs,
                callee: Callee,
            ) -> PyResult {
                let (zelf, $($n,)*) = args.bind_for::<(PyRef<S>, $($T,)*)>(vm, callee)?;

                (self)(&zelf, $($n,)*).to_pyresult(vm)
            }
        }
        impl<F, S, $($T,)* R, const METHOD: bool> IntoPyNativeFn<ReceiverFunctionKind<(OwnedParam<S>, $(OwnedParam<$T>,)*), R, VirtualMachine>> for ReceiverFunction<F, METHOD>
        where
            F: Fn(S, $($T,)* &VirtualMachine) -> R + PyThreadingConstraint + 'static,
            S: TryFromObject,
            $($T: FromArgs,)*
            R: ToPyResult,
        {
            const ARGS: &'static [SigArg] =
                Self::args(&[SigArg::from_arg::<S>("") $(, SigArg::from_arg::<$T>(""))*]);

            fn call(
                &self,
                vm: &VirtualMachine,
                receiver: Option<&PyObject>,
                args: FuncArgs,
                callee: Callee,
            ) -> PyResult {
                let receiver = native_receiver(receiver, callee, vm)?;
                // Even when binding checked the Python class, the Rust payload
                // still needs validation for safe manual registrations.
                let zelf = S::try_from_method_receiver(vm, receiver)?;
                let ($($n,)*) = Self::bind_args::<($($T,)*)>(args, vm, callee)?;
                (self.0)(zelf, $($n,)* vm).to_pyresult(vm)
            }
        }

        impl<F, S, $($T,)* R, const METHOD: bool> IntoPyNativeFn<ReceiverFunctionKind<(BorrowedParam<S>, $(OwnedParam<$T>,)*), R, VirtualMachine>> for ReceiverFunction<F, METHOD>
        where
            F: Fn(&Py<S>, $($T,)* &VirtualMachine) -> R + PyThreadingConstraint + 'static,
            S: PyPayload,
            $($T: FromArgs,)*
            R: ToPyResult,
        {
            const ARGS: &'static [SigArg] =
                Self::args(&[SigArg::marker("$self") $(, SigArg::from_arg::<$T>(""))*]);

            fn call(
                &self,
                vm: &VirtualMachine,
                receiver: Option<&PyObject>,
                args: FuncArgs,
                callee: Callee,
            ) -> PyResult {
                let receiver = native_receiver(receiver, callee, vm)?;
                // Even when binding checked the Python class, the Rust payload
                // still needs validation for safe manual registrations.
                let zelf = receiver.try_downcast_ref::<S>(vm)?;
                let ($($n,)*) = Self::bind_args::<($($T,)*)>(args, vm, callee)?;
                (self.0)(zelf, $($n,)* vm).to_pyresult(vm)
            }
        }

        impl<F, S, $($T,)* R, const METHOD: bool> IntoPyNativeFn<ReceiverFunctionKind<(RefParam<S>, $(OwnedParam<$T>,)*), R, VirtualMachine>> for ReceiverFunction<F, METHOD>
        where
            F: Fn(&S, $($T,)* &VirtualMachine) -> R + PyThreadingConstraint + 'static,
            S: PyPayload,
            $($T: FromArgs,)*
            R: ToPyResult,
        {
            const ARGS: &'static [SigArg] =
                Self::args(&[SigArg::marker("$self") $(, SigArg::from_arg::<$T>(""))*]);

            fn call(
                &self,
                vm: &VirtualMachine,
                receiver: Option<&PyObject>,
                args: FuncArgs,
                callee: Callee,
            ) -> PyResult {
                let receiver = native_receiver(receiver, callee, vm)?;
                // Even when binding checked the Python class, the Rust payload
                // still needs validation for safe manual registrations.
                let zelf = receiver.try_downcast_ref::<S>(vm)?;
                let ($($n,)*) = Self::bind_args::<($($T,)*)>(args, vm, callee)?;
                (self.0)(zelf, $($n,)* vm).to_pyresult(vm)
            }
        }

        impl<F, S, $($T,)* R, const METHOD: bool> IntoPyNativeFn<ReceiverFunctionKind<(OwnedParam<S>, $(OwnedParam<$T>,)*), R, ()>> for ReceiverFunction<F, METHOD>
        where
            F: Fn(S, $($T,)* ) -> R + PyThreadingConstraint + 'static,
            S: TryFromObject,
            $($T: FromArgs,)*
            R: ToPyResult,
        {
            const ARGS: &'static [SigArg] =
                Self::args(&[SigArg::from_arg::<S>("") $(, SigArg::from_arg::<$T>(""))*]);

            fn call(
                &self,
                vm: &VirtualMachine,
                receiver: Option<&PyObject>,
                args: FuncArgs,
                callee: Callee,
            ) -> PyResult {
                let receiver = native_receiver(receiver, callee, vm)?;
                // Even when binding checked the Python class, the Rust payload
                // still needs validation for safe manual registrations.
                let zelf = S::try_from_method_receiver(vm, receiver)?;
                let ($($n,)*) = Self::bind_args::<($($T,)*)>(args, vm, callee)?;
                (self.0)(zelf, $($n,)* ).to_pyresult(vm)
            }
        }

        impl<F, S, $($T,)* R, const METHOD: bool> IntoPyNativeFn<ReceiverFunctionKind<(BorrowedParam<S>, $(OwnedParam<$T>,)*), R, ()>> for ReceiverFunction<F, METHOD>
        where
            F: Fn(&Py<S>, $($T,)* ) -> R + PyThreadingConstraint + 'static,
            S: PyPayload,
            $($T: FromArgs,)*
            R: ToPyResult,
        {
            const ARGS: &'static [SigArg] =
                Self::args(&[SigArg::marker("$self") $(, SigArg::from_arg::<$T>(""))*]);

            fn call(
                &self,
                vm: &VirtualMachine,
                receiver: Option<&PyObject>,
                args: FuncArgs,
                callee: Callee,
            ) -> PyResult {
                let receiver = native_receiver(receiver, callee, vm)?;
                // Even when binding checked the Python class, the Rust payload
                // still needs validation for safe manual registrations.
                let zelf = receiver.try_downcast_ref::<S>(vm)?;
                let ($($n,)*) = Self::bind_args::<($($T,)*)>(args, vm, callee)?;
                (self.0)(zelf, $($n,)* ).to_pyresult(vm)
            }
        }

        impl<F, S, $($T,)* R, const METHOD: bool> IntoPyNativeFn<ReceiverFunctionKind<(RefParam<S>, $(OwnedParam<$T>,)*), R, ()>> for ReceiverFunction<F, METHOD>
        where
            F: Fn(&S, $($T,)* ) -> R + PyThreadingConstraint + 'static,
            S: PyPayload,
            $($T: FromArgs,)*
            R: ToPyResult,
        {
            const ARGS: &'static [SigArg] =
                Self::args(&[SigArg::marker("$self") $(, SigArg::from_arg::<$T>(""))*]);

            fn call(
                &self,
                vm: &VirtualMachine,
                receiver: Option<&PyObject>,
                args: FuncArgs,
                callee: Callee,
            ) -> PyResult {
                let receiver = native_receiver(receiver, callee, vm)?;
                // Even when binding checked the Python class, the Rust payload
                // still needs validation for safe manual registrations.
                let zelf = receiver.try_downcast_ref::<S>(vm)?;
                let ($($n,)*) = Self::bind_args::<($($T,)*)>(args, vm, callee)?;
                (self.0)(zelf, $($n,)* ).to_pyresult(vm)
            }
        }

    };
}

into_py_native_fn_tuple!();
into_py_native_fn_tuple!((v1, T1));
into_py_native_fn_tuple!((v1, T1), (v2, T2));
into_py_native_fn_tuple!((v1, T1), (v2, T2), (v3, T3));
into_py_native_fn_tuple!((v1, T1), (v2, T2), (v3, T3), (v4, T4));
into_py_native_fn_tuple!((v1, T1), (v2, T2), (v3, T3), (v4, T4), (v5, T5));
into_py_native_fn_tuple!((v1, T1), (v2, T2), (v3, T3), (v4, T4), (v5, T5), (v6, T6));
into_py_native_fn_tuple!(
    (v1, T1),
    (v2, T2),
    (v3, T3),
    (v4, T4),
    (v5, T5),
    (v6, T6),
    (v7, T7)
);

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::size_of_val;

    #[test]
    fn into_native_fn_noalloc() {
        fn py_func(_b: bool, _vm: &crate::VirtualMachine) -> i32 {
            1
        }
        assert_eq!(size_of_val(&py_func.into_func()), 0);
        let empty_closure = || "foo".to_owned();
        assert_eq!(size_of_val(&empty_closure.into_func()), 0);
        assert_eq!(size_of_val(static_func(empty_closure)), 0);
    }
    #[test]
    fn common_receiver_adapter_preserves_call_conventions() {
        use crate::{
            builtins::PyModule,
            function::{ItemDoc, PyMethodDef, PyMethodFlags},
        };

        fn no_args(_: &Py<PyModule>) {}
        fn one_arg(_: &Py<PyModule>, _: u32) {}
        fn two_args(_: &Py<PyModule>, _: u32, _: u32) {}
        fn arbitrary_args(_: &Py<PyModule>, _: FuncArgs) {}

        macro_rules! check {
            ($function:ident, $flags:expr) => {{
                const FUNCTION: PyMethodDef = PyMethodDef::new_const(
                    "example",
                    ReceiverFunction::<_, false>($function),
                    PyMethodFlags::EMPTY,
                    ItemDoc::NONE,
                );
                const METHOD: PyMethodDef = PyMethodDef::new_method_const(
                    "example",
                    $function,
                    PyMethodFlags::METHOD,
                    ItemDoc::NONE,
                );
                assert_eq!(FUNCTION.flags, $flags);
                assert_eq!(METHOD.flags, $flags.union(PyMethodFlags::METHOD));
                assert_eq!(size_of_val(&ReceiverFunction::<_, false>($function)), 0);
                assert_eq!(size_of_val(&ReceiverFunction::<_, true>($function)), 0);
            }};
        }
        check!(no_args, PyMethodFlags::NOARGS);
        check!(one_arg, PyMethodFlags::O);
        check!(two_args, PyMethodFlags::FASTCALL);
        check!(
            arbitrary_args,
            PyMethodFlags::FASTCALL.union(PyMethodFlags::KEYWORDS)
        );
    }

    #[test]
    fn method_receivers_are_borrowed_outside_python_arguments() {
        use crate::{AsObject, PyObjectRef, builtins::PyList};
        crate::Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let receiver = vm.ctx.new_list(vec![]);
            let count = receiver.as_object().strong_count();
            let value = vm.ctx.new_int(42).into();
            ReceiverFunction::<_, true>(move |zelf: &Py<PyList>, args: FuncArgs| {
                assert_eq!(zelf.as_object().strong_count(), count);
                assert_eq!(args.args.len(), 1);
                assert_eq!(args.kwargs.len(), 1);
            })
            .call(
                vm,
                Some(receiver.as_object()),
                FuncArgs::new(
                    vec![value],
                    core::iter::once(("named", vm.ctx.none())).collect::<super::super::KwArgs>(),
                ),
                Callee::named("borrowed"),
            )
            .unwrap();
            let payload = core::ptr::from_ref(&**receiver) as usize;
            ReceiverFunction::<_, true>(move |zelf: &PyList, _vm: &VirtualMachine| {
                assert_eq!(core::ptr::from_ref(zelf) as usize, payload);
            })
            .call(
                vm,
                Some(receiver.as_object()),
                FuncArgs::default(),
                Callee::named("payload"),
            )
            .unwrap();
            ReceiverFunction::<_, true>(move |zelf: PyRef<PyList>| {
                assert_eq!(zelf.as_object().strong_count(), count + 1);
            })
            .call(
                vm,
                Some(receiver.as_object()),
                FuncArgs::default(),
                Callee::named("owned"),
            )
            .unwrap();
            let result = ReceiverFunction::<_, true>(|zelf: PyObjectRef| zelf)
                .call(
                    vm,
                    Some(receiver.as_object()),
                    FuncArgs::default(),
                    Callee::named("object"),
                )
                .unwrap();
            assert!(result.is(receiver.as_object()));
        });
    }

    #[test]
    fn ordinary_functions_ignore_native_receivers() {
        use crate::{AsObject, builtins::PyList};
        crate::Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let receiver = vm.ctx.new_str("not the Python argument");
            let argument = vm.ctx.new_list(vec![]);
            let function = |value: &Py<PyList>| value.to_owned();
            let result = IntoPyNativeFn::call(
                &function,
                vm,
                Some(receiver.as_object()),
                FuncArgs::from(vec![argument.clone().into()]),
                Callee::named("ordinary"),
            )
            .unwrap();
            assert!(result.is(argument.as_object()));
        });
    }

    #[test]
    fn method_adapters_validate_missing_and_mismatched_payloads() {
        use crate::{
            AsObject,
            builtins::PyList,
            function::{ItemDoc, PyMethodFlags},
        };
        crate::Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let method = ReceiverFunction::<_, true>(|_zelf: &PyList| ());
            let error = method
                .call(vm, None, FuncArgs::default(), Callee::named("example"))
                .unwrap_err();
            assert_eq!(
                error.args().as_slice()[0]
                    .try_to_value::<String>(vm)
                    .unwrap(),
                "unbound method example needs an argument"
            );

            // This is a safe public construction API. A descriptor class check
            // alone cannot prove the Rust receiver payload matches the adapter.
            let definition =
                vm.ctx
                    .new_method_def("example", method, PyMethodFlags::METHOD, ItemDoc::NONE);
            let descriptor = definition.build_method(vm.ctx.types.object_type, vm);
            let wrong_receiver = vm.ctx.new_str("wrong payload");
            let error = descriptor
                .as_object()
                .call((wrong_receiver.clone(),), vm)
                .unwrap_err();
            assert!(error.class().is(vm.ctx.exceptions.type_error));
            let bound = descriptor.bind(wrong_receiver.into(), &vm.ctx);
            let error = bound.as_object().call((), vm).unwrap_err();
            assert!(error.class().is(vm.ctx.exceptions.type_error));
        });
    }
}

use super::{FromArgs, FuncArgs};
use crate::{
    convert::ToPyResult, object::PyThreadingConstraint, Py, PyPayload, PyRef, PyResult,
    VirtualMachine,
};
use std::marker::PhantomData;

/// A built-in Python function.
// PyCFunction in CPython
pub trait PyNativeFn:
    Fn(&VirtualMachine, FuncArgs) -> PyResult + PyThreadingConstraint + 'static
{
}
impl<F: Fn(&VirtualMachine, FuncArgs) -> PyResult + PyThreadingConstraint + 'static> PyNativeFn
    for F
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
    fn call(&self, vm: &VirtualMachine, args: FuncArgs) -> PyResult;

    /// `IntoPyNativeFn::into_func()` generates a PyNativeFn that performs the
    /// appropriate type and arity checking, any requested conversions, and then if
    /// successful calls the function with the extracted parameters.
    fn into_func(self) -> impl PyNativeFn {
        into_func(self)
    }
}

const fn into_func<F: IntoPyNativeFn<Kind>, Kind>(f: F) -> impl PyNativeFn {
    move |vm: &VirtualMachine, args| f.call(vm, args)
}

const fn zst_ref_out_of_thin_air<T: 'static>(x: T) -> &'static T {
    // if T is zero-sized, there's no issue forgetting it - even if it does have a Drop impl, it
    // would never get called anyway if we consider this semantically a Box::leak(Box::new(x))-type
    // operation. if T isn't zero-sized, we don't have to worry about it because we'll fail to compile.
    std::mem::forget(x);
    const {
        if std::mem::size_of::<T>() != 0 {
            panic!("can't use a non-zero-sized type here")
        }
        // SAFETY: we just confirmed that T is zero-sized, so we can
        //         pull a value of it out of thin air.
        unsafe { std::ptr::NonNull::<T>::dangling().as_ref() }
    }
}

/// Get the [`STATIC_FUNC`](IntoPyNativeFn::STATIC_FUNC) of the passed function. The same
/// requirements of zero-sizedness apply, see that documentation for details.
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
    #[inline(always)]
    fn call(&self, vm: &VirtualMachine, args: FuncArgs) -> PyResult {
        self.call_(vm, args)
    }
}

mod sealed {
    use super::*;
    pub trait PyNativeFnInternal<T, R, VM>: Sized + PyThreadingConstraint + 'static {
        fn call_(&self, vm: &VirtualMachine, args: FuncArgs) -> PyResult;
    }
}
use sealed::PyNativeFnInternal;

#[doc(hidden)]
pub struct OwnedParam<T>(PhantomData<T>);
#[doc(hidden)]
pub struct BorrowedParam<T>(PhantomData<T>);
#[doc(hidden)]
pub struct RefParam<T>(PhantomData<T>);

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
            fn call_(&self, vm: &VirtualMachine, args: FuncArgs) -> PyResult {
                let ($($n,)*) = args.bind::<($($T,)*)>(vm)?;

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
            fn call_(&self, vm: &VirtualMachine, args: FuncArgs) -> PyResult {
                let (zelf, $($n,)*) = args.bind::<(PyRef<S>, $($T,)*)>(vm)?;

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
            fn call_(&self, vm: &VirtualMachine, args: FuncArgs) -> PyResult {
                let (zelf, $($n,)*) = args.bind::<(PyRef<S>, $($T,)*)>(vm)?;

                (self)(&zelf, $($n,)* vm).to_pyresult(vm)
            }
        }

        impl<F, $($T,)* R> PyNativeFnInternal<($(OwnedParam<$T>,)*), R, ()> for F
        where
            F: Fn($($T,)*) -> R + PyThreadingConstraint + 'static,
            $($T: FromArgs,)*
            R: ToPyResult,
        {
            fn call_(&self, vm: &VirtualMachine, args: FuncArgs) -> PyResult {
                let ($($n,)*) = args.bind::<($($T,)*)>(vm)?;

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
            fn call_(&self, vm: &VirtualMachine, args: FuncArgs) -> PyResult {
                let (zelf, $($n,)*) = args.bind::<(PyRef<S>, $($T,)*)>(vm)?;

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
            fn call_(&self, vm: &VirtualMachine, args: FuncArgs) -> PyResult {
                let (zelf, $($n,)*) = args.bind::<(PyRef<S>, $($T,)*)>(vm)?;

                (self)(&zelf, $($n,)*).to_pyresult(vm)
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
    use std::mem::size_of_val;

    #[test]
    fn test_into_native_fn_noalloc() {
        fn py_func(_b: bool, _vm: &crate::VirtualMachine) -> i32 {
            1
        }
        assert_eq!(size_of_val(&py_func.into_func()), 0);
        let empty_closure = || "foo".to_owned();
        assert_eq!(size_of_val(&empty_closure.into_func()), 0);
        assert_eq!(size_of_val(static_func(empty_closure)), 0);
    }
}

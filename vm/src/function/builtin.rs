use super::{FromArgs, FuncArgs};
use crate::{
    convert::ToPyResult, object::PyThreadingConstraint, Py, PyPayload, PyRef, PyResult,
    VirtualMachine,
};
use std::marker::PhantomData;

/// A built-in Python function.
// PyCFunction in CPython
pub type PyNativeFn = py_dyn_fn!(dyn Fn(&VirtualMachine, FuncArgs) -> PyResult);

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
    fn into_func(self) -> &'static PyNativeFn {
        let boxed = Box::new(move |vm: &VirtualMachine, args| self.call(vm, args));
        Box::leak(boxed)
    }

    /// Equivalent to `into_func()`, but accessible as a constant. This is only
    /// valid if this function is zero-sized, i.e. that
    /// `std::mem::size_of::<F>() == 0`. If it isn't, use of this constant will
    /// raise a compile error.
    const STATIC_FUNC: &'static PyNativeFn = {
        if std::mem::size_of::<Self>() == 0 {
            &|vm, args| {
                // SAFETY: we just confirmed that Self is zero-sized, so there
                //         aren't any bytes in it that could be uninit.
                #[allow(clippy::uninit_assumed_init)]
                let f = unsafe { std::mem::MaybeUninit::<Self>::uninit().assume_init() };
                f.call(vm, args)
            }
        } else {
            panic!("function must be zero-sized to access STATIC_FUNC")
        }
    };
}

/// Get the [`STATIC_FUNC`](IntoPyNativeFn::STATIC_FUNC) of the passed function. The same
/// requirements of zero-sizedness apply, see that documentation for details.
#[inline(always)]
pub const fn static_func<Kind, F: IntoPyNativeFn<Kind>>(f: F) -> &'static PyNativeFn {
    // if f is zero-sized, there's no issue forgetting it - even if a capture of f does have a Drop
    // impl, it would never get called anyway. If you passed it to into_func, it would just get
    // Box::leak'd, and as a 'static reference it'll never be dropped. and if f isn't zero-sized,
    // we'll never reach this point anyway because we'll fail to compile.
    std::mem::forget(f);
    F::STATIC_FUNC
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

    #[test]
    fn test_into_native_fn_noalloc() {
        let check_zst = |f: &'static PyNativeFn| assert_eq!(std::mem::size_of_val(f), 0);
        fn py_func(_b: bool, _vm: &crate::VirtualMachine) -> i32 {
            1
        }
        check_zst(py_func.into_func());
        let empty_closure = || "foo".to_owned();
        check_zst(empty_closure.into_func());
        check_zst(static_func(empty_closure));
    }
}

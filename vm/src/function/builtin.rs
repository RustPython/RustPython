use super::{FromArgs, FuncArgs};
use crate::{
    convert::ToPyResult, pyobject::PyThreadingConstraint, PyPayload, PyRef, PyResult,
    VirtualMachine,
};
use std::marker::PhantomData;

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
    #[inline(always)]
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
            R: ToPyResult,
        {
            fn call_(&self, vm: &VirtualMachine, args: FuncArgs) -> PyResult {
                let ($($n,)*) = args.bind::<($($T,)*)>(vm)?;

                (self)($($n,)* vm).to_pyresult(vm)
            }
        }

        impl<F, S, $($T,)* R> PyNativeFuncInternal<(RefParam<S>, $(OwnedParam<$T>,)*), R, VirtualMachine> for F
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

        impl<F, $($T,)* R> PyNativeFuncInternal<($(OwnedParam<$T>,)*), R, ()> for F
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

        impl<F, S, $($T,)* R> PyNativeFuncInternal<(RefParam<S>, $(OwnedParam<$T>,)*), R, ()> for F
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

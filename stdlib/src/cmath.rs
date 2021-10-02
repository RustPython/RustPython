// TODO: Keep track of rust-num/num-complex/issues/2. A common trait could help with duplication
//       that exists between cmath and math.
pub(crate) use cmath::make_module;

/// This module provides access to mathematical functions for complex numbers.
#[pymodule]
mod cmath {
    use crate::vm::{
        builtins::{IntoPyComplex, IntoPyFloat},
        function::OptionalArg,
        PyResult, VirtualMachine,
    };
    use num_complex::Complex64;

    // Constants
    #[pyattr]
    use std::f64::consts::{E as e, PI as pi, TAU as tau};
    #[pyattr]
    use std::f64::{INFINITY as inf, NAN as nan};
    #[pyattr(name = "infj")]
    const INFJ: Complex64 = Complex64::new(0., std::f64::INFINITY);
    #[pyattr(name = "nanj")]
    const NANJ: Complex64 = Complex64::new(0., std::f64::NAN);

    #[pyfunction]
    fn phase(z: IntoPyComplex) -> f64 {
        z.to_complex().arg()
    }

    #[pyfunction]
    fn polar(x: IntoPyComplex) -> (f64, f64) {
        x.to_complex().to_polar()
    }

    #[pyfunction]
    fn rect(r: IntoPyFloat, phi: IntoPyFloat) -> Complex64 {
        Complex64::from_polar(r.to_f64(), phi.to_f64())
    }

    #[pyfunction]
    fn isinf(z: IntoPyComplex) -> bool {
        let Complex64 { re, im } = z.to_complex();
        re.is_infinite() || im.is_infinite()
    }

    #[pyfunction]
    fn isfinite(z: IntoPyComplex) -> bool {
        z.to_complex().is_finite()
    }

    #[pyfunction]
    fn isnan(z: IntoPyComplex) -> bool {
        z.to_complex().is_nan()
    }

    #[pyfunction]
    fn exp(z: IntoPyComplex, vm: &VirtualMachine) -> PyResult<Complex64> {
        let z = z.to_complex();
        result_or_overflow(z, z.exp(), vm)
    }

    #[pyfunction]
    fn sqrt(z: IntoPyComplex) -> Complex64 {
        z.to_complex().sqrt()
    }

    #[pyfunction]
    fn sin(z: IntoPyComplex) -> Complex64 {
        z.to_complex().sin()
    }

    #[pyfunction]
    fn asin(z: IntoPyComplex) -> Complex64 {
        z.to_complex().asin()
    }

    #[pyfunction]
    fn cos(z: IntoPyComplex) -> Complex64 {
        z.to_complex().cos()
    }

    #[pyfunction]
    fn acos(z: IntoPyComplex) -> Complex64 {
        z.to_complex().acos()
    }

    #[pyfunction]
    fn log(z: IntoPyComplex, base: OptionalArg<IntoPyFloat>) -> Complex64 {
        z.to_complex().log(
            base.into_option()
                .map(|base| base.to_f64())
                .unwrap_or(std::f64::consts::E),
        )
    }

    #[pyfunction]
    fn log10(z: IntoPyComplex) -> Complex64 {
        z.to_complex().log(10.0)
    }

    #[pyfunction]
    fn acosh(z: IntoPyComplex) -> Complex64 {
        z.to_complex().acosh()
    }

    #[pyfunction]
    fn atan(z: IntoPyComplex) -> Complex64 {
        z.to_complex().atan()
    }

    #[pyfunction]
    fn atanh(z: IntoPyComplex) -> Complex64 {
        z.to_complex().atanh()
    }

    #[pyfunction]
    fn tan(z: IntoPyComplex) -> Complex64 {
        z.to_complex().tan()
    }

    #[pyfunction]
    fn tanh(z: IntoPyComplex) -> Complex64 {
        z.to_complex().tanh()
    }

    #[pyfunction]
    fn asinh(z: IntoPyComplex) -> Complex64 {
        z.to_complex().asinh()
    }

    #[derive(FromArgs)]
    struct IsCloseArgs {
        #[pyarg(positional)]
        a: IntoPyComplex,
        #[pyarg(positional)]
        b: IntoPyComplex,
        #[pyarg(named, optional)]
        rel_tol: OptionalArg<IntoPyFloat>,
        #[pyarg(named, optional)]
        abs_tol: OptionalArg<IntoPyFloat>,
    }

    #[pyfunction]
    fn isclose(args: IsCloseArgs, vm: &VirtualMachine) -> PyResult<bool> {
        let a = args.a.to_complex();
        let b = args.b.to_complex();
        let rel_tol = args.rel_tol.map_or(1e-09, |value| value.to_f64());
        let abs_tol = args.abs_tol.map_or(0.0, |value| value.to_f64());

        if rel_tol < 0.0 || abs_tol < 0.0 {
            return Err(vm.new_value_error("tolerances must be non-negative".to_owned()));
        }

        if a == b {
            /* short circuit exact equality -- needed to catch two infinities of
            the same sign. And perhaps speeds things up a bit sometimes.
            */
            return Ok(true);
        }

        /* This catches the case of two infinities of opposite sign, or
        one infinity and one finite number. Two infinities of opposite
        sign would otherwise have an infinite relative tolerance.
        Two infinities of the same sign are caught by the equality check
        above.
        */
        if a.is_infinite() || b.is_infinite() {
            return Ok(false);
        }

        let diff = c_abs(b - a);

        Ok(diff <= (rel_tol * c_abs(b)) || (diff <= (rel_tol * c_abs(a))) || diff <= abs_tol)
    }

    #[inline]
    fn c_abs(Complex64 { re, im }: Complex64) -> f64 {
        re.hypot(im)
    }

    #[inline]
    fn result_or_overflow(
        value: Complex64,
        result: Complex64,
        vm: &VirtualMachine,
    ) -> PyResult<Complex64> {
        if !result.is_finite() && value.is_finite() {
            // CPython doesn't return `inf` when called with finite
            // values, it raises OverflowError instead.
            Err(vm.new_overflow_error("math range error".to_owned()))
        } else {
            Ok(result)
        }
    }
}

// TODO: Keep track of rust-num/num-complex/issues/2. A common trait could help with duplication
//       that exists between cmath and math.
pub(crate) use cmath::make_module;
#[pymodule]
mod cmath {
    use crate::vm::{
        PyResult, VirtualMachine,
        function::{ArgIntoComplex, ArgIntoFloat, OptionalArg},
    };
    use num_complex::Complex64;

    // Constants
    #[pyattr]
    use core::f64::consts::{E as e, PI as pi, TAU as tau};
    #[pyattr(name = "inf")]
    const INF: f64 = f64::INFINITY;
    #[pyattr(name = "nan")]
    const NAN: f64 = f64::NAN;
    #[pyattr(name = "infj")]
    const INFJ: Complex64 = Complex64::new(0., f64::INFINITY);
    #[pyattr(name = "nanj")]
    const NANJ: Complex64 = Complex64::new(0., f64::NAN);

    #[pyfunction]
    fn phase(z: ArgIntoComplex) -> f64 {
        z.into_complex().arg()
    }

    #[pyfunction]
    fn polar(x: ArgIntoComplex) -> (f64, f64) {
        x.into_complex().to_polar()
    }

    #[pyfunction]
    fn rect(r: ArgIntoFloat, phi: ArgIntoFloat) -> Complex64 {
        Complex64::from_polar(r.into_float(), phi.into_float())
    }

    #[pyfunction]
    fn isinf(z: ArgIntoComplex) -> bool {
        let Complex64 { re, im } = z.into_complex();
        re.is_infinite() || im.is_infinite()
    }

    #[pyfunction]
    fn isfinite(z: ArgIntoComplex) -> bool {
        z.into_complex().is_finite()
    }

    #[pyfunction]
    fn isnan(z: ArgIntoComplex) -> bool {
        z.into_complex().is_nan()
    }

    #[pyfunction]
    fn exp(z: ArgIntoComplex, vm: &VirtualMachine) -> PyResult<Complex64> {
        let z = z.into_complex();
        result_or_overflow(z, z.exp(), vm)
    }

    #[pyfunction]
    fn sqrt(z: ArgIntoComplex) -> Complex64 {
        z.into_complex().sqrt()
    }

    #[pyfunction]
    fn sin(z: ArgIntoComplex) -> Complex64 {
        z.into_complex().sin()
    }

    #[pyfunction]
    fn asin(z: ArgIntoComplex) -> Complex64 {
        z.into_complex().asin()
    }

    #[pyfunction]
    fn cos(z: ArgIntoComplex) -> Complex64 {
        z.into_complex().cos()
    }

    #[pyfunction]
    fn acos(z: ArgIntoComplex) -> Complex64 {
        z.into_complex().acos()
    }

    #[pyfunction]
    fn log(z: ArgIntoComplex, base: OptionalArg<ArgIntoComplex>) -> Complex64 {
        // TODO: Complex64.log with a negative base yields wrong results.
        //       Issue is with num_complex::Complex64 implementation of log
        //       which returns NaN when base is negative.
        //       log10(z) / log10(base) yields correct results but division
        //       doesn't handle pos/neg zero nicely. (i.e log(1, 0.5))
        z.into_complex().log(
            base.into_option()
                .map(|base| base.into_complex().re)
                .unwrap_or(core::f64::consts::E),
        )
    }

    #[pyfunction]
    fn log10(z: ArgIntoComplex) -> Complex64 {
        z.into_complex().log(10.0)
    }

    #[pyfunction]
    fn acosh(z: ArgIntoComplex) -> Complex64 {
        z.into_complex().acosh()
    }

    #[pyfunction]
    fn atan(z: ArgIntoComplex) -> Complex64 {
        z.into_complex().atan()
    }

    #[pyfunction]
    fn atanh(z: ArgIntoComplex) -> Complex64 {
        z.into_complex().atanh()
    }

    #[pyfunction]
    fn tan(z: ArgIntoComplex) -> Complex64 {
        z.into_complex().tan()
    }

    #[pyfunction]
    fn tanh(z: ArgIntoComplex) -> Complex64 {
        z.into_complex().tanh()
    }

    #[pyfunction]
    fn sinh(z: ArgIntoComplex) -> Complex64 {
        z.into_complex().sinh()
    }

    #[pyfunction]
    fn cosh(z: ArgIntoComplex) -> Complex64 {
        z.into_complex().cosh()
    }

    #[pyfunction]
    fn asinh(z: ArgIntoComplex) -> Complex64 {
        z.into_complex().asinh()
    }

    #[derive(FromArgs)]
    struct IsCloseArgs {
        #[pyarg(positional)]
        a: ArgIntoComplex,
        #[pyarg(positional)]
        b: ArgIntoComplex,
        #[pyarg(named, optional)]
        rel_tol: OptionalArg<ArgIntoFloat>,
        #[pyarg(named, optional)]
        abs_tol: OptionalArg<ArgIntoFloat>,
    }

    #[pyfunction]
    fn isclose(args: IsCloseArgs, vm: &VirtualMachine) -> PyResult<bool> {
        let a = args.a.into_complex();
        let b = args.b.into_complex();
        let rel_tol = args.rel_tol.map_or(1e-09, |v| v.into_float());
        let abs_tol = args.abs_tol.map_or(0.0, |v| v.into_float());

        if rel_tol < 0.0 || abs_tol < 0.0 {
            return Err(vm.new_value_error("tolerances must be non-negative"));
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
            Err(vm.new_overflow_error("math range error"))
        } else {
            Ok(result)
        }
    }
}

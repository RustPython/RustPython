// TODO: Keep track of rust-num/num-complex/issues/2. A common trait could help with duplication
//       that exists between cmath and math.
pub(crate) use cmath::make_module;
#[pymodule]
mod cmath {
    use crate::vm::{
        function::{ArgIntoComplex, ArgIntoFloat, OptionalArg},
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
    fn phase(z: ArgIntoComplex) -> f64 {
        z.arg()
    }

    #[pyfunction]
    fn polar(x: ArgIntoComplex) -> (f64, f64) {
        x.to_polar()
    }

    #[pyfunction]
    fn rect(r: ArgIntoFloat, phi: ArgIntoFloat) -> Complex64 {
        Complex64::from_polar(*r, *phi)
    }

    #[pyfunction]
    fn isinf(z: ArgIntoComplex) -> bool {
        let Complex64 { re, im } = *z;
        re.is_infinite() || im.is_infinite()
    }

    #[pyfunction]
    fn isfinite(z: ArgIntoComplex) -> bool {
        z.is_finite()
    }

    #[pyfunction]
    fn isnan(z: ArgIntoComplex) -> bool {
        z.is_nan()
    }

    #[pyfunction]
    fn exp(z: ArgIntoComplex, vm: &VirtualMachine) -> PyResult<Complex64> {
        let z = *z;
        result_or_overflow(z, z.exp(), vm)
    }

    #[pyfunction]
    fn sqrt(z: ArgIntoComplex) -> Complex64 {
        z.sqrt()
    }

    #[pyfunction]
    fn sin(z: ArgIntoComplex) -> Complex64 {
        z.sin()
    }

    #[pyfunction]
    fn asin(z: ArgIntoComplex) -> Complex64 {
        z.asin()
    }

    #[pyfunction]
    fn cos(z: ArgIntoComplex) -> Complex64 {
        z.cos()
    }

    #[pyfunction]
    fn acos(z: ArgIntoComplex) -> Complex64 {
        z.acos()
    }

    #[pyfunction]
    fn log(z: ArgIntoComplex, base: OptionalArg<ArgIntoComplex>) -> Complex64 {
        // TODO: Complex64.log with a negative base yields wrong results.
        //       Issue is with num_complex::Complex64 implementation of log
        //       which returns NaN when base is negative.
        //       log10(z) / log10(base) yields correct results but division
        //       doesn't handle pos/neg zero nicely. (i.e log(1, 0.5))
        z.log(
            base.into_option()
                .map(|base| base.re)
                .unwrap_or(std::f64::consts::E),
        )
    }

    #[pyfunction]
    fn log10(z: ArgIntoComplex) -> Complex64 {
        z.log(10.0)
    }

    #[pyfunction]
    fn acosh(z: ArgIntoComplex) -> Complex64 {
        z.acosh()
    }

    #[pyfunction]
    fn atan(z: ArgIntoComplex) -> Complex64 {
        z.atan()
    }

    #[pyfunction]
    fn atanh(z: ArgIntoComplex) -> Complex64 {
        z.atanh()
    }

    #[pyfunction]
    fn tan(z: ArgIntoComplex) -> Complex64 {
        z.tan()
    }

    #[pyfunction]
    fn tanh(z: ArgIntoComplex) -> Complex64 {
        z.tanh()
    }

    #[pyfunction]
    fn sinh(z: ArgIntoComplex) -> Complex64 {
        z.sinh()
    }

    #[pyfunction]
    fn cosh(z: ArgIntoComplex) -> Complex64 {
        z.cosh()
    }

    #[pyfunction]
    fn asinh(z: ArgIntoComplex) -> Complex64 {
        z.asinh()
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
        let a = *args.a;
        let b = *args.b;
        let rel_tol = args.rel_tol.map_or(1e-09, Into::into);
        let abs_tol = args.abs_tol.map_or(0.0, Into::into);

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

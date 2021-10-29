// TODO: Keep track of rust-num/num-complex/issues/2. A common trait could help with duplication
//       that exists between cmath and math.
pub(crate) use cmath::make_module;

/// This module provides access to mathematical functions for complex numbers.
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

    /// Return argument, also known as the phase angle, of a complex.
    #[pyfunction]
    fn phase(z: ArgIntoComplex) -> f64 {
        z.to_complex().arg()
    }

    /// Convert a complex from rectangular coordinates to polar coordinates.
    ///
    /// r is the distance from 0 and phi the phase angle.
    #[pyfunction]
    fn polar(x: ArgIntoComplex) -> (f64, f64) {
        x.to_complex().to_polar()
    }

    /// Convert from polar coordinates to rectangular coordinates.
    #[pyfunction]
    fn rect(r: ArgIntoFloat, phi: ArgIntoFloat) -> Complex64 {
        Complex64::from_polar(r.to_f64(), phi.to_f64())
    }

    /// Checks if the real or imaginary part of z is infinite.
    #[pyfunction]
    fn isinf(z: ArgIntoComplex) -> bool {
        let Complex64 { re, im } = z.to_complex();
        re.is_infinite() || im.is_infinite()
    }

    /// Return True if both the real and imaginary parts of z are finite, else False.
    #[pyfunction]
    fn isfinite(z: ArgIntoComplex) -> bool {
        z.to_complex().is_finite()
    }

    /// Checks if the real or imaginary part of z not a number (NaN)..
    #[pyfunction]
    fn isnan(z: ArgIntoComplex) -> bool {
        z.to_complex().is_nan()
    }

    /// Return the exponential value e**z.
    #[pyfunction]
    fn exp(z: ArgIntoComplex, vm: &VirtualMachine) -> PyResult<Complex64> {
        let z = z.to_complex();
        result_or_overflow(z, z.exp(), vm)
    }
    /// Return the square root of z.
    #[pyfunction]
    fn sqrt(z: ArgIntoComplex) -> Complex64 {
        z.to_complex().sqrt()
    }
    /// Return the sine of z
    #[pyfunction]
    fn sin(z: ArgIntoComplex) -> Complex64 {
        z.to_complex().sin()
    }

    #[pyfunction]
    fn asin(z: ArgIntoComplex) -> Complex64 {
        z.to_complex().asin()
    }

    /// Return the cosine of z
    #[pyfunction]
    fn cos(z: ArgIntoComplex) -> Complex64 {
        z.to_complex().cos()
    }

    #[pyfunction]
    fn acos(z: ArgIntoComplex) -> Complex64 {
        z.to_complex().acos()
    }

    /// log(z[, base]) -> the logarithm of z to the given base.
    ///
    /// If the base not specified, returns the natural logarithm (base e) of z.
    #[pyfunction]
    fn log(z: ArgIntoComplex, base: OptionalArg<ArgIntoComplex>) -> Complex64 {
        // TODO: Complex64.log with a negative base yields wrong results.
        //       Issue is with num_complex::Complex64 implementation of log
        //       which returns NaN when base is negative.
        //       log10(z) / log10(base) yields correct results but division
        //       doesn't handle pos/neg zero nicely. (i.e log(1, 0.5))
        z.to_complex().log(
            base.into_option()
                .map(|base| base.to_complex().re)
                .unwrap_or(std::f64::consts::E),
        )
    }

    /// Return the base-10 logarithm of z.
    #[pyfunction]
    fn log10(z: ArgIntoComplex) -> Complex64 {
        z.to_complex().log(10.0)
    }

    /// Return the inverse hyperbolic cosine of z.
    #[pyfunction]
    fn acosh(z: ArgIntoComplex) -> Complex64 {
        z.to_complex().acosh()
    }

    /// Return the inverse tangent of z.
    #[pyfunction]
    fn atan(z: ArgIntoComplex) -> Complex64 {
        z.to_complex().atan()
    }

    /// Return the inverse hyperbolic tangent of z.
    #[pyfunction]
    fn atanh(z: ArgIntoComplex) -> Complex64 {
        z.to_complex().atanh()
    }

    /// Return the tangent of z.
    #[pyfunction]
    fn tan(z: ArgIntoComplex) -> Complex64 {
        z.to_complex().tan()
    }

    /// Return the hyperbolic tangent of z.
    #[pyfunction]
    fn tanh(z: ArgIntoComplex) -> Complex64 {
        z.to_complex().tanh()
    }

    /// Return the hyperbolic sin of z.
    #[pyfunction]
    fn sinh(z: ArgIntoComplex) -> Complex64 {
        z.to_complex().sinh()
    }

    /// Return the hyperbolic cosine of z.
    #[pyfunction]
    fn cosh(z: ArgIntoComplex) -> Complex64 {
        z.to_complex().cosh()
    }

    /// Return the inverse hyperbolic sine of z.
    #[pyfunction]
    fn asinh(z: ArgIntoComplex) -> Complex64 {
        z.to_complex().asinh()
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

    /// Determine whether two complex numbers are close in value.
    ///
    ///   rel_tol
    ///     maximum difference for being considered "close", relative to the
    ///     magnitude of the input values
    ///   abs_tol
    ///     maximum difference for being considered "close", regardless of the
    ///     magnitude of the input values
    ///
    /// Return True if a is close in value to b, and False otherwise.
    ///
    /// For the values to be considered close, the difference between them must be
    /// smaller than at least one of the tolerances.
    ///
    /// -inf, inf and NaN behave similarly to the IEEE 754 Standard. That is, NaN is
    /// not close to anything, even itself. inf and -inf are only close to themselves.
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

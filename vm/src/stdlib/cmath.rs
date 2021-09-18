// TODO: Keep track of rust-num/num-complex/issues/2. A common trait could help with duplication
//       that exists between cmath and math.
use crate::{PyObjectRef, VirtualMachine};

pub(crate) fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let module = cmath::make_module(vm);
    let ctx = &vm.ctx;
    extend_module!(vm, module, {
        // Constants:
        "pi" => ctx.new_float(std::f64::consts::PI),
        "e" => ctx.new_float(std::f64::consts::E),
        "tau" => ctx.new_float(2.0 * std::f64::consts::PI),
        "inf" => ctx.new_float(f64::INFINITY),
        "infj" => ctx.new_complex(num_complex::Complex64::new(0., std::f64::INFINITY)),
        "nan" => ctx.new_float(f64::NAN),
        "nanj" => ctx.new_complex(num_complex::Complex64::new(0., std::f64::NAN)),
    });

    module
}

/// This module provides access to mathematical functions for complex numbers.
#[pymodule]
mod cmath {
    use crate::builtins::{complex::IntoPyComplex, float::IntoPyFloat};
    use crate::function::OptionalArg;
    use crate::{PyResult, VirtualMachine};
    use num_complex::Complex64;

    /// Return argument, also known as the phase angle, of a complex.
    #[pyfunction]
    fn phase(z: IntoPyComplex) -> f64 {
        z.to_complex().arg()
    }

    /// Convert a complex from rectangular coordinates to polar coordinates.
    ///
    /// r is the distance from 0 and phi the phase angle.
    #[pyfunction]
    fn polar(x: IntoPyComplex) -> (f64, f64) {
        x.to_complex().to_polar()
    }

    /// Convert from polar coordinates to rectangular coordinates.
    #[pyfunction]
    fn rect(r: IntoPyFloat, phi: IntoPyFloat) -> Complex64 {
        Complex64::from_polar(r.to_f64(), phi.to_f64())
    }

    /// Checks if the real or imaginary part of z is infinite.
    #[pyfunction]
    fn isinf(z: IntoPyComplex) -> bool {
        let Complex64 { re, im } = z.to_complex();
        re.is_infinite() || im.is_infinite()
    }

    /// Return True if both the real and imaginary parts of z are finite, else False.
    #[pyfunction]
    fn isfinite(z: IntoPyComplex) -> bool {
        z.to_complex().is_finite()
    }

    /// Checks if the real or imaginary part of z not a number (NaN)..
    #[pyfunction]
    fn isnan(z: IntoPyComplex) -> bool {
        z.to_complex().is_nan()
    }

    /// Return the exponential value e**z.
    #[pyfunction]
    fn exp(z: IntoPyComplex, vm: &VirtualMachine) -> PyResult<Complex64> {
        let z = z.to_complex();
        result_or_overflow(z, z.exp(), vm)
    }
    /// Return the square root of z.
    #[pyfunction]
    fn sqrt(z: IntoPyComplex) -> Complex64 {
        z.to_complex().sqrt()
    }
    /// Return the sine of z
    #[pyfunction]
    fn sin(z: IntoPyComplex) -> Complex64 {
        z.to_complex().sin()
    }

    /// Return the cosine of z
    #[pyfunction]
    fn cos(z: IntoPyComplex) -> Complex64 {
        z.to_complex().cos()
    }

    /// log(z[, base]) -> the logarithm of z to the given base.
    ///
    /// If the base not specified, returns the natural logarithm (base e) of z.
    #[pyfunction]
    fn log(z: IntoPyComplex, base: OptionalArg<IntoPyFloat>) -> Complex64 {
        z.to_complex().log(
            base.into_option()
                .map(|base| base.to_f64())
                .unwrap_or(std::f64::consts::E),
        )
    }

    /// Return the base-10 logarithm of z.
    #[pyfunction]
    fn log10(z: IntoPyComplex) -> Complex64 {
        z.to_complex().log(10.0)
    }

    /// Return the inverse hyperbolic cosine of z.
    #[pyfunction]
    fn acosh(z: IntoPyComplex) -> Complex64 {
        z.to_complex().acosh()
    }

    /// Return the tangent of z.
    #[pyfunction]
    fn tan(z: IntoPyComplex) -> Complex64 {
        z.to_complex().tan()
    }

    /// Return the hyperbolic tangent of z.
    #[pyfunction]
    fn tanh(z: IntoPyComplex) -> Complex64 {
        z.to_complex().tanh()
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

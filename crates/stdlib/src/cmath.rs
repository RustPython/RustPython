pub(crate) use cmath::module_def;

#[pymodule]
mod cmath {
    use crate::vm::{
        PyResult, VirtualMachine,
        function::{ArgIntoComplex, ArgIntoFloat, OptionalArg},
    };
    use num_complex::Complex64;

    use crate::math::pymath_exception;

    // Constants
    #[pyattr(name = "e")]
    const E: f64 = pymath::cmath::E;
    #[pyattr(name = "pi")]
    const PI: f64 = pymath::cmath::PI;
    #[pyattr(name = "tau")]
    const TAU: f64 = pymath::cmath::TAU;
    #[pyattr(name = "inf")]
    const INF: f64 = pymath::cmath::INF;
    #[pyattr(name = "nan")]
    const NAN: f64 = pymath::cmath::NAN;
    #[pyattr(name = "infj")]
    const INFJ: Complex64 = pymath::cmath::INFJ;
    #[pyattr(name = "nanj")]
    const NANJ: Complex64 = pymath::cmath::NANJ;

    #[pyfunction]
    fn phase(z: ArgIntoComplex, vm: &VirtualMachine) -> PyResult<f64> {
        pymath::cmath::phase(z.into_complex()).map_err(|err| pymath_exception(err, vm))
    }

    #[pyfunction]
    fn polar(x: ArgIntoComplex, vm: &VirtualMachine) -> PyResult<(f64, f64)> {
        pymath::cmath::polar(x.into_complex()).map_err(|err| pymath_exception(err, vm))
    }

    #[pyfunction]
    fn rect(r: ArgIntoFloat, phi: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<Complex64> {
        pymath::cmath::rect(r.into_float(), phi.into_float())
            .map_err(|err| pymath_exception(err, vm))
    }

    #[pyfunction]
    fn isinf(z: ArgIntoComplex) -> bool {
        pymath::cmath::isinf(z.into_complex())
    }

    #[pyfunction]
    fn isfinite(z: ArgIntoComplex) -> bool {
        pymath::cmath::isfinite(z.into_complex())
    }

    #[pyfunction]
    fn isnan(z: ArgIntoComplex) -> bool {
        pymath::cmath::isnan(z.into_complex())
    }

    #[pyfunction]
    fn exp(z: ArgIntoComplex, vm: &VirtualMachine) -> PyResult<Complex64> {
        pymath::cmath::exp(z.into_complex()).map_err(|err| pymath_exception(err, vm))
    }

    #[pyfunction]
    fn sqrt(z: ArgIntoComplex, vm: &VirtualMachine) -> PyResult<Complex64> {
        pymath::cmath::sqrt(z.into_complex()).map_err(|err| pymath_exception(err, vm))
    }

    #[pyfunction]
    fn sin(z: ArgIntoComplex, vm: &VirtualMachine) -> PyResult<Complex64> {
        pymath::cmath::sin(z.into_complex()).map_err(|err| pymath_exception(err, vm))
    }

    #[pyfunction]
    fn asin(z: ArgIntoComplex, vm: &VirtualMachine) -> PyResult<Complex64> {
        pymath::cmath::asin(z.into_complex()).map_err(|err| pymath_exception(err, vm))
    }

    #[pyfunction]
    fn cos(z: ArgIntoComplex, vm: &VirtualMachine) -> PyResult<Complex64> {
        pymath::cmath::cos(z.into_complex()).map_err(|err| pymath_exception(err, vm))
    }

    #[pyfunction]
    fn acos(z: ArgIntoComplex, vm: &VirtualMachine) -> PyResult<Complex64> {
        pymath::cmath::acos(z.into_complex()).map_err(|err| pymath_exception(err, vm))
    }

    #[pyfunction]
    fn log(
        z: ArgIntoComplex,
        base: OptionalArg<ArgIntoComplex>,
        vm: &VirtualMachine,
    ) -> PyResult<Complex64> {
        pymath::cmath::log(
            z.into_complex(),
            base.into_option().map(|b| b.into_complex()),
        )
        .map_err(|err| pymath_exception(err, vm))
    }

    #[pyfunction]
    fn log10(z: ArgIntoComplex, vm: &VirtualMachine) -> PyResult<Complex64> {
        pymath::cmath::log10(z.into_complex()).map_err(|err| pymath_exception(err, vm))
    }

    #[pyfunction]
    fn acosh(z: ArgIntoComplex, vm: &VirtualMachine) -> PyResult<Complex64> {
        pymath::cmath::acosh(z.into_complex()).map_err(|err| pymath_exception(err, vm))
    }

    #[pyfunction]
    fn atan(z: ArgIntoComplex, vm: &VirtualMachine) -> PyResult<Complex64> {
        pymath::cmath::atan(z.into_complex()).map_err(|err| pymath_exception(err, vm))
    }

    #[pyfunction]
    fn atanh(z: ArgIntoComplex, vm: &VirtualMachine) -> PyResult<Complex64> {
        pymath::cmath::atanh(z.into_complex()).map_err(|err| pymath_exception(err, vm))
    }

    #[pyfunction]
    fn tan(z: ArgIntoComplex, vm: &VirtualMachine) -> PyResult<Complex64> {
        pymath::cmath::tan(z.into_complex()).map_err(|err| pymath_exception(err, vm))
    }

    #[pyfunction]
    fn tanh(z: ArgIntoComplex, vm: &VirtualMachine) -> PyResult<Complex64> {
        pymath::cmath::tanh(z.into_complex()).map_err(|err| pymath_exception(err, vm))
    }

    #[pyfunction]
    fn sinh(z: ArgIntoComplex, vm: &VirtualMachine) -> PyResult<Complex64> {
        pymath::cmath::sinh(z.into_complex()).map_err(|err| pymath_exception(err, vm))
    }

    #[pyfunction]
    fn cosh(z: ArgIntoComplex, vm: &VirtualMachine) -> PyResult<Complex64> {
        pymath::cmath::cosh(z.into_complex()).map_err(|err| pymath_exception(err, vm))
    }

    #[pyfunction]
    fn asinh(z: ArgIntoComplex, vm: &VirtualMachine) -> PyResult<Complex64> {
        pymath::cmath::asinh(z.into_complex()).map_err(|err| pymath_exception(err, vm))
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
        let rel_tol = args.rel_tol.into_option().map(|v| v.into_float());
        let abs_tol = args.abs_tol.into_option().map(|v| v.into_float());

        pymath::cmath::isclose(a, b, rel_tol, abs_tol)
            .map_err(|_| vm.new_value_error("tolerances must be non-negative"))
    }
}

pub(crate) use math::make_module;

use crate::vm::{VirtualMachine, builtins::PyBaseExceptionRef};

#[pymodule]
mod math {
    use crate::vm::{
        AsObject, PyObject, PyObjectRef, PyRef, PyResult, VirtualMachine,
        builtins::{PyFloat, PyInt, PyIntRef, PyStrInterned, try_bigint_to_f64, try_f64_to_bigint},
        function::{ArgIndex, ArgIntoFloat, ArgIterable, Either, OptionalArg, PosArgs},
        identifier,
    };
    use malachite_bigint::BigInt;
    use num_traits::{Signed, ToPrimitive};

    use super::{float_repr, pymath_exception};

    // Constants
    #[pyattr]
    use core::f64::consts::{E as e, PI as pi, TAU as tau};

    #[pyattr(name = "inf")]
    const INF: f64 = f64::INFINITY;
    #[pyattr(name = "nan")]
    const NAN: f64 = f64::NAN;

    // Number theory functions:
    #[pyfunction]
    fn fabs(x: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        pymath::math::fabs(x.into_float()).map_err(|err| pymath_exception(err, vm))
    }

    #[pyfunction]
    fn isfinite(x: ArgIntoFloat) -> bool {
        pymath::math::isfinite(x.into_float())
    }

    #[pyfunction]
    fn isinf(x: ArgIntoFloat) -> bool {
        pymath::math::isinf(x.into_float())
    }

    #[pyfunction]
    fn isnan(x: ArgIntoFloat) -> bool {
        pymath::math::isnan(x.into_float())
    }

    #[derive(FromArgs)]
    struct IsCloseArgs {
        #[pyarg(positional)]
        a: ArgIntoFloat,
        #[pyarg(positional)]
        b: ArgIntoFloat,
        #[pyarg(named, optional)]
        rel_tol: OptionalArg<ArgIntoFloat>,
        #[pyarg(named, optional)]
        abs_tol: OptionalArg<ArgIntoFloat>,
    }

    #[pyfunction]
    fn isclose(args: IsCloseArgs, vm: &VirtualMachine) -> PyResult<bool> {
        let a = args.a.into_float();
        let b = args.b.into_float();
        let rel_tol = args.rel_tol.into_option().map(|v| v.into_float());
        let abs_tol = args.abs_tol.into_option().map(|v| v.into_float());

        pymath::math::isclose(a, b, rel_tol, abs_tol)
            .map_err(|_| vm.new_value_error("tolerances must be non-negative"))
    }

    #[pyfunction]
    fn copysign(x: ArgIntoFloat, y: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        pymath::math::copysign(x.into_float(), y.into_float())
            .map_err(|err| pymath_exception(err, vm))
    }

    // Power and logarithmic functions:
    #[pyfunction]
    fn exp(x: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        pymath::math::exp(x.into_float()).map_err(|err| pymath_exception(err, vm))
    }

    #[pyfunction]
    fn exp2(x: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        pymath::math::exp2(x.into_float()).map_err(|err| pymath_exception(err, vm))
    }

    #[pyfunction]
    fn expm1(x: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        pymath::math::expm1(x.into_float()).map_err(|err| pymath_exception(err, vm))
    }

    #[pyfunction]
    fn log(x: PyObjectRef, base: OptionalArg<ArgIntoFloat>, vm: &VirtualMachine) -> PyResult<f64> {
        let base = base.into_option().map(|v| v.into_float());
        // Check base first for proper error messages
        if let Some(b) = base {
            if b <= 0.0 {
                return Err(vm.new_value_error(format!(
                    "expected a positive input, got {}",
                    super::float_repr(b)
                )));
            }
            if b == 1.0 {
                return Err(vm.new_value_error("math domain error".to_owned()));
            }
        }
        // Handle BigInt specially for large values (only for actual int type, not float)
        if let Some(i) = x.downcast_ref::<PyInt>() {
            return pymath::math::log_bigint(i.as_bigint(), base).map_err(|err| match err {
                pymath::Error::EDOM => vm.new_value_error("expected a positive input".to_owned()),
                _ => pymath_exception(err, vm),
            });
        }
        let val = x.try_float(vm)?.to_f64();
        pymath::math::log(val, base).map_err(|err| match err {
            pymath::Error::EDOM => vm.new_value_error(format!(
                "expected a positive input, got {}",
                super::float_repr(val)
            )),
            _ => pymath_exception(err, vm),
        })
    }

    #[pyfunction]
    fn log1p(x: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        pymath::math::log1p(x.into_float()).map_err(|err| pymath_exception(err, vm))
    }

    #[pyfunction]
    fn log2(x: PyObjectRef, vm: &VirtualMachine) -> PyResult<f64> {
        // Handle BigInt specially for large values (only for actual int type, not float)
        if let Some(i) = x.downcast_ref::<PyInt>() {
            return pymath::math::log2_bigint(i.as_bigint()).map_err(|err| match err {
                pymath::Error::EDOM => vm.new_value_error("expected a positive input".to_owned()),
                _ => pymath_exception(err, vm),
            });
        }
        let val = x.try_float(vm)?.to_f64();
        pymath::math::log2(val).map_err(|err| match err {
            pymath::Error::EDOM => vm.new_value_error(format!(
                "expected a positive input, got {}",
                super::float_repr(val)
            )),
            _ => pymath_exception(err, vm),
        })
    }

    #[pyfunction]
    fn log10(x: PyObjectRef, vm: &VirtualMachine) -> PyResult<f64> {
        // Handle BigInt specially for large values (only for actual int type, not float)
        if let Some(i) = x.downcast_ref::<PyInt>() {
            return pymath::math::log10_bigint(i.as_bigint()).map_err(|err| match err {
                pymath::Error::EDOM => vm.new_value_error("expected a positive input".to_owned()),
                _ => pymath_exception(err, vm),
            });
        }
        let val = x.try_float(vm)?.to_f64();
        pymath::math::log10(val).map_err(|err| match err {
            pymath::Error::EDOM => vm.new_value_error(format!(
                "expected a positive input, got {}",
                super::float_repr(val)
            )),
            _ => pymath_exception(err, vm),
        })
    }

    #[pyfunction]
    fn pow(x: ArgIntoFloat, y: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        pymath::math::pow(x.into_float(), y.into_float()).map_err(|err| pymath_exception(err, vm))
    }

    #[pyfunction]
    fn sqrt(x: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        let val = x.into_float();
        pymath::math::sqrt(val).map_err(|err| match err {
            pymath::Error::EDOM => vm.new_value_error(format!(
                "expected a nonnegative input, got {}",
                super::float_repr(val)
            )),
            _ => pymath_exception(err, vm),
        })
    }

    // Trigonometric functions:
    #[pyfunction]
    fn acos(x: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        let val = x.into_float();
        pymath::math::acos(val).map_err(|err| match err {
            pymath::Error::EDOM => vm.new_value_error(format!(
                "expected a number in range from -1 up to 1, got {}",
                float_repr(val)
            )),
            _ => pymath_exception(err, vm),
        })
    }

    #[pyfunction]
    fn asin(x: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        let val = x.into_float();
        pymath::math::asin(val).map_err(|err| match err {
            pymath::Error::EDOM => vm.new_value_error(format!(
                "expected a number in range from -1 up to 1, got {}",
                float_repr(val)
            )),
            _ => pymath_exception(err, vm),
        })
    }

    #[pyfunction]
    fn atan(x: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        pymath::math::atan(x.into_float()).map_err(|err| pymath_exception(err, vm))
    }

    #[pyfunction]
    fn atan2(y: ArgIntoFloat, x: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        pymath::math::atan2(y.into_float(), x.into_float()).map_err(|err| pymath_exception(err, vm))
    }

    #[pyfunction]
    fn cos(x: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        let val = x.into_float();
        pymath::math::cos(val).map_err(|err| match err {
            pymath::Error::EDOM => {
                vm.new_value_error(format!("expected a finite input, got {}", float_repr(val)))
            }
            _ => pymath_exception(err, vm),
        })
    }

    #[pyfunction]
    fn hypot(coordinates: PosArgs<ArgIntoFloat>) -> f64 {
        let coords = ArgIntoFloat::vec_into_f64(coordinates.into_vec());
        pymath::math::hypot(&coords)
    }

    #[pyfunction]
    fn dist(p: Vec<ArgIntoFloat>, q: Vec<ArgIntoFloat>, vm: &VirtualMachine) -> PyResult<f64> {
        let p = ArgIntoFloat::vec_into_f64(p);
        let q = ArgIntoFloat::vec_into_f64(q);
        if p.len() != q.len() {
            return Err(vm.new_value_error("both points must have the same number of dimensions"));
        }
        Ok(pymath::math::dist(&p, &q))
    }

    #[pyfunction]
    fn sin(x: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        let val = x.into_float();
        pymath::math::sin(val).map_err(|err| match err {
            pymath::Error::EDOM => {
                vm.new_value_error(format!("expected a finite input, got {}", float_repr(val)))
            }
            _ => pymath_exception(err, vm),
        })
    }

    #[pyfunction]
    fn tan(x: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        let val = x.into_float();
        pymath::math::tan(val).map_err(|err| match err {
            pymath::Error::EDOM => {
                vm.new_value_error(format!("expected a finite input, got {}", float_repr(val)))
            }
            _ => pymath_exception(err, vm),
        })
    }

    #[pyfunction]
    fn degrees(x: ArgIntoFloat) -> f64 {
        pymath::math::degrees(x.into_float())
    }

    #[pyfunction]
    fn radians(x: ArgIntoFloat) -> f64 {
        pymath::math::radians(x.into_float())
    }

    // Hyperbolic functions:

    #[pyfunction]
    fn acosh(x: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        pymath::math::acosh(x.into_float()).map_err(|err| pymath_exception(err, vm))
    }

    #[pyfunction]
    fn asinh(x: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        pymath::math::asinh(x.into_float()).map_err(|err| pymath_exception(err, vm))
    }

    #[pyfunction]
    fn atanh(x: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        let val = x.into_float();
        pymath::math::atanh(val).map_err(|err| match err {
            pymath::Error::EDOM => vm.new_value_error(format!(
                "expected a number between -1 and 1, got {}",
                super::float_repr(val)
            )),
            _ => pymath_exception(err, vm),
        })
    }

    #[pyfunction]
    fn cosh(x: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        pymath::math::cosh(x.into_float()).map_err(|err| pymath_exception(err, vm))
    }

    #[pyfunction]
    fn sinh(x: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        pymath::math::sinh(x.into_float()).map_err(|err| pymath_exception(err, vm))
    }

    #[pyfunction]
    fn tanh(x: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        pymath::math::tanh(x.into_float()).map_err(|err| pymath_exception(err, vm))
    }

    // Special functions:
    #[pyfunction]
    fn erf(x: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        pymath::math::erf(x.into_float()).map_err(|err| pymath_exception(err, vm))
    }

    #[pyfunction]
    fn erfc(x: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        pymath::math::erfc(x.into_float()).map_err(|err| pymath_exception(err, vm))
    }

    #[pyfunction]
    fn gamma(x: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        let val = x.into_float();
        pymath::math::gamma(val).map_err(|err| match err {
            pymath::Error::EDOM => vm.new_value_error(format!(
                "expected a noninteger or positive integer, got {}",
                super::float_repr(val)
            )),
            _ => pymath_exception(err, vm),
        })
    }

    #[pyfunction]
    fn lgamma(x: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        pymath::math::lgamma(x.into_float()).map_err(|err| pymath_exception(err, vm))
    }

    fn try_magic_method(
        func_name: &'static PyStrInterned,
        vm: &VirtualMachine,
        value: &PyObject,
    ) -> PyResult {
        let method = vm.get_method_or_type_error(value.to_owned(), func_name, || {
            format!(
                "type '{}' doesn't define '{}' method",
                value.class().name(),
                func_name.as_str(),
            )
        })?;
        method.call((), vm)
    }

    #[pyfunction]
    fn trunc(x: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        try_magic_method(identifier!(vm, __trunc__), vm, &x)
    }

    #[pyfunction]
    fn ceil(x: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        // Only call __ceil__ if the class defines it - if it exists but is not callable,
        // the error should be propagated (not fall back to float conversion)
        if x.class().has_attr(identifier!(vm, __ceil__)) {
            return try_magic_method(identifier!(vm, __ceil__), vm, &x);
        }
        // __ceil__ not defined - fall back to float conversion
        if let Some(v) = x.try_float_opt(vm) {
            let v = try_f64_to_bigint(v?.to_f64().ceil(), vm)?;
            return Ok(vm.ctx.new_int(v).into());
        }
        Err(vm.new_type_error(format!(
            "type '{}' doesn't define '__ceil__' method",
            x.class().name(),
        )))
    }

    #[pyfunction]
    fn floor(x: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        // Only call __floor__ if the class defines it - if it exists but is not callable,
        // the error should be propagated (not fall back to float conversion)
        if x.class().has_attr(identifier!(vm, __floor__)) {
            return try_magic_method(identifier!(vm, __floor__), vm, &x);
        }
        // __floor__ not defined - fall back to float conversion
        if let Some(v) = x.try_float_opt(vm) {
            let v = try_f64_to_bigint(v?.to_f64().floor(), vm)?;
            return Ok(vm.ctx.new_int(v).into());
        }
        Err(vm.new_type_error(format!(
            "type '{}' doesn't define '__floor__' method",
            x.class().name(),
        )))
    }

    #[pyfunction]
    fn frexp(x: ArgIntoFloat) -> (f64, i32) {
        pymath::math::frexp(x.into_float())
    }

    #[pyfunction]
    fn ldexp(
        x: Either<PyRef<PyFloat>, PyIntRef>,
        i: PyIntRef,
        vm: &VirtualMachine,
    ) -> PyResult<f64> {
        let value = match x {
            Either::A(f) => f.to_f64(),
            Either::B(z) => try_bigint_to_f64(z.as_bigint(), vm)?,
        };
        pymath::math::ldexp_bigint(value, i.as_bigint()).map_err(|err| pymath_exception(err, vm))
    }

    #[pyfunction]
    fn cbrt(x: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        pymath::math::cbrt(x.into_float()).map_err(|err| pymath_exception(err, vm))
    }

    #[pyfunction]
    fn fsum(seq: ArgIterable<ArgIntoFloat>, vm: &VirtualMachine) -> PyResult<f64> {
        let values: Result<Vec<f64>, _> =
            seq.iter(vm)?.map(|r| r.map(|v| v.into_float())).collect();
        pymath::math::fsum(values?).map_err(|err| pymath_exception(err, vm))
    }

    #[pyfunction]
    fn modf(x: ArgIntoFloat) -> (f64, f64) {
        pymath::math::modf(x.into_float())
    }

    #[derive(FromArgs)]
    struct NextAfterArgs {
        #[pyarg(positional)]
        x: ArgIntoFloat,
        #[pyarg(positional)]
        y: ArgIntoFloat,
        #[pyarg(named, optional)]
        steps: OptionalArg<ArgIndex>,
    }

    #[pyfunction]
    fn nextafter(arg: NextAfterArgs, vm: &VirtualMachine) -> PyResult<f64> {
        let x = arg.x.into_float();
        let y = arg.y.into_float();

        let steps = match arg.steps.into_option() {
            Some(steps) => {
                let steps: i64 = steps.into_int_ref().try_to_primitive(vm)?;
                if steps < 0 {
                    return Err(vm.new_value_error("steps must be a non-negative integer"));
                }
                Some(steps as u64)
            }
            None => None,
        };
        Ok(pymath::math::nextafter(x, y, steps))
    }

    #[pyfunction]
    fn ulp(x: ArgIntoFloat) -> f64 {
        pymath::math::ulp(x.into_float())
    }

    #[pyfunction(name = "fmod")]
    fn py_fmod(x: ArgIntoFloat, y: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        pymath::math::fmod(x.into_float(), y.into_float()).map_err(|err| pymath_exception(err, vm))
    }

    #[pyfunction]
    fn remainder(x: ArgIntoFloat, y: ArgIntoFloat, vm: &VirtualMachine) -> PyResult<f64> {
        pymath::math::remainder(x.into_float(), y.into_float())
            .map_err(|err| pymath_exception(err, vm))
    }

    #[derive(FromArgs)]
    struct ProdArgs {
        #[pyarg(positional)]
        iterable: ArgIterable<PyObjectRef>,
        #[pyarg(named, optional)]
        start: OptionalArg<PyObjectRef>,
    }

    #[pyfunction]
    fn prod(args: ProdArgs, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        use crate::vm::builtins::PyInt;

        let iter = args.iterable;
        let start = args.start;

        // Check if start is provided and what type it is (exact types only, not subclasses)
        let (mut obj_result, start_is_int, start_is_float) = match &start {
            OptionalArg::Present(s) => {
                let is_int = s.class().is(vm.ctx.types.int_type);
                let is_float = s.class().is(vm.ctx.types.float_type);
                (Some(s.clone()), is_int, is_float)
            }
            OptionalArg::Missing => (None, true, false), // Default is int 1
        };

        let mut item_iter = iter.iter(vm)?;

        // Integer fast path
        if start_is_int && !start_is_float {
            let mut int_result: i64 = match &start {
                OptionalArg::Present(s) => {
                    if let Some(i) = s.downcast_ref::<PyInt>() {
                        match i.as_bigint().try_into() {
                            Ok(v) => v,
                            Err(_) => {
                                // Start overflows i64, fall through to generic path
                                obj_result = Some(s.clone());
                                i64::MAX // Will be ignored
                            }
                        }
                    } else {
                        1
                    }
                }
                OptionalArg::Missing => 1,
            };

            if obj_result.is_none() {
                loop {
                    let item = match item_iter.next() {
                        Some(r) => r?,
                        None => return Ok(vm.ctx.new_int(int_result).into()),
                    };

                    // Only use fast path for exact int type (not subclasses)
                    if item.class().is(vm.ctx.types.int_type)
                        && let Some(int_item) = item.downcast_ref::<PyInt>()
                        && let Ok(b) = int_item.as_bigint().try_into() as Result<i64, _>
                        && let Some(product) = int_result.checked_mul(b)
                    {
                        int_result = product;
                        continue;
                    }

                    // Overflow or non-int: restore to PyObject and continue
                    obj_result = Some(vm.ctx.new_int(int_result).into());
                    let temp = vm._mul(obj_result.as_ref().unwrap(), &item)?;
                    obj_result = Some(temp);
                    break;
                }
            }
        }

        // Float fast path
        let obj_float = obj_result
            .as_ref()
            .and_then(|obj| obj.clone().downcast::<PyFloat>().ok());
        if obj_float.is_some() || start_is_float {
            let mut flt_result: f64 = if let Some(ref f) = obj_float {
                f.to_f64()
            } else if start_is_float && let OptionalArg::Present(s) = &start {
                s.downcast_ref::<PyFloat>()
                    .map(|f| f.to_f64())
                    .unwrap_or(1.0)
            } else {
                1.0
            };

            loop {
                let item = match item_iter.next() {
                    Some(r) => r?,
                    None => return Ok(vm.ctx.new_float(flt_result).into()),
                };

                // Only use fast path for exact float/int types (not subclasses)
                if item.class().is(vm.ctx.types.float_type)
                    && let Some(f) = item.downcast_ref::<PyFloat>()
                {
                    flt_result *= f.to_f64();
                    continue;
                }
                if item.class().is(vm.ctx.types.int_type)
                    && let Some(i) = item.downcast_ref::<PyInt>()
                    && let Ok(v) = i.as_bigint().try_into() as Result<i64, _>
                {
                    flt_result *= v as f64;
                    continue;
                }

                // Non-exact-float/int: restore and continue with generic path
                obj_result = Some(vm.ctx.new_float(flt_result).into());
                let temp = vm._mul(obj_result.as_ref().unwrap(), &item)?;
                obj_result = Some(temp);
                break;
            }
        }

        // Generic path for remaining items
        let mut result = obj_result.unwrap_or_else(|| vm.ctx.new_int(1).into());
        for item in item_iter {
            let item = item?;
            result = vm._mul(&result, &item)?;
        }

        Ok(result)
    }

    #[pyfunction]
    fn sumprod(
        p: ArgIterable<PyObjectRef>,
        q: ArgIterable<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        use crate::vm::builtins::PyInt;

        let mut p_iter = p.iter(vm)?;
        let mut q_iter = q.iter(vm)?;

        // Fast path state
        let mut int_path_enabled = true;
        let mut int_total: i64 = 0;
        let mut int_total_in_use = false;
        let mut flt_p_values: Vec<f64> = Vec::new();
        let mut flt_q_values: Vec<f64> = Vec::new();

        // Fallback accumulator for generic Python path
        let mut obj_total: Option<PyObjectRef> = None;

        loop {
            let m_p = p_iter.next();
            let m_q = q_iter.next();

            let (p_i, q_i, finished) = match (m_p, m_q) {
                (Some(r_p), Some(r_q)) => (Some(r_p?), Some(r_q?), false),
                (None, None) => (None, None, true),
                _ => return Err(vm.new_value_error("Inputs are not the same length")),
            };

            // Integer fast path (only for exact int types, not subclasses)
            if int_path_enabled {
                if !finished {
                    let (p_i, q_i) = (p_i.as_ref().unwrap(), q_i.as_ref().unwrap());
                    if p_i.class().is(vm.ctx.types.int_type)
                        && q_i.class().is(vm.ctx.types.int_type)
                        && let (Some(p_int), Some(q_int)) =
                            (p_i.downcast_ref::<PyInt>(), q_i.downcast_ref::<PyInt>())
                        && let (Ok(p_val), Ok(q_val)) = (
                            p_int.as_bigint().try_into() as Result<i64, _>,
                            q_int.as_bigint().try_into() as Result<i64, _>,
                        )
                        && let Some(prod) = p_val.checked_mul(q_val)
                        && let Some(new_total) = int_total.checked_add(prod)
                    {
                        int_total = new_total;
                        int_total_in_use = true;
                        continue;
                    }
                }
                // Finalize int path
                int_path_enabled = false;
                if int_total_in_use {
                    let int_obj: PyObjectRef = vm.ctx.new_int(int_total).into();
                    obj_total = Some(match obj_total {
                        Some(total) => vm._add(&total, &int_obj)?,
                        None => int_obj,
                    });
                    int_total = 0;
                    int_total_in_use = false;
                }
            }

            // Float fast path - only when at least one value is exact float type
            // (not subclasses, to preserve custom __mul__/__add__ behavior)
            {
                if !finished {
                    let (p_i, q_i) = (p_i.as_ref().unwrap(), q_i.as_ref().unwrap());

                    let p_is_exact_float = p_i.class().is(vm.ctx.types.float_type);
                    let q_is_exact_float = q_i.class().is(vm.ctx.types.float_type);
                    let p_is_exact_int = p_i.class().is(vm.ctx.types.int_type);
                    let q_is_exact_int = q_i.class().is(vm.ctx.types.int_type);
                    let p_is_exact_numeric = p_is_exact_float || p_is_exact_int;
                    let q_is_exact_numeric = q_is_exact_float || q_is_exact_int;
                    let has_exact_float = p_is_exact_float || q_is_exact_float;

                    // Only use float path if at least one is exact float and both are exact int/float
                    if has_exact_float && p_is_exact_numeric && q_is_exact_numeric {
                        let p_flt = if let Some(f) = p_i.downcast_ref::<PyFloat>() {
                            Some(f.to_f64())
                        } else if let Some(i) = p_i.downcast_ref::<PyInt>() {
                            // PyLong_AsDouble fails for integers too large for f64
                            try_bigint_to_f64(i.as_bigint(), vm).ok()
                        } else {
                            None
                        };

                        let q_flt = if let Some(f) = q_i.downcast_ref::<PyFloat>() {
                            Some(f.to_f64())
                        } else if let Some(i) = q_i.downcast_ref::<PyInt>() {
                            // PyLong_AsDouble fails for integers too large for f64
                            try_bigint_to_f64(i.as_bigint(), vm).ok()
                        } else {
                            None
                        };

                        if let (Some(p_val), Some(q_val)) = (p_flt, q_flt) {
                            flt_p_values.push(p_val);
                            flt_q_values.push(q_val);
                            continue;
                        }
                    }
                }
                // Finalize float path
                if !flt_p_values.is_empty() {
                    let flt_result = pymath::math::sumprod(&flt_p_values, &flt_q_values);
                    let flt_obj: PyObjectRef = vm.ctx.new_float(flt_result).into();
                    obj_total = Some(match obj_total {
                        Some(total) => vm._add(&total, &flt_obj)?,
                        None => flt_obj,
                    });
                    flt_p_values.clear();
                    flt_q_values.clear();
                }
            }

            if finished {
                break;
            }

            // Generic Python path
            let (p_i, q_i) = (p_i.unwrap(), q_i.unwrap());

            // Collect current + remaining elements
            let p_remaining: Result<Vec<PyObjectRef>, _> =
                std::iter::once(Ok(p_i)).chain(p_iter).collect();
            let q_remaining: Result<Vec<PyObjectRef>, _> =
                std::iter::once(Ok(q_i)).chain(q_iter).collect();
            let (p_vec, q_vec) = (p_remaining?, q_remaining?);

            if p_vec.len() != q_vec.len() {
                return Err(vm.new_value_error("Inputs are not the same length"));
            }

            let mut total = obj_total.unwrap_or_else(|| vm.ctx.new_int(0).into());
            for (p_item, q_item) in p_vec.into_iter().zip(q_vec) {
                let prod = vm._mul(&p_item, &q_item)?;
                total = vm._add(&total, &prod)?;
            }
            return Ok(total);
        }

        Ok(obj_total.unwrap_or_else(|| vm.ctx.new_int(0).into()))
    }

    #[pyfunction]
    fn fma(
        x: ArgIntoFloat,
        y: ArgIntoFloat,
        z: ArgIntoFloat,
        vm: &VirtualMachine,
    ) -> PyResult<f64> {
        pymath::math::fma(x.into_float(), y.into_float(), z.into_float()).map_err(|err| match err {
            pymath::Error::EDOM => vm.new_value_error("invalid operation in fma"),
            pymath::Error::ERANGE => vm.new_overflow_error("overflow in fma"),
        })
    }

    // Integer functions:

    #[pyfunction]
    fn isqrt(x: ArgIndex, vm: &VirtualMachine) -> PyResult<BigInt> {
        let value = x.into_int_ref();
        pymath::math::integer::isqrt(value.as_bigint())
            .map_err(|_| vm.new_value_error("isqrt() argument must be nonnegative"))
    }

    #[pyfunction]
    fn gcd(args: PosArgs<ArgIndex>) -> BigInt {
        let ints: Vec<_> = args
            .into_vec()
            .into_iter()
            .map(|x| x.into_int_ref())
            .collect();
        let refs: Vec<_> = ints.iter().map(|x| x.as_bigint()).collect();
        pymath::math::integer::gcd(&refs)
    }

    #[pyfunction]
    fn lcm(args: PosArgs<ArgIndex>) -> BigInt {
        let ints: Vec<_> = args
            .into_vec()
            .into_iter()
            .map(|x| x.into_int_ref())
            .collect();
        let refs: Vec<_> = ints.iter().map(|x| x.as_bigint()).collect();
        pymath::math::integer::lcm(&refs)
    }

    #[pyfunction]
    fn factorial(x: PyIntRef, vm: &VirtualMachine) -> PyResult<BigInt> {
        // Check for negative before overflow - negative values are always invalid
        if x.as_bigint().is_negative() {
            return Err(vm.new_value_error("factorial() not defined for negative values"));
        }
        let n: i64 = x.try_to_primitive(vm).map_err(|_| {
            vm.new_overflow_error("factorial() argument should not exceed 9223372036854775807")
        })?;
        pymath::math::integer::factorial(n)
            .map(|r| r.into())
            .map_err(|_| vm.new_value_error("factorial() not defined for negative values"))
    }

    #[pyfunction]
    fn perm(
        n: ArgIndex,
        k: OptionalArg<Option<ArgIndex>>,
        vm: &VirtualMachine,
    ) -> PyResult<BigInt> {
        let n_int = n.into_int_ref();
        let n_big = n_int.as_bigint();

        if n_big.is_negative() {
            return Err(vm.new_value_error("n must be a non-negative integer"));
        }

        // k = None means k = n (factorial)
        let k_int = k.flatten().map(|k| k.into_int_ref());
        let k_big: Option<&BigInt> = k_int.as_ref().map(|k| k.as_bigint());

        if let Some(k_val) = k_big {
            if k_val.is_negative() {
                return Err(vm.new_value_error("k must be a non-negative integer"));
            }
            if k_val > n_big {
                return Ok(BigInt::from(0u8));
            }
        }

        // Convert k to u64 (required by pymath)
        let ki: u64 = match k_big {
            None => match n_big.to_u64() {
                Some(n) => n,
                None => {
                    return Err(vm.new_overflow_error(format!("n must not exceed {}", u64::MAX)));
                }
            },
            Some(k_val) => match k_val.to_u64() {
                Some(k) => k,
                None => {
                    return Err(vm.new_overflow_error(format!("k must not exceed {}", u64::MAX)));
                }
            },
        };

        // Fast path: n fits in i64
        if let Some(ni) = n_big.to_i64()
            && ni >= 0
            && ki > 1
        {
            let result = pymath::math::integer::perm(ni, Some(ki as i64))
                .map_err(|_| vm.new_value_error("perm() error"))?;
            return Ok(result.into());
        }

        // BigInt path: use perm_bigint
        let result = pymath::math::perm_bigint(n_big, ki);
        Ok(result.into())
    }

    #[pyfunction]
    fn comb(n: ArgIndex, k: ArgIndex, vm: &VirtualMachine) -> PyResult<BigInt> {
        let n_int = n.into_int_ref();
        let n_big = n_int.as_bigint();
        let k_int = k.into_int_ref();
        let k_big = k_int.as_bigint();

        if n_big.is_negative() {
            return Err(vm.new_value_error("n must be a non-negative integer"));
        }
        if k_big.is_negative() {
            return Err(vm.new_value_error("k must be a non-negative integer"));
        }

        // Fast path: n fits in i64
        if let Some(ni) = n_big.to_i64()
            && ni >= 0
        {
            // k overflow or k > n means result is 0
            let ki = match k_big.to_i64() {
                Some(k) if k >= 0 && k <= ni => k,
                _ => return Ok(BigInt::from(0u8)),
            };
            // Apply symmetry: use min(k, n-k)
            let ki = ki.min(ni - ki);
            if ki > 1 {
                let result = pymath::math::integer::comb(ni, ki)
                    .map_err(|_| vm.new_value_error("comb() error"))?;
                return Ok(result.into());
            }
            // ki <= 1 cases
            if ki == 0 {
                return Ok(BigInt::from(1u8));
            }
            return Ok(n_big.clone()); // ki == 1
        }

        // BigInt path: n doesn't fit in i64
        // Apply symmetry: k = min(k, n - k)
        let n_minus_k = n_big - k_big;
        if n_minus_k.is_negative() {
            return Ok(BigInt::from(0u8));
        }
        let effective_k = if &n_minus_k < k_big {
            &n_minus_k
        } else {
            k_big
        };

        // k must fit in u64
        let ki: u64 = match effective_k.to_u64() {
            Some(k) => k,
            None => {
                return Err(
                    vm.new_overflow_error(format!("min(n - k, k) must not exceed {}", u64::MAX))
                );
            }
        };

        let result = pymath::math::comb_bigint(n_big, ki);
        Ok(result.into())
    }
}

pub(crate) fn pymath_exception(err: pymath::Error, vm: &VirtualMachine) -> PyBaseExceptionRef {
    match err {
        pymath::Error::EDOM => vm.new_value_error("math domain error"),
        pymath::Error::ERANGE => vm.new_overflow_error("math range error"),
    }
}

/// Format a float in Python style (ensures trailing .0 for integers).
fn float_repr(value: f64) -> String {
    if value.is_nan() {
        "nan".to_owned()
    } else if value.is_infinite() {
        if value.is_sign_positive() {
            "inf".to_owned()
        } else {
            "-inf".to_owned()
        }
    } else {
        let s = format!("{}", value);
        // If no decimal point and not in scientific notation, add .0
        if !s.contains('.') && !s.contains('e') && !s.contains('E') {
            format!("{}.0", s)
        } else {
            s
        }
    }
}

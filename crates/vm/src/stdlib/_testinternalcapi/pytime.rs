//! Python/pytime.c conversions and the Modules/_testinternalcapi/pytime.c
//! functions that test them.

use crate::{
    Py, PyObject, PyResult, VirtualMachine,
    builtins::{PyBaseExceptionRef, PyInt},
};
use num_traits::ToPrimitive;

pub(crate) type PyTime = i64;

#[cfg(any(unix, windows))]
type TimeT = libc::time_t;
#[cfg(not(any(unix, windows)))]
type TimeT = i64;

// On Windows, timeval.tv_sec type is long.
#[cfg(windows)]
type TimevalSec = core::ffi::c_long;
#[cfg(not(windows))]
type TimevalSec = TimeT;

const SEC_TO_MS: PyTime = 1000;
const MS_TO_US: PyTime = 1000;
const SEC_TO_US: PyTime = SEC_TO_MS * MS_TO_US;
const US_TO_NS: PyTime = 1000;
const MS_TO_NS: PyTime = MS_TO_US * US_TO_NS;
const SEC_TO_NS: PyTime = SEC_TO_MS * MS_TO_NS;
const NS_TO_MS: PyTime = 1000 * 1000;
const NS_TO_US: PyTime = 1000;

/// _PyTime_round_t
#[derive(Clone, Copy, PartialEq, Eq)]
enum Round {
    Floor,
    Ceiling,
    HalfEven,
    Up,
}

/// check_time_rounding
fn check_time_rounding(round: i32, vm: &VirtualMachine) -> PyResult<Round> {
    match round {
        0 => Ok(Round::Floor),
        1 => Ok(Round::Ceiling),
        2 => Ok(Round::HalfEven),
        3 => Ok(Round::Up),
        _ => Err(vm.new_value_error("invalid rounding")),
    }
}

fn pytime_time_t_overflow(vm: &VirtualMachine) -> PyBaseExceptionRef {
    vm.new_overflow_error("timestamp out of range for platform time_t")
}

fn pytime_overflow(vm: &VirtualMachine) -> PyBaseExceptionRef {
    vm.new_overflow_error("timestamp out of range for C PyTime_t")
}

/// Compute t * k. Clamp to [PyTime_MIN; PyTime_MAX] on overflow.
fn pytime_mul(t: &mut PyTime, k: PyTime) -> bool {
    debug_assert!(k >= 0);
    match t.checked_mul(k) {
        Some(value) => {
            *t = value;
            true
        }
        None => {
            *t = if *t >= 0 { PyTime::MAX } else { PyTime::MIN };
            false
        }
    }
}

/// _PyLong_AsTime_t
fn pylong_as_time_t(int: &Py<PyInt>, vm: &VirtualMachine) -> PyResult<TimeT> {
    int.as_bigint()
        .to_i64()
        .and_then(|value| TimeT::try_from(value).ok())
        .ok_or_else(|| pytime_time_t_overflow(vm))
}

/// _PyTime_AsTime_t: clamp the value on overflow.
fn pytime_as_time_t<T: TryFrom<PyTime> + num_traits::Bounded>(t: PyTime) -> (T, bool) {
    match T::try_from(t) {
        Ok(value) => (value, true),
        Err(_) if t > 0 => (T::max_value(), false),
        Err(_) => (T::min_value(), false),
    }
}

/// Round to nearest with ties going to nearest even integer
/// (_PyTime_ROUND_HALF_EVEN)
fn pytime_round_half_even(x: f64) -> f64 {
    let rounded = x.round();
    if (x - rounded).abs() == 0.5 {
        // halfway case: round to even
        2.0 * (x / 2.0).round()
    } else {
        rounded
    }
}

fn pytime_round(x: f64, round: Round) -> f64 {
    match round {
        Round::HalfEven => pytime_round_half_even(x),
        Round::Ceiling => x.ceil(),
        Round::Floor => x.floor(),
        Round::Up if x >= 0.0 => x.ceil(),
        Round::Up => x.floor(),
    }
}

/// modf
fn modf(x: f64) -> (f64, f64) {
    let intpart = x.trunc();
    let floatpart = if x.is_infinite() {
        0.0_f64.copysign(x)
    } else {
        x - intpart
    };
    (floatpart, intpart)
}

/// Whether `intpart` is in the time_t range. The condition is true for NaN.
fn time_t_out_of_range(intpart: f64) -> bool {
    !((TimeT::MIN as f64) <= intpart && intpart < -(TimeT::MIN as f64))
}

fn pytime_double_to_denominator(
    d: f64,
    idenominator: i64,
    round: Round,
    vm: &VirtualMachine,
) -> PyResult<(TimeT, i64)> {
    let denominator = idenominator as f64;
    let (floatpart, mut intpart) = modf(d);
    let mut floatpart = pytime_round(floatpart * denominator, round);
    if floatpart >= denominator {
        floatpart -= denominator;
        intpart += 1.0;
    } else if floatpart < 0.0 {
        floatpart += denominator;
        intpart -= 1.0;
    }
    debug_assert!((0.0..denominator).contains(&floatpart));

    if time_t_out_of_range(intpart) {
        return Err(pytime_time_t_overflow(vm));
    }
    Ok((intpart as TimeT, floatpart as i64))
}

/// PyFloat_AsDouble, rejecting NaN.
fn pyfloat_as_double_not_nan(obj: &PyObject, vm: &VirtualMachine) -> PyResult<f64> {
    let d = obj.try_float(vm)?.to_f64();
    if d.is_nan() {
        return Err(vm.new_value_error("Invalid value NaN (not a number)"));
    }
    Ok(d)
}

fn pytime_object_to_denominator(
    obj: &PyObject,
    denominator: i64,
    round: Round,
    vm: &VirtualMachine,
) -> PyResult<(TimeT, i64)> {
    debug_assert!(denominator >= 1);
    if let Some(int) = obj.try_index_opt(vm) {
        let int = int?;
        return Ok((pylong_as_time_t(&int, vm)?, 0));
    }
    let d = pyfloat_as_double_not_nan(obj, vm)?;
    pytime_double_to_denominator(d, denominator, round, vm)
}

/// _PyTime_ObjectToTime_t
fn pytime_object_to_time_t(obj: &PyObject, round: Round, vm: &VirtualMachine) -> PyResult<TimeT> {
    if let Some(int) = obj.try_index_opt(vm) {
        let int = int?;
        return pylong_as_time_t(&int, vm);
    }
    let d = pytime_round(pyfloat_as_double_not_nan(obj, vm)?, round);
    let (_, intpart) = modf(d);
    if time_t_out_of_range(intpart) {
        return Err(pytime_time_t_overflow(vm));
    }
    Ok(intpart as TimeT)
}

fn pytime_from_double(
    value: f64,
    round: Round,
    unit_to_ns: PyTime,
    vm: &VirtualMachine,
) -> PyResult<PyTime> {
    // convert to a number of nanoseconds
    let d = pytime_round(value * unit_to_ns as f64, round);
    if !((PyTime::MIN as f64) <= d && d < -(PyTime::MIN as f64)) {
        return Err(pytime_overflow(vm));
    }
    Ok(d as PyTime)
}

fn pytime_from_object(
    obj: &PyObject,
    round: Round,
    unit_to_ns: PyTime,
    vm: &VirtualMachine,
) -> PyResult<PyTime> {
    if let Some(int) = obj.try_index_opt(vm) {
        let mut ns = int?
            .as_bigint()
            .to_i64()
            .ok_or_else(|| pytime_overflow(vm))?;
        if !pytime_mul(&mut ns, unit_to_ns) {
            return Err(pytime_overflow(vm));
        }
        return Ok(ns);
    }
    let d = pyfloat_as_double_not_nan(obj, vm)?;
    pytime_from_double(d, round, unit_to_ns, vm)
}

/// PyTime_AsSecondsDouble
pub(crate) fn pytime_as_seconds_double(ns: PyTime) -> f64 {
    if ns % SEC_TO_NS == 0 {
        // Divide using integers to avoid rounding issues on the integer part.
        // 1e-9 cannot be stored exactly in IEEE 64-bit.
        (ns / SEC_TO_NS) as f64
    } else {
        ns as f64 / 1e9
    }
}

fn pytime_divide_round_up(t: PyTime, k: PyTime) -> PyTime {
    debug_assert!(k > 1);
    let q = t / k;
    match (t % k != 0, t >= 0) {
        (false, _) => q,
        (true, true) => q + 1,
        (true, false) => q - 1,
    }
}

fn pytime_divide(t: PyTime, k: PyTime, round: Round) -> PyTime {
    debug_assert!(k > 1);
    match round {
        Round::HalfEven => {
            let x = t / k;
            let abs_r = (t % k).abs();
            if abs_r > k / 2 || (abs_r == k / 2 && x.abs() & 1 != 0) {
                if t >= 0 { x + 1 } else { x - 1 }
            } else {
                x
            }
        }
        Round::Ceiling if t >= 0 => pytime_divide_round_up(t, k),
        Round::Ceiling => t / k,
        Round::Floor if t >= 0 => t / k,
        Round::Floor | Round::Up => pytime_divide_round_up(t, k),
    }
}

/// Compute (t / k, t % k), making sure that 0 <= r < k. On underflow,
/// return (PyTime_MIN, 0) and false.
fn pytime_divmod(t: PyTime, k: PyTime) -> (PyTime, PyTime, bool) {
    debug_assert!(k > 1);
    let q = t / k;
    let r = t % k;
    if r >= 0 {
        (q, r, true)
    } else if q == PyTime::MIN {
        (PyTime::MIN, 0, false)
    } else {
        (q - 1, r + k, true)
    }
}

fn pytime_as_timeval_struct(
    t: PyTime,
    round: Round,
    raise_exc: bool,
    vm: &VirtualMachine,
) -> PyResult<(TimevalSec, PyTime)> {
    let us = pytime_divide(t, US_TO_NS, round);
    let (tv_sec, tv_usec, res) = pytime_divmod(us, SEC_TO_US);
    let (tv_sec, res2) = pytime_as_time_t::<TimevalSec>(tv_sec);
    let tv_usec = if res2 { tv_usec } else { 0 };
    if raise_exc && !(res && res2) {
        return Err(pytime_time_t_overflow(vm));
    }
    Ok((tv_sec, tv_usec))
}

fn pytime_as_timespec(
    ns: PyTime,
    raise_exc: bool,
    vm: &VirtualMachine,
) -> PyResult<(TimeT, PyTime)> {
    let (tv_sec, tv_nsec, res) = pytime_divmod(ns, SEC_TO_NS);
    let (tv_sec, res2) = pytime_as_time_t::<TimeT>(tv_sec);
    let tv_nsec = if res2 { tv_nsec } else { 0 };
    if raise_exc && !(res && res2) {
        return Err(pytime_time_t_overflow(vm));
    }
    Ok((tv_sec, tv_nsec))
}

/// PyLong_AsInt64
pub(crate) fn pylong_as_int64(obj: &PyObject, vm: &VirtualMachine) -> PyResult<PyTime> {
    obj.try_index(vm)?.try_to_primitive::<i64>(vm)
}

#[pymodule(sub, name = "_testinternalcapi")]
pub(super) mod pytime {
    use super::*;
    use crate::PyObjectRef;

    #[pyfunction(name = "_PyTime_FromSeconds")]
    fn test_pytime_fromseconds(seconds: i32) -> PyTime {
        PyTime::from(seconds) * SEC_TO_NS
    }

    #[pyfunction(name = "_PyTime_FromSecondsObject")]
    fn test_pytime_fromsecondsobject(
        obj: PyObjectRef,
        round: i32,
        vm: &VirtualMachine,
    ) -> PyResult<PyTime> {
        let round = check_time_rounding(round, vm)?;
        pytime_from_object(&obj, round, SEC_TO_NS, vm)
    }

    #[pyfunction(name = "_PyTime_AsTimeval")]
    fn test_pytime_as_timeval(
        obj: PyObjectRef,
        round: i32,
        vm: &VirtualMachine,
    ) -> PyResult<(TimevalSec, PyTime)> {
        let round = check_time_rounding(round, vm)?;
        let t = pylong_as_int64(&obj, vm)?;
        pytime_as_timeval_struct(t, round, true, vm)
    }

    #[pyfunction(name = "_PyTime_AsTimeval_clamp")]
    fn test_pytime_as_timeval_clamp(
        obj: PyObjectRef,
        round: i32,
        vm: &VirtualMachine,
    ) -> PyResult<(TimevalSec, PyTime)> {
        let round = check_time_rounding(round, vm)?;
        let t = pylong_as_int64(&obj, vm)?;
        pytime_as_timeval_struct(t, round, false, vm)
    }

    #[pyfunction(name = "_PyTime_AsTimespec")]
    fn test_pytime_as_timespec(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<(TimeT, PyTime)> {
        let t = pylong_as_int64(&obj, vm)?;
        pytime_as_timespec(t, true, vm)
    }

    #[pyfunction(name = "_PyTime_AsTimespec_clamp")]
    fn test_pytime_as_timespec_clamp(
        obj: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<(TimeT, PyTime)> {
        let t = pylong_as_int64(&obj, vm)?;
        pytime_as_timespec(t, false, vm)
    }

    #[pyfunction(name = "_PyTime_AsMilliseconds")]
    fn test_pytime_as_milliseconds(
        obj: PyObjectRef,
        round: i32,
        vm: &VirtualMachine,
    ) -> PyResult<PyTime> {
        let t = pylong_as_int64(&obj, vm)?;
        let round = check_time_rounding(round, vm)?;
        Ok(pytime_divide(t, NS_TO_MS, round))
    }

    #[pyfunction(name = "_PyTime_AsMicroseconds")]
    fn test_pytime_as_microseconds(
        obj: PyObjectRef,
        round: i32,
        vm: &VirtualMachine,
    ) -> PyResult<PyTime> {
        let t = pylong_as_int64(&obj, vm)?;
        let round = check_time_rounding(round, vm)?;
        Ok(pytime_divide(t, NS_TO_US, round))
    }

    #[pyfunction(name = "_PyTime_ObjectToTime_t")]
    fn test_pytime_object_to_time_t(
        obj: PyObjectRef,
        round: i32,
        vm: &VirtualMachine,
    ) -> PyResult<TimeT> {
        let round = check_time_rounding(round, vm)?;
        pytime_object_to_time_t(&obj, round, vm)
    }

    #[pyfunction(name = "_PyTime_ObjectToTimeval")]
    fn test_pytime_object_to_timeval(
        obj: PyObjectRef,
        round: i32,
        vm: &VirtualMachine,
    ) -> PyResult<(TimeT, i64)> {
        let round = check_time_rounding(round, vm)?;
        pytime_object_to_denominator(&obj, SEC_TO_US, round, vm)
    }

    #[pyfunction(name = "_PyTime_ObjectToTimespec")]
    fn test_pytime_object_to_timespec(
        obj: PyObjectRef,
        round: i32,
        vm: &VirtualMachine,
    ) -> PyResult<(TimeT, i64)> {
        let round = check_time_rounding(round, vm)?;
        pytime_object_to_denominator(&obj, SEC_TO_NS, round, vm)
    }
}

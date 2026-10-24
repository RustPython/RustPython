//! Signed nanosecond timestamps and the rounding used by Python's time APIs.

#![cfg_attr(target_env = "musl", allow(deprecated))]

use crate::{AsObject, PyObject, PyResult, VirtualMachine, builtins::PyFloat};
use num_traits::ToPrimitive;
use rustpython_host_env::time::{SEC_TO_NS, SEC_TO_US, US_TO_NS};

#[cfg(any(unix, windows))]
pub(crate) use rustpython_host_env::time::TimeT;
#[cfg(not(any(unix, windows)))]
pub(crate) type TimeT = i64;

const TIME_T_MIN: i64 = TimeT::MIN as _;
const TIME_T_MAX: i64 = TimeT::MAX as _;
const PYTIME_OVERFLOW: &str = "timestamp out of range for C PyTime_t";
const TIME_T_OVERFLOW: &str = "timestamp out of range for platform time_t";

#[derive(Clone, Copy)]
pub(crate) enum Round {
    Floor,
    Ceiling,
    HalfEven,
    Up,
}

impl Round {
    pub(crate) fn from_int(value: i32, vm: &VirtualMachine) -> PyResult<Self> {
        match value {
            0 => Ok(Self::Floor),
            1 => Ok(Self::Ceiling),
            2 => Ok(Self::HalfEven),
            3 => Ok(Self::Up),
            _ => Err(vm.new_value_error("invalid rounding")),
        }
    }

    fn round(self, value: f64) -> f64 {
        match self {
            Self::Floor => value.floor(),
            Self::Ceiling => value.ceil(),
            Self::HalfEven => value.round_ties_even(),
            Self::Up if value >= 0.0 => value.ceil(),
            Self::Up => value.floor(),
        }
    }

    pub(crate) fn divide(self, value: i64, divisor: i64) -> i64 {
        debug_assert!(divisor > 1);
        let quotient = value / divisor;
        let remainder = value % divisor;
        let round_away = match self {
            Self::Floor => remainder < 0,
            Self::Ceiling => remainder > 0,
            Self::HalfEven => {
                let remainder = remainder.abs();
                remainder > divisor / 2
                    || (divisor % 2 == 0 && remainder == divisor / 2 && quotient % 2 != 0)
            }
            Self::Up => remainder != 0,
        };
        // Adjust the quotient, not the input: adding to i64::MAX or
        // subtracting from i64::MIN before dividing would overflow.
        quotient + if round_away { value.signum() } else { 0 }
    }
}

pub(crate) fn from_seconds(seconds: i32) -> i64 {
    i64::from(seconds) * SEC_TO_NS
}

fn index_seconds(object: &PyObject, message: &str, vm: &VirtualMachine) -> PyResult<i64> {
    let seconds = object.try_index(vm).map_err(|err| {
        if err.fast_isinstance(vm.ctx.exceptions.overflow_error) {
            vm.new_overflow_error(message)
        } else {
            err
        }
    })?;
    seconds
        .as_bigint()
        .to_i64()
        .ok_or_else(|| vm.new_overflow_error(message))
}

fn float_seconds(object: &PyObject, vm: &VirtualMachine) -> PyResult<f64> {
    // PyFloat_AsDouble bypasses __float__ overrides on float subclasses.
    let seconds = match object.downcast_ref::<PyFloat>() {
        Some(float) => float.to_f64(),
        None => object.try_float(vm)?.to_f64(),
    };
    if seconds.is_nan() {
        return Err(vm.new_value_error("Invalid value NaN (not a number)"));
    }
    Ok(seconds)
}

pub(crate) fn from_seconds_object(
    object: &PyObject,
    round: Round,
    vm: &VirtualMachine,
) -> PyResult<i64> {
    // _PyTime_FromSecondsObject uses __index__ before __float__ and bounds
    // the converted value to signed 64-bit nanoseconds.
    if object.number().is_index() {
        index_seconds(object, PYTIME_OVERFLOW, vm)?
            .checked_mul(SEC_TO_NS)
            .ok_or_else(|| vm.new_overflow_error(PYTIME_OVERFLOW))
    } else {
        let nanoseconds = round.round(float_seconds(object, vm)? * SEC_TO_NS as f64);
        let minimum = i64::MIN as f64;
        // The maximum rounds up to 2**63 as a double, so use an exclusive bound.
        if !(minimum..-minimum).contains(&nanoseconds) {
            return Err(vm.new_overflow_error(PYTIME_OVERFLOW));
        }
        Ok(nanoseconds as i64)
    }
}

#[cfg(all(feature = "capi", feature = "host_env"))]
pub(crate) fn as_seconds_double(nanoseconds: i64) -> f64 {
    if nanoseconds % SEC_TO_NS == 0 {
        (nanoseconds / SEC_TO_NS) as f64
    } else {
        nanoseconds as f64 / SEC_TO_NS as f64
    }
}

fn check_time_t(seconds: i64, vm: &VirtualMachine) -> PyResult<i64> {
    if !(TIME_T_MIN..=TIME_T_MAX).contains(&seconds) {
        return Err(vm.new_overflow_error(TIME_T_OVERFLOW));
    }
    Ok(seconds)
}

fn float_to_time_t(seconds: f64, vm: &VirtualMachine) -> PyResult<i64> {
    let minimum = TIME_T_MIN as f64;
    if !(minimum..-minimum).contains(&seconds) {
        return Err(vm.new_overflow_error(TIME_T_OVERFLOW));
    }
    Ok(seconds as i64)
}

pub(crate) fn object_to_time_t(
    object: &PyObject,
    round: Round,
    vm: &VirtualMachine,
) -> PyResult<i64> {
    if object.number().is_index() {
        check_time_t(index_seconds(object, TIME_T_OVERFLOW, vm)?, vm)
    } else {
        float_to_time_t(round.round(float_seconds(object, vm)?), vm)
    }
}

pub(crate) fn object_to_denominator(
    object: &PyObject,
    denominator: i64,
    round: Round,
    vm: &VirtualMachine,
) -> PyResult<(i64, i64)> {
    if object.number().is_index() {
        return Ok((
            check_time_t(index_seconds(object, TIME_T_OVERFLOW, vm)?, vm)?,
            0,
        ));
    }
    let value = float_seconds(object, vm)?;
    let mut seconds = value.trunc();
    let denominator = denominator as f64;
    let mut numerator = round.round(value.fract() * denominator);
    if numerator >= denominator {
        numerator -= denominator;
        seconds += 1.0;
    } else if numerator < 0.0 {
        numerator += denominator;
        seconds -= 1.0;
    }
    Ok((float_to_time_t(seconds, vm)?, numerator as i64))
}

fn as_time_pair(value: i64, denominator: i64, minimum: i64, maximum: i64) -> (i64, i64) {
    let seconds = value.div_euclid(denominator);
    if seconds < minimum {
        (minimum, 0)
    } else if seconds > maximum {
        (maximum, 0)
    } else {
        (seconds, value.rem_euclid(denominator))
    }
}

pub(crate) fn as_timespec(
    nanoseconds: i64,
    clamp: bool,
    vm: &VirtualMachine,
) -> PyResult<(i64, i64)> {
    if !clamp {
        check_time_t(nanoseconds.div_euclid(SEC_TO_NS), vm)?;
    }
    Ok(as_time_pair(nanoseconds, SEC_TO_NS, TIME_T_MIN, TIME_T_MAX))
}

pub(crate) fn as_timeval(
    nanoseconds: i64,
    round: Round,
    clamp: bool,
    vm: &VirtualMachine,
) -> PyResult<(i64, i64)> {
    // Windows uses C long for timeval.tv_sec even when time_t is 64-bit.
    #[cfg(windows)]
    let (minimum, maximum) = (i64::from(i32::MIN), i64::from(i32::MAX));
    #[cfg(not(windows))]
    let (minimum, maximum) = (TIME_T_MIN, TIME_T_MAX);
    let microseconds = round.divide(nanoseconds, US_TO_NS);
    let seconds = microseconds.div_euclid(SEC_TO_US);
    if !clamp && !(minimum..=maximum).contains(&seconds) {
        return Err(vm.new_overflow_error(TIME_T_OVERFLOW));
    }
    Ok(as_time_pair(microseconds, SEC_TO_US, minimum, maximum))
}

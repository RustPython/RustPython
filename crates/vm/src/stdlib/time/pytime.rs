//! Signed nanosecond timestamps and the rounding used by Python's time APIs.

#![cfg_attr(target_env = "musl", allow(deprecated))]

use crate::{AsObject, PyObject, PyResult, VirtualMachine, builtins::PyFloat};
use num_traits::ToPrimitive;
use rustpython_host_env::time::SEC_TO_NS;

#[cfg(any(unix, windows))]
pub(crate) use rustpython_host_env::time::TimeT;
#[cfg(not(any(unix, windows)))]
pub(crate) type TimeT = i64;

const TIME_T_MIN: i64 = TimeT::MIN as _;
const TIME_T_MAX: i64 = TimeT::MAX as _;
const PYTIME_OVERFLOW: &str = "timestamp out of range for C PyTime_t";
const TIME_T_OVERFLOW: &str = "timestamp out of range for platform time_t";

/// `PY_TIMEOUT_MAX`: the longest finite native thread wait, in microseconds.
pub(crate) const THREAD_TIMEOUT_MAX_MICROSECONDS: i64 = cfg_select! {
    windows => 0xffff_fffe * 1_000,
    unix => i64::MAX / 1_000,
    _ => i64::MAX,
};

/// `_thread.TIMEOUT_MAX` also respects the signed nanosecond representation.
#[cfg(feature = "threading")]
pub(crate) const THREAD_TIMEOUT_MAX_SECONDS: i64 = {
    let platform_max = THREAD_TIMEOUT_MAX_MICROSECONDS / 1_000_000;
    let pytime_max = i64::MAX / 1_000_000_000;
    if platform_max < pytime_max {
        platform_max
    } else {
        pytime_max
    }
};

#[derive(Clone, Copy)]
pub enum Round {
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

    #[must_use]
    pub fn divide(self, value: i64, divisor: i64) -> i64 {
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

pub fn from_seconds_object(object: &PyObject, round: Round, vm: &VirtualMachine) -> PyResult<i64> {
    from_object(object, SEC_TO_NS, round, vm)
}

pub fn from_milliseconds_object(
    object: &PyObject,
    round: Round,
    vm: &VirtualMachine,
) -> PyResult<i64> {
    from_object(object, SEC_TO_NS / 1000, round, vm)
}

fn from_object(object: &PyObject, scale: i64, round: Round, vm: &VirtualMachine) -> PyResult<i64> {
    // _PyTime_FromSecondsObject and _PyTime_FromMillisecondsObject use
    // __index__ before __float__ and bound the result to signed nanoseconds.
    if object.number().is_index() {
        index_seconds(object, PYTIME_OVERFLOW, vm)?
            .checked_mul(scale)
            .ok_or_else(|| vm.new_overflow_error(PYTIME_OVERFLOW))
    } else {
        let nanoseconds = round.round(float_seconds(object, vm)? * scale as f64);
        let minimum = i64::MIN as f64;
        // The maximum rounds up to 2**63 as a double, so use an exclusive bound.
        if !(minimum..-minimum).contains(&nanoseconds) {
            return Err(vm.new_overflow_error(PYTIME_OVERFLOW));
        }
        Ok(nanoseconds as i64)
    }
}

/// Check the platform wait limit after the caller has handled negative values
/// and any API-specific indefinite-wait sentinel.
pub(crate) fn check_thread_timeout(nanoseconds: i64, vm: &VirtualMachine) -> PyResult<()> {
    if Round::Up.divide(nanoseconds, 1_000) > THREAD_TIMEOUT_MAX_MICROSECONDS {
        Err(vm.new_overflow_error("timeout value is too large"))
    } else {
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_wait_bound_checks_rounded_up_microseconds() {
        crate::Interpreter::without_stdlib(Default::default()).enter(|vm| {
            if let Some(max_nanoseconds) = THREAD_TIMEOUT_MAX_MICROSECONDS.checked_mul(1_000) {
                assert!(check_thread_timeout(max_nanoseconds, vm).is_ok());
                let error = check_thread_timeout(max_nanoseconds + 1, vm).unwrap_err();
                assert!(error.fast_isinstance(vm.ctx.exceptions.overflow_error));
            } else {
                assert!(check_thread_timeout(i64::MAX, vm).is_ok());
            }
        });
    }
}

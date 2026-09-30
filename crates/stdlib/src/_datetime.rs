//! Native `_datetime`, the accelerator `datetime.py` prefers over `_pydatetime`.
//!
//! A port of CPython's `Modules/_datetimemodule.c`; function names follow it.

// cspell:ignore ymd ordinal isocalendar ISO fromisoformat
// cspell:ignore dnum zreplacement colonzreplacement zname freplacement somezreplacement
// cspell:ignore tzoffset tzmicrosecond tzsign dtstr nogo dtobj tzusec timet
pub(crate) use _datetime::{PyTzInfo, datetime_type, module_def, timedelta_from_seconds};

#[pymodule]
mod _datetime {
    use crate::vm::{
        AsObject, Py, PyObject, PyObjectCell, PyObjectRef, PyPayload, PyRef, PyResult,
        VirtualMachine,
        builtins::{PyBytes, PyFloat, PyInt, PyStr, PyStrRef, PyTuple, PyType, PyTypeRef},
        class::{PyClassImpl, StaticType},
        common::{
            hash::PyHash,
            wtf8::{CodePoint, Wtf8Buf},
        },
        function::{ArgumentError, FromArgs, FuncArgs, OptionalArg, Param, PyComparisonValue},
        protocol::{PyNumber, PyNumberMethods},
        types::{AsNumber, Comparable, Constructor, Hashable, PyComparisonOp, Representable},
    };
    use crossbeam_utils::atomic::AtomicCell;
    use malachite_bigint::{BigInt, ToBigInt};
    use num_traits::{Signed, ToPrimitive, Zero};

    #[pyattr]
    const MINYEAR: i32 = 1;
    #[pyattr]
    const MAXYEAR: i32 = 9999;
    /// `date(9999, 12, 31).toordinal()`
    const MAXORDINAL: i32 = 3_652_059;
    const MAX_DELTA_DAYS: i32 = 999_999_999;

    const US_PER_MS: i64 = 1000;
    const US_PER_SECOND: i64 = 1_000_000;
    const US_PER_MINUTE: i64 = 60 * US_PER_SECOND;
    const US_PER_HOUR: i64 = 60 * US_PER_MINUTE;
    const SECONDS_PER_DAY: i64 = 24 * 3600;
    const US_PER_DAY: i64 = SECONDS_PER_DAY * US_PER_SECOND;
    const US_PER_WEEK: i64 = 7 * US_PER_DAY;

    // Math and calendar helpers

    /// Floor division and the matching non-negative remainder, for `y > 0`.
    fn divmod(x: i64, y: i64) -> (i64, i64) {
        debug_assert!(y > 0);
        (x.div_euclid(y), x.rem_euclid(y))
    }

    /// Floor division and remainder with the sign of the divisor, as Python's `divmod`.
    fn bigint_divmod(x: &BigInt, y: &BigInt) -> (BigInt, BigInt) {
        let mut q = x / y;
        let mut r = x % y;
        if !r.is_zero() && (r.is_negative() != y.is_negative()) {
            q -= 1;
            r += y;
        }
        (q, r)
    }

    /// `_PyLong_DivmodNear`: `m / n` rounded to the nearest integer, ties to even.
    fn divide_nearest(m: &BigInt, n: &BigInt, vm: &VirtualMachine) -> PyResult<BigInt> {
        if n.is_zero() {
            return Err(vm.new_zero_division_error("division by zero"));
        }
        let (q, r) = bigint_divmod(m, n);
        // Round up when 2r > n, or 2r == n and q is odd (for n > 0; mirrored for n < 0).
        let twice_r = &r * 2;
        let greater = if n.is_negative() {
            twice_r < *n
        } else {
            twice_r > *n
        };
        let tie = twice_r == *n;
        let q_odd = (&q % 2u8) != BigInt::zero();
        Ok(if greater || (tie && q_odd) { q + 1 } else { q })
    }

    const DAYS_IN_MONTH: [i32; 13] = [0, 31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    const DAYS_BEFORE_MONTH: [i32; 13] = [0, 0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];

    fn is_leap(year: i32) -> bool {
        let year = year as u32;
        year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
    }

    fn days_in_month(year: i32, month: i32) -> i32 {
        if month == 2 && is_leap(year) {
            29
        } else {
            DAYS_IN_MONTH[month as usize]
        }
    }

    fn days_before_month(year: i32, month: i32) -> i32 {
        DAYS_BEFORE_MONTH[month as usize] + i32::from(month > 2 && is_leap(year))
    }

    fn days_before_year(year: i32) -> i32 {
        let y = year - 1;
        y * 365 + y / 4 - y / 100 + y / 400
    }

    const DI4Y: i32 = 1461;
    const DI100Y: i32 = 36524;
    const DI400Y: i32 = 146_097;

    /// Ordinal (1 = 0001-01-01) to year, month, day.
    fn ord_to_ymd(ordinal: i32) -> (i32, i32, i32) {
        let ordinal = ordinal - 1;
        let n400 = ordinal / DI400Y;
        let n = ordinal % DI400Y;
        let mut year = n400 * 400 + 1;
        let n100 = n / DI100Y;
        let n = n % DI100Y;
        let n4 = n / DI4Y;
        let n = n % DI4Y;
        let n1 = n / 365;
        let mut n = n % 365;
        year += n100 * 100 + n4 * 4 + n1;
        if n1 == 4 || n100 == 4 {
            return (year - 1, 12, 31);
        }
        let leapyear = n1 == 3 && (n4 != 24 || n100 == 3);
        let mut month = (n + 50) >> 5;
        let mut preceding = DAYS_BEFORE_MONTH[month as usize] + i32::from(month > 2 && leapyear);
        if preceding > n {
            month -= 1;
            preceding -= days_in_month(year, month);
        }
        n -= preceding;
        (year, month, n + 1)
    }

    fn ymd_to_ord(year: i32, month: i32, day: i32) -> i32 {
        days_before_year(year) + days_before_month(year, month) + day
    }

    /// Monday == 0 ... Sunday == 6.
    fn weekday(year: i32, month: i32, day: i32) -> i32 {
        (ymd_to_ord(year, month, day) + 6) % 7
    }

    /// Ordinal of the Monday starting ISO week 1 of `year`.
    fn iso_week1_monday(year: i32) -> i32 {
        let first_day = ymd_to_ord(year, 1, 1);
        let first_weekday = (first_day + 6) % 7;
        let week1_monday = first_day - first_weekday;
        if first_weekday > 3 {
            week1_monday + 7
        } else {
            week1_monday
        }
    }

    /// ISO year, week, day to year, month, day; `Err` holds CPython's negative code.
    fn iso_to_ymd(iso_year: i32, iso_week: i32, iso_day: i32) -> Result<(i32, i32, i32), i32> {
        if !(MINYEAR..=MAXYEAR).contains(&iso_year) {
            return Err(-4);
        }
        if iso_week <= 0 || iso_week >= 53 {
            let mut out_of_range = true;
            if iso_week == 53 {
                let first_weekday = weekday(iso_year, 1, 1);
                if first_weekday == 3 || (first_weekday == 2 && is_leap(iso_year)) {
                    out_of_range = false;
                }
            }
            if out_of_range {
                return Err(-2);
            }
        }
        if iso_day <= 0 || iso_day >= 8 {
            return Err(-3);
        }
        let day_1 = iso_week1_monday(iso_year);
        let day_offset = (iso_week - 1) * 7 + iso_day - 1;
        Ok(ord_to_ymd(day_1 + day_offset))
    }

    // Range checks

    fn check_delta_day_range(days: i64, vm: &VirtualMachine) -> PyResult<i32> {
        if (-i64::from(MAX_DELTA_DAYS)..=i64::from(MAX_DELTA_DAYS)).contains(&days) {
            Ok(days as i32)
        } else {
            Err(vm.new_overflow_error(format!(
                "days={days}; must have magnitude <= {MAX_DELTA_DAYS}"
            )))
        }
    }

    fn check_date_args(year: i32, month: i32, day: i32, vm: &VirtualMachine) -> PyResult<()> {
        if !(MINYEAR..=MAXYEAR).contains(&year) {
            return Err(
                vm.new_value_error(format!("year must be in {MINYEAR}..{MAXYEAR}, not {year}"))
            );
        }
        if !(1..=12).contains(&month) {
            return Err(vm.new_value_error(format!("month must be in 1..12, not {month}")));
        }
        let dim = days_in_month(year, month);
        if day < 1 || day > dim {
            return Err(vm.new_value_error(format!(
                "day {day} must be in range 1..{dim} for month {month} in year {year}"
            )));
        }
        Ok(())
    }

    fn check_time_args(
        h: i32,
        m: i32,
        s: i32,
        us: i32,
        fold: i32,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        if !(0..=23).contains(&h) {
            return Err(vm.new_value_error(format!("hour must be in 0..23, not {h}")));
        }
        if !(0..=59).contains(&m) {
            return Err(vm.new_value_error(format!("minute must be in 0..59, not {m}")));
        }
        if !(0..=59).contains(&s) {
            return Err(vm.new_value_error(format!("second must be in 0..59, not {s}")));
        }
        if !(0..=999_999).contains(&us) {
            return Err(vm.new_value_error(format!("microsecond must be in 0..999999, not {us}")));
        }
        if fold != 0 && fold != 1 {
            return Err(vm.new_value_error(format!("fold must be either 0 or 1, not {fold}")));
        }
        Ok(())
    }

    // Normalization

    /// Carry `lo` into `hi` until `0 <= lo < factor`.
    fn normalize_pair(hi: &mut i64, lo: &mut i64, factor: i64) {
        if *lo < 0 || *lo >= factor {
            let (num_hi, rest) = divmod(*lo, factor);
            *hi += num_hi;
            *lo = rest;
        }
    }

    fn normalize_d_s_us(d: &mut i64, s: &mut i64, us: &mut i64) {
        normalize_pair(s, us, US_PER_SECOND);
        normalize_pair(d, s, SECONDS_PER_DAY);
    }

    /// Bring an out-of-range day back into its month; `Err` when the year leaves the range.
    fn normalize_y_m_d(y: &mut i64, m: &mut i64, d: &mut i64) -> Result<(), ()> {
        let dim = i64::from(days_in_month(*y as i32, *m as i32));
        if *d < 1 || *d > dim {
            if *d == 0 {
                *m -= 1;
                if *m > 0 {
                    *d = i64::from(days_in_month(*y as i32, *m as i32));
                } else {
                    *y -= 1;
                    *m = 12;
                    *d = 31;
                }
            } else if *d == dim + 1 {
                *m += 1;
                *d = 1;
                if *m > 12 {
                    *m = 1;
                    *y += 1;
                }
            } else {
                let ordinal = i64::from(ymd_to_ord(*y as i32, *m as i32, 1)) + *d - 1;
                if ordinal < 1 || ordinal > i64::from(MAXORDINAL) {
                    return Err(());
                }
                let (ny, nm, nd) = ord_to_ymd(ordinal as i32);
                (*y, *m, *d) = (i64::from(ny), i64::from(nm), i64::from(nd));
                return Ok(());
            }
        }
        if (i64::from(MINYEAR)..=i64::from(MAXYEAR)).contains(y) {
            Ok(())
        } else {
            Err(())
        }
    }

    fn date_overflow(vm: &VirtualMachine) -> crate::vm::builtins::PyBaseExceptionRef {
        vm.new_overflow_error("date value out of range")
    }

    // Shared object helpers

    /// `tp_name`: `datetime.timedelta` for the built-in types, the bare name for a subclass.
    fn type_name(obj: &PyObject) -> String {
        obj.class().slot_name().to_string()
    }

    fn checked_str_result(result: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        result.downcast::<PyStr>().map_err(|obj| {
            vm.new_type_error(format!(
                "__str__ returned non-string (type {})",
                obj.class().slot_name()
            ))
        })
    }

    /// Exact `int` value of an `int` (or subclass) object.
    fn as_int(obj: &PyObject) -> Option<&BigInt> {
        obj.downcast_ref::<PyInt>().map(|i| i.as_bigint())
    }

    /// `PyLong_FromDouble`, with CPython's errors for infinities and NaN.
    fn float_to_bigint(value: f64, vm: &VirtualMachine) -> PyResult<BigInt> {
        if value.is_nan() {
            return Err(vm.new_value_error("cannot convert float NaN to integer"));
        }
        if value.is_infinite() {
            return Err(vm.new_overflow_error("cannot convert float infinity to integer"));
        }
        Ok(value.to_bigint().expect("finite floats convert"))
    }

    /// `PyLong_AsInt` on an exact value.
    fn bigint_to_i32(value: &BigInt, vm: &VirtualMachine) -> PyResult<i32> {
        value
            .to_i32()
            .ok_or_else(|| vm.new_overflow_error("Python int too large to convert to C int"))
    }

    // timedelta

    #[pyattr]
    #[pyclass(module = "datetime", name = "timedelta")]
    #[derive(Debug, PyPayload)]
    pub(super) struct PyDelta {
        #[pymember]
        days: i32,
        #[pymember]
        seconds: i32,
        #[pymember]
        microseconds: i32,
        hashcode: AtomicCell<PyHash>,
    }

    impl PyDelta {
        fn new_unchecked(days: i32, seconds: i32, microseconds: i32) -> Self {
            Self {
                days,
                seconds,
                microseconds,
                hashcode: AtomicCell::new(-1),
            }
        }

        /// `new_delta_ex`: normalize when asked, check the day range and build an instance.
        fn new_ex(
            days: i64,
            seconds: i64,
            microseconds: i64,
            normalize: bool,
            vm: &VirtualMachine,
        ) -> PyResult<Self> {
            let (mut d, mut s, mut us) = (days, seconds, microseconds);
            if normalize {
                normalize_d_s_us(&mut d, &mut s, &mut us);
            }
            let d = check_delta_day_range(d, vm)?;
            Ok(Self::new_unchecked(d, s as i32, us as i32))
        }

        /// `new_delta`: a plain `timedelta` from components.
        fn new_ref(
            days: i64,
            seconds: i64,
            microseconds: i64,
            normalize: bool,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            Ok(Self::new_ex(days, seconds, microseconds, normalize, vm)?.into_ref(&vm.ctx))
        }

        fn to_microseconds(&self) -> i128 {
            (i128::from(self.days) * i128::from(SECONDS_PER_DAY) + i128::from(self.seconds))
                * i128::from(US_PER_SECOND)
                + i128::from(self.microseconds)
        }

        fn to_microseconds_big(&self) -> BigInt {
            BigInt::from(self.to_microseconds())
        }

        /// `microseconds_to_delta_ex`
        fn from_microseconds(us: &BigInt, vm: &VirtualMachine) -> PyResult<Self> {
            let (seconds, us) = bigint_divmod(us, &BigInt::from(US_PER_SECOND));
            let (days, seconds) = bigint_divmod(&seconds, &BigInt::from(SECONDS_PER_DAY));
            let days = bigint_to_i32(&days, vm)?;
            // `us` and `seconds` are in range by construction.
            Self::new_ex(
                i64::from(days),
                seconds.to_i64().unwrap_or_default(),
                us.to_i64().unwrap_or_default(),
                false,
                vm,
            )
        }

        fn from_microseconds_ref(us: &BigInt, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
            Ok(Self::from_microseconds(us, vm)?.into_ref(&vm.ctx))
        }

        fn from_microseconds_object(us: &PyObject, vm: &VirtualMachine) -> PyResult<Self> {
            if us.class().is(vm.ctx.types.int_type) {
                return Self::from_microseconds(as_int(us).unwrap(), vm);
            }
            let (seconds, us) = checked_delta_divmod(us, US_PER_SECOND, vm)?;
            let us: i32 = us.try_into_value(vm)?;
            if !(0..US_PER_SECOND as i32).contains(&us) {
                return Err(vm.new_type_error("divmod() returned a value out of range"));
            }
            let (days, seconds) = checked_delta_divmod(&seconds, SECONDS_PER_DAY, vm)?;
            let seconds: i32 = seconds.try_into_value(vm)?;
            if !(0..SECONDS_PER_DAY as i32).contains(&seconds) {
                return Err(vm.new_type_error("divmod() returned a value out of range"));
            }
            let days: i32 = days.try_into_value(vm)?;
            Self::new_ex(
                i64::from(days),
                i64::from(seconds),
                i64::from(us),
                false,
                vm,
            )
        }

        fn is_nonzero(&self) -> bool {
            self.days != 0 || self.seconds != 0 || self.microseconds != 0
        }

        fn cmp_key(&self) -> (i32, i32, i32) {
            (self.days, self.seconds, self.microseconds)
        }

        fn negative(&self, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
            Self::new_ref(
                -i64::from(self.days),
                -i64::from(self.seconds),
                -i64::from(self.microseconds),
                true,
                vm,
            )
        }

        fn getstate(&self, vm: &VirtualMachine) -> PyRef<PyTuple> {
            vm.new_tuple((self.days, self.seconds, self.microseconds))
        }
    }

    fn checked_delta_divmod(
        value: &PyObject,
        divisor: i64,
        vm: &VirtualMachine,
    ) -> PyResult<(PyObjectRef, PyObjectRef)> {
        let result = vm._divmod(value, vm.ctx.new_int(divisor).as_object())?;
        let Some(tuple) = result.downcast_ref::<PyTuple>() else {
            return Err(vm.new_type_error(format!(
                "divmod() returned non-tuple (type {})",
                type_name(&result)
            )));
        };
        let [quotient, remainder] = tuple.as_slice() else {
            return Err(vm.new_type_error(format!(
                "divmod() returned a tuple of size {}",
                tuple.as_slice().len()
            )));
        };
        Ok((quotient.clone(), remainder.clone()))
    }

    enum DeltaSum {
        Native(BigInt),
        Object(PyObjectRef),
    }

    impl DeltaSum {
        fn add_int(&mut self, term: BigInt, vm: &VirtualMachine) -> PyResult<()> {
            match self {
                Self::Native(value) => *value += term,
                Self::Object(value) => {
                    *value = vm._add(value, vm.ctx.new_bigint(&term).as_object())?;
                }
            }
            Ok(())
        }

        fn add_object(&mut self, term: &PyObject, vm: &VirtualMachine) -> PyResult<()> {
            let result = match self {
                Self::Native(value) => vm._add(vm.ctx.new_bigint(value).as_object(), term)?,
                Self::Object(value) => vm._add(value, term)?,
            };
            *self = Self::Object(result);
            Ok(())
        }

        fn is_odd(&self, vm: &VirtualMachine) -> PyResult<bool> {
            match self {
                Self::Native(value) => Ok((value % 2u8) != BigInt::zero()),
                Self::Object(value) => vm
                    ._and(value, vm.ctx.new_int(1).as_object())?
                    .try_to_bool(vm),
            }
        }

        fn into_delta(self, vm: &VirtualMachine) -> PyResult<PyDelta> {
            match self {
                Self::Native(value) => PyDelta::from_microseconds(&value, vm),
                Self::Object(value) => PyDelta::from_microseconds_object(&value, vm),
            }
        }
    }

    /// Fold `num` units of `factor` microseconds into `sofar`; float fractions go to `leftover`.
    fn accum(
        tag: &str,
        sofar: &mut DeltaSum,
        num: &PyObject,
        factor: i64,
        leftover: &mut f64,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        if let Some(int) = as_int(num) {
            if num.class().is(vm.ctx.types.int_type) {
                return sofar.add_int(int * factor, vm);
            }
            let product = vm._mul(num, vm.ctx.new_int(factor).as_object())?;
            return sofar.add_object(&product, vm);
        }
        if let Some(float) = num.downcast_ref::<PyFloat>() {
            let dnum = float.to_f64();
            let intpart = dnum.trunc();
            let fracpart = dnum - intpart;
            sofar.add_int(float_to_bigint(intpart, vm)? * factor, vm)?;
            if fracpart == 0.0 || !fracpart.is_finite() {
                return Ok(());
            }
            let dnum = factor as f64 * fracpart;
            let intpart = dnum.trunc();
            sofar.add_int(float_to_bigint(intpart, vm)?, vm)?;
            *leftover += dnum - intpart;
            return Ok(());
        }
        Err(vm.new_type_error(format!(
            "unsupported type for timedelta {tag} component: {}",
            type_name(num)
        )))
    }

    #[derive(FromArgs)]
    pub(super) struct DeltaArgs {
        #[pyarg(any, optional)]
        days: OptionalArg<PyObjectRef>,
        #[pyarg(any, optional)]
        seconds: OptionalArg<PyObjectRef>,
        #[pyarg(any, optional)]
        microseconds: OptionalArg<PyObjectRef>,
        #[pyarg(any, optional)]
        milliseconds: OptionalArg<PyObjectRef>,
        #[pyarg(any, optional)]
        minutes: OptionalArg<PyObjectRef>,
        #[pyarg(any, optional)]
        hours: OptionalArg<PyObjectRef>,
        #[pyarg(any, optional)]
        weeks: OptionalArg<PyObjectRef>,
    }

    impl Constructor for PyDelta {
        type Args = DeltaArgs;

        fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            let mut x = DeltaSum::Native(BigInt::zero());
            let mut leftover_us = 0.0f64;
            let parts: [(&str, &OptionalArg<PyObjectRef>, i64); 7] = [
                ("microseconds", &args.microseconds, 1),
                ("milliseconds", &args.milliseconds, US_PER_MS),
                ("seconds", &args.seconds, US_PER_SECOND),
                ("minutes", &args.minutes, US_PER_MINUTE),
                ("hours", &args.hours, US_PER_HOUR),
                ("days", &args.days, US_PER_DAY),
                ("weeks", &args.weeks, US_PER_WEEK),
            ];
            for (tag, value, factor) in parts {
                if let OptionalArg::Present(value) = value {
                    accum(tag, &mut x, value, factor, &mut leftover_us, vm)?;
                }
            }
            if leftover_us != 0.0 {
                // Round to the nearest whole microsecond, ties to even.
                let mut whole_us = leftover_us.round();
                if (whole_us - leftover_us).abs() == 0.5 {
                    let x_is_odd = f64::from(u8::from(x.is_odd(vm)?));
                    whole_us = 2.0 * ((leftover_us + x_is_odd) * 0.5).round() - x_is_odd;
                }
                x.add_int(BigInt::from(whole_us as i64), vm)?;
            }
            x.into_delta(vm)
        }
    }

    /// `delta_add` and friends take either operand as the `timedelta`.
    fn as_delta(obj: &PyObject) -> Option<&Py<PyDelta>> {
        obj.downcast_ref::<PyDelta>()
    }

    /// `float.as_integer_ratio()`, called as a method so subclasses can override it.
    fn float_ratio(float: &PyObject, vm: &VirtualMachine) -> PyResult<(PyObjectRef, PyObjectRef)> {
        let ratio = vm.call_method(float, "as_integer_ratio", ())?;
        let Some(tuple) = ratio.downcast_ref::<PyTuple>() else {
            return Err(vm.new_type_error(format!(
                "unexpected return type from as_integer_ratio(): expected tuple, not '{}'",
                type_name(&ratio)
            )));
        };
        let [num, den] = tuple.as_slice() else {
            return Err(vm.new_value_error("as_integer_ratio() must return a 2-tuple"));
        };
        Ok((num.clone(), den.clone()))
    }

    /// `multiply_truedivide_timedelta_float`: `divide` selects `/` over `*`.
    fn delta_float_op(
        delta: &PyDelta,
        float: &PyObject,
        divide: bool,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<PyDelta>> {
        let us = delta.to_microseconds_big();
        let (num, den) = float_ratio(float, vm)?;
        let (mul, div) = if divide { (den, num) } else { (num, den) };
        let product = if mul.class().is(vm.ctx.types.int_type) {
            us * as_int(&mul).unwrap()
        } else {
            let product = vm._mul(vm.ctx.new_bigint(&us).as_object(), &mul)?;
            as_int(&product)
                .cloned()
                .ok_or_else(|| vm.new_type_error("divmod() requires ints"))?
        };
        let divisor = as_int(&div).ok_or_else(|| vm.new_type_error("divmod() requires ints"))?;
        let out = divide_nearest(&product, divisor, vm)?;
        PyDelta::from_microseconds_ref(&out, vm)
    }

    impl PyDelta {
        fn nb_add(a: &PyObject, b: &PyObject, vm: &VirtualMachine) -> PyResult {
            let (Some(a), Some(b)) = (as_delta(a), as_delta(b)) else {
                return Ok(vm.ctx.not_implemented());
            };
            Ok(Self::new_ref(
                i64::from(a.days) + i64::from(b.days),
                i64::from(a.seconds) + i64::from(b.seconds),
                i64::from(a.microseconds) + i64::from(b.microseconds),
                true,
                vm,
            )?
            .into())
        }

        fn nb_subtract(a: &PyObject, b: &PyObject, vm: &VirtualMachine) -> PyResult {
            let (Some(a), Some(b)) = (as_delta(a), as_delta(b)) else {
                return Ok(vm.ctx.not_implemented());
            };
            Ok(Self::new_ref(
                i64::from(a.days) - i64::from(b.days),
                i64::from(a.seconds) - i64::from(b.seconds),
                i64::from(a.microseconds) - i64::from(b.microseconds),
                true,
                vm,
            )?
            .into())
        }

        fn nb_multiply(a: &PyObject, b: &PyObject, vm: &VirtualMachine) -> PyResult {
            let (delta, other) = match (as_delta(a), as_delta(b)) {
                (Some(delta), _) => (delta, b),
                (None, Some(delta)) => (delta, a),
                (None, None) => return Ok(vm.ctx.not_implemented()),
            };
            if let Some(int) = as_int(other) {
                if other.class().is(vm.ctx.types.int_type) {
                    let us = delta.to_microseconds_big() * int;
                    return Ok(Self::from_microseconds_ref(&us, vm)?.into());
                }
                let us = vm._mul(
                    other,
                    vm.ctx.new_bigint(&delta.to_microseconds_big()).as_object(),
                )?;
                return Ok(Self::from_microseconds_object(&us, vm)?
                    .into_ref(&vm.ctx)
                    .into());
            }
            if other.downcast_ref::<PyFloat>().is_some() {
                return Ok(delta_float_op(delta, other, false, vm)?.into());
            }
            Ok(vm.ctx.not_implemented())
        }

        fn nb_floor_divide(a: &PyObject, b: &PyObject, vm: &VirtualMachine) -> PyResult {
            let Some(delta) = as_delta(a) else {
                return Ok(vm.ctx.not_implemented());
            };
            if let Some(int) = as_int(b) {
                if !b.class().is(vm.ctx.types.int_type) {
                    let us = vm._floordiv(
                        vm.ctx.new_bigint(&delta.to_microseconds_big()).as_object(),
                        b,
                    )?;
                    return Ok(Self::from_microseconds_object(&us, vm)?
                        .into_ref(&vm.ctx)
                        .into());
                }
                if int.is_zero() {
                    return Err(vm.new_zero_division_error("division by zero"));
                }
                let (q, _) = bigint_divmod(&delta.to_microseconds_big(), int);
                return Ok(Self::from_microseconds_ref(&q, vm)?.into());
            }
            if let Some(other) = as_delta(b) {
                let right = other.to_microseconds_big();
                if right.is_zero() {
                    return Err(vm.new_zero_division_error("division by zero"));
                }
                let (q, _) = bigint_divmod(&delta.to_microseconds_big(), &right);
                return Ok(vm.ctx.new_bigint(&q).into());
            }
            Ok(vm.ctx.not_implemented())
        }

        fn nb_true_divide(a: &PyObject, b: &PyObject, vm: &VirtualMachine) -> PyResult {
            let Some(delta) = as_delta(a) else {
                return Ok(vm.ctx.not_implemented());
            };
            if let Some(other) = as_delta(b) {
                if !delta.is_nonzero() && other.days < 0 {
                    return Ok(vm.ctx.new_float(-0.0).into());
                }
                let left = vm.ctx.new_bigint(&delta.to_microseconds_big());
                let right = vm.ctx.new_bigint(&other.to_microseconds_big());
                return vm._truediv(left.as_object(), right.as_object());
            }
            if b.downcast_ref::<PyFloat>().is_some() {
                return Ok(delta_float_op(delta, b, true, vm)?.into());
            }
            if let Some(int) = as_int(b) {
                let us = divide_nearest(&delta.to_microseconds_big(), int, vm)?;
                return Ok(Self::from_microseconds_ref(&us, vm)?.into());
            }
            Ok(vm.ctx.not_implemented())
        }

        fn nb_remainder(a: &PyObject, b: &PyObject, vm: &VirtualMachine) -> PyResult {
            let (Some(a), Some(b)) = (as_delta(a), as_delta(b)) else {
                return Ok(vm.ctx.not_implemented());
            };
            let right = b.to_microseconds_big();
            if right.is_zero() {
                return Err(vm.new_zero_division_error("division by zero"));
            }
            let (_, r) = bigint_divmod(&a.to_microseconds_big(), &right);
            Ok(Self::from_microseconds_ref(&r, vm)?.into())
        }

        fn nb_divmod(a: &PyObject, b: &PyObject, vm: &VirtualMachine) -> PyResult {
            let (Some(a), Some(b)) = (as_delta(a), as_delta(b)) else {
                return Ok(vm.ctx.not_implemented());
            };
            let right = b.to_microseconds_big();
            if right.is_zero() {
                return Err(vm.new_zero_division_error("division by zero"));
            }
            let (q, r) = bigint_divmod(&a.to_microseconds_big(), &right);
            let delta = Self::from_microseconds_ref(&r, vm)?;
            Ok(vm.new_tuple((vm.ctx.new_bigint(&q), delta)).into())
        }
    }

    impl AsNumber for PyDelta {
        fn as_number() -> &'static PyNumberMethods {
            static AS_NUMBER: PyNumberMethods = PyNumberMethods {
                add: Some(PyDelta::nb_add),
                subtract: Some(PyDelta::nb_subtract),
                multiply: Some(PyDelta::nb_multiply),
                remainder: Some(PyDelta::nb_remainder),
                divmod: Some(PyDelta::nb_divmod),
                floor_divide: Some(PyDelta::nb_floor_divide),
                true_divide: Some(PyDelta::nb_true_divide),
                negative: Some(|num, vm| {
                    let delta = num.obj.downcast_ref::<PyDelta>().unwrap();
                    Ok(delta.negative(vm)?.into())
                }),
                positive: Some(|num, vm| {
                    let delta = num.obj.downcast_ref::<PyDelta>().unwrap();
                    Ok(PyDelta::new_ref(
                        i64::from(delta.days),
                        i64::from(delta.seconds),
                        i64::from(delta.microseconds),
                        false,
                        vm,
                    )?
                    .into())
                }),
                absolute: Some(|num, vm| {
                    let delta = num.obj.downcast_ref::<PyDelta>().unwrap();
                    if delta.days < 0 {
                        Ok(delta.negative(vm)?.into())
                    } else {
                        Ok(PyDelta::new_ref(
                            i64::from(delta.days),
                            i64::from(delta.seconds),
                            i64::from(delta.microseconds),
                            false,
                            vm,
                        )?
                        .into())
                    }
                }),
                boolean: Some(|num: PyNumber<'_>, _vm| {
                    Ok(num.obj.downcast_ref::<PyDelta>().unwrap().is_nonzero())
                }),
                ..PyNumberMethods::NOT_IMPLEMENTED
            };
            &AS_NUMBER
        }
    }

    impl Comparable for PyDelta {
        fn cmp(
            zelf: &Py<Self>,
            other: &PyObject,
            op: PyComparisonOp,
            _vm: &VirtualMachine,
        ) -> PyResult<PyComparisonValue> {
            let Some(other) = as_delta(other) else {
                return Ok(PyComparisonValue::NotImplemented);
            };
            Ok(op.eval_ord(zelf.cmp_key().cmp(&other.cmp_key())).into())
        }
    }

    impl Hashable for PyDelta {
        fn hash(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyHash> {
            let cached = zelf.hashcode.load();
            if cached != -1 {
                return Ok(cached);
            }
            let hash = zelf.getstate(vm).as_object().hash(vm)?;
            zelf.hashcode.store(hash);
            Ok(hash)
        }
    }

    impl Representable for PyDelta {
        fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            let mut args = Vec::new();
            if zelf.days != 0 {
                args.push(format!("days={}", zelf.days));
            }
            if zelf.seconds != 0 {
                args.push(format!("seconds={}", zelf.seconds));
            }
            if zelf.microseconds != 0 {
                args.push(format!("microseconds={}", zelf.microseconds));
            }
            let args = if args.is_empty() {
                "0".to_owned()
            } else {
                args.join(", ")
            };
            Ok(format!("{}({args})", type_name(zelf.as_object())))
        }
    }

    #[pyclass(
        with(Constructor, AsNumber, Comparable, Hashable, Representable),
        flags(BASETYPE)
    )]
    impl PyDelta {
        #[pymethod]
        fn total_seconds(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult {
            let us = vm.ctx.new_bigint(&zelf.to_microseconds_big());
            let per_second = vm.ctx.new_int(US_PER_SECOND);
            vm._truediv(us.as_object(), per_second.as_object())
        }

        #[pymethod]
        fn __reduce__(zelf: &Py<Self>, vm: &VirtualMachine) -> PyRef<PyTuple> {
            vm.new_tuple((zelf.class().to_owned(), zelf.getstate(vm)))
        }

        #[pyslot]
        #[allow(clippy::unnecessary_wraps)]
        fn slot_str(zelf: &PyObject, vm: &VirtualMachine) -> PyResult<PyStrRef> {
            let delta = zelf.downcast_ref::<Self>().unwrap();
            let us = delta.microseconds;
            let (minutes, seconds) = divmod(i64::from(delta.seconds), 60);
            let (hours, minutes) = divmod(minutes, 60);
            let days = delta.days;
            let mut out = String::new();
            if days != 0 {
                let plural = if days == 1 || days == -1 { "" } else { "s" };
                out.push_str(&format!("{days} day{plural}, "));
            }
            out.push_str(&format!("{hours}:{minutes:02}:{seconds:02}"));
            if us != 0 {
                out.push_str(&format!(".{us:06}"));
            }
            Ok(vm.ctx.new_str(out))
        }

        #[extend_class]
        fn extend_class(ctx: &crate::vm::Context, class: &Py<PyType>) {
            let make = |d, s, us| {
                PyRef::new_ref(Self::new_unchecked(d, s, us), class.to_owned(), None).into()
            };
            class.set_attr(ctx.intern_str("min"), make(-MAX_DELTA_DAYS, 0, 0));
            class.set_attr(
                ctx.intern_str("max"),
                make(MAX_DELTA_DAYS, 24 * 3600 - 1, 999_999),
            );
            class.set_attr(ctx.intern_str("resolution"), make(0, 0, 1));
        }
    }

    // Type checks

    fn date_type() -> &'static Py<PyType> {
        PyDate::static_type()
    }

    pub(crate) fn datetime_type() -> &'static Py<PyType> {
        PyDateTime::static_type()
    }

    pub(crate) fn timedelta_from_seconds(
        seconds: i64,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        let delta = PyDelta::new_ex(0, seconds, 0, true, vm)?;
        Ok(delta.into_ref(&vm.ctx).into())
    }

    fn time_type() -> &'static Py<PyType> {
        PyTime::static_type()
    }

    fn as_date(obj: &PyObject) -> Option<&Py<PyDate>> {
        obj.downcast_ref::<PyDate>()
    }

    fn as_datetime(obj: &PyObject) -> Option<&Py<PyDateTime>> {
        obj.downcast_ref::<PyDateTime>()
    }

    fn as_time(obj: &PyObject) -> Option<&Py<PyTime>> {
        obj.downcast_ref::<PyTime>()
    }

    fn is_tzinfo(obj: &PyObject) -> bool {
        obj.fast_isinstance(PyTzInfo::static_type())
    }

    /// The `tzinfo` a `time` or `datetime` holds; `None` for naive values and dates.
    fn get_tzinfo_member(obj: &PyObject) -> Option<PyObjectRef> {
        if let Some(dt) = as_datetime(obj) {
            dt.tzinfo.load_owned()
        } else if let Some(t) = as_time(obj) {
            t.tzinfo.clone()
        } else {
            None
        }
    }

    fn tzinfo_or_none(tzinfo: Option<&PyObjectRef>, vm: &VirtualMachine) -> PyObjectRef {
        tzinfo.cloned().unwrap_or_else(|| vm.ctx.none())
    }

    /// `tzinfo` argument to storage: Python `None` becomes `None`.
    fn tzinfo_arg(tzinfo: PyObjectRef, vm: &VirtualMachine) -> Option<PyObjectRef> {
        (!vm.is_none(&tzinfo)).then_some(tzinfo)
    }

    fn check_tzinfo_subclass(p: &PyObject, vm: &VirtualMachine) -> PyResult<()> {
        if vm.is_none(p) || is_tzinfo(p) {
            Ok(())
        } else {
            Err(vm.new_type_error(format!(
                "tzinfo argument must be None or of a tzinfo subclass, not type '{}'",
                type_name(p)
            )))
        }
    }

    fn same_tzinfo(a: Option<&PyObjectRef>, b: Option<&PyObjectRef>) -> bool {
        match (a, b) {
            (None, None) => true,
            (Some(a), Some(b)) => a.is(b),
            _ => false,
        }
    }

    /// An offset outside `(-24h, 24h)`.
    fn offset_out_of_range(offset: &PyDelta) -> bool {
        (offset.days == -1 && offset.seconds == 0 && offset.microseconds < 1)
            || offset.days < -1
            || offset.days >= 1
    }

    fn offset_range_error(
        offset: &PyObject,
        vm: &VirtualMachine,
    ) -> PyResult<crate::vm::builtins::PyBaseExceptionRef> {
        Ok(vm.new_value_error(format!(
            "offset must be a timedelta strictly between -timedelta(hours=24) and \
             timedelta(hours=24), not {}",
            offset.repr(vm)?
        )))
    }

    /// `call_tzinfo_method`: `tzinfo.<name>(arg)`, checked to be `None` or an in-range offset.
    fn call_tzinfo_method(
        tzinfo: Option<&PyObjectRef>,
        name: &str,
        arg: &PyObject,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyRef<PyDelta>>> {
        let Some(tzinfo) = tzinfo else {
            return Ok(None);
        };
        let offset = vm.call_method(tzinfo, name, (arg.to_owned(),))?;
        if vm.is_none(&offset) {
            return Ok(None);
        }
        match offset.downcast::<PyDelta>() {
            Ok(delta) => {
                if offset_out_of_range(&delta) {
                    return Err(offset_range_error(delta.as_object(), vm)?);
                }
                Ok(Some(delta))
            }
            Err(offset) => Err(vm.new_type_error(format!(
                "tzinfo.{name}() must return None or timedelta, not '{}'",
                type_name(&offset)
            ))),
        }
    }

    fn call_utcoffset(
        tzinfo: Option<&PyObjectRef>,
        arg: &PyObject,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyRef<PyDelta>>> {
        call_tzinfo_method(tzinfo, "utcoffset", arg, vm)
    }

    fn call_dst(
        tzinfo: Option<&PyObjectRef>,
        arg: &PyObject,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyRef<PyDelta>>> {
        call_tzinfo_method(tzinfo, "dst", arg, vm)
    }

    fn call_tzname(
        tzinfo: Option<&PyObjectRef>,
        arg: &PyObject,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyStrRef>> {
        let Some(tzinfo) = tzinfo else {
            return Ok(None);
        };
        let result = vm.call_method(tzinfo, "tzname", (arg.to_owned(),))?;
        if vm.is_none(&result) {
            return Ok(None);
        }
        result.downcast::<PyStr>().map(Some).map_err(|result| {
            vm.new_type_error(format!(
                "tzinfo.tzname() must return None or a string, not '{}'",
                type_name(&result)
            ))
        })
    }

    fn offset_to_py(offset: Option<PyRef<PyDelta>>, vm: &VirtualMachine) -> PyObjectRef {
        offset.map_or_else(|| vm.ctx.none(), Into::into)
    }

    /// `+HH<sep>MM[<sep>SS[.ffffff]]` for an offset.
    fn format_offset(offset: &PyDelta, sep: &str) -> String {
        let (sign, seconds, microseconds) = if offset.days < 0 {
            // -offset, normalized: days is -1 for any in-range negative offset.
            let us = -offset.to_microseconds();
            let s = (us / 1_000_000) as i64;
            ('-', s, (us % 1_000_000) as i64)
        } else {
            (
                '+',
                i64::from(offset.seconds),
                i64::from(offset.microseconds),
            )
        };
        let (minutes, seconds) = divmod(seconds, 60);
        let (hours, minutes) = divmod(minutes, 60);
        if microseconds != 0 {
            format!("{sign}{hours:02}{sep}{minutes:02}{sep}{seconds:02}.{microseconds:06}")
        } else if seconds != 0 {
            format!("{sign}{hours:02}{sep}{minutes:02}{sep}{seconds:02}")
        } else {
            format!("{sign}{hours:02}{sep}{minutes:02}")
        }
    }

    /// `format_utcoffset`: the formatted `utcoffset()`, empty when it is `None`.
    fn format_utcoffset(
        sep: &str,
        tzinfo: Option<&PyObjectRef>,
        arg: &PyObject,
        vm: &VirtualMachine,
    ) -> PyResult<String> {
        Ok(call_utcoffset(tzinfo, arg, vm)?
            .map(|offset| format_offset(&offset, sep))
            .unwrap_or_default())
    }

    fn append_keyword_tzinfo(
        mut repr: String,
        tzinfo: Option<&PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<String> {
        if let Some(tzinfo) = tzinfo {
            repr.pop();
            repr.push_str(&format!(", tzinfo={})", tzinfo.repr(vm)?));
        }
        Ok(repr)
    }

    fn append_keyword_fold(mut repr: String, fold: u8) -> String {
        if fold != 0 {
            repr.pop();
            repr.push_str(&format!(", fold={fold})"));
        }
        repr
    }

    fn format_ctime(year: i32, month: i32, day: i32, h: u8, m: u8, s: u8) -> String {
        const DAY_NAMES: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
        const MONTH_NAMES: [&str; 12] = [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ];
        // Pickle states may contain year zero, whose weekday can be negative.
        let wday = weekday(year, month, day).rem_euclid(7);
        format!(
            "{} {} {day:2} {h:02}:{m:02}:{s:02} {year:04}",
            DAY_NAMES[wday as usize],
            MONTH_NAMES[(month - 1) as usize]
        )
    }

    /// `time.struct_time` with weekday and year day computed.
    #[allow(clippy::too_many_arguments)]
    fn build_struct_time(
        y: i32,
        m: i32,
        d: i32,
        hh: i32,
        mm: i32,
        ss: i32,
        dstflag: i32,
        vm: &VirtualMachine,
    ) -> PyResult {
        let struct_time = vm.import("time", 0)?.get_attr("struct_time", vm)?;
        let fields: Vec<PyObjectRef> = [
            y,
            m,
            d,
            hh,
            mm,
            ss,
            weekday(y, m, d),
            days_before_month(y, m) + d,
            dstflag,
        ]
        .into_iter()
        .map(|v| vm.ctx.new_int(v).into())
        .collect();
        struct_time.call((vm.ctx.new_tuple(fields),), vm)
    }

    /// `wrap_strftime`: expand `%z`, `%:z`, `%Z`, `%f` and zero-pad years before `time.strftime`.
    fn wrap_strftime(
        object: &PyObject,
        format: &Py<PyStr>,
        timetuple: PyObjectRef,
        tzinfoarg: &PyObject,
        vm: &VirtualMachine,
    ) -> PyResult {
        let strftime = vm.import("time", 0)?.get_attr("strftime", vm)?;
        let chars: Vec<CodePoint> = format.as_wtf8().code_points().collect();
        let flen = chars.len();
        let mut out = Wtf8Buf::new();
        let (mut zreplacement, mut colonzreplacement) = (None::<String>, None::<String>);
        let mut zname: Option<Wtf8Buf> = None;
        let mut freplacement: Option<String> = None;
        let mut start = 0;
        let mut i = 0;
        let mut changed = false;
        let push_range = |out: &mut Wtf8Buf, from: usize, to: usize| {
            for &c in &chars[from..to] {
                out.push(c);
            }
        };
        while i < flen {
            if chars[i] != CodePoint::from('%') {
                i += 1;
                continue;
            }
            let end = i;
            i += 1;
            if i == flen {
                break;
            }
            let ch = chars[i].to_char();
            i += 1;
            let replacement: Wtf8Buf = match ch {
                Some('z') => {
                    if zreplacement.is_none() {
                        zreplacement = Some(make_somezreplacement(object, "", tzinfoarg, vm)?);
                    }
                    Wtf8Buf::from(zreplacement.as_deref().unwrap())
                }
                Some(':') if i < flen && chars[i] == CodePoint::from('z') => {
                    i += 1;
                    if colonzreplacement.is_none() {
                        colonzreplacement =
                            Some(make_somezreplacement(object, ":", tzinfoarg, vm)?);
                    }
                    Wtf8Buf::from(colonzreplacement.as_deref().unwrap())
                }
                Some('Z') => {
                    if zname.is_none() {
                        zname = Some(make_zreplacement(object, tzinfoarg, vm)?);
                    }
                    zname.clone().unwrap()
                }
                Some('f') => {
                    if freplacement.is_none() {
                        let us = as_datetime(object)
                            .map(|dt| dt.microsecond)
                            .or_else(|| as_time(object).map(|t| t.microsecond))
                            .unwrap_or(0);
                        freplacement = Some(format!("{us:06}"));
                    }
                    Wtf8Buf::from(freplacement.as_deref().unwrap())
                }
                Some(c @ ('Y' | 'G' | 'F' | 'C')) => {
                    // Zero-pad the year, which the platform strftime may not do.
                    let item = timetuple.get_item(vm.ctx.new_int(0).as_object(), vm)?;
                    let mut year: i64 = item.try_into_value(vm)?;
                    if year >= 1000 {
                        continue;
                    }
                    if c == 'G' {
                        let year_str = strftime.call(("%G", timetuple.clone()), vm)?;
                        year = vm
                            .ctx
                            .types
                            .int_type
                            .as_object()
                            .call((year_str,), vm)?
                            .try_into_value(vm)?;
                    }
                    let mut buf = if c == 'F' {
                        format!("{year:04}-%m-%d")
                    } else {
                        format!("{year:04}")
                    };
                    if c == 'C' {
                        buf.truncate(buf.len() - 2);
                    }
                    Wtf8Buf::from(buf.as_str())
                }
                _ => continue,
            };
            push_range(&mut out, start, end);
            start = i;
            out.push_wtf8(&replacement);
            changed = true;
        }
        let newformat: PyObjectRef = if changed {
            push_range(&mut out, start, flen);
            vm.ctx.new_str(out).into()
        } else {
            format.to_owned().into()
        };
        strftime.call((newformat, timetuple), vm)
    }

    fn make_somezreplacement(
        object: &PyObject,
        sep: &str,
        tzinfoarg: &PyObject,
        vm: &VirtualMachine,
    ) -> PyResult<String> {
        let tzinfo = get_tzinfo_member(object);
        if tzinfo.is_none() {
            return Ok(String::new());
        }
        format_utcoffset(sep, tzinfo.as_ref(), tzinfoarg, vm)
    }

    fn make_zreplacement(
        object: &PyObject,
        tzinfoarg: &PyObject,
        vm: &VirtualMachine,
    ) -> PyResult<Wtf8Buf> {
        let tzinfo = get_tzinfo_member(object);
        if tzinfo.is_none() {
            return Ok(Wtf8Buf::new());
        }
        let Some(name) = call_tzname(tzinfo.as_ref(), tzinfoarg, vm)? else {
            return Ok(Wtf8Buf::new());
        };
        // The name goes into the format: double its `%` signs.
        let replaced = vm.call_method(name.as_object(), "replace", ("%", "%%"))?;
        let replaced = replaced
            .downcast::<PyStr>()
            .map_err(|_| vm.new_type_error("tzname.replace() did not return a string"))?;
        Ok(replaced.as_wtf8().to_owned())
    }

    fn strftime_format_arg(format: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        format.downcast::<PyStr>().map_err(|format| {
            vm.new_type_error(format!(
                "strftime() argument 1 must be str, not {}",
                type_name(&format)
            ))
        })
    }

    /// `date_format`: an empty spec is `str(self)`, anything else goes to `self.strftime`.
    fn object_format(zelf: &PyObject, spec: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let spec = spec.downcast::<PyStr>().map_err(|spec| {
            vm.new_type_error(format!(
                "__format__() argument 1 must be str, not {}",
                type_name(&spec)
            ))
        })?;
        if spec.as_wtf8().is_empty() {
            return Ok(zelf.str(vm)?.into());
        }
        vm.call_method(zelf, "strftime", (spec,))
    }

    fn call_strptime(
        cls: &Py<PyType>,
        which: &str,
        args: (PyObjectRef, PyObjectRef),
        vm: &VirtualMachine,
    ) -> PyResult {
        let (string, format) = args;
        for (arg, n) in [(&string, 1), (&format, 2)] {
            if arg.downcast_ref::<PyStr>().is_none() {
                return Err(vm.new_type_error(format!(
                    "strptime() argument {n} must be str, not {}",
                    type_name(arg)
                )));
            }
        }
        let module = vm.import("_strptime", 0)?;
        vm.call_method(&module, which, (cls.to_owned(), string, format))
    }

    // ISO 8601 parsing, on UTF-8 bytes read as if NUL-terminated

    fn byte_at(s: &[u8], i: usize) -> u8 {
        s.get(i).copied().unwrap_or(0)
    }

    fn is_digit(c: u8) -> bool {
        c.is_ascii_digit()
    }

    /// Parse `num_digits` digits at `*p` into `var`; `false` on a non-digit.
    fn parse_digits(s: &[u8], p: &mut usize, var: &mut i32, num_digits: usize) -> bool {
        for _ in 0..num_digits {
            let c = byte_at(s, *p);
            *p += 1;
            let tmp = c.wrapping_sub(b'0');
            if tmp > 9 {
                return false;
            }
            *var = *var * 10 + i32::from(tmp);
        }
        true
    }

    /// `parse_isoformat_date`: 0 on success, CPython's negative codes on failure.
    fn parse_isoformat_date(
        s: &[u8],
        len: usize,
        year: &mut i32,
        month: &mut i32,
        day: &mut i32,
    ) -> i32 {
        let mut p = 0;
        if !parse_digits(s, &mut p, year, 4) {
            return -1;
        }
        let uses_separator = byte_at(s, p) == b'-';
        if uses_separator {
            p += 1;
        }
        if byte_at(s, p) == b'W' {
            p += 1;
            let (mut iso_week, mut iso_day) = (0, 0);
            if !parse_digits(s, &mut p, &mut iso_week, 2) {
                return -3;
            }
            if p < len {
                if uses_separator {
                    let c = byte_at(s, p);
                    p += 1;
                    if c != b'-' {
                        return -2;
                    }
                }
                if !parse_digits(s, &mut p, &mut iso_day, 1) {
                    return -4;
                }
            } else {
                iso_day = 1;
            }
            return match iso_to_ymd(*year, iso_week, iso_day) {
                Ok((y, m, d)) => {
                    (*year, *month, *day) = (y, m, d);
                    0
                }
                Err(rv) => -3 + rv,
            };
        }
        if !parse_digits(s, &mut p, month, 2) {
            return -1;
        }
        if uses_separator {
            let c = byte_at(s, p);
            p += 1;
            if c != b'-' {
                return -2;
            }
        }
        if !parse_digits(s, &mut p, day, 2) {
            return -1;
        }
        0
    }

    /// `parse_hh_mm_ss_ff` over `s[start..end]`; 0/1 on success (1: text remains).
    fn parse_hh_mm_ss_ff(s: &[u8], start: usize, end: usize, vals: &mut [i32; 4]) -> i32 {
        *vals = [0; 4];
        let mut p = start;
        let mut has_separator = true;
        for (i, val) in vals.iter_mut().take(3).enumerate() {
            if !parse_digits(s, &mut p, val, 2) {
                return -3;
            }
            let c = byte_at(s, p);
            p += 1;
            if i == 0 {
                has_separator = c == b':';
            }
            if c == b'.' || c == b',' {
                if i < 2 {
                    return -3;
                }
                if p >= end {
                    // A decimal mark with no digit after it.
                    return -3;
                }
                break;
            } else if p >= end {
                return i32::from(c != 0);
            } else if has_separator && c == b':' {
                if i == 2 {
                    return -4;
                }
                continue;
            } else if !has_separator {
                p -= 1;
            } else {
                return -4;
            }
        }
        let len_remains = end.saturating_sub(p);
        let to_parse = len_remains.min(6);
        if !parse_digits(s, &mut p, &mut vals[3], to_parse) {
            return -3;
        }
        const CORRECTION: [i32; 5] = [100_000, 10_000, 1000, 100, 10];
        if to_parse < 6 {
            vals[3] *= CORRECTION[to_parse - 1];
        }
        while is_digit(byte_at(s, p)) {
            p += 1;
        }
        i32::from(byte_at(s, p) != 0)
    }

    /// `parse_isoformat_time` over `s[start..start + len]`: 0 (naive), 1 (with offset) or `< 0`.
    fn parse_isoformat_time(
        s: &[u8],
        start: usize,
        len: usize,
        vals: &mut [i32; 4],
        tzoffset: &mut i32,
        tzmicrosecond: &mut i32,
    ) -> i32 {
        let p_end = start + len;
        let mut tzinfo_pos = start;
        loop {
            let c = byte_at(s, tzinfo_pos);
            if c == b'Z' || c == b'+' || c == b'-' {
                break;
            }
            tzinfo_pos += 1;
            if tzinfo_pos >= p_end {
                break;
            }
        }
        let rv = parse_hh_mm_ss_ff(s, start, tzinfo_pos, vals);
        if rv < 0 {
            return rv;
        } else if tzinfo_pos == p_end {
            return if rv == 1 { -5 } else { 0 };
        }
        if byte_at(s, tzinfo_pos) == b'Z' {
            *tzoffset = 0;
            *tzmicrosecond = 0;
            return if byte_at(s, tzinfo_pos + 1) != 0 {
                -5
            } else {
                1
            };
        }
        let tzsign = if byte_at(s, tzinfo_pos) == b'-' {
            -1
        } else {
            1
        };
        let mut tz = [0; 4];
        let rv = parse_hh_mm_ss_ff(s, tzinfo_pos + 1, p_end, &mut tz);
        *tzoffset = tzsign * (tz[0] * 3600 + tz[1] * 60 + tz[2]);
        *tzmicrosecond = tzsign * tz[3];
        if rv != 0 { -5 } else { 1 }
    }

    fn tzinfo_from_isoformat_results(
        rv: i32,
        tzoffset: i32,
        tz_useconds: i32,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>> {
        if rv != 1 {
            return Ok(None);
        }
        if tzoffset == 0 && tz_useconds == 0 {
            return Ok(Some(utc().to_owned().into()));
        }
        let delta = PyDelta::new_ref(0, i64::from(tzoffset), i64::from(tz_useconds), true, vm)?;
        Ok(Some(new_timezone(delta, None, vm)?.into()))
    }

    fn invalid_isoformat(
        dtstr: &PyObject,
        vm: &VirtualMachine,
    ) -> PyResult<crate::vm::builtins::PyBaseExceptionRef> {
        Ok(vm.new_value_error(format!("Invalid isoformat string: {}", dtstr.repr(vm)?)))
    }

    /// The UTF-8 bytes of a `str`; `None` when it holds lone surrogates.
    fn str_utf8(s: &PyStr) -> Option<&[u8]> {
        s.to_str().map(str::as_bytes)
    }

    // date

    #[pyattr]
    #[pyclass(module = "datetime", name = "date")]
    #[derive(Debug, PyPayload)]
    pub(super) struct PyDate {
        #[pymember]
        year: u16,
        #[pymember]
        month: u8,
        #[pymember]
        day: u8,
        hashcode: AtomicCell<PyHash>,
    }

    impl PyDate {
        fn new(year: i32, month: i32, day: i32) -> Self {
            Self {
                year: year as u16,
                month: month as u8,
                day: day as u8,
                hashcode: AtomicCell::new(-1),
            }
        }

        fn y(&self) -> i32 {
            i32::from(self.year)
        }

        fn m(&self) -> i32 {
            i32::from(self.month)
        }

        fn d(&self) -> i32 {
            i32::from(self.day)
        }

        fn ordinal(&self) -> i32 {
            ymd_to_ord(self.y(), self.m(), self.d())
        }

        fn state_bytes(&self) -> [u8; 4] {
            [
                (self.year >> 8) as u8,
                self.year as u8,
                self.month,
                self.day,
            ]
        }

        fn from_state(data: &[u8]) -> Self {
            Self::new(
                i32::from(data[0]) << 8 | i32::from(data[1]),
                i32::from(data[2]),
                i32::from(data[3]),
            )
        }

        fn key(&self) -> (u16, u8, u8) {
            (self.year, self.month, self.day)
        }
    }

    /// `new_date_subclass_ex`: plain types directly, subclasses through their constructor.
    fn new_date_subclass(
        y: i32,
        m: i32,
        d: i32,
        cls: &Py<PyType>,
        vm: &VirtualMachine,
    ) -> PyResult {
        if cls.is(date_type()) {
            check_date_args(y, m, d, vm)?;
            Ok(PyDate::new(y, m, d).into_ref(&vm.ctx).into())
        } else if cls.is(datetime_type()) {
            Ok(new_datetime(y, m, d, 0, 0, 0, 0, None, 0, datetime_type(), vm)?.into())
        } else {
            cls.as_object().call((y, m, d), vm)
        }
    }

    /// Pickle state from a `str` holding bytes as latin-1 code points.
    fn latin1_state(state: &PyStr, what: &str, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        state
            .as_wtf8()
            .code_points()
            .map(|c| u8::try_from(c.to_u32()).ok())
            .collect::<Option<Vec<u8>>>()
            .ok_or_else(|| {
                vm.new_value_error(format!(
                    "Failed to encode latin1 string when unpickling a {what} object. \
                     pickle.load(data, encoding='latin1') is assumed."
                ))
            })
    }

    /// The first byte-or-latin-1 state argument, if its length and sanity check fit.
    fn pickle_state(
        state: &PyObject,
        size: usize,
        sanity_index: usize,
        sane: impl Fn(u32) -> bool,
        what: &str,
        vm: &VirtualMachine,
    ) -> PyResult<Option<Vec<u8>>> {
        if let Some(bytes) = state.downcast_ref::<PyBytes>() {
            let data = bytes.as_bytes();
            if data.len() == size && sane(u32::from(data[sanity_index])) {
                return Ok(Some(data.to_vec()));
            }
        } else if let Some(s) = state.downcast_ref::<PyStr>()
            && s.char_len() == size
            && s.as_wtf8()
                .code_points()
                .nth(sanity_index)
                .is_some_and(|c| sane(c.to_u32()))
        {
            return latin1_state(s, what, vm).map(Some);
        }
        Ok(None)
    }

    fn month_is_sane(m: u32) -> bool {
        (1..=12).contains(&m)
    }

    #[derive(FromArgs)]
    struct DateArgs {
        #[pyarg(any)]
        year: i32,
        #[pyarg(any)]
        month: i32,
        #[pyarg(any)]
        day: i32,
    }

    impl Constructor for PyDate {
        type Args = FuncArgs;

        fn py_new(_cls: &Py<PyType>, args: FuncArgs, vm: &VirtualMachine) -> PyResult<Self> {
            if args.args.len() == 1
                && let Some(state) = pickle_state(&args.args[0], 4, 2, month_is_sane, "date", vm)?
            {
                return Ok(Self::from_state(&state));
            }
            let DateArgs { year, month, day } = args.bind(vm)?;
            check_date_args(year, month, day, vm)?;
            Ok(Self::new(year, month, day))
        }
    }

    /// `_PyTime_ObjectToTime_t` with floor rounding.
    fn object_to_time_t(obj: &PyObject, vm: &VirtualMachine) -> PyResult<i64> {
        if let Some(float) = obj.downcast_ref::<PyFloat>() {
            let d = float.to_f64();
            if d.is_nan() {
                return Err(vm.new_value_error("Invalid value NaN (not a number)"));
            }
            let d = d.floor();
            if !(-9.223_372_036_854_776e18..9.223_372_036_854_776e18).contains(&d) {
                return Err(time_t_overflow(vm));
            }
            return Ok(d as i64);
        }
        long_to_time_t(obj, vm)
    }

    fn time_t_overflow(vm: &VirtualMachine) -> crate::vm::builtins::PyBaseExceptionRef {
        vm.new_overflow_error("timestamp out of range for platform time_t")
    }

    /// `_PyLong_AsTime_t`
    fn long_to_time_t(obj: &PyObject, vm: &VirtualMachine) -> PyResult<i64> {
        let int = obj.try_index(vm)?;
        int.as_bigint().to_i64().ok_or_else(|| time_t_overflow(vm))
    }

    /// `_PyTime_ObjectToTimeval` with half-even rounding: whole seconds and microseconds.
    fn object_to_timeval(obj: &PyObject, vm: &VirtualMachine) -> PyResult<(i64, i32)> {
        if let Some(float) = obj.downcast_ref::<PyFloat>() {
            let d = float.to_f64();
            if d.is_nan() {
                return Err(vm.new_value_error("Invalid value NaN (not a number)"));
            }
            let mut intpart = d.trunc();
            let mut floatpart = (d - intpart) * 1e6;
            floatpart = round_half_even(floatpart);
            if floatpart >= 1e6 {
                floatpart -= 1e6;
                intpart += 1.0;
            } else if floatpart < 0.0 {
                floatpart += 1e6;
                intpart -= 1.0;
            }
            if !(-9.223_372_036_854_776e18..9.223_372_036_854_776e18).contains(&intpart) {
                return Err(time_t_overflow(vm));
            }
            return Ok((intpart as i64, floatpart as i32));
        }
        Ok((long_to_time_t(obj, vm)?, 0))
    }

    fn round_half_even(x: f64) -> f64 {
        let rounded = x.round();
        if (x - rounded).abs() == 0.5 {
            2.0 * (x / 2.0).round()
        } else {
            rounded
        }
    }

    /// Broken-down time from a platform call.
    #[derive(Clone, Copy)]
    struct Tm {
        year: i32,
        month: i32,
        day: i32,
        hour: i32,
        minute: i32,
        second: i32,
    }

    /// `_PyTime_localtime` / `_PyTime_gmtime`.
    fn platform_time(t: i64, local: bool, vm: &VirtualMachine) -> PyResult<Tm> {
        #[cfg(any(unix, windows))]
        {
            let t_c = libc::time_t::try_from(t).map_err(|_| time_t_overflow(vm))?;
            let tm = if local {
                rustpython_host_env::time::localtime_from_timestamp(t_c)
            } else {
                rustpython_host_env::time::gmtime_from_timestamp(t_c)
            };
            let Some(tm) = tm else {
                return Err(vm.new_last_errno_error());
            };
            Ok(Tm {
                year: tm.tm_year + 1900,
                month: tm.tm_mon + 1,
                day: tm.tm_mday,
                hour: tm.tm_hour,
                minute: tm.tm_min,
                second: tm.tm_sec,
            })
        }
        #[cfg(not(any(unix, windows)))]
        {
            let func = if local { "localtime" } else { "gmtime" };
            let st = vm.call_method(&*vm.import("time", 0)?, func, (t,))?;
            let field = |i: usize| -> PyResult<i32> {
                st.get_item(vm.ctx.new_int(i).as_object(), vm)?
                    .try_into_value(vm)
            };
            Ok(Tm {
                year: field(0)?,
                month: field(1)?,
                day: field(2)?,
                hour: field(3)?,
                minute: field(4)?,
                second: field(5)?,
            })
        }
    }

    /// `date(1970, 1, 1).toordinal()` in seconds.
    const EPOCH_SECONDS: i64 = 719_163 * 24 * 60 * 60;
    /// Largest fold in the IANA database, used to probe for folds.
    const MAX_FOLD_SECONDS: i64 = 24 * 3600;

    fn utc_to_seconds(
        y: i32,
        m: i32,
        d: i32,
        hh: i32,
        mm: i32,
        ss: i32,
        vm: &VirtualMachine,
    ) -> PyResult<i64> {
        if !(MINYEAR..=MAXYEAR).contains(&y) {
            return Err(
                vm.new_value_error(format!("year must be in {MINYEAR}..{MAXYEAR}, not {y}"))
            );
        }
        let ordinal = i64::from(ymd_to_ord(y, m, d));
        Ok(((ordinal * 24 + i64::from(hh)) * 60 + i64::from(mm)) * 60 + i64::from(ss))
    }

    /// `local`: seconds since 0001-01-01 of the local time at `u` (seconds, same origin).
    fn local(u: i64, vm: &VirtualMachine) -> PyResult<i64> {
        let tm = platform_time(u - EPOCH_SECONDS, true, vm)?;
        utc_to_seconds(tm.year, tm.month, tm.day, tm.hour, tm.minute, tm.second, vm)
    }

    /// `local_to_seconds`: the UTC seconds of a local wall time, honouring `fold`.
    #[allow(clippy::too_many_arguments)]
    fn local_to_seconds(
        y: i32,
        m: i32,
        d: i32,
        hh: i32,
        mm: i32,
        ss: i32,
        fold: bool,
        vm: &VirtualMachine,
    ) -> PyResult<i64> {
        let t = utc_to_seconds(y, m, d, hh, mm, ss, vm)?;
        let lt = local(t, vm)?;
        let a = lt - t;
        let u1 = t - a;
        let t1 = local(u1, vm)?;
        let b;
        if t1 == t {
            let u2 = if fold {
                u1 + MAX_FOLD_SECONDS
            } else {
                u1 - MAX_FOLD_SECONDS
            };
            let lt = local(u2, vm)?;
            b = lt - u2;
            if a == b {
                return Ok(u1);
            }
        } else {
            b = t1 - u1;
        }
        let u2 = t - b;
        let t2 = local(u2, vm)?;
        if t2 == t {
            return Ok(u2);
        }
        if t1 == t {
            return Ok(u1);
        }
        Ok(if fold { u1.min(u2) } else { u1.max(u2) })
    }

    #[pyclass(
        with(Constructor, AsNumber, Comparable, Hashable, Representable),
        flags(BASETYPE)
    )]
    impl PyDate {
        #[pyclassmethod]
        fn fromtimestamp(cls: PyTypeRef, timestamp: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            let t = object_to_time_t(&timestamp, vm)?;
            let tm = platform_time(t, true, vm)?;
            new_date_subclass(tm.year, tm.month, tm.day, &cls, vm)
        }

        #[pyclassmethod]
        fn today(cls: PyTypeRef, vm: &VirtualMachine) -> PyResult {
            let now = vm.call_method(&*vm.import("time", 0)?, "time", ())?;
            vm.call_method(cls.as_object(), "fromtimestamp", (now,))
        }

        #[pyclassmethod]
        fn fromordinal(cls: PyTypeRef, ordinal: i32, vm: &VirtualMachine) -> PyResult {
            if ordinal < 1 {
                return Err(vm.new_value_error("ordinal must be >= 1"));
            }
            let (y, m, d) = ord_to_ymd(ordinal);
            new_date_subclass(y, m, d, &cls, vm)
        }

        #[pyclassmethod]
        fn fromisoformat(cls: PyTypeRef, object: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            let dtstr = object;
            let Some(s) = dtstr.downcast_ref::<PyStr>() else {
                return Err(vm.new_type_error("fromisoformat: argument must be str"));
            };
            let Some(bytes) = str_utf8(s) else {
                return Err(invalid_isoformat(&dtstr, vm)?);
            };
            let len = bytes.len();
            let (mut y, mut m, mut d) = (0, 0, 0);
            let rv = if matches!(len, 7 | 8 | 10) {
                parse_isoformat_date(bytes, len, &mut y, &mut m, &mut d)
            } else {
                -1
            };
            if rv < 0 {
                return Err(invalid_isoformat(&dtstr, vm)?);
            }
            new_date_subclass(y, m, d, &cls, vm)
        }

        #[pyclassmethod]
        fn fromisocalendar(cls: PyTypeRef, args: IsoCalendarArgs, vm: &VirtualMachine) -> PyResult {
            let to_i32 = |obj: PyObjectRef| -> PyResult<i32> {
                obj.try_into_value::<i32>(vm).map_err(|err| {
                    if err.fast_isinstance(vm.ctx.exceptions.overflow_error) {
                        vm.new_value_error("ISO calendar component out of range")
                    } else {
                        err
                    }
                })
            };
            let (year, week, day) = (to_i32(args.year)?, to_i32(args.week)?, to_i32(args.day)?);
            match iso_to_ymd(year, week, day) {
                Ok((y, m, d)) => new_date_subclass(y, m, d, &cls, vm),
                Err(-4) => Err(
                    vm.new_value_error(format!("year must be in {MINYEAR}..{MAXYEAR}, not {year}"))
                ),
                Err(-2) => Err(vm.new_value_error(format!("Invalid week: {week}"))),
                Err(_) => {
                    Err(vm.new_value_error(format!("Invalid weekday: {day} (range is [1, 7])")))
                }
            }
        }

        #[pyclassmethod]
        fn strptime(
            cls: PyTypeRef,
            string: PyObjectRef,
            format: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult {
            call_strptime(&cls, "_strptime_datetime_date", (string, format), vm)
        }

        #[pymethod]
        fn ctime(zelf: &Py<Self>) -> String {
            format_ctime(zelf.y(), zelf.m(), zelf.d(), 0, 0, 0)
        }

        #[pymethod]
        fn strftime(zelf: PyObjectRef, args: StrftimeArgs, vm: &VirtualMachine) -> PyResult {
            let format = strftime_format_arg(args.format, vm)?;
            let tuple = vm.call_method(&zelf, "timetuple", ())?;
            wrap_strftime(&zelf, &format, tuple, &zelf, vm)
        }

        #[pymethod]
        fn __format__(zelf: PyObjectRef, spec: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            object_format(&zelf, spec, vm)
        }

        #[pymethod]
        fn timetuple(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult {
            build_struct_time(zelf.y(), zelf.m(), zelf.d(), 0, 0, 0, -1, vm)
        }

        #[pymethod]
        fn isocalendar(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult {
            let mut year = zelf.y();
            let mut week1_monday = iso_week1_monday(year);
            let today = zelf.ordinal();
            let (mut week, mut day) = divmod(i64::from(today - week1_monday), 7);
            if week < 0 {
                year -= 1;
                week1_monday = iso_week1_monday(year);
                (week, day) = divmod(i64::from(today - week1_monday), 7);
            } else if week >= 52 && today >= iso_week1_monday(year + 1) {
                year += 1;
                week = 0;
            }
            let items = vec![
                vm.ctx.new_int(year).into(),
                vm.ctx.new_int(week + 1).into(),
                vm.ctx.new_int(day + 1).into(),
            ];
            Ok(PyTuple::new_unchecked(items.into_boxed_slice())
                .into_ref_with_type(vm, PyIsoCalendarDate::make_static_type())?
                .into())
        }

        #[pymethod]
        fn isoformat(zelf: &Py<Self>) -> String {
            format!("{:04}-{:02}-{:02}", zelf.year, zelf.month, zelf.day)
        }

        #[pymethod]
        fn isoweekday(zelf: &Py<Self>) -> i32 {
            weekday(zelf.y(), zelf.m(), zelf.d()) + 1
        }

        #[pymethod]
        fn toordinal(zelf: &Py<Self>) -> i32 {
            zelf.ordinal()
        }

        #[pymethod]
        fn weekday(zelf: &Py<Self>) -> i32 {
            weekday(zelf.y(), zelf.m(), zelf.d())
        }

        #[pymethod]
        fn replace(zelf: &Py<Self>, args: DateReplaceArgs, vm: &VirtualMachine) -> PyResult {
            new_date_subclass(
                args.year.unwrap_or_else(|| zelf.y()),
                args.month.unwrap_or_else(|| zelf.m()),
                args.day.unwrap_or_else(|| zelf.d()),
                zelf.class(),
                vm,
            )
        }

        #[pymethod]
        fn __replace__(zelf: &Py<Self>, changes: ReplaceChanges, vm: &VirtualMachine) -> PyResult {
            Self::replace(zelf, changes.args.bind_for(vm, "replace")?, vm)
        }

        #[pymethod]
        fn __reduce__(zelf: &Py<Self>, vm: &VirtualMachine) -> PyRef<PyTuple> {
            let state = vm.ctx.new_bytes(zelf.state_bytes().to_vec());
            vm.new_tuple((zelf.class().to_owned(), (state,)))
        }

        #[pyslot]
        fn slot_str(zelf: &PyObject, vm: &VirtualMachine) -> PyResult<PyStrRef> {
            checked_str_result(vm.call_method(zelf, "isoformat", ())?, vm)
        }

        #[extend_class]
        fn extend_class(ctx: &crate::vm::Context, class: &Py<PyType>) {
            let make = |y, m, d| PyRef::new_ref(Self::new(y, m, d), class.to_owned(), None).into();
            class.set_attr(ctx.intern_str("min"), make(1, 1, 1));
            class.set_attr(ctx.intern_str("max"), make(MAXYEAR, 12, 31));
            let resolution = PyRef::new_ref(
                PyDelta::new_unchecked(1, 0, 0),
                PyDelta::static_type().to_owned(),
                None,
            );
            class.set_attr(ctx.intern_str("resolution"), resolution.into());
        }
    }

    #[derive(FromArgs)]
    struct StrftimeArgs {
        #[pyarg(any)]
        format: PyObjectRef,
    }

    #[derive(FromArgs)]
    struct IsoCalendarArgs {
        #[pyarg(any)]
        year: PyObjectRef,
        #[pyarg(any)]
        week: PyObjectRef,
        #[pyarg(any)]
        day: PyObjectRef,
    }

    /// `**changes` in the text signature. Binding still accepts the same
    /// arguments as `replace`, and a failure names `replace`.
    struct ReplaceChanges {
        args: FuncArgs,
    }

    impl FromArgs for ReplaceChanges {
        const PARAMS: Option<&'static [Param]> = Some(&[Param::var_keyword("changes")]);

        fn from_args(_vm: &VirtualMachine, args: &mut FuncArgs) -> Result<Self, ArgumentError> {
            Ok(Self {
                args: core::mem::take(args),
            })
        }
    }

    #[derive(FromArgs)]
    struct DateReplaceArgs {
        #[pyarg(any, optional, py_default = "unchanged")]
        year: OptionalArg<i32>,
        #[pyarg(any, optional, py_default = "unchanged")]
        month: OptionalArg<i32>,
        #[pyarg(any, optional, py_default = "unchanged")]
        day: OptionalArg<i32>,
    }

    impl PyDate {
        /// `add_date_timedelta`
        fn add_delta(
            zelf: &Py<Self>,
            delta: &PyDelta,
            negate: bool,
            vm: &VirtualMachine,
        ) -> PyResult {
            let (mut y, mut m) = (i64::from(zelf.year), i64::from(zelf.month));
            let deltadays = i64::from(delta.days);
            let mut d = i64::from(zelf.day) + if negate { -deltadays } else { deltadays };
            normalize_y_m_d(&mut y, &mut m, &mut d).map_err(|()| date_overflow(vm))?;
            new_date_subclass(y as i32, m as i32, d as i32, zelf.class(), vm)
        }

        fn nb_add(a: &PyObject, b: &PyObject, vm: &VirtualMachine) -> PyResult {
            if as_datetime(a).is_some() || as_datetime(b).is_some() {
                return Ok(vm.ctx.not_implemented());
            }
            if let Some(date) = as_date(a) {
                if let Some(delta) = as_delta(b) {
                    return Self::add_delta(date, delta, false, vm);
                }
            } else if let (Some(delta), Some(date)) = (as_delta(a), as_date(b)) {
                return Self::add_delta(date, delta, false, vm);
            }
            Ok(vm.ctx.not_implemented())
        }

        fn nb_subtract(a: &PyObject, b: &PyObject, vm: &VirtualMachine) -> PyResult {
            if as_datetime(a).is_some() || as_datetime(b).is_some() {
                return Ok(vm.ctx.not_implemented());
            }
            if let Some(left) = as_date(a) {
                if let Some(right) = as_date(b) {
                    let days = i64::from(left.ordinal()) - i64::from(right.ordinal());
                    return Ok(PyDelta::new_ref(days, 0, 0, false, vm)?.into());
                }
                if let Some(delta) = as_delta(b) {
                    return Self::add_delta(left, delta, true, vm);
                }
            }
            Ok(vm.ctx.not_implemented())
        }
    }

    impl AsNumber for PyDate {
        fn as_number() -> &'static PyNumberMethods {
            static AS_NUMBER: PyNumberMethods = PyNumberMethods {
                add: Some(PyDate::nb_add),
                subtract: Some(PyDate::nb_subtract),
                ..PyNumberMethods::NOT_IMPLEMENTED
            };
            &AS_NUMBER
        }
    }

    impl Comparable for PyDate {
        fn cmp(
            zelf: &Py<Self>,
            other: &PyObject,
            op: PyComparisonOp,
            _vm: &VirtualMachine,
        ) -> PyResult<PyComparisonValue> {
            // A datetime is a date, but comparing only its date part would be wrong.
            match as_date(other) {
                Some(other) if as_datetime(other.as_object()).is_none() => {
                    Ok(op.eval_ord(zelf.key().cmp(&other.key())).into())
                }
                _ => Ok(PyComparisonValue::NotImplemented),
            }
        }
    }

    impl Hashable for PyDate {
        fn hash(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyHash> {
            let cached = zelf.hashcode.load();
            if cached != -1 {
                return Ok(cached);
            }
            let hash = vm
                .ctx
                .new_bytes(zelf.state_bytes().to_vec())
                .as_object()
                .hash(vm)?;
            zelf.hashcode.store(hash);
            Ok(hash)
        }
    }

    impl Representable for PyDate {
        fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            Ok(format!(
                "{}({}, {}, {})",
                type_name(zelf.as_object()),
                zelf.year,
                zelf.month,
                zelf.day
            ))
        }
    }

    // IsoCalendarDate

    #[pyclass(module = "datetime", name = "IsoCalendarDate", base = PyTuple, no_attr)]
    #[repr(transparent)]
    #[derive(Debug)]
    pub(super) struct PyIsoCalendarDate(PyTuple);

    impl PyIsoCalendarDate {
        fn item(&self, i: usize) -> PyObjectRef {
            self.0.as_slice()[i].clone()
        }
    }

    #[derive(FromArgs)]
    pub(super) struct IsoCalendarNewArgs {
        #[pyarg(any)]
        year: i32,
        #[pyarg(any)]
        week: i32,
        #[pyarg(any)]
        weekday: i32,
    }

    impl Constructor for PyIsoCalendarDate {
        type Args = IsoCalendarNewArgs;

        fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            let items: Vec<PyObjectRef> = vec![
                vm.ctx.new_int(args.year).into(),
                vm.ctx.new_int(args.week).into(),
                vm.ctx.new_int(args.weekday).into(),
            ];
            Ok(Self(PyTuple::new_unchecked(items.into_boxed_slice())))
        }
    }

    impl Representable for PyIsoCalendarDate {
        fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
            Ok(format!(
                "{}(year={}, week={}, weekday={})",
                type_name(zelf.as_object()),
                zelf.item(0).str(vm)?,
                zelf.item(1).str(vm)?,
                zelf.item(2).str(vm)?
            ))
        }
    }

    #[pyclass(with(Constructor, Representable), flags(IMMUTABLETYPE))]
    impl PyIsoCalendarDate {
        #[pygetset]
        fn year(zelf: &Py<Self>) -> PyObjectRef {
            zelf.item(0)
        }

        #[pygetset]
        fn week(zelf: &Py<Self>) -> PyObjectRef {
            zelf.item(1)
        }

        #[pygetset]
        fn weekday(zelf: &Py<Self>) -> PyObjectRef {
            zelf.item(2)
        }

        #[pymethod]
        fn __reduce__(zelf: &Py<Self>, vm: &VirtualMachine) -> PyRef<PyTuple> {
            let items = vm.new_tuple((zelf.item(0), zelf.item(1), zelf.item(2)));
            vm.new_tuple((vm.ctx.types.tuple_type.to_owned(), (items,)))
        }
    }

    // tzinfo

    #[pyattr]
    #[pyclass(module = "datetime", name = "tzinfo")]
    #[derive(Debug, Default, PyPayload)]
    pub(crate) struct PyTzInfo {}

    impl Constructor for PyTzInfo {
        type Args = FuncArgs;

        fn py_new(_cls: &Py<PyType>, _args: FuncArgs, _vm: &VirtualMachine) -> PyResult<Self> {
            Ok(Self::default())
        }
    }

    fn tzinfo_nogo(name: &str, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_not_implemented_error(format!("a tzinfo subclass must implement {name}()")))
    }

    #[pyclass(with(Constructor), flags(BASETYPE))]
    impl PyTzInfo {
        #[pymethod]
        fn tzname(_zelf: &Py<Self>, _object: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            tzinfo_nogo("tzname", vm)
        }

        #[pymethod]
        fn utcoffset(_zelf: &Py<Self>, _object: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            tzinfo_nogo("utcoffset", vm)
        }

        #[pymethod]
        fn dst(_zelf: &Py<Self>, _object: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            tzinfo_nogo("dst", vm)
        }

        #[pymethod]
        fn fromutc(zelf: PyObjectRef, object: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            let dt = object;
            let Some(dtobj) = as_datetime(&dt) else {
                return Err(vm.new_type_error("fromutc: argument must be a datetime"));
            };
            if !dtobj.tzinfo.load_owned().is_some_and(|tz| tz.is(&zelf)) {
                return Err(vm.new_value_error("fromutc: dt.tzinfo is not self"));
            }
            let Some(off) = call_utcoffset(Some(&zelf), &dt, vm)? else {
                return Err(vm.new_value_error("fromutc: non-None utcoffset() result required"));
            };
            let Some(dst) = call_dst(Some(&zelf), &dt, vm)? else {
                return Err(vm.new_value_error("fromutc: non-None dst() result required"));
            };
            let delta = PyDelta::new_ex(
                i64::from(off.days) - i64::from(dst.days),
                i64::from(off.seconds) - i64::from(dst.seconds),
                i64::from(off.microseconds) - i64::from(dst.microseconds),
                true,
                vm,
            )?;
            let result = PyDateTime::add_delta(dtobj, &delta, 1, vm)?;
            let Some(dst) = call_dst(Some(&zelf), &result, vm)? else {
                return Err(vm.new_value_error(
                    "fromutc: tz.dst() gave inconsistent results; cannot convert",
                ));
            };
            if dst.is_nonzero() {
                let result_dt = as_datetime(&result).ok_or_else(|| {
                    vm.new_type_error("datetime arithmetic must return a datetime")
                })?;
                return PyDateTime::add_delta(result_dt, &dst, 1, vm);
            }
            Ok(result)
        }

        #[pymethod]
        fn __reduce__(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyRef<PyTuple>> {
            let args = match vm.get_attribute_opt(&zelf, "__getinitargs__")? {
                Some(getinitargs) => getinitargs.call((), vm)?,
                None => vm.ctx.empty_tuple.clone().into(),
            };
            let state = vm.call_method(&zelf, "__getstate__", ())?;
            Ok(vm.new_tuple((zelf.class().to_owned(), args, state)))
        }
    }

    // timezone

    #[pyattr]
    #[pyclass(module = "datetime", name = "timezone", base = PyTzInfo, traverse)]
    #[derive(Debug)]
    pub(super) struct PyTimeZone {
        #[pytraverse(skip)]
        base: PyTzInfo,
        offset: PyRef<PyDelta>,
        name: Option<PyStrRef>,
    }

    fn utc() -> &'static Py<PyTimeZone> {
        UTC.get().expect("the timezone type creates UTC")
    }

    rustpython_common::static_cell! {
        static UTC: PyRef<PyTimeZone>;
    }

    /// `new_timezone`: UTC for a zero, unnamed offset; the offset must be within a day.
    fn new_timezone(
        offset: PyRef<PyDelta>,
        name: Option<PyStrRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<PyTimeZone>> {
        if name.is_none() && !offset.is_nonzero() {
            return Ok(utc().to_owned());
        }
        if offset_out_of_range(&offset) {
            return Err(offset_range_error(offset.as_object(), vm)?);
        }
        Ok(PyTimeZone {
            base: PyTzInfo::default(),
            offset,
            name,
        }
        .into_ref(&vm.ctx))
    }

    #[derive(FromArgs)]
    pub(super) struct TimeZoneArgs {
        #[pyarg(any)]
        offset: PyObjectRef,
        #[pyarg(any, optional)]
        name: OptionalArg<PyObjectRef>,
    }

    impl Constructor for PyTimeZone {
        type Args = TimeZoneArgs;

        fn slot_new(_cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            let args: TimeZoneArgs = args.bind(vm)?;
            let offset = args.offset.downcast::<PyDelta>().map_err(|offset| {
                vm.new_type_error(format!(
                    "timezone() argument 1 must be datetime.timedelta, not {}",
                    type_name(&offset)
                ))
            })?;
            let name = match args.name {
                OptionalArg::Present(name) => Some(name.downcast::<PyStr>().map_err(|name| {
                    vm.new_type_error(format!(
                        "timezone() argument 2 must be str, not {}",
                        type_name(&name)
                    ))
                })?),
                OptionalArg::Missing => None,
            };
            Ok(new_timezone(offset, name, vm)?.into())
        }

        fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
            unreachable!("slot_new is overridden")
        }
    }

    fn timezone_check_argument(dt: &PyObject, meth: &str, vm: &VirtualMachine) -> PyResult<()> {
        if vm.is_none(dt) || as_datetime(dt).is_some() {
            Ok(())
        } else {
            Err(vm.new_type_error(format!(
                "{meth}(dt) argument must be a datetime instance or None, not {}",
                type_name(dt)
            )))
        }
    }

    impl PyTimeZone {
        fn name_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyStrRef {
            if let Some(name) = &zelf.name {
                return name.clone();
            }
            if zelf.is(utc()) || !zelf.offset.is_nonzero() {
                return vm.ctx.new_str("UTC");
            }
            vm.ctx
                .new_str(format!("UTC{}", format_offset(&zelf.offset, ":")))
        }
    }

    impl Comparable for PyTimeZone {
        fn cmp(
            zelf: &Py<Self>,
            other: &PyObject,
            op: PyComparisonOp,
            _vm: &VirtualMachine,
        ) -> PyResult<PyComparisonValue> {
            if !matches!(op, PyComparisonOp::Eq | PyComparisonOp::Ne) {
                return Ok(PyComparisonValue::NotImplemented);
            }
            let Some(other) = other.downcast_ref::<Self>() else {
                return Ok(PyComparisonValue::NotImplemented);
            };
            Ok(op
                .eval_ord(zelf.offset.cmp_key().cmp(&other.offset.cmp_key()))
                .into())
        }
    }

    impl Hashable for PyTimeZone {
        fn hash(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyHash> {
            <PyDelta as Hashable>::hash(&zelf.offset, vm)
        }
    }

    impl Representable for PyTimeZone {
        fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
            let type_name = type_name(zelf.as_object());
            if zelf.is(utc()) {
                return Ok(format!("{type_name}.utc"));
            }
            let offset = zelf.offset.as_object().repr(vm)?;
            Ok(match &zelf.name {
                None => format!("{type_name}({offset})"),
                Some(name) => format!("{type_name}({offset}, {})", name.as_object().repr(vm)?),
            })
        }
    }

    #[pyclass(with(Constructor, Comparable, Hashable, Representable))]
    impl PyTimeZone {
        #[pymethod]
        fn tzname(zelf: &Py<Self>, object: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyStrRef> {
            let dt = object;
            timezone_check_argument(&dt, "tzname", vm)?;
            Ok(Self::name_str(zelf, vm))
        }

        #[pymethod]
        fn utcoffset(
            zelf: &Py<Self>,
            object: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<PyDelta>> {
            let dt = object;
            timezone_check_argument(&dt, "utcoffset", vm)?;
            Ok(zelf.offset.clone())
        }

        #[pymethod]
        fn dst(_zelf: &Py<Self>, object: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            let dt = object;
            timezone_check_argument(&dt, "dst", vm)?;
            Ok(())
        }

        #[pymethod]
        fn fromutc(zelf: &Py<Self>, object: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            let dt = object;
            let Some(dtobj) = as_datetime(&dt) else {
                return Err(vm.new_type_error("fromutc: argument must be a datetime"));
            };
            if !dtobj.tzinfo.load_owned().is_some_and(|tz| tz.is(zelf)) {
                return Err(vm.new_value_error("fromutc: dt.tzinfo is not self"));
            }
            PyDateTime::add_delta(dtobj, &zelf.offset, 1, vm)
        }

        #[pymethod]
        fn __getinitargs__(zelf: &Py<Self>, vm: &VirtualMachine) -> PyRef<PyTuple> {
            match &zelf.name {
                None => vm.new_tuple((zelf.offset.clone(),)),
                Some(name) => vm.new_tuple((zelf.offset.clone(), name.clone())),
            }
        }

        #[pyslot]
        #[allow(clippy::unnecessary_wraps)]
        fn slot_str(zelf: &PyObject, vm: &VirtualMachine) -> PyResult<PyStrRef> {
            Ok(Self::name_str(zelf.downcast_ref::<Self>().unwrap(), vm))
        }

        #[extend_class]
        fn extend_class(ctx: &crate::vm::Context, class: &Py<PyType>) {
            let delta_type = PyDelta::static_type();
            let make = |seconds: i32| {
                let (days, seconds) = if seconds < 0 {
                    (-1, seconds + 24 * 3600)
                } else {
                    (0, seconds)
                };
                let offset = PyRef::new_ref(
                    PyDelta::new_unchecked(days, seconds, 0),
                    delta_type.to_owned(),
                    None,
                );
                PyRef::new_ref(
                    Self {
                        base: PyTzInfo::default(),
                        offset,
                        name: None,
                    },
                    class.to_owned(),
                    None,
                )
            };
            let utc = UTC.get_or_init(|| make(0));
            class.set_attr(ctx.intern_str("utc"), utc.clone().into());
            class.set_attr(ctx.intern_str("min"), make(-(23 * 3600 + 59 * 60)).into());
            class.set_attr(ctx.intern_str("max"), make(23 * 3600 + 59 * 60).into());
        }
    }

    // time

    #[pyattr]
    #[pyclass(module = "datetime", name = "time", traverse)]
    #[derive(Debug, PyPayload)]
    pub(super) struct PyTime {
        #[pytraverse(skip)]
        #[pymember]
        hour: u8,
        #[pytraverse(skip)]
        #[pymember]
        minute: u8,
        #[pytraverse(skip)]
        #[pymember]
        second: u8,
        #[pytraverse(skip)]
        #[pymember]
        fold: u8,
        #[pytraverse(skip)]
        #[pymember]
        microsecond: u32,
        #[pymember]
        tzinfo: Option<PyObjectRef>,
        #[pytraverse(skip)]
        hashcode: AtomicCell<PyHash>,
    }

    impl PyTime {
        fn new(h: i32, m: i32, s: i32, us: i32, tzinfo: Option<PyObjectRef>, fold: i32) -> Self {
            Self {
                hour: h as u8,
                minute: m as u8,
                second: s as u8,
                fold: fold as u8,
                microsecond: us as u32,
                tzinfo,
                hashcode: AtomicCell::new(-1),
            }
        }

        fn state_bytes(&self, proto: i32) -> [u8; 6] {
            let us = self.microsecond;
            let mut hour = self.hour;
            if proto > 3 && self.fold != 0 {
                hour |= 1 << 7;
            }
            [
                hour,
                self.minute,
                self.second,
                (us >> 16) as u8,
                (us >> 8) as u8,
                us as u8,
            ]
        }

        fn key(&self) -> (u8, u8, u8, u32) {
            (self.hour, self.minute, self.second, self.microsecond)
        }

        fn getstate(&self, proto: i32, vm: &VirtualMachine) -> PyRef<PyTuple> {
            let state = vm.ctx.new_bytes(self.state_bytes(proto).to_vec());
            match &self.tzinfo {
                None => vm.new_tuple((state,)),
                Some(tz) => vm.new_tuple((state, tz.clone())),
            }
        }
    }

    /// `new_time_ex2` for a given type, checking the fields.
    #[allow(clippy::too_many_arguments)]
    fn new_time(
        h: i32,
        m: i32,
        s: i32,
        us: i32,
        tzinfo: Option<PyObjectRef>,
        fold: i32,
        cls: &Py<PyType>,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<PyTime>> {
        check_time_args(h, m, s, us, fold, vm)?;
        if let Some(tz) = &tzinfo {
            check_tzinfo_subclass(tz, vm)?;
        }
        PyTime::new(h, m, s, us, tzinfo, fold).into_ref_with_type(vm, cls.to_owned())
    }

    /// `new_time_subclass_fold_ex`
    #[allow(clippy::too_many_arguments)]
    fn new_time_subclass(
        h: i32,
        m: i32,
        s: i32,
        us: i32,
        tzinfo: Option<PyObjectRef>,
        fold: i32,
        cls: &Py<PyType>,
        vm: &VirtualMachine,
    ) -> PyResult {
        if cls.is(time_type()) {
            return Ok(new_time(h, m, s, us, tzinfo, fold, cls, vm)?.into());
        }
        let args = vec![
            vm.ctx.new_int(h).into(),
            vm.ctx.new_int(m).into(),
            vm.ctx.new_int(s).into(),
            vm.ctx.new_int(us).into(),
            tzinfo_or_none(tzinfo.as_ref(), vm),
        ];
        call_subclass_fold(cls, fold, args, vm)
    }

    fn call_subclass_fold(
        cls: &Py<PyType>,
        fold: i32,
        args: Vec<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let kwargs = if fold != 0 {
            core::iter::once(("fold".to_owned(), vm.ctx.new_int(fold).into())).collect()
        } else {
            crate::vm::function::KwArgs::default()
        };
        cls.as_object().call(FuncArgs::new(args, kwargs), vm)
    }

    #[derive(FromArgs)]
    struct TimeArgs {
        #[pyarg(any, default = 0)]
        hour: i32,
        #[pyarg(any, default = 0)]
        minute: i32,
        #[pyarg(any, default = 0)]
        second: i32,
        #[pyarg(any, default = 0)]
        microsecond: i32,
        #[pyarg(any, optional)]
        tzinfo: OptionalArg<PyObjectRef>,
        #[pyarg(named, default = 0)]
        fold: i32,
    }

    fn pickle_tzinfo(
        tzinfo: Option<&PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>> {
        match tzinfo {
            Some(tz) if !vm.is_none(tz) => {
                if !is_tzinfo(tz) {
                    return Err(vm.new_type_error("bad tzinfo state arg"));
                }
                Ok(Some(tz.clone()))
            }
            _ => Ok(None),
        }
    }

    impl Constructor for PyTime {
        type Args = FuncArgs;

        fn py_new(_cls: &Py<PyType>, args: FuncArgs, vm: &VirtualMachine) -> PyResult<Self> {
            if (1..=2).contains(&args.args.len())
                && let Some(state) =
                    pickle_state(&args.args[0], 6, 0, |hour| (hour & 0x7f) < 24, "time", vm)?
            {
                let tzinfo = pickle_tzinfo(args.args.get(1), vm)?;
                let fold = (state[0] & 0x80) != 0;
                let us = i32::from(state[3]) << 16 | i32::from(state[4]) << 8 | i32::from(state[5]);
                return Ok(Self::new(
                    i32::from(state[0] & 0x7f),
                    i32::from(state[1]),
                    i32::from(state[2]),
                    us,
                    tzinfo,
                    i32::from(fold),
                ));
            }
            let a: TimeArgs = args.bind(vm)?;
            let tzinfo = a.tzinfo.into_option().and_then(|tz| tzinfo_arg(tz, vm));
            check_time_args(a.hour, a.minute, a.second, a.microsecond, a.fold, vm)?;
            if let Some(tz) = &tzinfo {
                check_tzinfo_subclass(tz, vm)?;
            }
            Ok(Self::new(
                a.hour,
                a.minute,
                a.second,
                a.microsecond,
                tzinfo,
                a.fold,
            ))
        }
    }

    #[derive(FromArgs)]
    struct TimeReplaceArgs {
        #[pyarg(any, optional, py_default = "unchanged")]
        hour: OptionalArg<i32>,
        #[pyarg(any, optional, py_default = "unchanged")]
        minute: OptionalArg<i32>,
        #[pyarg(any, optional, py_default = "unchanged")]
        second: OptionalArg<i32>,
        #[pyarg(any, optional, py_default = "unchanged")]
        microsecond: OptionalArg<i32>,
        #[pyarg(any, optional, py_default = "unchanged")]
        tzinfo: OptionalArg<PyObjectRef>,
        #[pyarg(named, optional, py_default = "unchanged")]
        fold: OptionalArg<i32>,
    }

    #[derive(FromArgs)]
    struct TimeIsoformatArgs {
        #[pyarg(any, optional)]
        timespec: OptionalArg<PyStrRef>,
    }

    /// `isoformat`'s time part for `timespec`, or `Err(())` for an unknown one.
    fn format_time_spec(
        timespec: Option<&str>,
        h: u8,
        m: u8,
        s: u8,
        us: u32,
    ) -> Result<String, ()> {
        let spec = match timespec {
            None | Some("auto") => {
                if us == 0 {
                    "seconds"
                } else {
                    "microseconds"
                }
            }
            Some(spec) => spec,
        };
        Ok(match spec {
            "hours" => format!("{h:02}"),
            "minutes" => format!("{h:02}:{m:02}"),
            "seconds" => format!("{h:02}:{m:02}:{s:02}"),
            "milliseconds" => format!("{h:02}:{m:02}:{s:02}.{:03}", us / 1000),
            "microseconds" => format!("{h:02}:{m:02}:{s:02}.{us:06}"),
            _ => return Err(()),
        })
    }

    #[pyclass(
        with(Constructor, Comparable, Hashable, Representable),
        flags(BASETYPE)
    )]
    impl PyTime {
        #[pyclassmethod]
        fn strptime(
            cls: PyTypeRef,
            string: PyObjectRef,
            format: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult {
            call_strptime(&cls, "_strptime_datetime_time", (string, format), vm)
        }

        #[pymethod]
        fn isoformat(
            zelf: &Py<Self>,
            args: TimeIsoformatArgs,
            vm: &VirtualMachine,
        ) -> PyResult<String> {
            let spec = args.timespec.into_option().map(|s| s.to_string());
            let mut result = format_time_spec(
                spec.as_deref(),
                zelf.hour,
                zelf.minute,
                zelf.second,
                zelf.microsecond,
            )
            .map_err(|()| vm.new_value_error("Unknown timespec value"))?;
            if zelf.tzinfo.is_some() {
                result.push_str(&format_utcoffset(
                    ":",
                    zelf.tzinfo.as_ref(),
                    &vm.ctx.none(),
                    vm,
                )?);
            }
            Ok(result)
        }

        #[pymethod]
        fn strftime(zelf: &Py<Self>, args: StrftimeArgs, vm: &VirtualMachine) -> PyResult {
            let format = strftime_format_arg(args.format, vm)?;
            // The year is forced to 1900 for time.strftime's sake.
            let fields: Vec<PyObjectRef> = [
                1900,
                1,
                1,
                i32::from(zelf.hour),
                i32::from(zelf.minute),
                i32::from(zelf.second),
                0,
                1,
                -1,
            ]
            .into_iter()
            .map(|v| vm.ctx.new_int(v).into())
            .collect();
            wrap_strftime(
                zelf.as_object(),
                &format,
                vm.ctx.new_tuple(fields).into(),
                &vm.ctx.none(),
                vm,
            )
        }

        #[pymethod]
        fn __format__(zelf: PyObjectRef, spec: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            object_format(&zelf, spec, vm)
        }

        #[pymethod]
        fn utcoffset(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult {
            Ok(offset_to_py(
                call_utcoffset(zelf.tzinfo.as_ref(), &vm.ctx.none(), vm)?,
                vm,
            ))
        }

        #[pymethod]
        fn tzname(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult {
            Ok(call_tzname(zelf.tzinfo.as_ref(), &vm.ctx.none(), vm)?
                .map_or_else(|| vm.ctx.none(), Into::into))
        }

        #[pymethod]
        fn dst(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult {
            Ok(offset_to_py(
                call_dst(zelf.tzinfo.as_ref(), &vm.ctx.none(), vm)?,
                vm,
            ))
        }

        #[pymethod]
        fn replace(zelf: &Py<Self>, args: TimeReplaceArgs, vm: &VirtualMachine) -> PyResult {
            let tzinfo = match args.tzinfo {
                OptionalArg::Present(tz) => tzinfo_arg(tz, vm),
                OptionalArg::Missing => zelf.tzinfo.clone(),
            };
            new_time_subclass(
                args.hour.unwrap_or_else(|| i32::from(zelf.hour)),
                args.minute.unwrap_or_else(|| i32::from(zelf.minute)),
                args.second.unwrap_or_else(|| i32::from(zelf.second)),
                args.microsecond.unwrap_or(zelf.microsecond as i32),
                tzinfo,
                args.fold.unwrap_or_else(|| i32::from(zelf.fold)),
                zelf.class(),
                vm,
            )
        }

        #[pymethod]
        fn __replace__(zelf: &Py<Self>, changes: ReplaceChanges, vm: &VirtualMachine) -> PyResult {
            Self::replace(zelf, changes.args.bind_for(vm, "replace")?, vm)
        }

        #[pyclassmethod]
        fn fromisoformat(cls: PyTypeRef, object: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            let tstr = object;
            let Some(s) = tstr.downcast_ref::<PyStr>() else {
                return Err(vm.new_type_error("fromisoformat: argument must be str"));
            };
            let Some(bytes) = str_utf8(s) else {
                return Err(invalid_isoformat(&tstr, vm)?);
            };
            // A leading `T` is optional where the string cannot be a date.
            let start = usize::from(byte_at(bytes, 0) == b'T');
            let len = bytes.len() - start;
            let mut vals = [0; 4];
            let (mut tzoffset, mut tzusec) = (0, 0);
            let rv = parse_isoformat_time(bytes, start, len, &mut vals, &mut tzoffset, &mut tzusec);
            if rv < 0 {
                return Err(invalid_isoformat(&tstr, vm)?);
            }
            let [mut hour, minute, second, microsecond] = vals;
            if hour == 24 {
                if minute == 0 && second == 0 && microsecond == 0 {
                    hour = 0;
                } else {
                    return Err(vm.new_value_error(
                        "minute, second, and microsecond must be 0 when hour is 24",
                    ));
                }
            }
            let tzinfo = tzinfo_from_isoformat_results(rv, tzoffset, tzusec, vm)?;
            if cls.is(time_type()) {
                return Ok(
                    new_time(hour, minute, second, microsecond, tzinfo, 0, &cls, vm)?.into(),
                );
            }
            cls.as_object().call(
                (
                    hour,
                    minute,
                    second,
                    microsecond,
                    tzinfo_or_none(tzinfo.as_ref(), vm),
                ),
                vm,
            )
        }

        #[pymethod]
        fn __reduce_ex__(zelf: &Py<Self>, proto: i32, vm: &VirtualMachine) -> PyRef<PyTuple> {
            vm.new_tuple((zelf.class().to_owned(), zelf.getstate(proto, vm)))
        }

        #[pymethod]
        fn __reduce__(zelf: &Py<Self>, vm: &VirtualMachine) -> PyRef<PyTuple> {
            vm.new_tuple((zelf.class().to_owned(), zelf.getstate(2, vm)))
        }

        #[pyslot]
        fn slot_str(zelf: &PyObject, vm: &VirtualMachine) -> PyResult<PyStrRef> {
            checked_str_result(vm.call_method(zelf, "isoformat", ())?, vm)
        }

        #[extend_class]
        fn extend_class(ctx: &crate::vm::Context, class: &Py<PyType>) {
            let make = |h, m, s, us| {
                PyRef::new_ref(Self::new(h, m, s, us, None, 0), class.to_owned(), None).into()
            };
            class.set_attr(ctx.intern_str("min"), make(0, 0, 0, 0));
            class.set_attr(ctx.intern_str("max"), make(23, 59, 59, 999_999));
            let resolution = PyRef::new_ref(
                PyDelta::new_unchecked(0, 0, 1),
                PyDelta::static_type().to_owned(),
                None,
            );
            class.set_attr(ctx.intern_str("resolution"), resolution.into());
        }
    }

    impl Comparable for PyTime {
        fn cmp(
            zelf: &Py<Self>,
            other: &PyObject,
            op: PyComparisonOp,
            vm: &VirtualMachine,
        ) -> PyResult<PyComparisonValue> {
            let Some(other) = as_time(other) else {
                return Ok(PyComparisonValue::NotImplemented);
            };
            if same_tzinfo(zelf.tzinfo.as_ref(), other.tzinfo.as_ref()) {
                return Ok(op.eval_ord(zelf.key().cmp(&other.key())).into());
            }
            let none = vm.ctx.none();
            let offset1 = call_utcoffset(zelf.tzinfo.as_ref(), &none, vm)?;
            let offset2 = call_utcoffset(other.tzinfo.as_ref(), &none, vm)?;
            match (&offset1, &offset2) {
                (None, None) => Ok(op.eval_ord(zelf.key().cmp(&other.key())).into()),
                (Some(o1), Some(o2)) if o1.cmp_key() == o2.cmp_key() => {
                    Ok(op.eval_ord(zelf.key().cmp(&other.key())).into())
                }
                (Some(o1), Some(o2)) => {
                    let secs = |t: &Self, o: &PyDelta| {
                        i64::from(t.hour) * 3600 + i64::from(t.minute) * 60 + i64::from(t.second)
                            - i64::from(o.days) * 86400
                            - i64::from(o.seconds)
                    };
                    let k1 = (secs(zelf, o1), zelf.microsecond);
                    let k2 = (secs(other, o2), other.microsecond);
                    Ok(op.eval_ord(k1.cmp(&k2)).into())
                }
                _ => match op {
                    PyComparisonOp::Eq => Ok(false.into()),
                    PyComparisonOp::Ne => Ok(true.into()),
                    _ => {
                        Err(vm.new_type_error("can't compare offset-naive and offset-aware times"))
                    }
                },
            }
        }
    }

    impl Hashable for PyTime {
        fn hash(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyHash> {
            let cached = zelf.hashcode.load();
            if cached != -1 {
                return Ok(cached);
            }
            // `time.utcoffset()` passes None to the tzinfo, so fold cannot change it.
            let offset = call_utcoffset(zelf.tzinfo.as_ref(), &vm.ctx.none(), vm)?;
            let hash = match offset {
                None => vm
                    .ctx
                    .new_bytes(zelf.state_bytes(0).to_vec())
                    .as_object()
                    .hash(vm)?,
                Some(offset) => {
                    let seconds = i64::from(zelf.hour) * 3600
                        + i64::from(zelf.minute) * 60
                        + i64::from(zelf.second);
                    let delta = PyDelta::new_ref(
                        -i64::from(offset.days),
                        seconds - i64::from(offset.seconds),
                        i64::from(zelf.microsecond) - i64::from(offset.microseconds),
                        true,
                        vm,
                    )?;
                    <PyDelta as Hashable>::hash(&delta, vm)?
                }
            };
            zelf.hashcode.store(hash);
            Ok(hash)
        }
    }

    impl Representable for PyTime {
        fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
            let type_name = type_name(zelf.as_object());
            let (h, m, s, us) = (zelf.hour, zelf.minute, zelf.second, zelf.microsecond);
            let repr = if us != 0 {
                format!("{type_name}({h}, {m}, {s}, {us})")
            } else if s != 0 {
                format!("{type_name}({h}, {m}, {s})")
            } else {
                format!("{type_name}({h}, {m})")
            };
            let repr = append_keyword_tzinfo(repr, zelf.tzinfo.as_ref(), vm)?;
            Ok(append_keyword_fold(repr, zelf.fold))
        }
    }

    // datetime

    #[pyattr]
    #[pyclass(module = "datetime", name = "datetime", base = PyDate, traverse)]
    #[derive(Debug)]
    pub(super) struct PyDateTime {
        #[pytraverse(skip)]
        date: PyDate,
        #[pytraverse(skip)]
        #[pymember]
        hour: u8,
        #[pytraverse(skip)]
        #[pymember]
        minute: u8,
        #[pytraverse(skip)]
        #[pymember]
        second: u8,
        #[pytraverse(skip)]
        #[pymember]
        fold: u8,
        #[pytraverse(skip)]
        #[pymember]
        microsecond: u32,
        // astimezone updates the newly constructed subclass instance in place.
        #[pymember]
        tzinfo: PyObjectCell,
    }

    impl PyDateTime {
        fn set_tzinfo(&self, tzinfo: PyObjectRef) {
            drop(self.tzinfo.store(Some(tzinfo)));
        }

        #[allow(clippy::too_many_arguments)]
        fn new(
            y: i32,
            mo: i32,
            d: i32,
            h: i32,
            mi: i32,
            s: i32,
            us: i32,
            tzinfo: Option<PyObjectRef>,
            fold: i32,
        ) -> Self {
            Self {
                date: PyDate::new(y, mo, d),
                hour: h as u8,
                minute: mi as u8,
                second: s as u8,
                fold: fold as u8,
                microsecond: us as u32,
                tzinfo: tzinfo.into(),
            }
        }

        fn fields(&self) -> (i32, i32, i32, i32, i32, i32, i32) {
            (
                self.date.y(),
                self.date.m(),
                self.date.d(),
                i32::from(self.hour),
                i32::from(self.minute),
                i32::from(self.second),
                self.microsecond as i32,
            )
        }

        fn state_bytes(&self, proto: i32) -> [u8; 10] {
            let d = self.date.state_bytes();
            let us = self.microsecond;
            let mut month = d[2];
            if proto > 3 && self.fold != 0 {
                month |= 1 << 7;
            }
            [
                d[0],
                d[1],
                month,
                d[3],
                self.hour,
                self.minute,
                self.second,
                (us >> 16) as u8,
                (us >> 8) as u8,
                us as u8,
            ]
        }

        fn key(&self) -> (u16, u8, u8, u8, u8, u8, u32) {
            (
                self.date.year,
                self.date.month,
                self.date.day,
                self.hour,
                self.minute,
                self.second,
                self.microsecond,
            )
        }

        fn getstate(&self, proto: i32, vm: &VirtualMachine) -> PyRef<PyTuple> {
            let state = vm.ctx.new_bytes(self.state_bytes(proto).to_vec());
            match self.tzinfo.load_owned() {
                None => vm.new_tuple((state,)),
                Some(tz) => vm.new_tuple((state, tz)),
            }
        }

        fn utcoffset_of(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<Option<PyRef<PyDelta>>> {
            call_utcoffset(zelf.tzinfo.load_owned().as_ref(), zelf.as_object(), vm)
        }

        /// `add_datetime_timedelta`: `factor` is 1 to add, -1 to subtract.
        fn add_delta(
            zelf: &Py<Self>,
            delta: &PyDelta,
            factor: i64,
            vm: &VirtualMachine,
        ) -> PyResult {
            let (y, mo, d, h, mi, s, us) = zelf.fields();
            let (mut y, mut mo, mut d) = (
                i64::from(y),
                i64::from(mo),
                i64::from(d) + i64::from(delta.days) * factor,
            );
            let (mut h, mut mi) = (i64::from(h), i64::from(mi));
            let mut s = i64::from(s) + i64::from(delta.seconds) * factor;
            let mut us = i64::from(us) + i64::from(delta.microseconds) * factor;
            normalize_pair(&mut s, &mut us, 1_000_000);
            normalize_pair(&mut mi, &mut s, 60);
            normalize_pair(&mut h, &mut mi, 60);
            normalize_pair(&mut d, &mut h, 24);
            normalize_y_m_d(&mut y, &mut mo, &mut d).map_err(|()| date_overflow(vm))?;
            new_datetime_subclass(
                y as i32,
                mo as i32,
                d as i32,
                h as i32,
                mi as i32,
                s as i32,
                us as i32,
                zelf.tzinfo.load_owned(),
                0,
                zelf.class(),
                vm,
            )
        }

        /// A copy with `fold` flipped, keeping the type.
        fn flip_fold(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
            let (y, mo, d, h, mi, s, us) = zelf.fields();
            new_datetime(
                y,
                mo,
                d,
                h,
                mi,
                s,
                us,
                zelf.tzinfo.load_owned(),
                i32::from(zelf.fold == 0),
                zelf.class(),
                vm,
            )
        }

        /// PEP 495: equality fails when either offset depends on `fold`.
        fn pep495_eq_exception(
            zelf: &Py<Self>,
            other: &Py<Self>,
            offset_self: Option<&PyRef<PyDelta>>,
            offset_other: Option<&PyRef<PyDelta>>,
            vm: &VirtualMachine,
        ) -> PyResult<bool> {
            let differs = |a: Option<&PyRef<PyDelta>>, b: Option<&PyRef<PyDelta>>| match (a, b) {
                (Some(a), Some(b)) => a.cmp_key() != b.cmp_key(),
                (None, None) => false,
                _ => true,
            };
            let flipped = Self::flip_fold(zelf, vm)?;
            let flip = Self::utcoffset_of(&flipped, vm)?;
            if differs(flip.as_ref(), offset_self) {
                return Ok(true);
            }
            let flipped = Self::flip_fold(other, vm)?;
            let flip = Self::utcoffset_of(&flipped, vm)?;
            Ok(differs(flip.as_ref(), offset_other))
        }

        /// `datetime - datetime`
        fn subtract_datetime(
            left: &Py<Self>,
            right: &Py<Self>,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<PyDelta>> {
            let (offset1, offset2) = if same_tzinfo(
                left.tzinfo.load_owned().as_ref(),
                right.tzinfo.load_owned().as_ref(),
            ) {
                (None, None)
            } else {
                let o1 = Self::utcoffset_of(left, vm)?;
                let o2 = Self::utcoffset_of(right, vm)?;
                if o1.is_some() != o2.is_some() {
                    return Err(
                        vm.new_type_error("can't subtract offset-naive and offset-aware datetimes")
                    );
                }
                (o1, o2)
            };
            let (ly, lmo, ld, lh, lmi, ls, lus) = left.fields();
            let (ry, rmo, rd, rh, rmi, rs, rus) = right.fields();
            let delta_d = i64::from(ymd_to_ord(ly, lmo, ld)) - i64::from(ymd_to_ord(ry, rmo, rd));
            let delta_s =
                i64::from(lh - rh) * 3600 + i64::from(lmi - rmi) * 60 + i64::from(ls - rs);
            let delta_us = i64::from(lus - rus);
            let (mut dd, mut ds, mut dus) = (delta_d, delta_s, delta_us);
            if let (Some(o1), Some(o2)) = (&offset1, &offset2)
                && o1.cmp_key() != o2.cmp_key()
            {
                // result - (offset1 - offset2)
                dd -= i64::from(o1.days) - i64::from(o2.days);
                ds -= i64::from(o1.seconds) - i64::from(o2.seconds);
                dus -= i64::from(o1.microseconds) - i64::from(o2.microseconds);
            }
            PyDelta::new_ref(dd, ds, dus, true, vm)
        }

        fn nb_add(a: &PyObject, b: &PyObject, vm: &VirtualMachine) -> PyResult {
            if let Some(dt) = as_datetime(a) {
                if let Some(delta) = as_delta(b) {
                    return Self::add_delta(dt, delta, 1, vm);
                }
            } else if let (Some(delta), Some(dt)) = (as_delta(a), as_datetime(b)) {
                return Self::add_delta(dt, delta, 1, vm);
            }
            Ok(vm.ctx.not_implemented())
        }

        fn nb_subtract(a: &PyObject, b: &PyObject, vm: &VirtualMachine) -> PyResult {
            if let Some(left) = as_datetime(a) {
                if let Some(right) = as_datetime(b) {
                    return Ok(Self::subtract_datetime(left, right, vm)?.into());
                }
                if let Some(delta) = as_delta(b) {
                    return Self::add_delta(left, delta, -1, vm);
                }
            }
            Ok(vm.ctx.not_implemented())
        }
    }

    /// `new_datetime_ex2` for a given type, checking the fields.
    #[allow(clippy::too_many_arguments)]
    fn new_datetime(
        y: i32,
        mo: i32,
        d: i32,
        h: i32,
        mi: i32,
        s: i32,
        us: i32,
        tzinfo: Option<PyObjectRef>,
        fold: i32,
        cls: &Py<PyType>,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<PyDateTime>> {
        check_date_args(y, mo, d, vm)?;
        check_time_args(h, mi, s, us, fold, vm)?;
        if let Some(tz) = &tzinfo {
            check_tzinfo_subclass(tz, vm)?;
        }
        PyDateTime::new(y, mo, d, h, mi, s, us, tzinfo, fold).into_ref_with_type(vm, cls.to_owned())
    }

    /// `new_datetime_subclass_fold_ex`
    #[allow(clippy::too_many_arguments)]
    fn new_datetime_subclass(
        y: i32,
        mo: i32,
        d: i32,
        h: i32,
        mi: i32,
        s: i32,
        us: i32,
        tzinfo: Option<PyObjectRef>,
        fold: i32,
        cls: &Py<PyType>,
        vm: &VirtualMachine,
    ) -> PyResult {
        if cls.is(datetime_type()) {
            return Ok(new_datetime(y, mo, d, h, mi, s, us, tzinfo, fold, cls, vm)?.into());
        }
        let args = [y, mo, d, h, mi, s, us]
            .into_iter()
            .map(|v| vm.ctx.new_int(v).into())
            .chain(core::iter::once(tzinfo_or_none(tzinfo.as_ref(), vm)))
            .collect();
        call_subclass_fold(cls, fold, args, vm)
    }

    /// `datetime_from_timet_and_us`, detecting a fold for naive local times.
    fn datetime_from_timet_and_us(
        cls: &Py<PyType>,
        local_tm: bool,
        timet: i64,
        us: i32,
        tzinfo: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let tm = platform_time(timet, local_tm, vm)?;
        let second = tm.second.min(59);
        let mut fold = 0;
        let skip_probe = cfg!(windows) && timet - MAX_FOLD_SECONDS <= 0;
        if tzinfo.is_none() && local_tm && !skip_probe {
            let result_seconds =
                utc_to_seconds(tm.year, tm.month, tm.day, tm.hour, tm.minute, second, vm)?;
            let probe_seconds = local(EPOCH_SECONDS + timet - MAX_FOLD_SECONDS, vm)?;
            let transition = result_seconds - probe_seconds - MAX_FOLD_SECONDS;
            if transition < 0 {
                let probe_seconds = local(EPOCH_SECONDS + timet + transition, vm)?;
                if probe_seconds == result_seconds {
                    fold = 1;
                }
            }
        }
        new_datetime_subclass(
            tm.year, tm.month, tm.day, tm.hour, tm.minute, second, us, tzinfo, fold, cls, vm,
        )
    }

    /// `datetime_best_possible`: the current time to the microsecond.
    fn datetime_best_possible(
        cls: &Py<PyType>,
        local_tm: bool,
        tzinfo: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let now = std::time::SystemTime::now();
        let (secs, us) = match now.duration_since(std::time::UNIX_EPOCH) {
            Ok(d) => (d.as_secs() as i64, d.subsec_micros() as i32),
            Err(e) => {
                let d = e.duration();
                let mut secs = -(d.as_secs() as i64);
                let mut us = -(d.subsec_micros() as i32);
                if us < 0 {
                    us += 1_000_000;
                    secs -= 1;
                }
                (secs, us)
            }
        };
        datetime_from_timet_and_us(cls, local_tm, secs, us, tzinfo, vm)
    }

    /// `local_timezone_from_timestamp`: the local zone at a timestamp as a fixed `timezone`.
    fn local_timezone_from_timestamp(
        timestamp: i64,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<PyTimeZone>> {
        let st = vm.call_method(&*vm.import("time", 0)?, "localtime", (timestamp,))?;
        let gmtoff: i64 = st.get_attr("tm_gmtoff", vm)?.try_into_value(vm)?;
        let zone = st.get_attr("tm_zone", vm)?;
        let delta = PyDelta::new_ref(0, gmtoff, 0, true, vm)?;
        let name = if vm.is_none(&zone) {
            None
        } else {
            Some(
                zone.downcast::<PyStr>()
                    .map_err(|_| vm.new_type_error("tm_zone must be str"))?,
            )
        };
        new_timezone(delta, name, vm)
    }

    #[derive(FromArgs)]
    struct DateTimeArgs {
        #[pyarg(any)]
        year: i32,
        #[pyarg(any)]
        month: i32,
        #[pyarg(any)]
        day: i32,
        #[pyarg(any, default = 0)]
        hour: i32,
        #[pyarg(any, default = 0)]
        minute: i32,
        #[pyarg(any, default = 0)]
        second: i32,
        #[pyarg(any, default = 0)]
        microsecond: i32,
        #[pyarg(any, optional)]
        tzinfo: OptionalArg<PyObjectRef>,
        #[pyarg(named, default = 0)]
        fold: i32,
    }

    impl Constructor for PyDateTime {
        type Args = FuncArgs;

        fn py_new(_cls: &Py<PyType>, args: FuncArgs, vm: &VirtualMachine) -> PyResult<Self> {
            if (1..=2).contains(&args.args.len())
                && let Some(state) = pickle_state(
                    &args.args[0],
                    10,
                    2,
                    |month| month_is_sane(month & 0x7f),
                    "datetime",
                    vm,
                )?
            {
                let tzinfo = pickle_tzinfo(args.args.get(1), vm)?;
                let fold = (state[2] & 0x80) != 0;
                let date = PyDate::from_state(&[state[0], state[1], state[2] & 0x7f, state[3]]);
                let us = i32::from(state[7]) << 16 | i32::from(state[8]) << 8 | i32::from(state[9]);
                return Ok(Self::new(
                    date.y(),
                    date.m(),
                    date.d(),
                    i32::from(state[4]),
                    i32::from(state[5]),
                    i32::from(state[6]),
                    us,
                    tzinfo,
                    i32::from(fold),
                ));
            }
            let a: DateTimeArgs = args.bind(vm)?;
            let tzinfo = a.tzinfo.into_option().and_then(|tz| tzinfo_arg(tz, vm));
            check_date_args(a.year, a.month, a.day, vm)?;
            check_time_args(a.hour, a.minute, a.second, a.microsecond, a.fold, vm)?;
            if let Some(tz) = &tzinfo {
                check_tzinfo_subclass(tz, vm)?;
            }
            Ok(Self::new(
                a.year,
                a.month,
                a.day,
                a.hour,
                a.minute,
                a.second,
                a.microsecond,
                tzinfo,
                a.fold,
            ))
        }
    }

    #[derive(FromArgs)]
    struct NowArgs {
        #[pyarg(any, optional, py_default = "None")]
        tz: OptionalArg<PyObjectRef>,
    }

    #[derive(FromArgs)]
    struct FromTimestampArgs {
        #[pyarg(any)]
        timestamp: PyObjectRef,
        #[pyarg(any, optional)]
        tz: OptionalArg<PyObjectRef>,
    }

    #[derive(FromArgs)]
    struct CombineArgs {
        #[pyarg(any)]
        date: PyObjectRef,
        #[pyarg(any)]
        time: PyObjectRef,
        #[pyarg(any, optional)]
        tzinfo: OptionalArg<PyObjectRef>,
    }

    #[derive(FromArgs)]
    struct DateTimeIsoformatArgs {
        #[pyarg(any, optional)]
        sep: OptionalArg<PyObjectRef>,
        #[pyarg(any, optional)]
        timespec: OptionalArg<PyStrRef>,
    }

    #[derive(FromArgs)]
    struct DateTimeReplaceArgs {
        #[pyarg(any, optional, py_default = "unchanged")]
        year: OptionalArg<i32>,
        #[pyarg(any, optional, py_default = "unchanged")]
        month: OptionalArg<i32>,
        #[pyarg(any, optional, py_default = "unchanged")]
        day: OptionalArg<i32>,
        #[pyarg(any, optional, py_default = "unchanged")]
        hour: OptionalArg<i32>,
        #[pyarg(any, optional, py_default = "unchanged")]
        minute: OptionalArg<i32>,
        #[pyarg(any, optional, py_default = "unchanged")]
        second: OptionalArg<i32>,
        #[pyarg(any, optional, py_default = "unchanged")]
        microsecond: OptionalArg<i32>,
        #[pyarg(any, optional, py_default = "unchanged")]
        tzinfo: OptionalArg<PyObjectRef>,
        #[pyarg(named, optional, py_default = "unchanged")]
        fold: OptionalArg<i32>,
    }

    #[derive(FromArgs)]
    struct AstimezoneArgs {
        #[pyarg(any, optional)]
        tz: OptionalArg<PyObjectRef>,
    }

    /// `_find_isoformat_datetime_separator`
    fn find_isoformat_datetime_separator(s: &[u8], len: usize) -> usize {
        if len == 7 {
            return 7;
        }
        if byte_at(s, 4) == b'-' {
            if byte_at(s, 5) == b'W' {
                if len < 8 {
                    return usize::MAX;
                }
                if len > 8 && byte_at(s, 8) == b'-' {
                    if len == 9 {
                        return usize::MAX;
                    }
                    if len > 10 && is_digit(byte_at(s, 10)) {
                        return 8;
                    }
                    return 10;
                }
                return 8;
            }
            return 10;
        }
        if byte_at(s, 4) == b'W' {
            let mut idx = 7;
            while idx < len {
                if !is_digit(byte_at(s, idx)) {
                    break;
                }
                idx += 1;
            }
            if idx < 9 {
                return idx;
            }
            return if idx % 2 == 0 { 7 } else { 8 };
        }
        8
    }

    #[pyclass(
        with(Constructor, AsNumber, Comparable, Hashable, Representable),
        flags(BASETYPE)
    )]
    impl PyDateTime {
        #[pyclassmethod]
        fn now(cls: PyTypeRef, args: NowArgs, vm: &VirtualMachine) -> PyResult {
            let tz = args.tz.unwrap_or_none(vm);
            check_tzinfo_subclass(&tz, vm)?;
            let tzinfo = tzinfo_arg(tz, vm);
            let result = datetime_best_possible(&cls, tzinfo.is_none(), tzinfo.clone(), vm)?;
            match tzinfo {
                Some(tz) => vm.call_method(&tz, "fromutc", (result,)),
                None => Ok(result),
            }
        }

        #[pyclassmethod]
        fn utcnow(cls: PyTypeRef, vm: &VirtualMachine) -> PyResult {
            crate::vm::warn::warn(
                vm.ctx
                    .new_str(
                        "datetime.datetime.utcnow() is deprecated and scheduled for removal in a \
                     future version. Use timezone-aware objects to represent datetimes in UTC: \
                     datetime.datetime.now(datetime.UTC).",
                    )
                    .into(),
                Some(vm.ctx.exceptions.deprecation_warning.to_owned()),
                1,
                None,
                vm,
            )?;
            datetime_best_possible(&cls, false, None, vm)
        }

        #[pyclassmethod]
        fn fromtimestamp(cls: PyTypeRef, args: FromTimestampArgs, vm: &VirtualMachine) -> PyResult {
            let tz = args.tz.unwrap_or_none(vm);
            check_tzinfo_subclass(&tz, vm)?;
            let tzinfo = tzinfo_arg(tz, vm);
            let (timet, us) = object_to_timeval(&args.timestamp, vm)?;
            let result =
                datetime_from_timet_and_us(&cls, tzinfo.is_none(), timet, us, tzinfo.clone(), vm)?;
            match tzinfo {
                Some(tz) => vm.call_method(&tz, "fromutc", (result,)),
                None => Ok(result),
            }
        }

        #[pyclassmethod]
        fn utcfromtimestamp(
            cls: PyTypeRef,
            timestamp: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult {
            crate::vm::warn::warn(
                vm.ctx.new_str(
                    "datetime.datetime.utcfromtimestamp() is deprecated and scheduled for removal \
                     in a future version. Use timezone-aware objects to represent datetimes in \
                     UTC: datetime.datetime.fromtimestamp(timestamp, datetime.UTC).",
                )
                .into(),
                Some(vm.ctx.exceptions.deprecation_warning.to_owned()),
                1,
                None,
                vm,
            )?;
            let (timet, us) = object_to_timeval(&timestamp, vm)?;
            datetime_from_timet_and_us(&cls, false, timet, us, None, vm)
        }

        #[pyclassmethod]
        fn strptime(
            cls: PyTypeRef,
            string: PyObjectRef,
            format: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult {
            call_strptime(&cls, "_strptime_datetime_datetime", (string, format), vm)
        }

        #[pyclassmethod]
        fn combine(cls: PyTypeRef, args: CombineArgs, vm: &VirtualMachine) -> PyResult {
            let Some(date) = as_date(&args.date) else {
                return Err(vm.new_type_error(format!(
                    "combine() argument 1 must be datetime.date, not {}",
                    type_name(&args.date)
                )));
            };
            let Some(time) = as_time(&args.time) else {
                return Err(vm.new_type_error(format!(
                    "combine() argument 2 must be datetime.time, not {}",
                    type_name(&args.time)
                )));
            };
            let tzinfo = match args.tzinfo {
                OptionalArg::Present(tz) => tzinfo_arg(tz, vm),
                OptionalArg::Missing => time.tzinfo.clone(),
            };
            new_datetime_subclass(
                date.y(),
                date.m(),
                date.d(),
                i32::from(time.hour),
                i32::from(time.minute),
                i32::from(time.second),
                time.microsecond as i32,
                tzinfo,
                i32::from(time.fold),
                &cls,
                vm,
            )
        }

        #[pyclassmethod]
        fn fromisoformat(cls: PyTypeRef, object: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            let dtstr = object;
            let Some(s) = dtstr.downcast_ref::<PyStr>() else {
                return Err(vm.new_type_error("fromisoformat: argument must be str"));
            };
            // A surrogate is allowed as the separator only: replace it with `T`.
            if s.char_len() < 7 {
                return Err(invalid_isoformat(&dtstr, vm)?);
            }
            let mut owned;
            let mut text = s.to_str();
            if text.is_none() {
                let mut chars: Vec<CodePoint> = s.as_wtf8().code_points().collect();
                for pos in [7usize, 8, 10] {
                    if pos > chars.len() {
                        break;
                    }
                    if chars.get(pos).is_some_and(|c| c.to_char().is_none()) {
                        chars[pos] = CodePoint::from('T');
                        break;
                    }
                }
                owned = String::new();
                for c in &chars {
                    match c.to_char() {
                        Some(c) => owned.push(c),
                        None => return Err(invalid_isoformat(&dtstr, vm)?),
                    }
                }
                text = Some(owned.as_str());
            }
            let bytes = text.unwrap().as_bytes();
            let mut len = bytes.len();
            let separator_location = find_isoformat_datetime_separator(bytes, len);
            let (mut year, mut month, mut day) = (0, 0, 0);
            let mut vals = [0; 4];
            let (mut tzoffset, mut tzusec) = (0, 0);
            let mut rv = if separator_location == usize::MAX {
                -1
            } else {
                parse_isoformat_date(bytes, separator_location, &mut year, &mut month, &mut day)
            };
            if rv == 0 && len > separator_location {
                let mut p = separator_location;
                let c = byte_at(bytes, p);
                p += if c & 0x80 == 0 {
                    1
                } else {
                    match c & 0xf0 {
                        0xe0 => 3,
                        0xf0 => 4,
                        _ => 2,
                    }
                };
                len = len.saturating_sub(p);
                rv = parse_isoformat_time(bytes, p, len, &mut vals, &mut tzoffset, &mut tzusec);
            }
            if rv < 0 {
                return Err(invalid_isoformat(&dtstr, vm)?);
            }
            let tzinfo = tzinfo_from_isoformat_results(rv, tzoffset, tzusec, vm)?;
            let [mut hour, minute, second, microsecond] = vals;
            if hour == 24 && (1..=12).contains(&month) {
                let d_in_month = days_in_month(year, month);
                if day <= d_in_month {
                    if minute == 0 && second == 0 && microsecond == 0 {
                        hour = 0;
                        day += 1;
                        if day > d_in_month {
                            day = 1;
                            month += 1;
                            if month > 12 {
                                month = 1;
                                year += 1;
                            }
                        }
                    } else {
                        return Err(vm.new_value_error(
                            "minute, second, and microsecond must be 0 when hour is 24",
                        ));
                    }
                }
            }
            new_datetime_subclass(
                year,
                month,
                day,
                hour,
                minute,
                second,
                microsecond,
                tzinfo,
                0,
                &cls,
                vm,
            )
        }

        #[pymethod]
        fn utcoffset(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult {
            Ok(offset_to_py(Self::utcoffset_of(zelf, vm)?, vm))
        }

        #[pymethod]
        fn dst(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult {
            Ok(offset_to_py(
                call_dst(zelf.tzinfo.load_owned().as_ref(), zelf.as_object(), vm)?,
                vm,
            ))
        }

        #[pymethod]
        fn tzname(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult {
            Ok(
                call_tzname(zelf.tzinfo.load_owned().as_ref(), zelf.as_object(), vm)?
                    .map_or_else(|| vm.ctx.none(), Into::into),
            )
        }

        #[pymethod]
        fn date(zelf: &Py<Self>, vm: &VirtualMachine) -> PyRef<PyDate> {
            PyDate::new(zelf.date.y(), zelf.date.m(), zelf.date.d()).into_ref(&vm.ctx)
        }

        #[pymethod]
        fn time(zelf: &Py<Self>, vm: &VirtualMachine) -> PyRef<PyTime> {
            PyTime::new(
                i32::from(zelf.hour),
                i32::from(zelf.minute),
                i32::from(zelf.second),
                zelf.microsecond as i32,
                None,
                i32::from(zelf.fold),
            )
            .into_ref(&vm.ctx)
        }

        #[pymethod]
        fn timetz(zelf: &Py<Self>, vm: &VirtualMachine) -> PyRef<PyTime> {
            PyTime::new(
                i32::from(zelf.hour),
                i32::from(zelf.minute),
                i32::from(zelf.second),
                zelf.microsecond as i32,
                zelf.tzinfo.load_owned(),
                i32::from(zelf.fold),
            )
            .into_ref(&vm.ctx)
        }

        #[pymethod]
        fn ctime(zelf: &Py<Self>) -> String {
            format_ctime(
                zelf.date.y(),
                zelf.date.m(),
                zelf.date.d(),
                zelf.hour,
                zelf.minute,
                zelf.second,
            )
        }

        #[pymethod]
        fn timetuple(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult {
            let dstflag = match call_dst(zelf.tzinfo.load_owned().as_ref(), zelf.as_object(), vm)? {
                Some(dst) => i32::from(dst.is_nonzero()),
                None => -1,
            };
            let (y, mo, d, h, mi, s, _) = zelf.fields();
            build_struct_time(y, mo, d, h, mi, s, dstflag, vm)
        }

        #[pymethod]
        fn timestamp(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult {
            if zelf.tzinfo.load_owned().is_some() {
                let epoch = new_datetime(
                    1970,
                    1,
                    1,
                    0,
                    0,
                    0,
                    0,
                    Some(utc().to_owned().into()),
                    0,
                    datetime_type(),
                    vm,
                )?;
                let delta = Self::subtract_datetime(zelf, &epoch, vm)?;
                return PyDelta::total_seconds(&delta, vm);
            }
            let (y, mo, d, h, mi, s, us) = zelf.fields();
            let seconds = local_to_seconds(y, mo, d, h, mi, s, zelf.fold != 0, vm)?;
            Ok(vm
                .ctx
                .new_float((seconds - EPOCH_SECONDS) as f64 + f64::from(us) / 1e6)
                .into())
        }

        #[pymethod]
        fn utctimetuple(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult {
            let utcself: PyObjectRef = match Self::utcoffset_of(zelf, vm)? {
                None => zelf.to_owned().into(),
                Some(offset) => Self::add_delta(zelf, &offset, -1, vm)?,
            };
            let utc = as_datetime(&utcself)
                .ok_or_else(|| vm.new_type_error("datetime arithmetic must return a datetime"))?;
            let (y, mo, d, h, mi, s, _) = utc.fields();
            build_struct_time(y, mo, d, h, mi, s, 0, vm)
        }

        #[pymethod]
        fn isoformat(
            zelf: &Py<Self>,
            args: DateTimeIsoformatArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyStrRef> {
            let sep = match &args.sep {
                OptionalArg::Missing => CodePoint::from('T'),
                OptionalArg::Present(sep) => {
                    let Some(sep_str) = sep.downcast_ref::<PyStr>() else {
                        return Err(vm.new_type_error(format!(
                            "isoformat() argument 1 must be a unicode character, not {}",
                            type_name(sep)
                        )));
                    };
                    let chars: Vec<CodePoint> = sep_str.as_wtf8().code_points().collect();
                    match chars[..] {
                        [c] => c,
                        _ => {
                            return Err(vm.new_type_error(format!(
                                "isoformat() argument 1 must be a unicode character, not a string of length {}",
                                chars.len()
                            )));
                        }
                    }
                }
            };
            let spec = args.timespec.into_option().map(|s| s.to_string());
            let time = format_time_spec(
                spec.as_deref(),
                zelf.hour,
                zelf.minute,
                zelf.second,
                zelf.microsecond,
            )
            .map_err(|()| vm.new_value_error("Unknown timespec value"))?;
            let mut result = Wtf8Buf::from(
                format!(
                    "{:04}-{:02}-{:02}",
                    zelf.date.year, zelf.date.month, zelf.date.day
                )
                .as_str(),
            );
            result.push(sep);
            result.push_str(&time);
            result.push_str(&format_utcoffset(
                ":",
                zelf.tzinfo.load_owned().as_ref(),
                zelf.as_object(),
                vm,
            )?);
            Ok(vm.ctx.new_str(result))
        }

        #[pymethod]
        fn replace(zelf: &Py<Self>, args: DateTimeReplaceArgs, vm: &VirtualMachine) -> PyResult {
            let tzinfo = match args.tzinfo {
                OptionalArg::Present(tz) => tzinfo_arg(tz, vm),
                OptionalArg::Missing => zelf.tzinfo.load_owned(),
            };
            let (y, mo, d, h, mi, s, us) = zelf.fields();
            new_datetime_subclass(
                args.year.unwrap_or(y),
                args.month.unwrap_or(mo),
                args.day.unwrap_or(d),
                args.hour.unwrap_or(h),
                args.minute.unwrap_or(mi),
                args.second.unwrap_or(s),
                args.microsecond.unwrap_or(us),
                tzinfo,
                args.fold.unwrap_or_else(|| i32::from(zelf.fold)),
                zelf.class(),
                vm,
            )
        }

        #[pymethod]
        fn __replace__(zelf: &Py<Self>, changes: ReplaceChanges, vm: &VirtualMachine) -> PyResult {
            Self::replace(zelf, changes.args.bind_for(vm, "replace")?, vm)
        }

        #[pymethod]
        fn astimezone(zelf: &Py<Self>, args: AstimezoneArgs, vm: &VirtualMachine) -> PyResult {
            let tz = args.tz.unwrap_or_none(vm);
            check_tzinfo_subclass(&tz, vm)?;
            let target = tzinfo_arg(tz, vm);
            let local_zone = |vm: &VirtualMachine| -> PyResult<PyObjectRef> {
                let (y, mo, d, h, mi, s, _) = zelf.fields();
                let fold = zelf.fold != 0;
                let mut seconds = local_to_seconds(y, mo, d, h, mi, s, fold, vm)?;
                let seconds2 = local_to_seconds(y, mo, d, h, mi, s, !fold, vm)?;
                if seconds2 != seconds && (seconds2 > seconds) == fold {
                    seconds = seconds2;
                }
                Ok(local_timezone_from_timestamp(seconds - EPOCH_SECONDS, vm)?.into())
            };
            let self_tzinfo = match zelf.tzinfo.load_owned() {
                Some(tz) => tz,
                None => local_zone(vm)?,
            };
            if target.as_ref().is_some_and(|t| t.is(&self_tzinfo)) {
                return Ok(zelf.to_owned().into());
            }
            let offset = match call_utcoffset(Some(&self_tzinfo), zelf.as_object(), vm)? {
                Some(offset) => offset,
                None => {
                    let naive_zone = local_zone(vm)?;
                    call_utcoffset(Some(&naive_zone), zelf.as_object(), vm)?
                        .ok_or_else(|| vm.new_value_error("local utcoffset() is None"))?
                }
            };
            // Subclass construction can publish this object, so keep its identity
            // and use an atomic reference when replacing its timezone.
            let utc_result = Self::add_delta(zelf, &offset, -1, vm)?
                .downcast::<Self>()
                .map_err(|_| vm.new_type_error("datetime arithmetic must return a datetime"))?;
            utc_result.set_tzinfo(utc().to_owned().into());
            let tzinfo: PyObjectRef = match target {
                Some(tz) => tz,
                None => {
                    let epoch = new_datetime(
                        1970,
                        1,
                        1,
                        0,
                        0,
                        0,
                        0,
                        Some(utc().to_owned().into()),
                        0,
                        datetime_type(),
                        vm,
                    )?;
                    let delta = Self::subtract_datetime(&utc_result, &epoch, vm)?;
                    let seconds = delta.to_microseconds().div_euclid(1_000_000);
                    let timestamp = i64::try_from(seconds).map_err(|_| time_t_overflow(vm))?;
                    local_timezone_from_timestamp(timestamp, vm)?.into()
                }
            };
            utc_result.set_tzinfo(tzinfo.clone());
            vm.call_method(&tzinfo, "fromutc", (utc_result,))
        }

        #[pymethod]
        fn __reduce_ex__(zelf: &Py<Self>, proto: i32, vm: &VirtualMachine) -> PyRef<PyTuple> {
            vm.new_tuple((zelf.class().to_owned(), zelf.getstate(proto, vm)))
        }

        #[pymethod]
        fn __reduce__(zelf: &Py<Self>, vm: &VirtualMachine) -> PyRef<PyTuple> {
            vm.new_tuple((zelf.class().to_owned(), zelf.getstate(2, vm)))
        }

        #[pyslot]
        fn slot_str(zelf: &PyObject, vm: &VirtualMachine) -> PyResult<PyStrRef> {
            checked_str_result(vm.call_method(zelf, "isoformat", (" ",))?, vm)
        }

        #[extend_class]
        fn extend_class(ctx: &crate::vm::Context, class: &Py<PyType>) {
            let make = |y, mo, d, h, mi, s, us| {
                PyRef::new_ref(
                    Self::new(y, mo, d, h, mi, s, us, None, 0),
                    class.to_owned(),
                    None,
                )
                .into()
            };
            class.set_attr(ctx.intern_str("min"), make(1, 1, 1, 0, 0, 0, 0));
            class.set_attr(
                ctx.intern_str("max"),
                make(MAXYEAR, 12, 31, 23, 59, 59, 999_999),
            );
            let resolution = PyRef::new_ref(
                PyDelta::new_unchecked(0, 0, 1),
                PyDelta::static_type().to_owned(),
                None,
            );
            class.set_attr(ctx.intern_str("resolution"), resolution.into());
        }
    }

    impl AsNumber for PyDateTime {
        fn as_number() -> &'static PyNumberMethods {
            static AS_NUMBER: PyNumberMethods = PyNumberMethods {
                add: Some(PyDateTime::nb_add),
                subtract: Some(PyDateTime::nb_subtract),
                ..PyNumberMethods::NOT_IMPLEMENTED
            };
            &AS_NUMBER
        }
    }

    impl Comparable for PyDateTime {
        fn cmp(
            zelf: &Py<Self>,
            other: &PyObject,
            op: PyComparisonOp,
            vm: &VirtualMachine,
        ) -> PyResult<PyComparisonValue> {
            let Some(other) = as_datetime(other) else {
                return Ok(PyComparisonValue::NotImplemented);
            };
            if same_tzinfo(
                zelf.tzinfo.load_owned().as_ref(),
                other.tzinfo.load_owned().as_ref(),
            ) {
                return Ok(op.eval_ord(zelf.key().cmp(&other.key())).into());
            }
            let offset1 = Self::utcoffset_of(zelf, vm)?;
            let offset2 = Self::utcoffset_of(other, vm)?;
            let eq_op = matches!(op, PyComparisonOp::Eq | PyComparisonOp::Ne);
            let same_offsets = match (&offset1, &offset2) {
                (None, None) => true,
                (Some(a), Some(b)) => a.cmp_key() == b.cmp_key(),
                _ => false,
            };
            if same_offsets {
                let mut ord = zelf.key().cmp(&other.key());
                if eq_op
                    && ord.is_eq()
                    && Self::pep495_eq_exception(
                        zelf,
                        other,
                        offset1.as_ref(),
                        offset2.as_ref(),
                        vm,
                    )?
                {
                    ord = core::cmp::Ordering::Greater;
                }
                return Ok(op.eval_ord(ord).into());
            }
            if offset1.is_some() && offset2.is_some() {
                let delta = Self::subtract_datetime(zelf, other, vm)?;
                let mut ord = delta.cmp_key().cmp(&(0, 0, 0));
                if eq_op
                    && ord.is_eq()
                    && Self::pep495_eq_exception(
                        zelf,
                        other,
                        offset1.as_ref(),
                        offset2.as_ref(),
                        vm,
                    )?
                {
                    ord = core::cmp::Ordering::Greater;
                }
                return Ok(op.eval_ord(ord).into());
            }
            match op {
                PyComparisonOp::Eq => Ok(false.into()),
                PyComparisonOp::Ne => Ok(true.into()),
                _ => {
                    Err(vm.new_type_error("can't compare offset-naive and offset-aware datetimes"))
                }
            }
        }
    }

    impl Hashable for PyDateTime {
        fn hash(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyHash> {
            let cached = zelf.date.hashcode.load();
            if cached != -1 {
                return Ok(cached);
            }
            let self0: PyRef<Self> = if zelf.fold != 0 {
                let (y, mo, d, h, mi, s, us) = zelf.fields();
                new_datetime(
                    y,
                    mo,
                    d,
                    h,
                    mi,
                    s,
                    us,
                    zelf.tzinfo.load_owned(),
                    0,
                    zelf.class(),
                    vm,
                )?
            } else {
                zelf.to_owned()
            };
            let hash = match Self::utcoffset_of(&self0, vm)? {
                None => vm
                    .ctx
                    .new_bytes(zelf.state_bytes(0).to_vec())
                    .as_object()
                    .hash(vm)?,
                Some(offset) => {
                    let (_, _, _, h, mi, s, us) = zelf.fields();
                    let days = i64::from(zelf.date.ordinal());
                    let seconds = i64::from(h) * 3600 + i64::from(mi) * 60 + i64::from(s);
                    let delta = PyDelta::new_ref(
                        days - i64::from(offset.days),
                        seconds - i64::from(offset.seconds),
                        i64::from(us) - i64::from(offset.microseconds),
                        true,
                        vm,
                    )?;
                    <PyDelta as Hashable>::hash(&delta, vm)?
                }
            };
            zelf.date.hashcode.store(hash);
            Ok(hash)
        }
    }

    impl Representable for PyDateTime {
        fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
            let type_name = type_name(zelf.as_object());
            let (y, mo, d, h, mi, s, us) = zelf.fields();
            let repr = if us != 0 {
                format!("{type_name}({y}, {mo}, {d}, {h}, {mi}, {s}, {us})")
            } else if s != 0 {
                format!("{type_name}({y}, {mo}, {d}, {h}, {mi}, {s})")
            } else {
                format!("{type_name}({y}, {mo}, {d}, {h}, {mi})")
            };
            let repr = append_keyword_fold(repr, zelf.fold);
            append_keyword_tzinfo(repr, zelf.tzinfo.load_owned().as_ref(), vm)
        }
    }

    #[pyattr(name = "UTC")]
    fn utc_attr(_vm: &VirtualMachine) -> PyRef<PyTimeZone> {
        utc().to_owned()
    }
}

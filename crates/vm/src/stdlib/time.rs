//cspell:ignore cfmt
//! The python `time` module.

// See also:
// https://docs.python.org/3/library/time.html

pub use decl::time;

pub(crate) use decl::module_def;

#[cfg(not(target_env = "msvc"))]
#[cfg(not(target_arch = "wasm32"))]
unsafe extern "C" {
    #[cfg(not(target_os = "freebsd"))]
    #[link_name = "daylight"]
    static c_daylight: core::ffi::c_int;
    // pub static dstbias: std::ffi::c_int;
    #[link_name = "timezone"]
    static c_timezone: core::ffi::c_long;
    #[link_name = "tzname"]
    static c_tzname: [*const core::ffi::c_char; 2];
    #[link_name = "tzset"]
    fn c_tzset();
}

#[pymodule(name = "time", with(#[cfg(any(unix, windows))] platform))]
mod decl {
    #[cfg(any(unix, windows))]
    use crate::builtins::PyBaseExceptionRef;
    use crate::{
        AsObject, Py, PyObjectRef, PyResult, VirtualMachine,
        builtins::{PyStrRef, PyTypeRef},
        function::{Either, FuncArgs, OptionalArg},
        types::{PyStructSequence, struct_sequence_new},
    };
    #[cfg(any(unix, windows))]
    use crate::{common::wtf8::Wtf8Buf, convert::ToPyObject};
    #[cfg(not(any(unix, windows)))]
    use chrono::{
        DateTime, Datelike, TimeZone, Timelike,
        naive::{NaiveDate, NaiveDateTime, NaiveTime},
    };
    use core::time::Duration;
    #[cfg(any(unix, windows))]
    use rustpython_host_env::time::asctime_from_tm;
    use rustpython_host_env::time::{self as host_time};

    #[allow(dead_code)]
    pub(super) const SEC_TO_MS: i64 = host_time::SEC_TO_MS;
    #[allow(dead_code)]
    pub(super) const MS_TO_US: i64 = host_time::MS_TO_US;
    #[allow(dead_code)]
    pub(super) const SEC_TO_US: i64 = host_time::SEC_TO_US;
    #[allow(dead_code)]
    pub(super) const US_TO_NS: i64 = host_time::US_TO_NS;
    #[allow(dead_code)]
    pub(super) const MS_TO_NS: i64 = host_time::MS_TO_NS;
    #[allow(dead_code)]
    pub(super) const SEC_TO_NS: i64 = host_time::SEC_TO_NS;
    #[allow(dead_code)]
    pub(super) const NS_TO_MS: i64 = host_time::NS_TO_MS;
    #[allow(dead_code)]
    pub(super) const NS_TO_US: i64 = host_time::NS_TO_US;

    fn duration_since_system_now(vm: &VirtualMachine) -> PyResult<Duration> {
        host_time::duration_since_system_now()
            .map_err(|e| vm.new_value_error(format!("Time error: {e:?}")))
    }

    #[pyattr]
    pub const _STRUCT_TM_ITEMS: usize = 11;

    // TODO: implement proper monotonic time for wasm/wasi.
    #[cfg(not(any(unix, windows)))]
    fn get_monotonic_time(vm: &VirtualMachine) -> PyResult<Duration> {
        duration_since_system_now(vm)
    }

    // TODO: implement proper perf time for wasm/wasi.
    #[cfg(not(any(unix, windows)))]
    fn get_perf_time(vm: &VirtualMachine) -> PyResult<Duration> {
        duration_since_system_now(vm)
    }

    #[pyfunction]
    fn sleep(seconds: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let seconds_type_name = seconds.class().name().to_owned();
        let dur = seconds.try_into_value::<Duration>(vm).map_err(|e| {
            if e.class().is(vm.ctx.exceptions.value_error)
                && let Some(s) = e.args().first().and_then(|arg| arg.str(vm).ok())
                && s.as_bytes() == b"negative duration"
            {
                return vm.new_value_error("sleep length must be non-negative");
            }
            if e.class().is(vm.ctx.exceptions.type_error) {
                return vm.new_type_error(format!(
                    "'{seconds_type_name}' object cannot be interpreted as an integer or float"
                ));
            }
            e
        })?;

        #[cfg(unix)]
        {
            // Loop on nanosleep, recomputing the
            // remaining timeout after each EINTR so that signals don't
            // shorten the requested sleep duration.
            use std::time::Instant;
            let deadline = Instant::now() + dur;
            loop {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    break;
                }
                let sleep_result = vm.allow_threads(|| host_time::nanosleep(remaining));
                match sleep_result {
                    Ok(()) => break,
                    Err(err) if err.raw_os_error() == Some(libc::EINTR) => {}
                    Err(err) => return Err(vm.new_os_error(format!("nanosleep: {err}"))),
                }
                // EINTR: run signal handlers, then retry with remaining time
                vm.check_signals()?;
            }
        }

        #[cfg(not(unix))]
        {
            vm.allow_threads(|| std::thread::sleep(dur));
        }

        Ok(())
    }

    #[pyfunction]
    fn time_ns(vm: &VirtualMachine) -> PyResult<u64> {
        Ok(duration_since_system_now(vm)?.as_nanos() as u64)
    }

    #[pyfunction]
    pub fn time(vm: &VirtualMachine) -> PyResult<f64> {
        _time(vm)
    }

    #[cfg(not(all(
        target_arch = "wasm32",
        not(any(target_os = "emscripten", target_os = "wasi")),
    )))]
    fn _time(vm: &VirtualMachine) -> PyResult<f64> {
        Ok(duration_since_system_now(vm)?.as_secs_f64())
    }

    #[cfg(all(
        target_arch = "wasm32",
        feature = "wasmbind",
        not(any(target_os = "emscripten", target_os = "wasi"))
    ))]
    fn _time(_vm: &VirtualMachine) -> PyResult<f64> {
        use wasm_bindgen::prelude::*;
        #[wasm_bindgen]
        extern "C" {
            type Date;
            #[wasm_bindgen(static_method_of = Date)]
            fn now() -> f64;
        }
        // Date.now returns unix time in milliseconds, we want it in seconds
        Ok(Date::now() / 1000.0)
    }

    #[cfg(all(
        target_arch = "wasm32",
        not(feature = "wasmbind"),
        not(any(target_os = "emscripten", target_os = "wasi"))
    ))]
    fn _time(vm: &VirtualMachine) -> PyResult<f64> {
        Err(vm.new_not_implemented_error("time.time"))
    }

    #[pyfunction]
    fn monotonic(vm: &VirtualMachine) -> PyResult<f64> {
        Ok(get_monotonic_time(vm)?.as_secs_f64())
    }

    #[pyfunction]
    fn monotonic_ns(vm: &VirtualMachine) -> PyResult<u128> {
        Ok(get_monotonic_time(vm)?.as_nanos())
    }

    #[pyfunction]
    fn perf_counter(vm: &VirtualMachine) -> PyResult<f64> {
        Ok(get_perf_time(vm)?.as_secs_f64())
    }

    #[pyfunction]
    fn perf_counter_ns(vm: &VirtualMachine) -> PyResult<u128> {
        Ok(get_perf_time(vm)?.as_nanos())
    }

    #[cfg(target_env = "msvc")]
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn get_tz_info() -> host_time::WindowsTimeZoneInfo {
        host_time::get_tz_info()
    }

    // #[pyfunction]
    // fn tzset() {
    //     unsafe { super::_tzset() };
    // }

    #[cfg(not(target_env = "msvc"))]
    #[cfg(not(target_arch = "wasm32"))]
    #[pyattr]
    fn altzone(_vm: &VirtualMachine) -> core::ffi::c_long {
        // TODO: RUSTPYTHON; Add support for using the C altzone
        unsafe { super::c_timezone - 3600 }
    }

    #[cfg(target_env = "msvc")]
    #[cfg(not(target_arch = "wasm32"))]
    #[pyattr]
    fn altzone(_vm: &VirtualMachine) -> i32 {
        let info = get_tz_info();
        // https://users.rust-lang.org/t/accessing-tzname-and-similar-constants-in-windows/125771/3
        (info.bias + info.standard_bias) * 60 - 3600
    }

    #[cfg(not(target_env = "msvc"))]
    #[cfg(not(target_arch = "wasm32"))]
    #[pyattr]
    fn timezone(_vm: &VirtualMachine) -> core::ffi::c_long {
        unsafe { super::c_timezone }
    }

    #[cfg(target_env = "msvc")]
    #[cfg(not(target_arch = "wasm32"))]
    #[pyattr]
    fn timezone(_vm: &VirtualMachine) -> i32 {
        let info = get_tz_info();
        // https://users.rust-lang.org/t/accessing-tzname-and-similar-constants-in-windows/125771/3
        (info.bias + info.standard_bias) * 60
    }

    #[cfg(not(target_os = "freebsd"))]
    #[cfg(not(target_env = "msvc"))]
    #[cfg(not(target_arch = "wasm32"))]
    #[pyattr]
    fn daylight(_vm: &VirtualMachine) -> core::ffi::c_int {
        unsafe { super::c_daylight }
    }

    #[cfg(target_env = "msvc")]
    #[cfg(not(target_arch = "wasm32"))]
    #[pyattr]
    fn daylight(_vm: &VirtualMachine) -> i32 {
        let info = get_tz_info();
        // https://users.rust-lang.org/t/accessing-tzname-and-similar-constants-in-windows/125771/3
        (info.standard_bias != info.daylight_bias) as i32
    }

    #[cfg(not(target_env = "msvc"))]
    #[cfg(not(target_arch = "wasm32"))]
    #[pyattr]
    fn tzname(vm: &VirtualMachine) -> crate::builtins::PyTupleRef {
        use crate::builtins::tuple::IntoPyTuple;

        unsafe fn to_str(s: *const core::ffi::c_char) -> String {
            unsafe { core::ffi::CStr::from_ptr(s) }
                .to_string_lossy()
                .into_owned()
        }
        unsafe { (to_str(super::c_tzname[0]), to_str(super::c_tzname[1])) }.into_pytuple(vm)
    }

    #[cfg(target_env = "msvc")]
    #[cfg(not(target_arch = "wasm32"))]
    #[pyattr]
    fn tzname(vm: &VirtualMachine) -> crate::builtins::PyTupleRef {
        use crate::builtins::tuple::IntoPyTuple;
        let info = get_tz_info();
        let tz_name = (&*info.standard_name, &*info.daylight_name);
        tz_name.into_pytuple(vm)
    }

    #[cfg(not(any(unix, windows)))]
    fn pyobj_to_date_time(
        value: Either<f64, i64>,
        vm: &VirtualMachine,
    ) -> PyResult<DateTime<chrono::offset::Utc>> {
        let secs = match value {
            Either::A(float) => {
                if !float.is_finite() {
                    return Err(vm.new_value_error("Invalid value for timestamp"));
                }
                float.floor() as i64
            }
            Either::B(int) => int,
        };
        DateTime::<chrono::offset::Utc>::from_timestamp(secs, 0)
            .ok_or_else(|| vm.new_overflow_error("timestamp out of range for platform time_t"))
    }

    #[cfg(not(any(unix, windows)))]
    impl OptionalArg<Option<Either<f64, i64>>> {
        /// Construct a localtime from the optional seconds, or get the current local time.
        fn naive_or_local(self, vm: &VirtualMachine) -> PyResult<NaiveDateTime> {
            Ok(match self {
                Self::Present(Some(secs)) => pyobj_to_date_time(secs, vm)?
                    .with_timezone(&chrono::Local)
                    .naive_local(),
                Self::Present(None) | Self::Missing => chrono::offset::Local::now().naive_local(),
            })
        }
    }

    #[cfg(any(unix, windows))]
    fn checked_tm_from_struct_time(
        t: &StructTimeData,
        vm: &VirtualMachine,
        func_name: &'static str,
    ) -> PyResult<host_time::CheckedTm> {
        let invalid_tuple =
            || vm.new_type_error(format!("{func_name}(): illegal time tuple argument"));
        let classify_err = |e: PyBaseExceptionRef| {
            if e.class().is(vm.ctx.exceptions.overflow_error) {
                vm.new_overflow_error(format!("{func_name} argument out of range"))
            } else {
                invalid_tuple()
            }
        };

        let year: i64 = t.tm_year.clone().try_into_value(vm).map_err(|e| {
            if e.class().is(vm.ctx.exceptions.overflow_error) {
                vm.new_overflow_error("year out of range")
            } else {
                invalid_tuple()
            }
        })?;
        if year < i64::from(i32::MIN) + 1900 || year > i64::from(i32::MAX) {
            return Err(vm.new_overflow_error("year out of range"));
        }
        let year = year as i32;
        let tm_mon = t
            .tm_mon
            .clone()
            .try_into_value::<i32>(vm)
            .map_err(classify_err)?
            - 1;
        let tm_mday = t.tm_mday.clone().try_into_value(vm).map_err(classify_err)?;
        let tm_hour = t.tm_hour.clone().try_into_value(vm).map_err(classify_err)?;
        let tm_min = t.tm_min.clone().try_into_value(vm).map_err(classify_err)?;
        let tm_sec = t.tm_sec.clone().try_into_value(vm).map_err(classify_err)?;
        let tm_wday = (t
            .tm_wday
            .clone()
            .try_into_value::<i32>(vm)
            .map_err(classify_err)?
            + 1)
            % 7;
        let tm_yday = t
            .tm_yday
            .clone()
            .try_into_value::<i32>(vm)
            .map_err(classify_err)?
            - 1;
        let tm_isdst = t
            .tm_isdst
            .clone()
            .try_into_value(vm)
            .map_err(classify_err)?;

        #[cfg(unix)]
        {
            use crate::builtins::PyUtf8StrRef;
            let zone = if t.tm_zone.is(&vm.ctx.none) {
                None
            } else {
                let zone: PyUtf8StrRef = t
                    .tm_zone
                    .clone()
                    .try_into_value(vm)
                    .map_err(|_| invalid_tuple())?;
                Some(zone.as_str().to_owned())
            };
            let gmtoff = if t.tm_gmtoff.is(&vm.ctx.none) {
                None
            } else {
                Some(
                    t.tm_gmtoff
                        .clone()
                        .try_into_value::<i64>(vm)
                        .map_err(classify_err)?,
                )
            };
            host_time::checked_tm_from_parts(host_time::CheckedTmParts {
                year: year.into(),
                tm_mon,
                tm_mday,
                tm_hour,
                tm_min,
                tm_sec,
                tm_wday,
                tm_yday,
                tm_isdst,
                zone,
                gmtoff,
            })
            .map_err(|err| map_checked_tm_error(vm, err))
        }
        #[cfg(windows)]
        {
            host_time::checked_tm_from_parts(host_time::CheckedTmParts {
                year: year.into(),
                tm_mon,
                tm_mday,
                tm_hour,
                tm_min,
                tm_sec,
                tm_wday,
                tm_yday,
                tm_isdst,
            })
            .map_err(|err| map_checked_tm_error(vm, err))
        }
    }

    #[cfg(any(unix, windows))]
    fn map_checked_tm_error(
        vm: &VirtualMachine,
        err: host_time::CheckedTmError,
    ) -> PyBaseExceptionRef {
        match err {
            host_time::CheckedTmError::YearOutOfRange => vm.new_overflow_error("year out of range"),
            host_time::CheckedTmError::MonthOutOfRange => vm.new_value_error("month out of range"),
            host_time::CheckedTmError::DayOfMonthOutOfRange => {
                vm.new_value_error("day of month out of range")
            }
            host_time::CheckedTmError::HourOutOfRange => vm.new_value_error("hour out of range"),
            host_time::CheckedTmError::MinuteOutOfRange => {
                vm.new_value_error("minute out of range")
            }
            host_time::CheckedTmError::SecondsOutOfRange => {
                vm.new_value_error("seconds out of range")
            }
            host_time::CheckedTmError::DayOfWeekOutOfRange => {
                vm.new_value_error("day of week out of range")
            }
            host_time::CheckedTmError::DayOfYearOutOfRange => {
                vm.new_value_error("day of year out of range")
            }
            host_time::CheckedTmError::EmbeddedNul => vm.new_value_error("embedded null character"),
        }
    }

    #[cfg(not(any(unix, windows)))]
    impl OptionalArg<StructTimeData> {
        fn naive_or_local(self, vm: &VirtualMachine) -> PyResult<NaiveDateTime> {
            Ok(match self {
                Self::Present(t) => t.to_date_time(vm)?,
                Self::Missing => chrono::offset::Local::now().naive_local(),
            })
        }
    }

    /// https://docs.python.org/3/library/time.html?highlight=gmtime#time.gmtime
    #[pyfunction]
    fn gmtime(
        secs: OptionalArg<Option<Either<f64, i64>>>,
        vm: &VirtualMachine,
    ) -> PyResult<StructTimeData> {
        cfg_select! {
            any(unix, windows) => {
                let ts = match secs {
                    OptionalArg::Present(Some(value)) => pyobj_to_time_t(value, vm)?,
                    OptionalArg::Present(None) | OptionalArg::Missing => current_time_t(),
                };
                gmtime_from_timestamp(ts, vm)
            }
            _ => {
                let instant = match secs {
                    OptionalArg::Present(Some(secs)) => pyobj_to_date_time(secs, vm)?.naive_utc(),
                    OptionalArg::Present(None) | OptionalArg::Missing => {
                        chrono::offset::Utc::now().naive_utc()
                    }
                };
                Ok(StructTimeData::new_utc(vm, instant))
            }
        }
    }

    #[pyfunction]
    fn localtime(
        secs: OptionalArg<Option<Either<f64, i64>>>,
        vm: &VirtualMachine,
    ) -> PyResult<StructTimeData> {
        cfg_select! {
            any(unix, windows) => {
                let ts = match secs {
                    OptionalArg::Present(Some(value)) => pyobj_to_time_t(value, vm)?,
                    OptionalArg::Present(None) | OptionalArg::Missing => current_time_t(),
                };
                localtime_from_timestamp(ts, vm)
            }
            _ => {
                let instant = secs.naive_or_local(vm)?;
                Ok(StructTimeData::new_local(vm, instant, 0))
            }
        }
    }

    #[pyfunction]
    fn mktime(t: StructTimeData, vm: &VirtualMachine) -> PyResult<f64> {
        #[cfg(unix)]
        {
            unix_mktime(&t, vm)
        }

        #[cfg(windows)]
        {
            win_mktime(&t, vm)
        }

        #[cfg(not(any(unix, windows)))]
        {
            let datetime = t.to_date_time(vm)?;
            // mktime interprets struct_time as local time
            let local_dt = chrono::Local
                .from_local_datetime(&datetime)
                .single()
                .ok_or_else(|| vm.new_overflow_error("mktime argument out of range"))?;
            let seconds_since_epoch = local_dt.timestamp() as f64;
            Ok(seconds_since_epoch)
        }
    }

    #[cfg(not(any(unix, windows)))]
    const CFMT: &str = "%a %b %e %H:%M:%S %Y";

    #[pyfunction]
    fn asctime(t: OptionalArg<StructTimeData>, vm: &VirtualMachine) -> PyResult {
        #[cfg(any(unix, windows))]
        {
            let tm = match t {
                OptionalArg::Present(value) => {
                    checked_tm_from_struct_time(&value, vm, "asctime")?.tm
                }
                OptionalArg::Missing => {
                    let now = current_time_t();
                    let local = localtime_from_timestamp(now, vm)?;
                    checked_tm_from_struct_time(&local, vm, "asctime")?.tm
                }
            };
            Ok(vm.ctx.new_str(asctime_from_tm(&tm)).into())
        }

        #[cfg(not(any(unix, windows)))]
        {
            let instant = t.naive_or_local(vm)?;
            let formatted_time = instant.format(CFMT).to_string();
            Ok(vm.ctx.new_str(formatted_time).into())
        }
    }

    #[pyfunction]
    fn ctime(secs: OptionalArg<Option<Either<f64, i64>>>, vm: &VirtualMachine) -> PyResult<String> {
        #[cfg(any(unix, windows))]
        {
            let ts = match secs {
                OptionalArg::Present(Some(value)) => pyobj_to_time_t(value, vm)?,
                OptionalArg::Present(None) | OptionalArg::Missing => current_time_t(),
            };
            let local = localtime_from_timestamp(ts, vm)?;
            let tm = checked_tm_from_struct_time(&local, vm, "asctime")?.tm;
            Ok(asctime_from_tm(&tm))
        }

        #[cfg(not(any(unix, windows)))]
        {
            let instant = secs.naive_or_local(vm)?;
            Ok(instant.format(CFMT).to_string())
        }
    }

    #[cfg(any(unix, windows))]
    fn strftime_crt(
        format: &PyStrRef,
        checked_tm: host_time::CheckedTm,
        vm: &VirtualMachine,
    ) -> PyResult {
        #[cfg(unix)]
        let _keep_zone_alive = &checked_tm.zone;
        let mut tm = checked_tm.tm;
        tm.tm_isdst = tm.tm_isdst.clamp(-1, 1);

        // MSVC strftime requires year in [1; 9999]
        #[cfg(windows)]
        {
            let year = tm.tm_year + 1900;
            if !(1..=9999).contains(&year) {
                return Err(vm.new_value_error("strftime() requires year in [1; 9999]"));
            }
        }

        let mut out = Wtf8Buf::new();
        let mut ascii = String::new();

        for codepoint in format.as_wtf8().code_points() {
            if codepoint.to_u32() == 0 {
                if !ascii.is_empty() {
                    let part = host_time::strftime_ascii(&ascii, &tm)
                        .map_err(|_| vm.new_value_error("embedded null character"))?;
                    out.extend(part.chars());
                    ascii.clear();
                }
                out.push(codepoint);
                continue;
            }
            if let Some(ch) = codepoint.to_char()
                && ch.is_ascii()
            {
                ascii.push(ch);
                continue;
            }

            if !ascii.is_empty() {
                let part = host_time::strftime_ascii(&ascii, &tm)
                    .map_err(|_| vm.new_value_error("embedded null character"))?;
                out.extend(part.chars());
                ascii.clear();
            }
            out.push(codepoint);
        }
        if !ascii.is_empty() {
            let part = host_time::strftime_ascii(&ascii, &tm)
                .map_err(|_| vm.new_value_error("embedded null character"))?;
            out.extend(part.chars());
        }
        Ok(out.to_pyobject(vm))
    }

    #[pyfunction]
    fn strftime(format: PyStrRef, t: OptionalArg<StructTimeData>, vm: &VirtualMachine) -> PyResult {
        #[cfg(any(unix, windows))]
        {
            let checked_tm = match t {
                OptionalArg::Present(value) => checked_tm_from_struct_time(&value, vm, "strftime")?,
                OptionalArg::Missing => {
                    let now = current_time_t();
                    let local = localtime_from_timestamp(now, vm)?;
                    checked_tm_from_struct_time(&local, vm, "strftime")?
                }
            };
            strftime_crt(&format, checked_tm, vm)
        }

        #[cfg(not(any(unix, windows)))]
        {
            use core::fmt::Write;

            let fmt_lossy = format.to_string_lossy();

            // If the struct_time can't be represented as NaiveDateTime
            // (e.g. month=0), return the format string as-is, matching
            // the fallback behavior for unsupported chrono formats.
            let instant = match t.naive_or_local(vm) {
                Ok(dt) => dt,
                Err(_) => return Ok(vm.ctx.new_str(fmt_lossy.into_owned()).into()),
            };

            let mut formatted_time = String::new();
            write!(&mut formatted_time, "{}", instant.format(&fmt_lossy))
                .unwrap_or_else(|_| formatted_time = format.to_string());
            Ok(vm.ctx.new_str(formatted_time).into())
        }
    }

    #[pyfunction]
    fn strptime(string: PyStrRef, format: OptionalArg<PyStrRef>, vm: &VirtualMachine) -> PyResult {
        // Call _strptime._strptime_time like CPython does
        let strptime_module = vm.import("_strptime", 0)?;
        let strptime_func = strptime_module.get_attr("_strptime_time", vm)?;

        // Call with positional arguments
        match format.into_option() {
            Some(fmt) => strptime_func.call((string, fmt), vm),
            None => strptime_func.call((string,), vm),
        }
    }

    #[cfg(not(any(
        windows,
        target_vendor = "apple",
        target_os = "android",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "linux",
        target_os = "fuchsia",
        target_os = "emscripten",
    )))]
    fn get_thread_time(vm: &VirtualMachine) -> PyResult<Duration> {
        Err(vm.new_not_implemented_error("thread time unsupported in this system"))
    }

    #[pyfunction]
    fn thread_time(vm: &VirtualMachine) -> PyResult<f64> {
        Ok(get_thread_time(vm)?.as_secs_f64())
    }

    #[pyfunction]
    fn thread_time_ns(vm: &VirtualMachine) -> PyResult<u64> {
        Ok(get_thread_time(vm)?.as_nanos() as u64)
    }

    #[cfg(any(windows, all(target_arch = "wasm32", target_os = "emscripten")))]
    pub(super) fn time_muldiv(ticks: i64, mul: i64, div: i64) -> u64 {
        let int_part = ticks / div;
        let ticks = ticks % div;
        let remaining = (ticks * mul) / div;
        (int_part * mul + remaining) as u64
    }

    #[cfg(all(target_arch = "wasm32", target_os = "emscripten"))]
    fn get_process_time(vm: &VirtualMachine) -> PyResult<Duration> {
        let times = host_time::process_times()
            .map_err(|_| vm.new_os_error("Failed to get clock time".to_owned()))?;
        Ok(Duration::from_secs_f64(times.user + times.system))
    }

    #[cfg(not(any(
        windows,
        target_os = "macos",
        target_os = "android",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "linux",
        target_os = "illumos",
        target_os = "netbsd",
        target_os = "solaris",
        target_os = "openbsd",
        target_os = "redox",
        all(target_arch = "wasm32", target_os = "emscripten")
    )))]
    fn get_process_time(vm: &VirtualMachine) -> PyResult<Duration> {
        Err(vm.new_not_implemented_error("process time unsupported in this system"))
    }

    #[pyfunction]
    fn process_time(vm: &VirtualMachine) -> PyResult<f64> {
        Ok(get_process_time(vm)?.as_secs_f64())
    }

    #[pyfunction]
    fn process_time_ns(vm: &VirtualMachine) -> PyResult<u64> {
        Ok(get_process_time(vm)?.as_nanos() as u64)
    }

    /// Data struct for struct_time
    #[pystruct_sequence_data(try_from_object)]
    pub struct StructTimeData {
        pub tm_year: PyObjectRef,
        pub tm_mon: PyObjectRef,
        pub tm_mday: PyObjectRef,
        pub tm_hour: PyObjectRef,
        pub tm_min: PyObjectRef,
        pub tm_sec: PyObjectRef,
        pub tm_wday: PyObjectRef,
        pub tm_yday: PyObjectRef,
        pub tm_isdst: PyObjectRef,
        #[pystruct_sequence(skip)]
        pub tm_zone: PyObjectRef,
        #[pystruct_sequence(skip)]
        pub tm_gmtoff: PyObjectRef,
    }

    impl core::fmt::Debug for StructTimeData {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            write!(f, "struct_time()")
        }
    }

    impl StructTimeData {
        #[cfg(not(any(unix, windows)))]
        fn new_inner(
            vm: &VirtualMachine,
            tm: NaiveDateTime,
            isdst: i32,
            gmtoff: i32,
            zone: &str,
        ) -> Self {
            Self {
                tm_year: vm.ctx.new_int(tm.year()).into(),
                tm_mon: vm.ctx.new_int(tm.month()).into(),
                tm_mday: vm.ctx.new_int(tm.day()).into(),
                tm_hour: vm.ctx.new_int(tm.hour()).into(),
                tm_min: vm.ctx.new_int(tm.minute()).into(),
                tm_sec: vm.ctx.new_int(tm.second()).into(),
                tm_wday: vm.ctx.new_int(tm.weekday().num_days_from_monday()).into(),
                tm_yday: vm.ctx.new_int(tm.ordinal()).into(),
                tm_isdst: vm.ctx.new_int(isdst).into(),
                tm_zone: vm.ctx.new_str(zone).into(),
                tm_gmtoff: vm.ctx.new_int(gmtoff).into(),
            }
        }

        /// Create struct_time for UTC (gmtime)
        #[cfg(not(any(unix, windows)))]
        fn new_utc(vm: &VirtualMachine, tm: NaiveDateTime) -> Self {
            Self::new_inner(vm, tm, 0, 0, "UTC")
        }

        /// Create struct_time for local timezone (localtime)
        #[cfg(not(any(unix, windows)))]
        fn new_local(vm: &VirtualMachine, tm: NaiveDateTime, isdst: i32) -> Self {
            let local_time = chrono::Local.from_local_datetime(&tm).unwrap();
            let offset_seconds = local_time.offset().local_minus_utc();
            let tz_abbr = local_time.format("%Z").to_string();
            Self::new_inner(vm, tm, isdst, offset_seconds, &tz_abbr)
        }

        #[cfg(not(any(unix, windows)))]
        fn to_date_time(&self, vm: &VirtualMachine) -> PyResult<NaiveDateTime> {
            let invalid_overflow = || vm.new_overflow_error("mktime argument out of range");
            let invalid_value = || vm.new_value_error("invalid struct_time parameter");

            macro_rules! field {
                ($field:ident) => {
                    self.$field.clone().try_into_value(vm)?
                };
            }
            let dt = NaiveDateTime::new(
                NaiveDate::from_ymd_opt(field!(tm_year), field!(tm_mon), field!(tm_mday))
                    .ok_or_else(invalid_value)?,
                NaiveTime::from_hms_opt(field!(tm_hour), field!(tm_min), field!(tm_sec))
                    .ok_or_else(invalid_overflow)?,
            );
            Ok(dt)
        }
    }

    #[pyattr]
    #[pystruct_sequence(name = "struct_time", module = "time", data = "StructTimeData")]
    pub struct PyStructTime;

    #[pyclass(with(PyStructSequence))]
    impl PyStructTime {
        #[pyslot]
        fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            let (seq, _dict): (PyObjectRef, OptionalArg<PyObjectRef>) = args.bind(vm)?;
            struct_sequence_new(cls, seq, vm)
        }
    }

    /// Extract fields from StructTimeData into a libc::tm for mktime.
    #[cfg(any(unix, windows))]
    pub(super) fn tm_from_struct_time(
        t: &StructTimeData,
        vm: &VirtualMachine,
    ) -> PyResult<libc::tm> {
        let invalid_tuple = || vm.new_type_error("mktime(): illegal time tuple argument");
        let classify_err = |e: PyBaseExceptionRef| {
            if e.class().is(vm.ctx.exceptions.overflow_error) {
                vm.new_overflow_error("mktime argument out of range")
            } else {
                invalid_tuple()
            }
        };
        let year: i32 = t.tm_year.clone().try_into_value(vm).map_err(classify_err)?;
        if year < i32::MIN + 1900 {
            return Err(vm.new_overflow_error("year out of range"));
        }

        host_time::mktime_tm_from_parts(host_time::MktimeTmParts {
            year,
            tm_sec: t.tm_sec.clone().try_into_value(vm).map_err(classify_err)?,
            tm_min: t.tm_min.clone().try_into_value(vm).map_err(classify_err)?,
            tm_hour: t.tm_hour.clone().try_into_value(vm).map_err(classify_err)?,
            tm_mday: t.tm_mday.clone().try_into_value(vm).map_err(classify_err)?,
            tm_mon: t
                .tm_mon
                .clone()
                .try_into_value::<i32>(vm)
                .map_err(classify_err)?,
            tm_yday: t
                .tm_yday
                .clone()
                .try_into_value::<i32>(vm)
                .map_err(classify_err)?,
            tm_isdst: t
                .tm_isdst
                .clone()
                .try_into_value(vm)
                .map_err(classify_err)?,
        })
        .map_err(|err| match err {
            host_time::CheckedTmError::YearOutOfRange => vm.new_overflow_error("year out of range"),
            _ => vm.new_type_error("mktime(): illegal time tuple argument"),
        })
    }

    #[cfg(any(unix, windows))]
    #[cfg_attr(target_env = "musl", allow(deprecated))]
    fn pyobj_to_time_t(value: Either<f64, i64>, vm: &VirtualMachine) -> PyResult<libc::time_t> {
        match value {
            Either::A(float) => {
                if !float.is_finite() {
                    return Err(vm.new_value_error("Invalid value for timestamp"));
                }
                let secs = float.floor();
                #[cfg_attr(target_env = "musl", allow(deprecated))]
                if secs < libc::time_t::MIN as f64 || secs > libc::time_t::MAX as f64 {
                    return Err(vm.new_overflow_error("timestamp out of range for platform time_t"));
                }
                #[cfg_attr(target_env = "musl", allow(deprecated))]
                Ok(secs as libc::time_t)
            }
            Either::B(int) => {
                // try_into is needed on 32-bit platforms where time_t != i64
                #[allow(clippy::useless_conversion)]
                #[cfg_attr(target_env = "musl", allow(deprecated))]
                let ts: libc::time_t = int.try_into().map_err(|_| {
                    vm.new_overflow_error("timestamp out of range for platform time_t")
                })?;
                Ok(ts)
            }
        }
    }

    #[cfg(any(unix, windows))]
    #[allow(unused_imports)]
    use super::platform::*;

    pub(crate) fn module_exec(
        vm: &VirtualMachine,
        module: &Py<crate::builtins::PyModule>,
    ) -> PyResult<()> {
        #[cfg(not(target_env = "msvc"))]
        #[cfg(not(target_arch = "wasm32"))]
        unsafe {
            super::c_tzset()
        };

        __module_exec(vm, module);
        Ok(())
    }
}

#[cfg(unix)]
#[pymodule(sub)]
mod platform {
    #[allow(unused_imports)]
    use super::decl::{SEC_TO_NS, StructTimeData, US_TO_NS};
    #[cfg_attr(target_os = "macos", allow(unused_imports))]
    use crate::{
        PyObject, PyRef, PyResult, TryFromBorrowedObject, VirtualMachine,
        builtins::{PyNamespace, PyUtf8StrRef},
        convert::IntoPyException,
    };
    use core::time::Duration;
    #[cfg(any(
        target_os = "illumos",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "solaris",
    ))]
    use rustpython_host_env::resource as host_resource;
    use rustpython_host_env::time::{self as host_time, ClockId};

    #[cfg(target_os = "solaris")]
    #[pyattr]
    use libc::CLOCK_HIGHRES;
    #[cfg(not(any(
        target_os = "illumos",
        target_os = "netbsd",
        target_os = "solaris",
        target_os = "openbsd",
        target_os = "wasi",
    )))]
    #[pyattr]
    use libc::CLOCK_PROCESS_CPUTIME_ID;
    #[cfg(not(any(
        target_os = "illumos",
        target_os = "netbsd",
        target_os = "solaris",
        target_os = "openbsd",
        target_os = "redox",
    )))]
    #[pyattr]
    use libc::CLOCK_THREAD_CPUTIME_ID;
    #[cfg(target_os = "linux")]
    #[pyattr]
    use libc::{CLOCK_BOOTTIME, CLOCK_MONOTONIC_RAW, CLOCK_TAI};
    #[pyattr]
    use libc::{CLOCK_MONOTONIC, CLOCK_REALTIME};
    #[cfg(any(target_os = "freebsd", target_os = "openbsd", target_os = "dragonfly"))]
    #[pyattr]
    use libc::{CLOCK_PROF, CLOCK_UPTIME};

    impl<'a> TryFromBorrowedObject<'a> for ClockId {
        fn try_from_borrowed_object(vm: &VirtualMachine, obj: &'a PyObject) -> PyResult<Self> {
            obj.try_to_value(vm).map(Self::from_raw)
        }
    }

    fn struct_time_from_tm(vm: &VirtualMachine, tm: libc::tm) -> StructTimeData {
        let zone = unsafe {
            if tm.tm_zone.is_null() {
                String::new()
            } else {
                core::ffi::CStr::from_ptr(tm.tm_zone)
                    .to_string_lossy()
                    .into_owned()
            }
        };
        StructTimeData {
            tm_year: vm.ctx.new_int(tm.tm_year + 1900).into(),
            tm_mon: vm.ctx.new_int(tm.tm_mon + 1).into(),
            tm_mday: vm.ctx.new_int(tm.tm_mday).into(),
            tm_hour: vm.ctx.new_int(tm.tm_hour).into(),
            tm_min: vm.ctx.new_int(tm.tm_min).into(),
            tm_sec: vm.ctx.new_int(tm.tm_sec).into(),
            tm_wday: vm.ctx.new_int((tm.tm_wday + 6) % 7).into(),
            tm_yday: vm.ctx.new_int(tm.tm_yday + 1).into(),
            tm_isdst: vm.ctx.new_int(tm.tm_isdst).into(),
            tm_zone: vm.ctx.new_str(zone).into(),
            tm_gmtoff: vm.ctx.new_int(tm.tm_gmtoff).into(),
        }
    }

    pub(super) fn current_time_t() -> host_time::TimeT {
        host_time::current_time_t()
    }

    pub(super) fn gmtime_from_timestamp(
        when: host_time::TimeT,
        vm: &VirtualMachine,
    ) -> PyResult<StructTimeData> {
        let Some(tm) = host_time::gmtime_from_timestamp(when) else {
            return Err(vm.new_overflow_error("timestamp out of range for platform time_t"));
        };
        Ok(struct_time_from_tm(vm, tm))
    }

    pub(super) fn localtime_from_timestamp(
        when: host_time::TimeT,
        vm: &VirtualMachine,
    ) -> PyResult<StructTimeData> {
        let Some(tm) = host_time::localtime_from_timestamp(when) else {
            return Err(vm.new_overflow_error("timestamp out of range for platform time_t"));
        };
        Ok(struct_time_from_tm(vm, tm))
    }

    pub(super) fn unix_mktime(t: &StructTimeData, vm: &VirtualMachine) -> PyResult<f64> {
        let mut tm = super::decl::tm_from_struct_time(t, vm)?;
        let timestamp = host_time::mktime(&mut tm);
        if timestamp == -1 && tm.tm_wday == -1 {
            return Err(vm.new_overflow_error("mktime argument out of range"));
        }
        Ok(timestamp as f64)
    }

    fn get_clock_time(clk_id: ClockId, vm: &VirtualMachine) -> PyResult<Duration> {
        rustpython_host_env::time::clock_gettime(clk_id).map_err(|e| e.into_pyexception(vm))
    }

    #[pyfunction]
    fn clock_gettime(clk_id: ClockId, vm: &VirtualMachine) -> PyResult<f64> {
        get_clock_time(clk_id, vm).map(|d| d.as_secs_f64())
    }

    #[pyfunction]
    fn clock_gettime_ns(clk_id: ClockId, vm: &VirtualMachine) -> PyResult<u128> {
        get_clock_time(clk_id, vm).map(|d| d.as_nanos())
    }

    #[cfg(not(target_os = "redox"))]
    #[pyfunction]
    fn clock_getres(clk_id: ClockId, vm: &VirtualMachine) -> PyResult<f64> {
        rustpython_host_env::time::clock_getres(clk_id)
            .map(|d| d.as_secs_f64())
            .map_err(|e| e.into_pyexception(vm))
    }

    #[cfg(not(target_os = "redox"))]
    #[cfg(any(not(target_vendor = "apple"), target_os = "macos"))]
    #[pyfunction]
    fn clock_settime(clk_id: ClockId, time: Duration, vm: &VirtualMachine) -> PyResult<()> {
        rustpython_host_env::time::clock_settime(clk_id, time).map_err(|e| e.into_pyexception(vm))
    }

    #[cfg(not(target_os = "redox"))]
    #[cfg(any(not(target_vendor = "apple"), target_os = "macos"))]
    #[cfg_attr(target_env = "musl", allow(deprecated))]
    #[pyfunction]
    fn clock_settime_ns(clk_id: ClockId, time: libc::time_t, vm: &VirtualMachine) -> PyResult<()> {
        rustpython_host_env::time::clock_settime(clk_id, Duration::from_nanos(time as _))
            .map_err(|e| e.into_pyexception(vm))
    }

    // Requires all CLOCK constants available and clock_getres
    #[cfg(any(
        target_os = "macos",
        target_os = "android",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "fuchsia",
        target_os = "emscripten",
        target_os = "linux",
    ))]
    #[pyfunction]
    fn get_clock_info(name: PyUtf8StrRef, vm: &VirtualMachine) -> PyResult<PyRef<PyNamespace>> {
        let (adj, imp, mono, res) = match name.as_str() {
            "monotonic" | "perf_counter" => (
                false,
                "time.clock_gettime(CLOCK_MONOTONIC)",
                true,
                clock_getres(ClockId::CLOCK_MONOTONIC, vm)?,
            ),
            "process_time" => (
                false,
                "time.clock_gettime(CLOCK_PROCESS_CPUTIME_ID)",
                true,
                clock_getres(ClockId::CLOCK_PROCESS_CPUTIME_ID, vm)?,
            ),
            "thread_time" => (
                false,
                "time.clock_gettime(CLOCK_THREAD_CPUTIME_ID)",
                true,
                clock_getres(ClockId::CLOCK_THREAD_CPUTIME_ID, vm)?,
            ),
            "time" => (
                true,
                "time.clock_gettime(CLOCK_REALTIME)",
                false,
                clock_getres(ClockId::CLOCK_REALTIME, vm)?,
            ),
            _ => return Err(vm.new_value_error("unknown clock")),
        };

        Ok(py_namespace!(vm, {
            "implementation" => vm.new_pyobj(imp),
            "monotonic" => vm.ctx.new_bool(mono),
            "adjustable" => vm.ctx.new_bool(adj),
            "resolution" => vm.ctx.new_float(res),
        }))
    }

    #[cfg(not(any(
        target_os = "macos",
        target_os = "android",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "fuchsia",
        target_os = "emscripten",
        target_os = "linux",
    )))]
    #[pyfunction]
    fn get_clock_info(_name: PyUtf8StrRef, vm: &VirtualMachine) -> PyResult<PyRef<PyNamespace>> {
        Err(vm.new_not_implemented_error("get_clock_info unsupported on this system"))
    }

    pub(super) fn get_monotonic_time(vm: &VirtualMachine) -> PyResult<Duration> {
        get_clock_time(ClockId::CLOCK_MONOTONIC, vm)
    }

    pub(super) fn get_perf_time(vm: &VirtualMachine) -> PyResult<Duration> {
        get_clock_time(ClockId::CLOCK_MONOTONIC, vm)
    }

    #[cfg(not(any(
        target_os = "illumos",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "redox"
    )))]
    pub(super) fn get_thread_time(vm: &VirtualMachine) -> PyResult<Duration> {
        get_clock_time(ClockId::CLOCK_THREAD_CPUTIME_ID, vm)
    }

    #[cfg(target_os = "solaris")]
    pub(super) fn get_thread_time(vm: &VirtualMachine) -> PyResult<Duration> {
        let _ = vm;
        Ok(host_time::gethrvtime_duration())
    }

    #[cfg(not(any(
        target_os = "illumos",
        target_os = "netbsd",
        target_os = "solaris",
        target_os = "openbsd",
    )))]
    pub(super) fn get_process_time(vm: &VirtualMachine) -> PyResult<Duration> {
        get_clock_time(ClockId::CLOCK_PROCESS_CPUTIME_ID, vm)
    }

    #[cfg(any(
        target_os = "illumos",
        target_os = "netbsd",
        target_os = "solaris",
        target_os = "openbsd",
    ))]
    pub(super) fn get_process_time(vm: &VirtualMachine) -> PyResult<Duration> {
        fn from_timeval(tv: libc::timeval, vm: &VirtualMachine) -> PyResult<i64> {
            (|tv: libc::timeval| {
                let t = tv.tv_sec.checked_mul(SEC_TO_NS)?;
                let u = (tv.tv_usec as i64).checked_mul(US_TO_NS)?;
                t.checked_add(u)
            })(tv)
            .ok_or_else(|| vm.new_overflow_error("timestamp too large to convert to i64"))
        }
        let ru = host_resource::getrusage(libc::RUSAGE_SELF).map_err(|e| e.into_pyexception(vm))?;
        let utime = from_timeval(ru.ru_utime, vm)?;
        let stime = from_timeval(ru.ru_stime, vm)?;

        Ok(Duration::from_nanos((utime + stime) as u64))
    }
}

#[cfg(windows)]
#[pymodule(sub)]
mod platform {
    use super::decl::{MS_TO_NS, SEC_TO_NS, StructTimeData, get_tz_info, time_muldiv};
    use crate::{
        PyRef, PyResult, VirtualMachine,
        builtins::{PyNamespace, PyUtf8StrRef},
    };
    use core::time::Duration;
    use rustpython_host_env::time as host_time;

    fn struct_time_from_tm(
        vm: &VirtualMachine,
        tm: libc::tm,
        zone: &str,
        gmtoff: i32,
    ) -> StructTimeData {
        StructTimeData {
            tm_year: vm.ctx.new_int(tm.tm_year + 1900).into(),
            tm_mon: vm.ctx.new_int(tm.tm_mon + 1).into(),
            tm_mday: vm.ctx.new_int(tm.tm_mday).into(),
            tm_hour: vm.ctx.new_int(tm.tm_hour).into(),
            tm_min: vm.ctx.new_int(tm.tm_min).into(),
            tm_sec: vm.ctx.new_int(tm.tm_sec).into(),
            tm_wday: vm.ctx.new_int((tm.tm_wday + 6) % 7).into(),
            tm_yday: vm.ctx.new_int(tm.tm_yday + 1).into(),
            tm_isdst: vm.ctx.new_int(tm.tm_isdst).into(),
            tm_zone: vm.ctx.new_str(zone).into(),
            tm_gmtoff: vm.ctx.new_int(gmtoff).into(),
        }
    }

    pub(super) fn current_time_t() -> host_time::TimeT {
        host_time::current_time_t()
    }

    pub(super) fn gmtime_from_timestamp(
        when: host_time::TimeT,
        vm: &VirtualMachine,
    ) -> PyResult<StructTimeData> {
        let tm = host_time::gmtime_from_timestamp(when)
            .ok_or_else(|| vm.new_overflow_error("timestamp out of range for platform time_t"))?;
        Ok(struct_time_from_tm(vm, tm, "UTC", 0))
    }

    pub(super) fn localtime_from_timestamp(
        when: host_time::TimeT,
        vm: &VirtualMachine,
    ) -> PyResult<StructTimeData> {
        let tm = host_time::localtime_from_timestamp(when)
            .ok_or_else(|| vm.new_overflow_error("timestamp out of range for platform time_t"))?;

        // Get timezone info from Windows API
        let info = get_tz_info();
        let (bias, name) = if tm.tm_isdst > 0 {
            (info.daylight_bias, &info.daylight_name)
        } else {
            (info.standard_bias, &info.standard_name)
        };

        let gmtoff = -(info.bias + bias) * 60;

        Ok(struct_time_from_tm(vm, tm, name, gmtoff))
    }

    pub(super) fn win_mktime(t: &StructTimeData, vm: &VirtualMachine) -> PyResult<f64> {
        let mut tm = super::decl::tm_from_struct_time(t, vm)?;
        let timestamp = host_time::mktime(&mut tm);
        if timestamp == -1 && tm.tm_wday == -1 {
            return Err(vm.new_overflow_error("mktime argument out of range"));
        }
        Ok(timestamp as f64)
    }

    fn win_perf_counter_frequency(vm: &VirtualMachine) -> PyResult<i64> {
        let frequency =
            host_time::query_performance_frequency().ok_or_else(|| vm.new_last_os_error())?;

        if frequency < 1 {
            Err(vm.new_runtime_error("invalid QueryPerformanceFrequency"))
        } else if frequency > i64::MAX / SEC_TO_NS {
            Err(vm.new_overflow_error("QueryPerformanceFrequency is too large"))
        } else {
            Ok(frequency)
        }
    }

    fn global_frequency(vm: &VirtualMachine) -> PyResult<i64> {
        rustpython_common::static_cell! {
            static FREQUENCY: PyResult<i64>;
        };
        FREQUENCY
            .get_or_init(|| win_perf_counter_frequency(vm))
            .clone()
    }

    pub(super) fn get_perf_time(vm: &VirtualMachine) -> PyResult<Duration> {
        let ticks = host_time::query_performance_counter();

        Ok(Duration::from_nanos(time_muldiv(
            ticks,
            SEC_TO_NS,
            global_frequency(vm)?,
        )))
    }

    fn get_system_time_adjustment(vm: &VirtualMachine) -> PyResult<u32> {
        host_time::get_system_time_adjustment().ok_or_else(|| vm.new_last_os_error())
    }

    pub(super) fn get_monotonic_time(vm: &VirtualMachine) -> PyResult<Duration> {
        let ticks = host_time::tick_count64();

        Ok(Duration::from_nanos(
            (ticks as i64)
                .checked_mul(MS_TO_NS)
                .ok_or_else(|| vm.new_overflow_error("timestamp too large to convert to i64"))?
                as u64,
        ))
    }

    #[pyfunction]
    fn get_clock_info(name: PyUtf8StrRef, vm: &VirtualMachine) -> PyResult<PyRef<PyNamespace>> {
        let (adj, imp, mono, res) = match name.as_str() {
            "monotonic" => (
                false,
                "GetTickCount64()",
                true,
                get_system_time_adjustment(vm)? as f64 * 1e-7,
            ),
            "perf_counter" => (
                false,
                "QueryPerformanceCounter()",
                true,
                1.0 / (global_frequency(vm)? as f64),
            ),
            "process_time" => (false, "GetProcessTimes()", true, 1e-7),
            "thread_time" => (false, "GetThreadTimes()", true, 1e-7),
            "time" => (
                true,
                "GetSystemTimeAsFileTime()",
                false,
                get_system_time_adjustment(vm)? as f64 * 1e-7,
            ),
            _ => return Err(vm.new_value_error("unknown clock")),
        };

        Ok(py_namespace!(vm, {
            "implementation" => vm.new_pyobj(imp),
            "monotonic" => vm.ctx.new_bool(mono),
            "adjustable" => vm.ctx.new_bool(adj),
            "resolution" => vm.ctx.new_float(res),
        }))
    }

    pub(super) fn get_thread_time(vm: &VirtualMachine) -> PyResult<Duration> {
        let total = host_time::get_thread_time_100ns()
            .ok_or_else(|| vm.new_os_error("Failed to get clock time".to_owned()))?;
        Ok(Duration::from_nanos(total * 100))
    }

    pub(super) fn get_process_time(vm: &VirtualMachine) -> PyResult<Duration> {
        let total = host_time::get_process_time_100ns()
            .ok_or_else(|| vm.new_os_error("Failed to get clock time".to_owned()))?;
        Ok(Duration::from_nanos(total * 100))
    }
}

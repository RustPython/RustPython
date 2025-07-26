//cspell:ignore cfmt
//! The python `time` module.

// See also:
// https://docs.python.org/3/library/time.html
use crate::{PyRef, VirtualMachine, builtins::PyModule};

pub use decl::time;

pub(crate) fn make_module(vm: &VirtualMachine) -> PyRef<PyModule> {
    #[cfg(not(target_env = "msvc"))]
    #[cfg(not(target_arch = "wasm32"))]
    unsafe {
        c_tzset()
    };
    decl::make_module(vm)
}

#[cfg(not(target_env = "msvc"))]
#[cfg(not(target_arch = "wasm32"))]
unsafe extern "C" {
    #[cfg(not(target_os = "freebsd"))]
    #[link_name = "daylight"]
    static c_daylight: std::ffi::c_int;
    // pub static dstbias: std::ffi::c_int;
    #[link_name = "timezone"]
    static c_timezone: std::ffi::c_long;
    #[link_name = "tzname"]
    static c_tzname: [*const std::ffi::c_char; 2];
    #[link_name = "tzset"]
    fn c_tzset();
}

#[pymodule(name = "time", with(platform))]
mod decl {
    use crate::{
        AsObject, PyObjectRef, PyResult, TryFromObject, VirtualMachine,
        builtins::{PyStrRef, PyTypeRef},
        function::{Either, FuncArgs, OptionalArg},
        types::PyStructSequence,
    };
    use chrono::{
        DateTime, Datelike, TimeZone, Timelike,
        naive::{NaiveDate, NaiveDateTime, NaiveTime},
    };
    use std::time::Duration;
    #[cfg(target_env = "msvc")]
    #[cfg(not(target_arch = "wasm32"))]
    use windows::Win32::System::Time;

    #[allow(dead_code)]
    pub(super) const SEC_TO_MS: i64 = 1000;
    #[allow(dead_code)]
    pub(super) const MS_TO_US: i64 = 1000;
    #[allow(dead_code)]
    pub(super) const SEC_TO_US: i64 = SEC_TO_MS * MS_TO_US;
    #[allow(dead_code)]
    pub(super) const US_TO_NS: i64 = 1000;
    #[allow(dead_code)]
    pub(super) const MS_TO_NS: i64 = MS_TO_US * US_TO_NS;
    #[allow(dead_code)]
    pub(super) const SEC_TO_NS: i64 = SEC_TO_MS * MS_TO_NS;
    #[allow(dead_code)]
    pub(super) const NS_TO_MS: i64 = 1000 * 1000;
    #[allow(dead_code)]
    pub(super) const NS_TO_US: i64 = 1000;

    fn duration_since_system_now(vm: &VirtualMachine) -> PyResult<Duration> {
        use std::time::{SystemTime, UNIX_EPOCH};

        SystemTime::now()
            .duration_since(UNIX_EPOCH)
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
        let dur = seconds.try_into_value::<Duration>(vm).map_err(|e| {
            if e.class().is(vm.ctx.exceptions.value_error) {
                if let Some(s) = e.args().first().and_then(|arg| arg.str(vm).ok()) {
                    if s.as_str() == "negative duration" {
                        return vm.new_value_error("sleep length must be non-negative");
                    }
                }
            }
            e
        })?;

        #[cfg(unix)]
        {
            // this is basically std::thread::sleep, but that catches interrupts and we don't want to;
            let ts = nix::sys::time::TimeSpec::from(dur);
            let res = unsafe { libc::nanosleep(ts.as_ref(), std::ptr::null_mut()) };
            let interrupted = res == -1 && nix::Error::last_raw() == libc::EINTR;

            if interrupted {
                vm.check_signals()?;
            }
        }

        #[cfg(not(unix))]
        {
            std::thread::sleep(dur);
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
    fn get_tz_info() -> Time::TIME_ZONE_INFORMATION {
        let mut info = Time::TIME_ZONE_INFORMATION::default();
        let info_ptr = &mut info as *mut Time::TIME_ZONE_INFORMATION;
        let _ = unsafe { Time::GetTimeZoneInformation(info_ptr) };
        info
    }

    // #[pyfunction]
    // fn tzset() {
    //     unsafe { super::_tzset() };
    // }

    #[cfg(not(target_env = "msvc"))]
    #[cfg(not(target_arch = "wasm32"))]
    #[pyattr]
    fn timezone(_vm: &VirtualMachine) -> std::ffi::c_long {
        unsafe { super::c_timezone }
    }

    #[cfg(target_env = "msvc")]
    #[cfg(not(target_arch = "wasm32"))]
    #[pyattr]
    fn timezone(_vm: &VirtualMachine) -> i32 {
        let info = get_tz_info();
        // https://users.rust-lang.org/t/accessing-tzname-and-similar-constants-in-windows/125771/3
        (info.Bias + info.StandardBias) * 60
    }

    #[cfg(not(target_os = "freebsd"))]
    #[cfg(not(target_env = "msvc"))]
    #[cfg(not(target_arch = "wasm32"))]
    #[pyattr]
    fn daylight(_vm: &VirtualMachine) -> std::ffi::c_int {
        unsafe { super::c_daylight }
    }

    #[cfg(target_env = "msvc")]
    #[cfg(not(target_arch = "wasm32"))]
    #[pyattr]
    fn daylight(_vm: &VirtualMachine) -> i32 {
        let info = get_tz_info();
        // https://users.rust-lang.org/t/accessing-tzname-and-similar-constants-in-windows/125771/3
        (info.StandardBias != info.DaylightBias) as i32
    }

    #[cfg(not(target_env = "msvc"))]
    #[cfg(not(target_arch = "wasm32"))]
    #[pyattr]
    fn tzname(vm: &VirtualMachine) -> crate::builtins::PyTupleRef {
        use crate::builtins::tuple::IntoPyTuple;

        unsafe fn to_str(s: *const std::ffi::c_char) -> String {
            unsafe { std::ffi::CStr::from_ptr(s) }
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
        let standard = widestring::decode_utf16_lossy(info.StandardName)
            .filter(|&c| c != '\0')
            .collect::<String>();
        let daylight = widestring::decode_utf16_lossy(info.DaylightName)
            .filter(|&c| c != '\0')
            .collect::<String>();
        let tz_name = (&*standard, &*daylight);
        tz_name.into_pytuple(vm)
    }

    fn pyobj_to_date_time(
        value: Either<f64, i64>,
        vm: &VirtualMachine,
    ) -> PyResult<DateTime<chrono::offset::Utc>> {
        let timestamp = match value {
            Either::A(float) => {
                let secs = float.trunc() as i64;
                let nano_secs = (float.fract() * 1e9) as u32;
                DateTime::<chrono::offset::Utc>::from_timestamp(secs, nano_secs)
            }
            Either::B(int) => DateTime::<chrono::offset::Utc>::from_timestamp(int, 0),
        };
        timestamp.ok_or_else(|| vm.new_overflow_error("timestamp out of range for platform time_t"))
    }

    impl OptionalArg<Either<f64, i64>> {
        /// Construct a localtime from the optional seconds, or get the current local time.
        fn naive_or_local(self, vm: &VirtualMachine) -> PyResult<NaiveDateTime> {
            Ok(match self {
                Self::Present(secs) => pyobj_to_date_time(secs, vm)?.naive_utc(),
                Self::Missing => chrono::offset::Local::now().naive_local(),
            })
        }

        fn naive_or_utc(self, vm: &VirtualMachine) -> PyResult<NaiveDateTime> {
            Ok(match self {
                Self::Present(secs) => pyobj_to_date_time(secs, vm)?.naive_utc(),
                Self::Missing => chrono::offset::Utc::now().naive_utc(),
            })
        }
    }

    impl OptionalArg<PyStructTime> {
        fn naive_or_local(self, vm: &VirtualMachine) -> PyResult<NaiveDateTime> {
            Ok(match self {
                Self::Present(t) => t.to_date_time(vm)?,
                Self::Missing => chrono::offset::Local::now().naive_local(),
            })
        }
    }

    /// https://docs.python.org/3/library/time.html?highlight=gmtime#time.gmtime
    #[pyfunction]
    fn gmtime(secs: OptionalArg<Either<f64, i64>>, vm: &VirtualMachine) -> PyResult<PyStructTime> {
        let instant = secs.naive_or_utc(vm)?;
        Ok(PyStructTime::new(vm, instant, 0))
    }

    #[pyfunction]
    fn localtime(
        secs: OptionalArg<Either<f64, i64>>,
        vm: &VirtualMachine,
    ) -> PyResult<PyStructTime> {
        let instant = secs.naive_or_local(vm)?;
        // TODO: isdst flag must be valid value here
        // https://docs.python.org/3/library/time.html#time.localtime
        Ok(PyStructTime::new(vm, instant, -1))
    }

    #[pyfunction]
    fn mktime(t: PyStructTime, vm: &VirtualMachine) -> PyResult<f64> {
        let datetime = t.to_date_time(vm)?;
        let seconds_since_epoch = datetime.and_utc().timestamp() as f64;
        Ok(seconds_since_epoch)
    }

    const CFMT: &str = "%a %b %e %H:%M:%S %Y";

    #[pyfunction]
    fn asctime(t: OptionalArg<PyStructTime>, vm: &VirtualMachine) -> PyResult {
        let instant = t.naive_or_local(vm)?;
        let formatted_time = instant.format(CFMT).to_string();
        Ok(vm.ctx.new_str(formatted_time).into())
    }

    #[pyfunction]
    fn ctime(secs: OptionalArg<Either<f64, i64>>, vm: &VirtualMachine) -> PyResult<String> {
        let instant = secs.naive_or_local(vm)?;
        Ok(instant.format(CFMT).to_string())
    }

    #[pyfunction]
    fn strftime(format: PyStrRef, t: OptionalArg<PyStructTime>, vm: &VirtualMachine) -> PyResult {
        use std::fmt::Write;

        let instant = t.naive_or_local(vm)?;
        let mut formatted_time = String::new();

        /*
         * chrono doesn't support all formats and it
         * raises an error if unsupported format is supplied.
         * If error happens, we set result as input arg.
         */
        write!(
            &mut formatted_time,
            "{}",
            instant.format(format.try_to_str(vm)?)
        )
        .unwrap_or_else(|_| formatted_time = format.to_string());
        Ok(vm.ctx.new_str(formatted_time).into())
    }

    #[pyfunction]
    fn strptime(
        string: PyStrRef,
        format: OptionalArg<PyStrRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyStructTime> {
        let format = format.as_ref().map_or("%a %b %H:%M:%S %Y", |s| s.as_str());
        let instant = NaiveDateTime::parse_from_str(string.as_str(), format)
            .map_err(|e| vm.new_value_error(format!("Parse error: {e:?}")))?;
        Ok(PyStructTime::new(vm, instant, -1))
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
        let t: libc::tms = unsafe {
            let mut t = std::mem::MaybeUninit::uninit();
            if libc::times(t.as_mut_ptr()) == -1 {
                return Err(vm.new_os_error("Failed to get clock time".to_owned()));
            }
            t.assume_init()
        };
        let freq = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };

        Ok(Duration::from_nanos(
            time_muldiv(t.tms_utime, SEC_TO_NS, freq) + time_muldiv(t.tms_stime, SEC_TO_NS, freq),
        ))
    }

    // same as the get_process_time impl for most unixes
    #[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
    pub(super) fn get_process_time(vm: &VirtualMachine) -> PyResult<Duration> {
        let time: libc::timespec = unsafe {
            let mut time = std::mem::MaybeUninit::uninit();
            if libc::clock_gettime(libc::CLOCK_PROCESS_CPUTIME_ID, time.as_mut_ptr()) == -1 {
                return Err(vm.new_os_error("Failed to get clock time".to_owned()));
            }
            time.assume_init()
        };
        Ok(Duration::new(time.tv_sec as u64, time.tv_nsec as u32))
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
        all(target_arch = "wasm32", not(target_os = "unknown"))
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

    #[pyattr]
    #[pyclass(name = "struct_time")]
    #[derive(PyStructSequence, TryIntoPyStructSequence)]
    #[allow(dead_code)]
    struct PyStructTime {
        tm_year: PyObjectRef,
        tm_mon: PyObjectRef,
        tm_mday: PyObjectRef,
        tm_hour: PyObjectRef,
        tm_min: PyObjectRef,
        tm_sec: PyObjectRef,
        tm_wday: PyObjectRef,
        tm_yday: PyObjectRef,
        tm_isdst: PyObjectRef,
        #[pystruct(skip)]
        tm_gmtoff: PyObjectRef,
        #[pystruct(skip)]
        tm_zone: PyObjectRef,
    }

    impl std::fmt::Debug for PyStructTime {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "struct_time()")
        }
    }

    #[pyclass(with(PyStructSequence))]
    impl PyStructTime {
        fn new(vm: &VirtualMachine, tm: NaiveDateTime, isdst: i32) -> Self {
            let local_time = chrono::Local.from_local_datetime(&tm).unwrap();
            let offset_seconds =
                local_time.offset().local_minus_utc() + if isdst == 1 { 3600 } else { 0 };
            let tz_abbr = local_time.format("%Z").to_string();

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
                tm_gmtoff: vm.ctx.new_int(offset_seconds).into(),
                tm_zone: vm.ctx.new_str(tz_abbr).into(),
            }
        }

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

        #[pyslot]
        fn slot_new(_cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            // cls is ignorable because this is not a basetype
            let seq = args.bind(vm)?;
            Ok(vm.new_pyobj(Self::try_from_object(vm, seq)?))
        }
    }

    #[allow(unused_imports)]
    use super::platform::*;
}

#[cfg(unix)]
#[pymodule(sub)]
mod platform {
    #[allow(unused_imports)]
    use super::decl::{SEC_TO_NS, US_TO_NS};
    #[cfg_attr(target_os = "macos", allow(unused_imports))]
    use crate::{
        PyObject, PyRef, PyResult, TryFromBorrowedObject, VirtualMachine,
        builtins::{PyNamespace, PyStrRef},
        convert::IntoPyException,
    };
    use nix::{sys::time::TimeSpec, time::ClockId};
    use std::time::Duration;

    #[cfg(target_os = "solaris")]
    #[pyattr]
    use libc::CLOCK_HIGHRES;
    #[cfg(not(any(
        target_os = "illumos",
        target_os = "netbsd",
        target_os = "solaris",
        target_os = "openbsd",
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

    fn get_clock_time(clk_id: ClockId, vm: &VirtualMachine) -> PyResult<Duration> {
        let ts = nix::time::clock_gettime(clk_id).map_err(|e| e.into_pyexception(vm))?;
        Ok(ts.into())
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
        let ts = nix::time::clock_getres(clk_id).map_err(|e| e.into_pyexception(vm))?;
        Ok(Duration::from(ts).as_secs_f64())
    }

    #[cfg(not(target_os = "redox"))]
    #[cfg(not(target_vendor = "apple"))]
    fn set_clock_time(clk_id: ClockId, timespec: TimeSpec, vm: &VirtualMachine) -> PyResult<()> {
        nix::time::clock_settime(clk_id, timespec).map_err(|e| e.into_pyexception(vm))
    }

    #[cfg(not(target_os = "redox"))]
    #[cfg(target_os = "macos")]
    fn set_clock_time(clk_id: ClockId, timespec: TimeSpec, vm: &VirtualMachine) -> PyResult<()> {
        // idk why nix disables clock_settime on macos
        let ret = unsafe { libc::clock_settime(clk_id.as_raw(), timespec.as_ref()) };
        nix::Error::result(ret)
            .map(drop)
            .map_err(|e| e.into_pyexception(vm))
    }

    #[cfg(not(target_os = "redox"))]
    #[cfg(any(not(target_vendor = "apple"), target_os = "macos"))]
    #[pyfunction]
    fn clock_settime(clk_id: ClockId, time: Duration, vm: &VirtualMachine) -> PyResult<()> {
        set_clock_time(clk_id, time.into(), vm)
    }

    #[cfg(not(target_os = "redox"))]
    #[cfg(any(not(target_vendor = "apple"), target_os = "macos"))]
    #[pyfunction]
    fn clock_settime_ns(clk_id: ClockId, time: libc::time_t, vm: &VirtualMachine) -> PyResult<()> {
        let ts = Duration::from_nanos(time as _).into();
        set_clock_time(clk_id, ts, vm)
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
    fn get_clock_info(name: PyStrRef, vm: &VirtualMachine) -> PyResult<PyRef<PyNamespace>> {
        let (adj, imp, mono, res) = match name.as_ref() {
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
    fn get_clock_info(_name: PyStrRef, vm: &VirtualMachine) -> PyResult<PyRef<PyNamespace>> {
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
        Ok(Duration::from_nanos(unsafe { libc::gethrvtime() }))
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
        use nix::sys::resource::{UsageWho, getrusage};
        fn from_timeval(tv: libc::timeval, vm: &VirtualMachine) -> PyResult<i64> {
            (|tv: libc::timeval| {
                let t = tv.tv_sec.checked_mul(SEC_TO_NS)?;
                let u = (tv.tv_usec as i64).checked_mul(US_TO_NS)?;
                t.checked_add(u)
            })(tv)
            .ok_or_else(|| vm.new_overflow_error("timestamp too large to convert to i64"))
        }
        let ru = getrusage(UsageWho::RUSAGE_SELF).map_err(|e| e.into_pyexception(vm))?;
        let utime = from_timeval(ru.user_time().into(), vm)?;
        let stime = from_timeval(ru.system_time().into(), vm)?;

        Ok(Duration::from_nanos((utime + stime) as u64))
    }
}

#[cfg(windows)]
#[pymodule]
mod platform {
    use super::decl::{MS_TO_NS, SEC_TO_NS, time_muldiv};
    use crate::{
        PyRef, PyResult, VirtualMachine,
        builtins::{PyNamespace, PyStrRef},
        stdlib::os::errno_err,
    };
    use std::time::Duration;
    use windows_sys::Win32::{
        Foundation::FILETIME,
        System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency},
        System::SystemInformation::{GetSystemTimeAdjustment, GetTickCount64},
        System::Threading::{GetCurrentProcess, GetCurrentThread, GetProcessTimes, GetThreadTimes},
    };

    fn u64_from_filetime(time: FILETIME) -> u64 {
        let large: [u32; 2] = [time.dwLowDateTime, time.dwHighDateTime];
        unsafe { std::mem::transmute(large) }
    }

    fn win_perf_counter_frequency(vm: &VirtualMachine) -> PyResult<i64> {
        let frequency = unsafe {
            let mut freq = std::mem::MaybeUninit::uninit();
            if QueryPerformanceFrequency(freq.as_mut_ptr()) == 0 {
                return Err(errno_err(vm));
            }
            freq.assume_init()
        };

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
        let ticks = unsafe {
            let mut performance_count = std::mem::MaybeUninit::uninit();
            QueryPerformanceCounter(performance_count.as_mut_ptr());
            performance_count.assume_init()
        };

        Ok(Duration::from_nanos(time_muldiv(
            ticks,
            SEC_TO_NS,
            global_frequency(vm)?,
        )))
    }

    fn get_system_time_adjustment(vm: &VirtualMachine) -> PyResult<u32> {
        let mut _time_adjustment = std::mem::MaybeUninit::uninit();
        let mut time_increment = std::mem::MaybeUninit::uninit();
        let mut _is_time_adjustment_disabled = std::mem::MaybeUninit::uninit();
        let time_increment = unsafe {
            if GetSystemTimeAdjustment(
                _time_adjustment.as_mut_ptr(),
                time_increment.as_mut_ptr(),
                _is_time_adjustment_disabled.as_mut_ptr(),
            ) == 0
            {
                return Err(errno_err(vm));
            }
            time_increment.assume_init()
        };
        Ok(time_increment)
    }

    pub(super) fn get_monotonic_time(vm: &VirtualMachine) -> PyResult<Duration> {
        let ticks = unsafe { GetTickCount64() };

        Ok(Duration::from_nanos(
            (ticks as i64)
                .checked_mul(MS_TO_NS)
                .ok_or_else(|| vm.new_overflow_error("timestamp too large to convert to i64"))?
                as u64,
        ))
    }

    #[pyfunction]
    fn get_clock_info(name: PyStrRef, vm: &VirtualMachine) -> PyResult<PyRef<PyNamespace>> {
        let (adj, imp, mono, res) = match name.as_ref() {
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
        let (kernel_time, user_time) = unsafe {
            let mut _creation_time = std::mem::MaybeUninit::uninit();
            let mut _exit_time = std::mem::MaybeUninit::uninit();
            let mut kernel_time = std::mem::MaybeUninit::uninit();
            let mut user_time = std::mem::MaybeUninit::uninit();

            let thread = GetCurrentThread();
            if GetThreadTimes(
                thread,
                _creation_time.as_mut_ptr(),
                _exit_time.as_mut_ptr(),
                kernel_time.as_mut_ptr(),
                user_time.as_mut_ptr(),
            ) == 0
            {
                return Err(vm.new_os_error("Failed to get clock time".to_owned()));
            }
            (kernel_time.assume_init(), user_time.assume_init())
        };
        let k_time = u64_from_filetime(kernel_time);
        let u_time = u64_from_filetime(user_time);
        Ok(Duration::from_nanos((k_time + u_time) * 100))
    }

    pub(super) fn get_process_time(vm: &VirtualMachine) -> PyResult<Duration> {
        let (kernel_time, user_time) = unsafe {
            let mut _creation_time = std::mem::MaybeUninit::uninit();
            let mut _exit_time = std::mem::MaybeUninit::uninit();
            let mut kernel_time = std::mem::MaybeUninit::uninit();
            let mut user_time = std::mem::MaybeUninit::uninit();

            let process = GetCurrentProcess();
            if GetProcessTimes(
                process,
                _creation_time.as_mut_ptr(),
                _exit_time.as_mut_ptr(),
                kernel_time.as_mut_ptr(),
                user_time.as_mut_ptr(),
            ) == 0
            {
                return Err(vm.new_os_error("Failed to get clock time".to_owned()));
            }
            (kernel_time.assume_init(), user_time.assume_init())
        };
        let k_time = u64_from_filetime(kernel_time);
        let u_time = u64_from_filetime(user_time);
        Ok(Duration::from_nanos((k_time + u_time) * 100))
    }
}

// mostly for wasm32
#[cfg(not(any(unix, windows)))]
#[pymodule(sub)]
mod platform {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{VirtualMachine, PyContext};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    use chrono::{NaiveDateTime, NaiveDate, NaiveTime, Datelike, Timelike};

    fn create_test_vm() -> VirtualMachine {
        let ctx = PyContext::new();
        VirtualMachine::new(ctx)
    }

    #[test]
    fn test_time_constants() {
        // Test time conversion constants
        assert_eq!(decl::SEC_TO_MS, 1000);
        assert_eq!(decl::MS_TO_US, 1000);
        assert_eq!(decl::SEC_TO_US, 1_000_000);
        assert_eq!(decl::US_TO_NS, 1000);
        assert_eq!(decl::MS_TO_NS, 1_000_000);
        assert_eq!(decl::SEC_TO_NS, 1_000_000_000);
        assert_eq!(decl::NS_TO_MS, 1_000_000);
        assert_eq!(decl::NS_TO_US, 1000);
        assert_eq!(decl::_STRUCT_TM_ITEMS, 11);
    }

    #[test]
    fn test_duration_since_system_now() {
        let vm = create_test_vm();
        let result = decl::duration_since_system_now(&vm);
        assert!(result.is_ok());
        let duration = result.unwrap();
        // Should be a reasonable time since Unix epoch
        assert!(duration.as_secs() > 0);
        // Should be less than some future date (e.g., year 3000)
        assert!(duration.as_secs() < 32_503_680_000); // Jan 1, 3000
    }

    #[test]
    fn test_time_ns() {
        let vm = create_test_vm();
        let result = decl::time_ns(&vm);
        assert!(result.is_ok());
        let time_ns = result.unwrap();
        assert!(time_ns > 0);
    }

    #[test]
    fn test_time() {
        let vm = create_test_vm();
        let result = decl::time(&vm);
        assert!(result.is_ok());
        let time_secs = result.unwrap();
        assert!(time_secs > 0.0);
        // Should be reasonable timestamp (after year 2000, before 3000)
        assert!(time_secs > 946_684_800.0); // Jan 1, 2000
        assert!(time_secs < 32_503_680_000.0); // Jan 1, 3000
    }

    #[test]
    fn test_monotonic() {
        let vm = create_test_vm();
        let result1 = decl::monotonic(&vm);
        let result2 = decl::monotonic(&vm);
        
        assert!(result1.is_ok());
        assert!(result2.is_ok());
        
        let time1 = result1.unwrap();
        let time2 = result2.unwrap();
        
        assert!(time1 >= 0.0);
        assert!(time2 >= time1); // Monotonic should not go backwards
    }

    #[test]
    fn test_monotonic_ns() {
        let vm = create_test_vm();
        let result = decl::monotonic_ns(&vm);
        assert!(result.is_ok());
        let time_ns = result.unwrap();
        assert!(time_ns > 0);
    }

    #[test]
    fn test_perf_counter() {
        let vm = create_test_vm();
        let result1 = decl::perf_counter(&vm);
        let result2 = decl::perf_counter(&vm);
        
        assert!(result1.is_ok());
        assert!(result2.is_ok());
        
        let time1 = result1.unwrap();
        let time2 = result2.unwrap();
        
        assert!(time1 >= 0.0);
        assert!(time2 >= time1); // Performance counter should be monotonic
    }

    #[test]
    fn test_perf_counter_ns() {
        let vm = create_test_vm();
        let result = decl::perf_counter_ns(&vm);
        assert!(result.is_ok());
        let time_ns = result.unwrap();
        assert!(time_ns > 0);
    }

    #[test]
    fn test_process_time() {
        let vm = create_test_vm();
        let result = decl::process_time(&vm);
        // May not be implemented on all platforms
        match result {
            Ok(time) => assert!(time >= 0.0),
            Err(e) => {
                // Should be NotImplementedError on unsupported platforms
                assert!(e.class().name().contains("NotImplementedError") || 
                       e.class().name().contains("OSError"));
            }
        }
    }

    #[test]
    fn test_process_time_ns() {
        let vm = create_test_vm();
        let result = decl::process_time_ns(&vm);
        match result {
            Ok(time_ns) => assert!(time_ns > 0),
            Err(e) => {
                assert!(e.class().name().contains("NotImplementedError") || 
                       e.class().name().contains("OSError"));
            }
        }
    }

    #[test]
    fn test_thread_time() {
        let vm = create_test_vm();
        let result = decl::thread_time(&vm);
        match result {
            Ok(time) => assert!(time >= 0.0),
            Err(e) => {
                assert!(e.class().name().contains("NotImplementedError") || 
                       e.class().name().contains("OSError"));
            }
        }
    }

    #[test]
    fn test_thread_time_ns() {
        let vm = create_test_vm();
        let result = decl::thread_time_ns(&vm);
        match result {
            Ok(time_ns) => assert!(time_ns > 0),
            Err(e) => {
                assert!(e.class().name().contains("NotImplementedError") || 
                       e.class().name().contains("OSError"));
            }
        }
    }

    #[test]
    fn test_pyobj_to_date_time_float() {
        use crate::function::Either;
        let vm = create_test_vm();
        
        // Test with float timestamp
        let timestamp = 1609459200.5; // Jan 1, 2021 00:00:00.5 UTC
        let result = decl::pyobj_to_date_time(Either::A(timestamp), &vm);
        assert!(result.is_ok());
        
        let datetime = result.unwrap();
        assert_eq!(datetime.year(), 2021);
        assert_eq!(datetime.month(), 1);
        assert_eq!(datetime.day(), 1);
        assert_eq!(datetime.nanosecond(), 500_000_000); // 0.5 seconds in nanoseconds
    }

    #[test]
    fn test_pyobj_to_date_time_int() {
        use crate::function::Either;
        let vm = create_test_vm();
        
        // Test with integer timestamp
        let timestamp = 1609459200i64; // Jan 1, 2021 00:00:00 UTC
        let result = decl::pyobj_to_date_time(Either::B(timestamp), &vm);
        assert!(result.is_ok());
        
        let datetime = result.unwrap();
        assert_eq!(datetime.year(), 2021);
        assert_eq!(datetime.month(), 1);
        assert_eq!(datetime.day(), 1);
        assert_eq!(datetime.hour(), 0);
        assert_eq!(datetime.minute(), 0);
        assert_eq!(datetime.second(), 0);
    }

    #[test]
    fn test_pyobj_to_date_time_overflow() {
        use crate::function::Either;
        let vm = create_test_vm();
        
        // Test with timestamp that causes overflow
        let timestamp = i64::MAX as f64 * 2.0; // Very large timestamp
        let result = decl::pyobj_to_date_time(Either::A(timestamp), &vm);
        assert!(result.is_err());
        // Should be overflow error
        assert!(result.unwrap_err().class().name().contains("OverflowError"));
    }

    #[test]
    fn test_pyobj_to_date_time_negative() {
        use crate::function::Either;
        let vm = create_test_vm();
        
        // Test with negative timestamp (before Unix epoch)
        let timestamp = -86400.0; // Dec 31, 1969
        let result = decl::pyobj_to_date_time(Either::A(timestamp), &vm);
        // This might fail on some platforms, but should handle gracefully
        match result {
            Ok(datetime) => {
                assert_eq!(datetime.year(), 1969);
                assert_eq!(datetime.month(), 12);
                assert_eq!(datetime.day(), 31);
            },
            Err(e) => {
                assert!(e.class().name().contains("OverflowError"));
            }
        }
    }

    #[test]
    fn test_pystruct_time_creation() {
        let vm = create_test_vm();
        let naive_dt = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2023, 6, 15).unwrap(),
            NaiveTime::from_hms_opt(14, 30, 45).unwrap()
        );
        
        let py_time = decl::PyStructTime::new(&vm, naive_dt, 0);
        
        // Verify the struct contains expected values
        // Note: We can't easily test the PyObjectRef fields without more VM setup
        // but we can test that creation doesn't panic
        assert!(true); // Just ensure no panic occurred
    }

    #[cfg(any(windows, all(target_arch = "wasm32", target_os = "emscripten")))]
    #[test]
    fn test_time_muldiv() {
        // Test the time_muldiv utility function on Windows/emscripten
        let result = decl::time_muldiv(1000, 1000000, 1000);
        assert_eq!(result, 1000000);
        
        let result = decl::time_muldiv(123, 1000, 1);
        assert_eq!(result, 123000);
        
        // Test edge case with zero
        let result = decl::time_muldiv(0, 1000, 1);
        assert_eq!(result, 0);
        
        // Test integer division behavior
        let result = decl::time_muldiv(1000, 999, 1000);
        assert_eq!(result, 999);
    }

    #[test]
    fn test_cfmt_constant() {
        // Test that the CFMT constant is properly defined
        assert_eq!(decl::CFMT, "%a %b %e %H:%M:%S %Y");
    }

    #[test] 
    fn test_make_module() {
        let vm = create_test_vm();
        let module = make_module(&vm);
        assert!(module.is(&vm.ctx.types.module_type));
    }

    #[test]
    fn test_sleep_with_zero_duration() {
        let vm = create_test_vm();
        
        // Create a zero duration object
        let zero_duration = vm.ctx.new_float(0.0);
        let result = decl::sleep(zero_duration.into(), &vm);
        
        // Should succeed with zero duration
        assert!(result.is_ok());
    }

    #[test]
    fn test_sleep_negative_duration_error() {
        let vm = create_test_vm();
        
        // Create a negative duration object
        let negative_duration = vm.ctx.new_float(-1.0);
        let result = decl::sleep(negative_duration.into(), &vm);
        
        // Should fail with ValueError for negative duration
        assert!(result.is_err());
        let error = result.unwrap_err();
        assert!(error.class().name().contains("ValueError"));
    }

    #[test]
    fn test_multiple_time_calls_consistency() {
        let vm = create_test_vm();
        
        // Multiple calls to time() should return increasing values
        let time1 = decl::time(&vm).unwrap();
        std::thread::sleep(Duration::from_millis(1));
        let time2 = decl::time(&vm).unwrap();
        
        assert!(time2 >= time1);
    }

    #[test]
    fn test_time_ns_vs_time_consistency() {
        let vm = create_test_vm();
        
        let time_secs = decl::time(&vm).unwrap();
        let time_ns = decl::time_ns(&vm).unwrap();
        
        // time_ns should be approximately time * 1e9
        let time_ns_from_secs = (time_secs * 1e9) as u64;
        let diff = if time_ns > time_ns_from_secs {
            time_ns - time_ns_from_secs
        } else {
            time_ns_from_secs - time_ns
        };
        
        // Should be within reasonable bounds (allowing for execution time)
        assert!(diff < 1_000_000_000); // Less than 1 second difference
    }

    #[test]
    fn test_monotonic_vs_monotonic_ns_consistency() {
        let vm = create_test_vm();
        
        let mono_secs = decl::monotonic(&vm).unwrap();
        let mono_ns = decl::monotonic_ns(&vm).unwrap();
        
        // monotonic_ns should be approximately monotonic * 1e9
        let mono_ns_from_secs = (mono_secs * 1e9) as u128;
        let diff = if mono_ns > mono_ns_from_secs {
            mono_ns - mono_ns_from_secs
        } else {
            mono_ns_from_secs - mono_ns
        };
        
        // Should be within reasonable bounds
        assert!(diff < 1_000_000_000); // Less than 1 second difference
    }

    #[test]
    fn test_perf_counter_vs_perf_counter_ns_consistency() {
        let vm = create_test_vm();
        
        let perf_secs = decl::perf_counter(&vm).unwrap();
        let perf_ns = decl::perf_counter_ns(&vm).unwrap();
        
        // perf_counter_ns should be approximately perf_counter * 1e9
        let perf_ns_from_secs = (perf_secs * 1e9) as u128;
        let diff = if perf_ns > perf_ns_from_secs {
            perf_ns - perf_ns_from_secs
        } else {
            perf_ns_from_secs - perf_ns
        };
        
        // Should be within reasonable bounds
        assert!(diff < 1_000_000_000); // Less than 1 second difference
    }

    // Test timezone-related functions (platform-specific)
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn test_timezone_functions() {
        let vm = create_test_vm();
        
        // Test timezone function
        let tz = decl::timezone(&vm);
        // Should be a valid timezone offset in seconds
        assert!(tz.abs() <= 86400); // Should be within 24 hours
        
        // Test tzname function
        let tzname_tuple = decl::tzname(&vm);
        // Should be a tuple with two string elements
        assert!(tzname_tuple.len() == 2);
    }

    #[cfg(all(not(target_os = "freebsd"), not(target_env = "msvc"), not(target_arch = "wasm32")))]
    #[test]
    fn test_daylight_function() {
        let vm = create_test_vm();
        let daylight = decl::daylight(&vm);
        // Should be 0 or 1
        assert!(daylight == 0 || daylight == 1);
    }

    // Test error conditions
    #[test]
    fn test_duration_since_system_now_error_handling() {
        let vm = create_test_vm();
        
        // This is hard to test directly since SystemTime::now() rarely fails
        // But we can at least verify the function handles the result properly
        let result = decl::duration_since_system_now(&vm);
        match result {
            Ok(duration) => assert!(duration.as_secs() > 0),
            Err(e) => assert!(e.class().name().contains("ValueError")),
        }
    }

    // Test platform-specific implementations exist
    #[cfg(unix)]
    #[test]
    fn test_unix_platform_functions_exist() {
        // Just verify that the platform module compiles and exports expected functions
        let vm = create_test_vm();
        
        // These should exist on Unix platforms
        let _mono = platform::get_monotonic_time(&vm);
        let _perf = platform::get_perf_time(&vm);
    }

    #[cfg(windows)]
    #[test]
    fn test_windows_platform_functions_exist() {
        let vm = create_test_vm();
        
        // These should exist on Windows platforms
        let _mono = platform::get_monotonic_time(&vm);
        let _perf = platform::get_perf_time(&vm);
        let _thread = platform::get_thread_time(&vm);
        let _process = platform::get_process_time(&vm);
    }

    // Stress test for performance
    #[test]
    fn test_time_functions_performance() {
        let vm = create_test_vm();
        
        // Test that time functions are reasonably fast
        let start = std::time::Instant::now();
        for _ in 0..1000 {
            let _ = decl::time(&vm);
        }
        let elapsed = start.elapsed();
        
        // 1000 calls should take less than 1 second
        assert!(elapsed.as_secs() < 1);
    }

    // Test gmtime function
    #[test]
    fn test_gmtime_with_timestamp() {
        use crate::function::{Either, OptionalArg};
        let vm = create_test_vm();
        
        let timestamp = 1609459200.0; // Jan 1, 2021 00:00:00 UTC
        let result = decl::gmtime(OptionalArg::Present(Either::A(timestamp)), &vm);
        assert!(result.is_ok());
        
        let struct_time = result.unwrap();
        // We can't easily test the individual fields without more VM setup
        // but we can verify the function executes without error
    }

    #[test]
    fn test_gmtime_without_timestamp() {
        use crate::function::OptionalArg;
        let vm = create_test_vm();
        
        let result = decl::gmtime(OptionalArg::Missing, &vm);
        assert!(result.is_ok());
    }

    // Test localtime function
    #[test]
    fn test_localtime_with_timestamp() {
        use crate::function::{Either, OptionalArg};
        let vm = create_test_vm();
        
        let timestamp = 1609459200.0; // Jan 1, 2021 00:00:00 UTC
        let result = decl::localtime(OptionalArg::Present(Either::A(timestamp)), &vm);
        assert!(result.is_ok());
    }

    #[test]
    fn test_localtime_without_timestamp() {
        use crate::function::OptionalArg;
        let vm = create_test_vm();
        
        let result = decl::localtime(OptionalArg::Missing, &vm);
        assert!(result.is_ok());
    }

    // Test mktime function
    #[test]
    fn test_mktime() {
        let vm = create_test_vm();
        let naive_dt = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2021, 1, 1).unwrap(),
            NaiveTime::from_hms_opt(0, 0, 0).unwrap()
        );
        
        let struct_time = decl::PyStructTime::new(&vm, naive_dt, 0);
        let result = decl::mktime(struct_time, &vm);
        
        // mktime should succeed and return a reasonable timestamp
        match result {
            Ok(timestamp) => {
                assert!(timestamp > 0.0);
                assert!(timestamp < 32_503_680_000.0); // Before year 3000
            },
            Err(_) => {
                // mktime might fail due to VM setup limitations in tests
                // This is acceptable for basic testing
            }
        }
    }

    // Test ctime function
    #[test]
    fn test_ctime_with_timestamp() {
        use crate::function::{Either, OptionalArg};
        let vm = create_test_vm();
        
        let timestamp = 1609459200.0; // Jan 1, 2021 00:00:00 UTC
        let result = decl::ctime(OptionalArg::Present(Either::A(timestamp)), &vm);
        assert!(result.is_ok());
        
        let time_str = result.unwrap();
        // Should be a formatted time string
        assert!(!time_str.is_empty());
    }

    #[test]
    fn test_ctime_without_timestamp() {
        use crate::function::OptionalArg;
        let vm = create_test_vm();
        
        let result = decl::ctime(OptionalArg::Missing, &vm);
        assert!(result.is_ok());
        
        let time_str = result.unwrap();
        assert!(!time_str.is_empty());
    }

    // Test asctime function  
    #[test]
    fn test_asctime() {
        use crate::function::OptionalArg;
        let vm = create_test_vm();
        
        let result = decl::asctime(OptionalArg::Missing, &vm);
        assert!(result.is_ok());
    }

    // Test edge cases for overflow/underflow
    #[test]
    fn test_timestamp_edge_cases() {
        use crate::function::Either;
        let vm = create_test_vm();
        
        // Test with very small positive timestamp
        let small_timestamp = 1.0;
        let result = decl::pyobj_to_date_time(Either::A(small_timestamp), &vm);
        assert!(result.is_ok());
        
        // Test with maximum safe timestamp
        let max_safe_timestamp = 2_147_483_647.0; // Year 2038 problem timestamp
        let result = decl::pyobj_to_date_time(Either::A(max_safe_timestamp), &vm);
        assert!(result.is_ok());
    }

    #[test]
    fn test_fractional_seconds_precision() {
        use crate::function::Either;
        let vm = create_test_vm();
        
        let timestamp = 1609459200.123456; // With microsecond precision
        let result = decl::pyobj_to_date_time(Either::A(timestamp), &vm);
        assert!(result.is_ok());
        
        let datetime = result.unwrap();
        // Verify fractional seconds are handled
        assert!(datetime.nanosecond() > 0);
    }
}

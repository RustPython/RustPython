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
    static c_daylight: core::ffi::c_int;
    // pub static dstbias: std::ffi::c_int;
    #[link_name = "timezone"]
    static c_timezone: core::ffi::c_long;
    #[link_name = "tzname"]
    static c_tzname: [*const core::ffi::c_char; 2];
    #[link_name = "tzset"]
    fn c_tzset();
}

#[pymodule(name = "time", with(platform))]
mod decl {
    use crate::{
        AsObject, PyObjectRef, PyResult, VirtualMachine,
        builtins::{PyStrRef, PyTypeRef, PyUtf8StrRef},
        function::{Either, FuncArgs, OptionalArg},
        types::{PyStructSequence, struct_sequence_new},
    };
    use chrono::{
        DateTime, Datelike, TimeZone, Timelike,
        naive::{NaiveDate, NaiveDateTime, NaiveTime},
    };
    use core::time::Duration;
    #[cfg(target_env = "msvc")]
    #[cfg(not(target_arch = "wasm32"))]
    use windows_sys::Win32::System::Time::{GetTimeZoneInformation, TIME_ZONE_INFORMATION};

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
            if e.class().is(vm.ctx.exceptions.value_error)
                && let Some(s) = e.args().first().and_then(|arg| arg.str(vm).ok())
                && s.as_str() == "negative duration"
            {
                return vm.new_value_error("sleep length must be non-negative");
            }
            e
        })?;

        #[cfg(unix)]
        {
            // this is basically std::thread::sleep, but that catches interrupts and we don't want to;
            let ts = nix::sys::time::TimeSpec::from(dur);
            let res = unsafe { libc::nanosleep(ts.as_ref(), core::ptr::null_mut()) };
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
    fn get_tz_info() -> TIME_ZONE_INFORMATION {
        let mut info: TIME_ZONE_INFORMATION = unsafe { std::mem::zeroed() };
        unsafe { GetTimeZoneInformation(&mut info) };
        info
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
        (info.Bias + info.StandardBias) * 60 - 3600
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
        (info.Bias + info.StandardBias) * 60
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
        (info.StandardBias != info.DaylightBias) as i32
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
                Self::Present(secs) => pyobj_to_date_time(secs, vm)?
                    .with_timezone(&chrono::Local)
                    .naive_local(),
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
        secs: OptionalArg<Either<f64, i64>>,
        vm: &VirtualMachine,
    ) -> PyResult<StructTimeData> {
        let instant = secs.naive_or_utc(vm)?;
        Ok(StructTimeData::new_utc(vm, instant))
    }

    #[pyfunction]
    fn localtime(
        secs: OptionalArg<Either<f64, i64>>,
        vm: &VirtualMachine,
    ) -> PyResult<StructTimeData> {
        let instant = secs.naive_or_local(vm)?;
        // TODO: isdst flag must be valid value here
        // https://docs.python.org/3/library/time.html#time.localtime
        Ok(StructTimeData::new_local(vm, instant, -1))
    }

    #[pyfunction]
    fn mktime(t: StructTimeData, vm: &VirtualMachine) -> PyResult<f64> {
        let datetime = t.to_date_time(vm)?;
        // mktime interprets struct_time as local time
        let local_dt = chrono::Local
            .from_local_datetime(&datetime)
            .single()
            .ok_or_else(|| vm.new_overflow_error("mktime argument out of range"))?;
        let seconds_since_epoch = local_dt.timestamp() as f64;
        Ok(seconds_since_epoch)
    }

    const CFMT: &str = "%a %b %e %H:%M:%S %Y";

    #[pyfunction]
    fn asctime(t: OptionalArg<StructTimeData>, vm: &VirtualMachine) -> PyResult {
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
    fn strftime(
        format: PyUtf8StrRef,
        t: OptionalArg<StructTimeData>,
        vm: &VirtualMachine,
    ) -> PyResult {
        use core::fmt::Write;

        let instant = t.naive_or_local(vm)?;

        // On Windows/AIX/Solaris, %y format with year < 1900 is not supported
        #[cfg(any(windows, target_os = "aix", target_os = "solaris"))]
        if instant.year() < 1900 && format.as_str().contains("%y") {
            let msg = "format %y requires year >= 1900 on Windows";
            return Err(vm.new_value_error(msg.to_owned()));
        }

        let mut formatted_time = String::new();

        /*
         * chrono doesn't support all formats and it
         * raises an error if unsupported format is supplied.
         * If error happens, we set result as input arg.
         */
        write!(&mut formatted_time, "{}", instant.format(format.as_str()))
            .unwrap_or_else(|_| formatted_time = format.to_string());
        Ok(vm.ctx.new_str(formatted_time).into())
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
        pub tm_gmtoff: PyObjectRef,
        #[pystruct_sequence(skip)]
        pub tm_zone: PyObjectRef,
    }

    impl core::fmt::Debug for StructTimeData {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            write!(f, "struct_time()")
        }
    }

    impl StructTimeData {
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
                tm_gmtoff: vm.ctx.new_int(gmtoff).into(),
                tm_zone: vm.ctx.new_str(zone).into(),
            }
        }

        /// Create struct_time for UTC (gmtime)
        fn new_utc(vm: &VirtualMachine, tm: NaiveDateTime) -> Self {
            Self::new_inner(vm, tm, 0, 0, "UTC")
        }

        /// Create struct_time for local timezone (localtime)
        fn new_local(vm: &VirtualMachine, tm: NaiveDateTime, isdst: i32) -> Self {
            let local_time = chrono::Local.from_local_datetime(&tm).unwrap();
            let offset_seconds =
                local_time.offset().local_minus_utc() + if isdst == 1 { 3600 } else { 0 };
            let tz_abbr = local_time.format("%Z").to_string();
            Self::new_inner(vm, tm, isdst, offset_seconds, &tz_abbr)
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
    }

    #[pyattr]
    #[pystruct_sequence(name = "struct_time", module = "time", data = "StructTimeData")]
    pub struct PyStructTime;

    #[pyclass(with(PyStructSequence))]
    impl PyStructTime {
        #[pyslot]
        fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            let seq: PyObjectRef = args.bind(vm)?;
            struct_sequence_new(cls, seq, vm)
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
    use core::time::Duration;
    use nix::{sys::time::TimeSpec, time::ClockId};

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
        unsafe { core::mem::transmute(large) }
    }

    fn win_perf_counter_frequency(vm: &VirtualMachine) -> PyResult<i64> {
        let frequency = unsafe {
            let mut freq = std::mem::MaybeUninit::uninit();
            if QueryPerformanceFrequency(freq.as_mut_ptr()) == 0 {
                return Err(vm.new_last_os_error());
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
                return Err(vm.new_last_os_error());
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

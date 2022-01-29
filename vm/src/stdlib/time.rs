//! The python `time` module.

// See also:
// https://docs.python.org/3/library/time.html
use crate::{PyObjectRef, VirtualMachine};

pub use time::*;

pub(crate) fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let module = time::make_module(vm);
    #[cfg(unix)]
    unix::extend_module(vm, &module);
    #[cfg(windows)]
    windows::extend_module(vm, &module);

    module
}

#[pymodule(name = "time")]
mod time {
    use crate::{
        builtins::{PyStrRef, PyTypeRef},
        function::{FuncArgs, OptionalArg},
        utils::Either,
        PyObjectRef, PyResult, PyStructSequence, TryFromObject, VirtualMachine,
    };
    use chrono::{
        naive::{NaiveDate, NaiveDateTime, NaiveTime},
        Datelike, Timelike,
    };
    use std::time::Duration;

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
            .map_err(|e| vm.new_value_error(format!("Time error: {:?}", e)))
    }

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

    #[cfg(not(unix))]
    #[pyfunction]
    fn sleep(dur: Duration) {
        std::thread::sleep(dur);
    }

    #[cfg(not(target_os = "wasi"))]
    #[pyfunction]
    fn time_ns(vm: &VirtualMachine) -> PyResult<u64> {
        Ok(duration_since_system_now(vm)?.as_nanos() as u64)
    }

    #[pyfunction]
    pub fn time(vm: &VirtualMachine) -> PyResult<f64> {
        _time(vm)
    }

    #[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
    fn _time(vm: &VirtualMachine) -> PyResult<f64> {
        Ok(duration_since_system_now(vm)?.as_secs_f64())
    }

    #[cfg(all(target_arch = "wasm32", not(target_os = "wasi")))]
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

    fn pyobj_to_naive_date_time(
        value: Either<f64, i64>,
        vm: &VirtualMachine,
    ) -> PyResult<NaiveDateTime> {
        let timestamp = match value {
            Either::A(float) => {
                let secs = float.trunc() as i64;
                let nsecs = (float.fract() * 1e9) as u32;
                NaiveDateTime::from_timestamp_opt(secs, nsecs)
            }
            Either::B(int) => NaiveDateTime::from_timestamp_opt(int, 0),
        };
        timestamp.ok_or_else(|| {
            vm.new_overflow_error("timestamp out of range for platform time_t".to_owned())
        })
    }

    impl OptionalArg<Either<f64, i64>> {
        /// Construct a localtime from the optional seconds, or get the current local time.
        fn naive_or_local(self, vm: &VirtualMachine) -> PyResult<NaiveDateTime> {
            Ok(match self {
                OptionalArg::Present(secs) => pyobj_to_naive_date_time(secs, vm)?,
                OptionalArg::Missing => chrono::offset::Local::now().naive_local(),
            })
        }

        fn naive_or_utc(self, vm: &VirtualMachine) -> PyResult<NaiveDateTime> {
            Ok(match self {
                OptionalArg::Present(secs) => pyobj_to_naive_date_time(secs, vm)?,
                OptionalArg::Missing => chrono::offset::Utc::now().naive_utc(),
            })
        }
    }

    impl OptionalArg<PyStructTime> {
        fn naive_or_local(self, vm: &VirtualMachine) -> PyResult<NaiveDateTime> {
            Ok(match self {
                OptionalArg::Present(t) => t.to_date_time(vm)?,
                OptionalArg::Missing => chrono::offset::Local::now().naive_local(),
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
        let seconds_since_epoch = datetime.timestamp() as f64;
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
        let instant = t.naive_or_local(vm)?;
        let formatted_time = instant.format(format.as_str()).to_string();
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
            .map_err(|e| vm.new_value_error(format!("Parse error: {:?}", e)))?;
        Ok(PyStructTime::new(vm, instant, -1))
    }

    #[cfg(not(any(
        windows,
        target_os = "macos",
        target_os = "android",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "linux",
        target_os = "fuchsia",
        target_os = "emscripten",
    )))]
    fn get_thread_time(vm: &VirtualMachine) -> PyResult<Duration> {
        Err(vm.new_not_implemented_error("thread time unsupported in this system".to_owned()))
    }

    #[pyfunction]
    fn thread_time(vm: &VirtualMachine) -> PyResult<f64> {
        Ok(get_thread_time(vm)?.as_secs_f64())
    }

    #[pyfunction]
    fn thread_time_ns(vm: &VirtualMachine) -> PyResult<u64> {
        Ok(get_thread_time(vm)?.as_nanos() as u64)
    }

    #[cfg(any(windows, all(target_arch = "wasm32", target_arch = "emscripten")))]
    pub(super) fn time_muldiv(ticks: i64, mul: i64, div: i64) -> u64 {
        let intpart = ticks / div;
        let ticks = ticks % div;
        let remaining = (ticks * mul) / div;
        (intpart * mul + remaining) as u64
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
        Err(vm.new_not_implemented_error("process time unsupported in this system".to_owned()))
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
    #[derive(PyStructSequence)]
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
    }

    impl std::fmt::Debug for PyStructTime {
        fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(f, "struct_time()")
        }
    }

    #[pyimpl(with(PyStructSequence))]
    impl PyStructTime {
        fn new(vm: &VirtualMachine, tm: NaiveDateTime, isdst: i32) -> Self {
            PyStructTime {
                tm_year: vm.ctx.new_int(tm.year()).into(),
                tm_mon: vm.ctx.new_int(tm.month()).into(),
                tm_mday: vm.ctx.new_int(tm.day()).into(),
                tm_hour: vm.ctx.new_int(tm.hour()).into(),
                tm_min: vm.ctx.new_int(tm.minute()).into(),
                tm_sec: vm.ctx.new_int(tm.second()).into(),
                tm_wday: vm.ctx.new_int(tm.weekday().num_days_from_monday()).into(),
                tm_yday: vm.ctx.new_int(tm.ordinal()).into(),
                tm_isdst: vm.ctx.new_int(isdst).into(),
            }
        }

        fn to_date_time(&self, vm: &VirtualMachine) -> PyResult<NaiveDateTime> {
            let invalid = || vm.new_value_error("invalid struct_time parameter".to_owned());
            macro_rules! field {
                ($field:ident) => {
                    self.$field.clone().try_into_value(vm)?
                };
            }
            let dt = NaiveDateTime::new(
                NaiveDate::from_ymd_opt(field!(tm_year), field!(tm_mon), field!(tm_mday))
                    .ok_or_else(invalid)?,
                NaiveTime::from_hms_opt(field!(tm_hour), field!(tm_min), field!(tm_sec))
                    .ok_or_else(invalid)?,
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

    impl TryFromObject for PyStructTime {
        fn try_from_object(vm: &VirtualMachine, seq: PyObjectRef) -> PyResult<Self> {
            let seq = vm.extract_elements::<PyObjectRef>(&seq)?;
            if seq.len() != 9 {
                return Err(
                    vm.new_type_error("time.struct_time() takes a sequence of length 9".to_owned())
                );
            }
            let mut i = seq.into_iter();
            Ok(PyStructTime {
                tm_year: i.next().unwrap(),
                tm_mon: i.next().unwrap(),
                tm_mday: i.next().unwrap(),
                tm_hour: i.next().unwrap(),
                tm_min: i.next().unwrap(),
                tm_sec: i.next().unwrap(),
                tm_wday: i.next().unwrap(),
                tm_yday: i.next().unwrap(),
                tm_isdst: i.next().unwrap(),
            })
        }
    }

    #[allow(unused_imports)]
    #[cfg(unix)]
    use super::unix::*;
    #[cfg(windows)]
    use super::windows::*;
}

#[cfg(unix)]
#[pymodule(name = "time")]
mod unix {
    #[allow(unused_imports)]
    use super::{SEC_TO_NS, US_TO_NS};
    #[cfg_attr(target_os = "macos", allow(unused_imports))]
    use crate::{
        builtins::{try_bigint_to_f64, PyFloat, PyIntRef, PyNamespace, PyStrRef},
        stdlib::os,
        utils::Either,
        PyRef, PyResult, VirtualMachine,
    };
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

    fn get_clock_time(clk_id: PyIntRef, vm: &VirtualMachine) -> PyResult<Duration> {
        let mut timespec = std::mem::MaybeUninit::uninit();
        let ts: libc::timespec = unsafe {
            if libc::clock_gettime(clk_id.try_to_primitive(vm)?, timespec.as_mut_ptr()) == -1 {
                return Err(os::errno_err(vm));
            }
            timespec.assume_init()
        };
        Ok(Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32))
    }

    #[pyfunction]
    fn clock_gettime(clk_id: PyIntRef, vm: &VirtualMachine) -> PyResult<f64> {
        get_clock_time(clk_id, vm).map(|d| d.as_secs_f64())
    }

    #[pyfunction]
    fn clock_gettime_ns(clk_id: PyIntRef, vm: &VirtualMachine) -> PyResult<u128> {
        get_clock_time(clk_id, vm).map(|d| d.as_nanos())
    }

    #[cfg(not(target_os = "redox"))]
    #[pyfunction]
    fn clock_getres(clk_id: PyIntRef, vm: &VirtualMachine) -> PyResult<f64> {
        let mut timespec = std::mem::MaybeUninit::uninit();
        let ts: libc::timespec = unsafe {
            if libc::clock_getres(clk_id.try_to_primitive(vm)?, timespec.as_mut_ptr()) == -1 {
                return Err(os::errno_err(vm));
            }
            timespec.assume_init()
        };
        Ok(Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32).as_secs_f64())
    }

    #[cfg(not(target_os = "redox"))]
    fn set_clock_time(
        clk_id: PyIntRef,
        timespec: libc::timespec,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let res = unsafe { libc::clock_settime(clk_id.try_to_primitive(vm)?, &timespec) };
        if res == -1 {
            return Err(os::errno_err(vm));
        }
        Ok(())
    }

    #[cfg(not(target_os = "redox"))]
    #[pyfunction]
    fn clock_settime(
        clk_id: PyIntRef,
        time: Either<PyRef<PyFloat>, PyIntRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let time = match time {
            Either::A(f) => f.to_f64(),
            Either::B(z) => try_bigint_to_f64(z.as_bigint(), vm)?,
        };
        let nanos = time.fract() * (SEC_TO_NS as f64);
        let ts = libc::timespec {
            tv_sec: time.floor() as libc::time_t,
            tv_nsec: nanos as _,
        };
        set_clock_time(clk_id, ts, vm)
    }

    #[cfg(not(target_os = "redox"))]
    #[pyfunction]
    fn clock_settime_ns(clk_id: PyIntRef, time: PyIntRef, vm: &VirtualMachine) -> PyResult<()> {
        let time: libc::time_t = time.try_to_primitive(vm)?;
        let ts = libc::timespec {
            tv_sec: time / (SEC_TO_NS as libc::time_t),
            tv_nsec: time.rem_euclid(SEC_TO_NS as libc::time_t) as _,
        };
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
                clock_getres(vm.ctx.new_int(CLOCK_MONOTONIC), vm)?,
            ),
            "process_time" => (
                false,
                "time.clock_gettime(CLOCK_PROCESS_CPUTIME_ID)",
                true,
                clock_getres(vm.ctx.new_int(CLOCK_PROCESS_CPUTIME_ID), vm)?,
            ),
            "thread_time" => (
                false,
                "time.clock_gettime(CLOCK_THREAD_CPUTIME_ID)",
                true,
                clock_getres(vm.ctx.new_int(CLOCK_THREAD_CPUTIME_ID), vm)?,
            ),
            "time" => (
                true,
                "time.clock_gettime(CLOCK_REALTIME)",
                false,
                clock_getres(vm.ctx.new_int(CLOCK_REALTIME), vm)?,
            ),
            _ => return Err(vm.new_value_error("unknown clock".to_owned())),
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
    fn get_clock_info(_name: PyStrRef, vm: &VirtualMachine) -> PyResult<PyNamespace> {
        Err(vm.new_not_implemented_error("get_clock_info unsupported on this system".to_owned()))
    }

    pub(super) fn get_monotonic_time(vm: &VirtualMachine) -> PyResult<Duration> {
        get_clock_time(vm.ctx.new_int(CLOCK_MONOTONIC), vm)
    }

    pub(super) fn get_perf_time(vm: &VirtualMachine) -> PyResult<Duration> {
        get_clock_time(vm.ctx.new_int(CLOCK_MONOTONIC), vm)
    }

    #[pyfunction]
    fn sleep(dur: Duration, vm: &VirtualMachine) -> PyResult<()> {
        // this is basically std::thread::sleep, but that catches interrupts and we don't want to;

        let mut ts = libc::timespec {
            tv_sec: std::cmp::min(libc::time_t::max_value() as u64, dur.as_secs()) as libc::time_t,
            tv_nsec: dur.subsec_nanos() as _,
        };
        let res = unsafe { libc::nanosleep(&ts, &mut ts) };
        let interrupted = res == -1 && nix::errno::errno() == libc::EINTR;

        if interrupted {
            vm.check_signals()?;
        }

        Ok(())
    }

    #[cfg(not(any(
        target_os = "illumos",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "redox"
    )))]
    pub(super) fn get_thread_time(vm: &VirtualMachine) -> PyResult<Duration> {
        let time: libc::timespec = unsafe {
            let mut time = std::mem::MaybeUninit::uninit();
            if libc::clock_gettime(CLOCK_THREAD_CPUTIME_ID, time.as_mut_ptr()) == -1 {
                return Err(vm.new_os_error("Failed to get clock time".to_owned()));
            }
            time.assume_init()
        };
        Ok(Duration::new(time.tv_sec as u64, time.tv_nsec as u32))
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
        let time: libc::timespec = unsafe {
            let mut time = std::mem::MaybeUninit::uninit();
            if libc::clock_gettime(CLOCK_PROCESS_CPUTIME_ID, time.as_mut_ptr()) == -1 {
                return Err(vm.new_os_error("Failed to get clock time".to_owned()));
            }
            time.assume_init()
        };
        Ok(Duration::new(time.tv_sec as u64, time.tv_nsec as u32))
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
            .ok_or_else(|| {
                vm.new_overflow_error("timestamp too large to convert to i64".to_owned())
            })
        }
        let ru: libc::rusage = unsafe {
            let mut ru = std::mem::MaybeUninit::uninit();
            if libc::getrusage(libc::RUSAGE_SELF, ru.as_mut_ptr()) == -1 {
                return Err(vm.new_os_error("Failed to get clock time".to_owned()));
            }
            ru.assume_init()
        };
        let utime = from_timeval(ru.ru_utime, vm)?;
        let stime = from_timeval(ru.ru_stime, vm)?;

        Ok(Duration::from_nanos((utime + stime) as u64))
    }
}

#[cfg(windows)]
#[pymodule(name = "time")]
mod windows {
    use super::{time_muldiv, MS_TO_NS, SEC_TO_NS};
    use crate::{
        builtins::{PyNamespace, PyStrRef},
        stdlib::os::errno_err,
        PyRef, PyResult, VirtualMachine,
    };
    use std::time::Duration;
    use winapi::shared::{minwindef::FILETIME, ntdef::ULARGE_INTEGER};
    use winapi::um::processthreadsapi::{
        GetCurrentProcess, GetCurrentThread, GetProcessTimes, GetThreadTimes,
    };
    use winapi::um::profileapi::{QueryPerformanceCounter, QueryPerformanceFrequency};
    use winapi::um::sysinfoapi::{GetSystemTimeAdjustment, GetTickCount64};

    fn u64_from_filetime(time: FILETIME) -> u64 {
        unsafe {
            let mut large = std::mem::MaybeUninit::<ULARGE_INTEGER>::uninit();
            {
                let m = (*large.as_mut_ptr()).u_mut();
                m.LowPart = time.dwLowDateTime;
                m.HighPart = time.dwHighDateTime;
            }
            let large = large.assume_init();
            *large.QuadPart()
        }
    }

    fn win_perf_counter_frequency(vm: &VirtualMachine) -> PyResult<i64> {
        let freq = unsafe {
            let mut freq = std::mem::MaybeUninit::uninit();
            if QueryPerformanceFrequency(freq.as_mut_ptr()) == 0 {
                return Err(errno_err(vm));
            }
            freq.assume_init()
        };
        let frequency = unsafe { freq.QuadPart() };

        if *frequency < 1 {
            Err(vm.new_runtime_error("invalid QueryPerformanceFrequency".to_owned()))
        } else if *frequency > i64::MAX / SEC_TO_NS {
            Err(vm.new_overflow_error("QueryPerformanceFrequency is too large".to_owned()))
        } else {
            Ok(*frequency)
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
        let now = unsafe {
            let mut performance_count = std::mem::MaybeUninit::uninit();
            QueryPerformanceCounter(performance_count.as_mut_ptr());
            performance_count.assume_init()
        };

        let ticks = unsafe { now.QuadPart() };
        Ok(Duration::from_nanos(time_muldiv(
            *ticks,
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
            (ticks as i64).checked_mul(MS_TO_NS).ok_or_else(|| {
                vm.new_overflow_error("timestamp too large to convert to i64".to_owned())
            })? as u64,
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
            _ => return Err(vm.new_value_error("unknown clock".to_owned())),
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
        let ktime = u64_from_filetime(kernel_time);
        let utime = u64_from_filetime(user_time);
        Ok(Duration::from_nanos((ktime + utime) * 100))
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
        let ktime = u64_from_filetime(kernel_time);
        let utime = u64_from_filetime(user_time);
        Ok(Duration::from_nanos((ktime + utime) * 100))
    }
}

//! The python `time` module.

// See also:
// https://docs.python.org/3/library/time.html

use crate::builtins::{PyStrRef, PyTypeRef};
use crate::function::{FuncArgs, OptionalArg};
use crate::utils::Either;
use crate::vm::VirtualMachine;
use crate::{PyClassImpl, PyObjectRef, PyResult, PyStructSequence, TryFromObject};
use chrono::{
    naive::{NaiveDate, NaiveDateTime, NaiveTime},
    Datelike, Timelike,
};
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

#[allow(dead_code)]
const SEC_TO_MS: i64 = 1000;
#[allow(dead_code)]
const MS_TO_US: i64 = 1000;
#[allow(dead_code)]
const SEC_TO_US: i64 = SEC_TO_MS * MS_TO_US;
#[allow(dead_code)]
const US_TO_NS: i64 = 1000;
#[allow(dead_code)]
const MS_TO_NS: i64 = MS_TO_US * US_TO_NS;
#[allow(dead_code)]
const SEC_TO_NS: i64 = SEC_TO_MS * MS_TO_NS;
#[allow(dead_code)]
const NS_TO_MS: i64 = 1000 * 1000;
#[allow(dead_code)]
const NS_TO_US: i64 = 1000;

#[cfg(not(unix))]
fn time_sleep(dur: std::time::Duration) {
    std::thread::sleep(dur);
}

#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
pub fn get_time() -> f64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(v) => v.as_secs_f64(),
        Err(err) => panic!("Time error: {:?}", err),
    }
}

#[cfg(all(target_arch = "wasm32", not(target_os = "wasi")))]
pub fn get_time() -> f64 {
    use wasm_bindgen::prelude::*;
    #[wasm_bindgen]
    extern "C" {
        type Date;
        #[wasm_bindgen(static_method_of = Date)]
        fn now() -> f64;
    }
    // Date.now returns unix time in milliseconds, we want it in seconds
    Date::now() / 1000.0
}

fn time_time_ns(_vm: &VirtualMachine) -> u64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(v) => v.as_nanos() as u64,
        Err(_) => unsafe { std::hint::unreachable_unchecked() }, // guaranteed to be not to be happen with now() + UNIX_EPOCH,
    }
}

fn time_time(_vm: &VirtualMachine) -> f64 {
    get_time()
}

fn time_monotonic(_vm: &VirtualMachine) -> f64 {
    // TODO: implement proper monotonic time!
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(v) => v.as_secs_f64(),
        Err(err) => panic!("Time error: {:?}", err),
    }
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

/// https://docs.python.org/3/library/time.html?highlight=gmtime#time.gmtime
fn time_gmtime(secs: OptionalArg<Either<f64, i64>>, vm: &VirtualMachine) -> PyResult<PyStructTime> {
    let default = chrono::offset::Utc::now().naive_utc();
    let instant = match secs {
        OptionalArg::Present(secs) => pyobj_to_naive_date_time(secs, vm)?,
        OptionalArg::Missing => default,
    };
    Ok(PyStructTime::new(vm, instant, 0))
}

fn time_localtime(
    secs: OptionalArg<Either<f64, i64>>,
    vm: &VirtualMachine,
) -> PyResult<PyStructTime> {
    let instant = optional_or_localtime(secs, vm)?;
    // TODO: isdst flag must be valid value here
    // https://docs.python.org/3/library/time.html#time.localtime
    Ok(PyStructTime::new(vm, instant, -1))
}

fn time_mktime(t: PyStructTime, vm: &VirtualMachine) -> PyResult {
    let datetime = t.to_date_time(vm)?;
    let seconds_since_epoch = datetime.timestamp() as f64;
    Ok(vm.ctx.new_float(seconds_since_epoch))
}

/// Construct a localtime from the optional seconds, or get the current local time.
fn optional_or_localtime(
    secs: OptionalArg<Either<f64, i64>>,
    vm: &VirtualMachine,
) -> PyResult<NaiveDateTime> {
    let default = chrono::offset::Local::now().naive_local();
    Ok(match secs {
        OptionalArg::Present(secs) => pyobj_to_naive_date_time(secs, vm)?,
        OptionalArg::Missing => default,
    })
}

const CFMT: &str = "%a %b %e %H:%M:%S %Y";

fn time_asctime(t: OptionalArg<PyStructTime>, vm: &VirtualMachine) -> PyResult {
    let default = chrono::offset::Local::now().naive_local();
    let instant = match t {
        OptionalArg::Present(t) => t.to_date_time(vm)?,
        OptionalArg::Missing => default,
    };
    let formatted_time = instant.format(CFMT).to_string();
    Ok(vm.ctx.new_str(formatted_time))
}

fn time_ctime(secs: OptionalArg<Either<f64, i64>>, vm: &VirtualMachine) -> PyResult<String> {
    let instant = optional_or_localtime(secs, vm)?;
    Ok(instant.format(CFMT).to_string())
}

fn time_strftime(format: PyStrRef, t: OptionalArg<PyStructTime>, vm: &VirtualMachine) -> PyResult {
    let default = chrono::offset::Local::now().naive_local();
    let instant = match t {
        OptionalArg::Present(t) => t.to_date_time(vm)?,
        OptionalArg::Missing => default,
    };
    let formatted_time = instant.format(format.as_str()).to_string();
    Ok(vm.ctx.new_str(formatted_time))
}

fn time_strptime(
    string: PyStrRef,
    format: OptionalArg<PyStrRef>,
    vm: &VirtualMachine,
) -> PyResult<PyStructTime> {
    let format = match format {
        OptionalArg::Present(ref format) => format.as_str(),
        OptionalArg::Missing => "%a %b %H:%M:%S %Y",
    };
    let instant = NaiveDateTime::parse_from_str(string.as_str(), format)
        .map_err(|e| vm.new_value_error(format!("Parse error: {:?}", e)))?;
    Ok(PyStructTime::new(vm, instant, -1))
}

fn time_thread_time(vm: &VirtualMachine) -> PyResult<f64> {
    Ok(get_thread_time(vm)?.as_secs_f64())
}

fn time_thread_time_ns(vm: &VirtualMachine) -> PyResult<u64> {
    Ok(get_thread_time(vm)?.as_nanos() as u64)
}

#[cfg(any(
    all(target_arch = "wasm32", not(target_os = "unknown")),
    target_os = "redox"
))]
fn get_process_time(vm: &VirtualMachine) -> PyResult<std::time::Duration> {
    fn time_muldiv(ticks: i64, mul: i64, div: i64) -> u64 {
        let intpart = ticks / div;
        let ticks = ticks % div;
        let remaining = (ticks * mul) / div;
        (intpart * mul + remaining) as u64
    }

    let mut t: libc::tms = unsafe { std::mem::zeroed() };
    if unsafe { libc::times(&mut t) } == -1 {
        return Err(vm.new_os_error("Failed to get clock time".to_owned()));
    }

    #[cfg(target_os = "wasi")]
    let freq = 60;
    #[cfg(not(target_os = "wasi"))]
    let freq = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };

    Ok(std::time::Duration::from_nanos(
        time_muldiv(t.tms_utime, SEC_TO_NS, freq) + time_muldiv(t.tms_stime, SEC_TO_NS, freq),
    ))
}

#[cfg(not(any(
    windows,
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
fn get_process_time(vm: &VirtualMachine) -> PyResult<std::time::Duration> {
    Err(vm.new_not_implemented_error("thread time unsupported in this system".to_owned()))
}

fn time_process_time(vm: &VirtualMachine) -> PyResult<f64> {
    Ok(get_process_time(vm)?.as_secs_f64())
}

fn time_process_time_ns(vm: &VirtualMachine) -> PyResult<u64> {
    Ok(get_process_time(vm)?.as_nanos() as u64)
}

#[pyclass(module = "time", name = "struct_time")]
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

impl fmt::Debug for PyStructTime {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "struct_time()")
    }
}

#[pyimpl(with(PyStructSequence))]
impl PyStructTime {
    fn new(vm: &VirtualMachine, tm: NaiveDateTime, isdst: i32) -> Self {
        PyStructTime {
            tm_year: vm.ctx.new_int(tm.year()),
            tm_mon: vm.ctx.new_int(tm.month()),
            tm_mday: vm.ctx.new_int(tm.day()),
            tm_hour: vm.ctx.new_int(tm.hour()),
            tm_min: vm.ctx.new_int(tm.minute()),
            tm_sec: vm.ctx.new_int(tm.second()),
            tm_wday: vm.ctx.new_int(tm.weekday().num_days_from_monday()),
            tm_yday: vm.ctx.new_int(tm.ordinal()),
            tm_isdst: vm.ctx.new_int(isdst),
        }
    }

    fn to_date_time(&self, vm: &VirtualMachine) -> PyResult<NaiveDateTime> {
        let invalid = || vm.new_value_error("invalid struct_time parameter".to_owned());
        macro_rules! field {
            ($field:ident) => {
                TryFromObject::try_from_object(vm, self.$field.clone())?
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
    fn tp_new(_cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
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

#[cfg(unix)]
mod unix {
    use crate::vm::VirtualMachine;
    use crate::PyResult;
    use std::time::Duration;

    pub(super) fn time_sleep(dur: Duration, vm: &VirtualMachine) -> PyResult<()> {
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
    #[cfg(any(
        target_os = "android",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "linux",
        target_os = "openbsd",
    ))]
    pub(super) fn get_thread_time(vm: &VirtualMachine) -> PyResult<Duration> {
        let mut time = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        if unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut time) } == -1 {
            return Err(vm.new_os_error("Failed to get clock time".to_owned()));
        }
        Ok(Duration::new(time.tv_sec as u64, time.tv_nsec as u32))
    }

    #[cfg(target_os = "solaris")]
    pub(super) fn get_thread_time(vm: &VirtualMachine) -> PyResult<Duration> {
        Ok(Duration::from_nanos(unsafe { libc::gethrvtime() }))
    }

    #[cfg(not(any(
        target_os = "android",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "linux",
        target_os = "openbsd",
        target_os = "solaris",
    )))]
    pub(super) fn get_thread_time(vm: &VirtualMachine) -> PyResult<Duration> {
        Err(vm.new_not_implemented_error("thread time unsupported in this system".to_owned()))
    }

    #[cfg(any(
        target_os = "android",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "linux"
    ))]
    pub(super) fn get_process_time(vm: &VirtualMachine) -> PyResult<Duration> {
        let mut time = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };

        if unsafe { libc::clock_gettime(libc::CLOCK_PROCESS_CPUTIME_ID, &mut time) } == -1 {
            return Err(vm.new_os_error("Failed to get clock time".to_owned()));
        }

        Ok(Duration::new(time.tv_sec as u64, time.tv_nsec as u32))
    }

    #[cfg(any(
        target_os = "illumos",
        target_os = "netbsd",
        target_os = "solaris",
        target_os = "openbsd"
    ))]
    pub(super) fn get_process_time(vm: &VirtualMachine) -> PyResult<Duration> {
        let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
        if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut ru) } == -1 {
            return Err(vm.new_os_error("Failed to get clock time".to_owned()));
        }

        fn from_timeval(tv: libc::timeval, vm: &VirtualMachine) -> PyResult<i64> {
            use super::{SEC_TO_NS, US_TO_NS};

            (|tv: libc::timeval| {
                let t = tv.tv_sec.checked_mul(SEC_TO_NS)?;
                #[cfg(target_os = netbsd)]
                let u = tv.tv_usec.checked_mul(US_TO_NS as i32)? as i64;
                #[cfg(not(target_os = netbsd))]
                let u = tv.tv_usec.checked_mul(US_TO_NS)?;
                t.checked_add(u)
            })(tv)
            .ok_or_else(|| {
                vm.new_overflow_error("timestamp too large to convert to i64".to_owned())
            })
        }

        let utime = time_fromtimeval(ru.ru_utime, vm)?;
        let stime = time_fromtimeval(ru.ru_stime, vm)?;

        Ok(Duration::from_nanos(utime + stime))
    }
}
#[cfg(unix)]
use unix::*;

#[cfg(windows)]
mod windows {
    use crate::vm::VirtualMachine;
    use crate::PyResult;
    use std::time::Duration;
    use winapi::shared::{minwindef::FILETIME, ntdef::ULARGE_INTEGER};
    use winapi::um::processthreadsapi::{
        GetCurrentProcess, GetCurrentThread, GetProcessTimes, GetThreadTimes,
    };

    fn u64_from_filetime(time: FILETIME) -> u64 {
        unsafe {
            let mut large: ULARGE_INTEGER = std::mem::zeroed();
            large.u_mut().LowPart = time.dwLowDateTime;
            large.u_mut().HighPart = time.dwHighDateTime;
            *large.QuadPart()
        }
    }

    pub(super) fn get_thread_time(vm: &VirtualMachine) -> PyResult<Duration> {
        let mut _creation_time = FILETIME::default();
        let mut _exit_time = FILETIME::default();
        let mut kernel_time = FILETIME::default();
        let mut user_time = FILETIME::default();

        if unsafe {
            let thread = GetCurrentThread();
            GetThreadTimes(
                thread,
                &mut _creation_time,
                &mut _exit_time,
                &mut kernel_time,
                &mut user_time,
            )
        } == 0
        {
            return Err(vm.new_os_error("Failed to get clock time".to_owned()));
        }
        let ktime = u64_from_filetime(kernel_time);
        let utime = u64_from_filetime(user_time);
        Ok(Duration::from_nanos((ktime + utime) * 100))
    }

    pub(super) fn get_process_time(vm: &VirtualMachine) -> PyResult<Duration> {
        let mut _creation_time = FILETIME::default();
        let mut _exit_time = FILETIME::default();
        let mut kernel_time = FILETIME::default();
        let mut user_time = FILETIME::default();

        if unsafe {
            let process = GetCurrentProcess();
            GetProcessTimes(
                process,
                &mut _creation_time,
                &mut _exit_time,
                &mut kernel_time,
                &mut user_time,
            )
        } == 0
        {
            return Err(vm.new_os_error("Failed to get clock time".to_owned()));
        }
        let ktime = u64_from_filetime(kernel_time);
        let utime = u64_from_filetime(user_time);
        Ok(Duration::from_nanos((ktime + utime) * 100))
    }
}
#[cfg(windows)]
use windows::*;

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    let struct_time_type = PyStructTime::make_class(ctx);

    let module = py_module!(vm, "time", {
        "asctime" => named_function!(ctx, time, asctime),
        "ctime" => named_function!(ctx, time, ctime),
        "gmtime" => named_function!(ctx, time, gmtime),
        "mktime" => named_function!(ctx, time, mktime),
        "localtime" => named_function!(ctx, time, localtime),
        "monotonic" => named_function!(ctx, time, monotonic),
        "strftime" => named_function!(ctx, time, strftime),
        "strptime" => named_function!(ctx, time, strptime),
        "sleep" => named_function!(ctx, time, sleep),
        "struct_time" => struct_time_type,
        "time" => named_function!(ctx, time, time),
        "perf_counter" => named_function!(ctx, time, time), // TODO: fix
        "thread_time" => named_function!(ctx, time, thread_time),
        "thread_time_ns" => named_function!(ctx, time, thread_time_ns),
        "process_time" => named_function!(ctx, time, process_time),
        "process_time_ns" => named_function!(ctx, time, process_time_ns),
    });

    #[cfg(not(target_os = "wasi"))]
    extend_module!(vm, module, {
        "time_ns" => named_function!(ctx, time, time_ns),
    });

    module
}

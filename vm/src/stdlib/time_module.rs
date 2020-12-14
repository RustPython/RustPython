//! The python `time` module.
/// See also:
/// https://docs.python.org/3/library/time.html
use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::naive::{NaiveDate, NaiveDateTime, NaiveTime};
use chrono::{Datelike, Timelike};

use crate::builtins::pystr::PyStrRef;
use crate::builtins::pytype::PyTypeRef;
use crate::builtins::tuple::PyTupleRef;
use crate::function::OptionalArg;
use crate::pyobject::{
    BorrowValue, Either, PyClassImpl, PyObjectRef, PyResult, PyStructSequence, TryFromObject,
};
use crate::vm::VirtualMachine;

#[cfg(unix)]
fn time_sleep(dur: Duration, vm: &VirtualMachine) -> PyResult<()> {
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

#[cfg(not(unix))]
fn time_sleep(dur: Duration) {
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
fn time_gmtime(secs: OptionalArg<Either<f64, i64>>, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
    let default = chrono::offset::Utc::now().naive_utc();
    let instant = match secs {
        OptionalArg::Present(secs) => pyobj_to_naive_date_time(secs, vm)?,
        OptionalArg::Missing => default,
    };
    Ok(PyStructTime::new(vm, instant, 0).into_obj(vm))
}

fn time_localtime(
    secs: OptionalArg<Either<f64, i64>>,
    vm: &VirtualMachine,
) -> PyResult<PyObjectRef> {
    let instant = optional_or_localtime(secs, vm)?;
    // TODO: isdst flag must be valid value here
    // https://docs.python.org/3/library/time.html#time.localtime
    Ok(PyStructTime::new(vm, instant, -1).into_obj(vm))
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
    let formatted_time = instant.format(&CFMT).to_string();
    Ok(vm.ctx.new_str(formatted_time))
}

fn time_ctime(secs: OptionalArg<Either<f64, i64>>, vm: &VirtualMachine) -> PyResult<String> {
    let instant = optional_or_localtime(secs, vm)?;
    Ok(instant.format(&CFMT).to_string())
}

fn time_strftime(format: PyStrRef, t: OptionalArg<PyStructTime>, vm: &VirtualMachine) -> PyResult {
    let default = chrono::offset::Local::now().naive_local();
    let instant = match t {
        OptionalArg::Present(t) => t.to_date_time(vm)?,
        OptionalArg::Missing => default,
    };
    let formatted_time = instant.format(format.borrow_value()).to_string();
    Ok(vm.ctx.new_str(formatted_time))
}

fn time_strptime(string: PyStrRef, format: OptionalArg<PyStrRef>, vm: &VirtualMachine) -> PyResult {
    let format = match format {
        OptionalArg::Present(ref format) => format.borrow_value(),
        OptionalArg::Missing => "%a %b %H:%M:%S %Y",
    };
    let instant = NaiveDateTime::parse_from_str(string.borrow_value(), format)
        .map_err(|e| vm.new_value_error(format!("Parse error: {:?}", e)))?;
    Ok(PyStructTime::new(vm, instant, -1).into_obj(vm))
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

    fn into_obj(self, vm: &VirtualMachine) -> PyObjectRef {
        self.into_struct_sequence(vm).unwrap().into_object()
    }

    #[pyslot]
    fn tp_new(_cls: PyTypeRef, seq: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
        // cls is ignorable because this is not a basetype
        Self::try_from_object(vm, seq)?.into_struct_sequence(vm)
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

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    let struct_time_type = PyStructTime::make_class(ctx);

    py_module!(vm, "time", {
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
    })
}

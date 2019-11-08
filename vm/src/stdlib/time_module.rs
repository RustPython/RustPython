//! The python `time` module.
/// See also:
/// https://docs.python.org/3/library/time.html
use std::fmt;
use std::ops::Range;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::naive::NaiveDateTime;
use chrono::{Datelike, Timelike};

use crate::function::OptionalArg;
use crate::obj::objint::PyInt;
use crate::obj::objsequence::{get_sequence_index, PySliceableSequence};
use crate::obj::objslice::PySlice;
use crate::obj::objstr::PyStringRef;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{Either, PyClassImpl, PyObjectRef, PyRef, PyResult, PyValue, TypeProtocol};
use crate::vm::VirtualMachine;

#[cfg(unix)]
fn time_sleep(seconds: f64, vm: &VirtualMachine) -> PyResult<()> {
    // this is basically std::thread::sleep, but that catches interrupts and we don't want to
    let dur = Duration::from_secs_f64(seconds);

    let mut ts = libc::timespec {
        tv_sec: std::cmp::min(libc::time_t::max_value() as u64, dur.as_secs()) as libc::time_t,
        tv_nsec: dur.subsec_nanos().into(),
    };
    let res = unsafe { libc::nanosleep(&ts, &mut ts) };
    let interrupted = res == -1 && nix::errno::errno() == libc::EINTR;

    if interrupted {
        vm.check_signals()?;
    }

    Ok(())
}

#[cfg(not(unix))]
fn time_sleep(seconds: f64, _vm: &VirtualMachine) {
    std::thread::sleep(Duration::from_secs_f64(seconds));
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

fn pyobj_to_naive_date_time(value: Either<f64, i64>) -> NaiveDateTime {
    match value {
        Either::A(float) => {
            let secs = float.trunc() as i64;
            let nsecs = (float.fract() * 1e9) as u32;
            NaiveDateTime::from_timestamp(secs, nsecs)
        }
        Either::B(int) => NaiveDateTime::from_timestamp(int, 0),
    }
}

/// https://docs.python.org/3/library/time.html?highlight=gmtime#time.gmtime
fn time_gmtime(
    secs: OptionalArg<Either<f64, i64>>,
    _vm: &VirtualMachine,
) -> PyResult<PyStructTime> {
    let default = chrono::offset::Utc::now().naive_utc();
    let instant = match secs {
        OptionalArg::Present(secs) => pyobj_to_naive_date_time(secs),
        OptionalArg::Missing => default,
    };
    let value = PyStructTime::new(instant, 0);
    Ok(value)
}

fn time_localtime(secs: OptionalArg<Either<f64, i64>>, _vm: &VirtualMachine) -> PyStructTime {
    let instant = optional_or_localtime(secs);
    // TODO: isdst flag must be valid value here
    // https://docs.python.org/3/library/time.html#time.localtime
    PyStructTime::new(instant, -1)
}

fn time_mktime(t: PyStructTimeRef, vm: &VirtualMachine) -> PyResult {
    let datetime = t.get_date_time();
    let seconds_since_epoch = datetime.timestamp() as f64;
    Ok(vm.ctx.new_float(seconds_since_epoch))
}

/// Construct a localtime from the optional seconds, or get the current local time.
fn optional_or_localtime(secs: OptionalArg<Either<f64, i64>>) -> NaiveDateTime {
    let default = chrono::offset::Local::now().naive_local();
    match secs {
        OptionalArg::Present(secs) => pyobj_to_naive_date_time(secs),
        OptionalArg::Missing => default,
    }
}

const CFMT: &str = "%a %b %e %H:%M:%S %Y";

fn time_asctime(t: OptionalArg<PyStructTimeRef>, vm: &VirtualMachine) -> PyResult {
    let default = chrono::offset::Local::now().naive_local();
    let instant = match t {
        OptionalArg::Present(t) => t.get_date_time(),
        OptionalArg::Missing => default,
    };
    let formatted_time = instant.format(&CFMT).to_string();
    Ok(vm.ctx.new_str(formatted_time))
}

fn time_ctime(secs: OptionalArg<Either<f64, i64>>, _vm: &VirtualMachine) -> String {
    let instant = optional_or_localtime(secs);
    instant.format(&CFMT).to_string()
}

fn time_strftime(
    format: PyStringRef,
    t: OptionalArg<PyStructTimeRef>,
    vm: &VirtualMachine,
) -> PyResult {
    let default = chrono::offset::Local::now().naive_local();
    let instant = match t {
        OptionalArg::Present(t) => t.get_date_time(),
        OptionalArg::Missing => default,
    };
    let formatted_time = instant.format(format.as_str()).to_string();
    Ok(vm.ctx.new_str(formatted_time))
}

fn time_strptime(
    string: PyStringRef,
    format: OptionalArg<PyStringRef>,
    vm: &VirtualMachine,
) -> PyResult<PyStructTime> {
    let format = match format {
        OptionalArg::Present(ref format) => format.as_str(),
        OptionalArg::Missing => "%a %b %H:%M:%S %Y",
    };
    let instant = NaiveDateTime::parse_from_str(string.as_str(), format)
        .map_err(|e| vm.new_value_error(format!("Parse error: {:?}", e)))?;
    let struct_time = PyStructTime::new(instant, -1);
    Ok(struct_time)
}

#[pyclass(name = "struct_time")]
struct PyStructTime {
    tm: NaiveDateTime,
    isdst: i32,
}

type PyStructTimeRef = PyRef<PyStructTime>;

impl fmt::Debug for PyStructTime {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "struct_time()")
    }
}

impl PyValue for PyStructTime {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("time", "struct_time")
    }
}

#[pyimpl]
impl PyStructTime {
    fn new(tm: NaiveDateTime, isdst: i32) -> Self {
        PyStructTime { tm, isdst }
    }

    #[pymethod(name = "__repr__")]
    fn repr(&self, _vm: &VirtualMachine) -> String {
        // TODO: extract year day and isdst somehow..
        format!(
            "time.struct_time(tm_year={}, tm_mon={}, tm_mday={}, tm_hour={}, tm_min={}, tm_sec={}, tm_wday={}, tm_yday={}, tm_isdst={})",
            self._year(), self._mon(), self._mday(),
            self._hour(), self._min(), self._sec(),
            self._wday(), self._yday(), self._isdst(),
        )
    }

    fn get_date_time(&self) -> NaiveDateTime {
        self.tm
    }

    #[pymethod(name = "__len__")]
    fn len(&self, _vm: &VirtualMachine) -> usize {
        9
    }

    #[pymethod(name = "__getitem__")]
    fn getitem(&self, subscript: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if subscript.payload::<PyInt>().is_some() {
            let needle = subscript.downcast().unwrap();
            let index = get_sequence_index(vm, &needle, 9)?;
            let tm_fn = TM_FUNCTIONS[index];
            Ok(vm.new_int(tm_fn(self)))
        } else if subscript.payload::<PySlice>().is_some() {
            let values = self.get_slice_items(vm, &subscript)?;
            let objs = values.iter().map(|v| vm.new_int(*v));
            Ok(vm.ctx.new_tuple(objs.collect()))
        } else {
            Err(vm.new_type_error(format!(
                "TypeError: tuple indices must be integers or slices, {}",
                subscript.class().name
            )))
        }
    }

    #[inline]
    fn _year(&self) -> i32 {
        self.tm.date().year()
    }

    #[inline]
    fn _mon(&self) -> i32 {
        self.tm.date().month() as i32
    }

    #[inline]
    fn _mday(&self) -> i32 {
        self.tm.date().day() as i32
    }

    #[inline]
    fn _hour(&self) -> i32 {
        self.tm.time().hour() as i32
    }

    #[inline]
    fn _min(&self) -> i32 {
        self.tm.time().minute() as i32
    }

    #[inline]
    fn _sec(&self) -> i32 {
        self.tm.time().second() as i32
    }

    #[inline]
    fn _wday(&self) -> i32 {
        self.tm.date().weekday().num_days_from_monday() as i32
    }

    #[inline]
    fn _yday(&self) -> i32 {
        self.tm.date().ordinal() as i32
    }

    #[inline]
    fn _isdst(&self) -> i32 {
        self.isdst
    }

    #[pyproperty(name = "tm_year")]
    fn tm_year(&self, _vm: &VirtualMachine) -> i32 {
        self._year()
    }

    #[pyproperty(name = "tm_mon")]
    fn tm_mon(&self, _vm: &VirtualMachine) -> u32 {
        self._mon() as u32
    }

    #[pyproperty(name = "tm_mday")]
    fn tm_mday(&self, _vm: &VirtualMachine) -> u32 {
        self._mday() as u32
    }

    #[pyproperty(name = "tm_hour")]
    fn tm_hour(&self, _vm: &VirtualMachine) -> u32 {
        self._hour() as u32
    }

    #[pyproperty(name = "tm_min")]
    fn tm_min(&self, _vm: &VirtualMachine) -> u32 {
        self._min() as u32
    }

    #[pyproperty(name = "tm_sec")]
    fn tm_sec(&self, _vm: &VirtualMachine) -> u32 {
        self._sec() as u32
    }

    #[pyproperty(name = "tm_wday")]
    fn tm_wday(&self, _vm: &VirtualMachine) -> u32 {
        self._wday() as u32
    }

    #[pyproperty(name = "tm_yday")]
    fn tm_yday(&self, _vm: &VirtualMachine) -> u32 {
        self._yday() as u32
    }

    #[pyproperty(name = "tm_isdst")]
    fn tm_isdst(&self, _vm: &VirtualMachine) -> i32 {
        self._isdst()
    }
}

type TmFunction = fn(&PyStructTime) -> i32;
const TM_FUNCTIONS: [TmFunction; 9] = [
    PyStructTime::_year,
    PyStructTime::_mon,
    PyStructTime::_mday,
    PyStructTime::_hour,
    PyStructTime::_min,
    PyStructTime::_sec,
    PyStructTime::_wday,
    PyStructTime::_yday,
    PyStructTime::_isdst,
];

impl PySliceableSequence for PyStructTime {
    type Sliced = Vec<i32>;

    fn do_slice(&self, range: Range<usize>) -> Self::Sliced {
        if let Some(fs) = TM_FUNCTIONS.get(range) {
            fs.iter().map(|f| f(self)).collect()
        } else {
            vec![]
        }
    }

    fn do_slice_reverse(&self, range: Range<usize>) -> Self::Sliced {
        if let Some(fs) = TM_FUNCTIONS.get(range) {
            fs.iter().rev().map(|f| f(self)).collect()
        } else {
            vec![]
        }
    }

    fn do_stepped_slice(&self, range: Range<usize>, step: usize) -> Self::Sliced {
        if let Some(fs) = TM_FUNCTIONS.get(range) {
            fs.iter().cloned().step_by(step).map(|f| f(self)).collect()
        } else {
            vec![]
        }
    }

    fn do_stepped_slice_reverse(&self, range: Range<usize>, step: usize) -> Self::Sliced {
        if let Some(fs) = TM_FUNCTIONS.get(range) {
            fs.iter()
                .rev()
                .cloned()
                .step_by(step)
                .map(|f| f(self))
                .collect()
        } else {
            vec![]
        }
    }

    fn empty() -> Self::Sliced {
        panic!("struct_time is not empty");
    }

    fn len(&self) -> usize {
        TM_FUNCTIONS.len()
    }

    fn is_empty(&self) -> bool {
        false
    }
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    let struct_time_type = PyStructTime::make_class(ctx);

    py_module!(vm, "time", {
        "asctime" => ctx.new_rustfunc(time_asctime),
        "ctime" => ctx.new_rustfunc(time_ctime),
        "gmtime" => ctx.new_rustfunc(time_gmtime),
        "mktime" => ctx.new_rustfunc(time_mktime),
        "localtime" => ctx.new_rustfunc(time_localtime),
        "monotonic" => ctx.new_rustfunc(time_monotonic),
        "strftime" => ctx.new_rustfunc(time_strftime),
        "strptime" => ctx.new_rustfunc(time_strptime),
        "sleep" => ctx.new_rustfunc(time_sleep),
        "struct_time" => struct_time_type,
        "time" => ctx.new_rustfunc(time_time)
    })
}

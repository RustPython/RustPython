//! The python `time` module.
/// See also:
/// https://docs.python.org/3/library/time.html
use std::fmt;
use std::ops::Range;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::function::{OptionalArg, PyFuncArgs};
use crate::obj::objfloat::PyFloatRef;
use crate::obj::objint::PyInt;
use crate::obj::objsequence::get_sequence_index;
use crate::obj::objsequence::PySliceableSequence;
use crate::obj::objslice::PySlice;
use crate::obj::objstr::PyStringRef;
use crate::obj::objtype::PyClassRef;
use crate::obj::{objfloat, objint, objtype};
use crate::pyobject::{PyClassImpl, PyObjectRef, PyRef, PyResult, PyValue, TypeProtocol};
use crate::vm::VirtualMachine;

use num_traits::cast::ToPrimitive;

use chrono::naive::NaiveDateTime;
use chrono::{Datelike, Timelike};

#[cfg(unix)]
fn time_sleep(seconds: PyFloatRef, vm: &VirtualMachine) -> PyResult<()> {
    // this is basically std::thread::sleep, but that catches interrupts and we don't want to
    let seconds = seconds.to_f64();
    let secs = seconds.trunc() as u64;
    let nsecs = (seconds.fract() * 1e9) as i64;

    let mut ts = libc::timespec {
        tv_sec: std::cmp::min(libc::time_t::max_value() as u64, secs) as libc::time_t,
        tv_nsec: nsecs,
    };
    let res = unsafe { libc::nanosleep(&ts, &mut ts) };
    let interrupted = res == -1 && nix::errno::errno() == libc::EINTR;

    if interrupted {
        crate::stdlib::signal::check_signals(vm)?;
    }

    Ok(())
}

#[cfg(not(unix))]
fn time_sleep(seconds: PyFloatRef, vm: &VirtualMachine) -> PyResult<()> {
    let seconds = seconds.to_f64();
    let secs: u64 = seconds.trunc() as u64;
    let nanos: u32 = (seconds.fract() * 1e9) as u32;
    let duration = Duration::new(secs, nanos);
    std::thread::sleep(duration);
    Ok(())
}

fn duration_to_f64(d: Duration) -> f64 {
    (d.as_secs() as f64) + (f64::from(d.subsec_nanos()) / 1e9)
}

fn time_time(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args);
    let x = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(v) => duration_to_f64(v),
        Err(err) => panic!("Error: {:?}", err),
    };
    let value = vm.ctx.new_float(x);
    Ok(value)
}

fn time_monotonic(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args);
    // TODO: implement proper monotonic time!
    let x = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(v) => duration_to_f64(v),
        Err(err) => panic!("Error: {:?}", err),
    };
    let value = vm.ctx.new_float(x);
    Ok(value)
}

fn pyfloat_to_secs_and_nanos(seconds: &PyObjectRef) -> (i64, u32) {
    let seconds = objfloat::get_value(seconds);
    let secs: i64 = seconds.trunc() as i64;
    let nanos: u32 = (seconds.fract() * 1e9) as u32;
    (secs, nanos)
}

fn pyobj_to_naive_date_time(
    value: &PyObjectRef,
    vm: &VirtualMachine,
) -> PyResult<Option<NaiveDateTime>> {
    if objtype::isinstance(value, &vm.ctx.float_type()) {
        let (seconds, nanos) = pyfloat_to_secs_and_nanos(&value);
        let dt = NaiveDateTime::from_timestamp(seconds, nanos);
        Ok(Some(dt))
    } else if objtype::isinstance(&value, &vm.ctx.int_type()) {
        let seconds = objint::get_value(&value).to_i64().unwrap();
        let dt = NaiveDateTime::from_timestamp(seconds, 0);
        Ok(Some(dt))
    } else {
        Err(vm.new_type_error("Expected float, int or None".to_string()))
    }
}

/// https://docs.python.org/3/library/time.html?highlight=gmtime#time.gmtime
fn time_gmtime(secs: OptionalArg<PyObjectRef>, vm: &VirtualMachine) -> PyResult<PyStructTime> {
    let default = chrono::offset::Utc::now().naive_utc();
    let instant = match secs {
        OptionalArg::Present(secs) => pyobj_to_naive_date_time(&secs, vm)?.unwrap_or(default),
        OptionalArg::Missing => default,
    };
    let value = PyStructTime::new(instant, 0);
    Ok(value)
}

fn time_localtime(secs: OptionalArg<PyObjectRef>, vm: &VirtualMachine) -> PyResult<PyStructTime> {
    let instant = optional_or_localtime(secs, vm)?;
    // TODO: isdst flag must be valid value here
    // https://docs.python.org/3/library/time.html#time.localtime
    let value = PyStructTime::new(instant, -1);
    Ok(value)
}

fn time_mktime(t: PyStructTimeRef, vm: &VirtualMachine) -> PyResult {
    let datetime = t.get_date_time();
    let seconds_since_epoch = datetime.timestamp() as f64;
    Ok(vm.ctx.new_float(seconds_since_epoch))
}

/// Construct a localtime from the optional seconds, or get the current local time.
fn optional_or_localtime(
    secs: OptionalArg<PyObjectRef>,
    vm: &VirtualMachine,
) -> PyResult<NaiveDateTime> {
    let default = chrono::offset::Local::now().naive_local();
    let instant = match secs {
        OptionalArg::Present(secs) => pyobj_to_naive_date_time(&secs, vm)?.unwrap_or(default),
        OptionalArg::Missing => default,
    };
    Ok(instant)
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

fn time_ctime(secs: OptionalArg<PyObjectRef>, vm: &VirtualMachine) -> PyResult<String> {
    let instant = optional_or_localtime(secs, vm)?;
    let formatted_time = instant.format(&CFMT).to_string();
    Ok(formatted_time)
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
    let formatted_time = instant.format(&format.value).to_string();
    Ok(vm.ctx.new_str(formatted_time))
}

fn time_strptime(
    string: PyStringRef,
    format: OptionalArg<PyStringRef>,
    vm: &VirtualMachine,
) -> PyResult<PyStructTime> {
    let format: String = match format {
        OptionalArg::Present(format) => format.value.clone(),
        OptionalArg::Missing => "%a %b %H:%M:%S %Y".to_string(),
    };
    let instant = NaiveDateTime::parse_from_str(&string.value, &format)
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

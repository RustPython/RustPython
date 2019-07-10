//! The python `time` module.
/// See also:
/// https://docs.python.org/3/library/time.html
use std::fmt;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::function::{OptionalArg, PyFuncArgs};
use crate::obj::objtype::PyClassRef;
use crate::obj::{objfloat, objint, objtype};
use crate::pyobject::{PyClassImpl, PyObjectRef, PyResult, PyValue};
use crate::vm::VirtualMachine;

use num_traits::cast::ToPrimitive;

use chrono::naive::NaiveDateTime;
use chrono::{Datelike, Timelike};

fn time_sleep(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(seconds, Some(vm.ctx.float_type()))]);
    let seconds = objfloat::get_value(seconds);
    let secs: u64 = seconds.trunc() as u64;
    let nanos: u32 = (seconds.fract() * 1e9) as u32;
    let duration = Duration::new(secs, nanos);
    thread::sleep(duration);
    Ok(vm.get_none())
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
    let value = PyStructTime::new(instant);
    Ok(value)
}

fn time_localtime(secs: OptionalArg<PyObjectRef>, vm: &VirtualMachine) -> PyResult<PyStructTime> {
    let default = chrono::offset::Local::now().naive_local();
    let instant = match secs {
        OptionalArg::Present(secs) => pyobj_to_naive_date_time(&secs, vm)?.unwrap_or(default),
        OptionalArg::Missing => default,
    };
    let value = PyStructTime::new(instant);
    Ok(value)
}

#[pyclass(name = "struct_time")]
struct PyStructTime {
    tm: NaiveDateTime,
}

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
    fn new(tm: NaiveDateTime) -> Self {
        PyStructTime { tm }
    }

    #[pymethod(name = "__repr__")]
    fn repr(&self, _vm: &VirtualMachine) -> String {
        // TODO: extract year day and isdst somehow..
        format!(
            "time.struct_time(tm_year={}, tm_mon={}, tm_mday={}, tm_hour={}, tm_min={}, tm_sec={}, tm_wday={})",
            self.tm.date().year(), self.tm.date().month(), self.tm.date().day(),
            self.tm.time().hour(), self.tm.time().minute(), self.tm.time().second(),
            self.tm.date().weekday().num_days_from_monday()
            )
    }

    #[pyproperty(name = "tm_year")]
    fn tm_year(&self, _vm: &VirtualMachine) -> i32 {
        self.tm.date().year()
    }

    #[pyproperty(name = "tm_mon")]
    fn tm_mon(&self, _vm: &VirtualMachine) -> u32 {
        self.tm.date().month()
    }

    #[pyproperty(name = "tm_mday")]
    fn tm_mday(&self, _vm: &VirtualMachine) -> u32 {
        self.tm.date().day()
    }

    #[pyproperty(name = "tm_hour")]
    fn tm_hour(&self, _vm: &VirtualMachine) -> u32 {
        self.tm.time().hour()
    }

    #[pyproperty(name = "tm_min")]
    fn tm_min(&self, _vm: &VirtualMachine) -> u32 {
        self.tm.time().minute()
    }

    #[pyproperty(name = "tm_sec")]
    fn tm_sec(&self, _vm: &VirtualMachine) -> u32 {
        self.tm.time().second()
    }

    #[pyproperty(name = "tm_wday")]
    fn tm_wday(&self, _vm: &VirtualMachine) -> u32 {
        self.tm.date().weekday().num_days_from_monday()
    }
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    let struct_time_type = PyStructTime::make_class(ctx);

    py_module!(vm, "time", {
        "gmtime" => ctx.new_rustfunc(time_gmtime),
        "localtime" => ctx.new_rustfunc(time_localtime),
        "monotonic" => ctx.new_rustfunc(time_monotonic),
        "sleep" => ctx.new_rustfunc(time_sleep),
        "struct_time" => struct_time_type,
        "time" => ctx.new_rustfunc(time_time)
    })
}

//! The python `time` module.

use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::function::PyFuncArgs;
use crate::obj::objfloat;
use crate::pyobject::{PyObjectRef, PyResult};
use crate::vm::VirtualMachine;

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

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    py_module!(vm, "time", {
        "sleep" => ctx.new_rustfunc(time_sleep),
        "time" => ctx.new_rustfunc(time_time)
    })
}

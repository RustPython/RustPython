//! The python `time` module.

use super::super::obj::{objfloat, objtype};
use super::super::pyobject::{PyContext, PyFuncArgs, PyObjectRef, PyResult, TypeProtocol};
use super::super::VirtualMachine;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn time_sleep(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(seconds, Some(vm.ctx.float_type()))]);
    let seconds = objfloat::get_value(seconds);
    let secs: u64 = seconds.trunc() as u64;
    let nanos: u32 = (seconds.fract() * 1e9) as u32;
    let duration = Duration::new(secs, nanos);
    thread::sleep(duration);
    Ok(vm.get_none())
}

fn duration_to_f64(d: Duration) -> f64 {
    (d.as_secs() as f64) + ((d.subsec_nanos() as f64) / 1e9)
}

fn time_time(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args);
    let x = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(v) => duration_to_f64(v),
        Err(err) => panic!("Error: {:?}", err),
    };
    let value = vm.ctx.new_float(x);
    Ok(value)
}

pub fn mk_module(ctx: &PyContext) -> PyObjectRef {
    py_item!(ctx, mod time {
        fn sleep = time_sleep;
        fn time = time_time;
    })
}

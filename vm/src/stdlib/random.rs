//! Random module.

use rand::distributions::Distribution;
use rand_distr::Normal;

use crate::pyobject::{PyObjectRef, PyResult};
use crate::vm::VirtualMachine;

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    py_module!(vm, "random", {
        "gauss" => ctx.new_rustfunc(random_normalvariate), // TODO: is this the same?
        "normalvariate" => ctx.new_rustfunc(random_normalvariate),
        "random" => ctx.new_rustfunc(random_random),
        // "weibull", ctx.new_rustfunc(random_weibullvariate),
    })
}

fn random_normalvariate(mu: f64, sigma: f64, vm: &VirtualMachine) -> PyResult<f64> {
    let normal = Normal::new(mu, sigma).map_err(|rand_err| {
        vm.new_exception(
            vm.ctx.exceptions.arithmetic_error.clone(),
            format!("invalid normal distribution: {:?}", rand_err),
        )
    })?;
    let value = normal.sample(&mut rand::thread_rng());
    Ok(value)
}

fn random_random(_vm: &VirtualMachine) -> f64 {
    rand::random()
}

/*
 * TODO: enable this function:
fn random_weibullvariate(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(alpha, Some(vm.ctx.float_type())), (beta, Some(vm.ctx.float_type()))]);
    let alpha = objfloat::get_value(alpha);
    let beta = objfloat::get_value(beta);
    let weibull = Weibull::new(alpha, beta);
    let value = weibull.sample(&mut rand::thread_rng());
    let py_value = vm.ctx.new_float(value);
    Ok(py_value)
}
*/

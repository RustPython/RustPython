//! Random module.

extern crate rand;

use super::super::obj::{objfloat, objtype};
use super::super::pyobject::{PyContext, PyFuncArgs, PyObjectRef, PyResult, TypeProtocol};
use super::super::VirtualMachine;
use stdlib::random::rand::distributions::{Distribution, Normal};

pub fn mk_module(ctx: &PyContext) -> PyObjectRef {
    // TODO: implement more random functions.
    py_item!(ctx, mod random {
        fn gauss = random_gauss;
        fn normalvariate = random_normalvariate;
        fn random = random_random;
    })
}

fn random_gauss(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    // TODO: is this the same?
    random_normalvariate(vm, args)
}

fn random_normalvariate(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [
            (mu, Some(vm.ctx.float_type())),
            (sigma, Some(vm.ctx.float_type()))
        ]
    );
    let mu = objfloat::get_value(mu);
    let sigma = objfloat::get_value(sigma);
    let normal = Normal::new(mu, sigma);
    let value = normal.sample(&mut rand::thread_rng());
    let py_value = vm.ctx.new_float(value);
    Ok(py_value)
}

fn random_random(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args);
    let value = rand::random::<f64>();
    let py_value = vm.ctx.new_float(value);
    Ok(py_value)
}

/*
 * TODO: enable this function:
fn random_weibullvariate(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(alpha, Some(vm.ctx.float_type())), (beta, Some(vm.ctx.float_type()))]);
    let alpha = objfloat::get_value(alpha);
    let beta = objfloat::get_value(beta);
    let weibull = Weibull::new(alpha, beta);
    let value = weibull.sample(&mut rand::thread_rng());
    let py_value = vm.ctx.new_float(value);
    Ok(py_value)
}
*/

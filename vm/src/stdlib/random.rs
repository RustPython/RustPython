//! Random module.

use std::cell::RefCell;

use num_bigint::{BigInt, Sign};

use rand::distributions::Distribution;
use rand::{RngCore, SeedableRng};
use rand::rngs::SmallRng;
use rand_distr::Normal;

use crate::function::OptionalArg;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{PyClassImpl, PyObjectRef, PyRef, PyValue, PyResult};

use crate::vm::VirtualMachine;

#[pyclass(name = "Random")]
#[derive(Debug)]
struct PyRandom {
    rng: RefCell<SmallRng>
}

impl PyValue for PyRandom {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("_random", "Random")
    }
}

#[pyimpl]
impl PyRandom {
    #[pyslot(new)]
    fn new(cls: PyClassRef, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        PyRandom {
            rng: RefCell::new(SmallRng::from_entropy())
        }.into_ref_with_type(vm, cls)
    }

    #[pymethod] 
    fn seed(&self, n: Option<usize>, vm: &VirtualMachine) -> PyResult {
        let rng = match n {
            None => SmallRng::from_entropy(),
            Some(n) => {
                let seed = n as u64;
                SmallRng::seed_from_u64(seed)
            }
        };
        
        *self.rng.borrow_mut() = rng;
        
        Ok(vm.ctx.none())
    }

    #[pymethod]
    fn getrandbits(&self, k: usize, vm: &VirtualMachine) -> PyResult {
        let bytes = (k - 1) / 8 + 1;
        let mut bytearray = vec![0u8; bytes];
        self.rng.borrow_mut().fill_bytes(&mut bytearray);

        let bits = bytes % 8;
        if bits > 0 {
            bytearray[0] >>= 8 - bits;
        }
        
        println!("{:?}", k);
        println!("{:?}", bytearray);

        let result = BigInt::from_bytes_be(Sign::Plus, &bytearray);
        Ok(vm.ctx.new_bigint(&result))
    }
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    let random_type = PyRandom::make_class(ctx);

    py_module!(vm, "_random", {
        "Random" => random_type,
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

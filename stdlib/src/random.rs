//! Random module.

pub(crate) use _random::make_module;

#[pymodule]
mod _random {
    use crate::common::lock::PyMutex;
    use crate::vm::{
        PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
        builtins::{PyInt, PyTupleRef},
        convert::ToPyException,
        function::OptionalOption,
        types::{Constructor, Initializer},
    };
    use itertools::Itertools;
    use malachite_bigint::{BigInt, BigUint, Sign};
    use mt19937::MT19937;
    use num_traits::{Signed, Zero};
    use rand_core::{RngCore, SeedableRng};
    use rustpython_vm::types::DefaultConstructor;

    #[pyattr]
    #[pyclass(name = "Random")]
    #[derive(Debug, PyPayload, Default)]
    struct PyRandom {
        rng: PyMutex<MT19937>,
    }

    impl DefaultConstructor for PyRandom {}

    impl Initializer for PyRandom {
        type Args = OptionalOption;

        fn init(zelf: PyRef<Self>, x: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
            zelf.seed(x, vm)
        }
    }

    #[pyclass(flags(BASETYPE), with(Constructor, Initializer))]
    impl PyRandom {
        #[pymethod]
        fn random(&self) -> f64 {
            let mut rng = self.rng.lock();
            mt19937::gen_res53(&mut *rng)
        }

        #[pymethod]
        fn seed(&self, n: OptionalOption<PyObjectRef>, vm: &VirtualMachine) -> PyResult<()> {
            *self.rng.lock() = match n.flatten() {
                Some(n) => {
                    // Fallback to using hash if object isn't Int-like.
                    let (_, mut key) = match n.downcast::<PyInt>() {
                        Ok(n) => n.as_bigint().abs(),
                        Err(obj) => BigInt::from(obj.hash(vm)?).abs(),
                    }
                    .to_u32_digits();
                    if cfg!(target_endian = "big") {
                        key.reverse();
                    }
                    let key = if key.is_empty() { &[0] } else { key.as_slice() };
                    MT19937::new_with_slice_seed(key)
                }
                None => MT19937::try_from_os_rng()
                    .map_err(|e| std::io::Error::from(e).to_pyexception(vm))?,
            };
            Ok(())
        }

        #[pymethod]
        fn getrandbits(&self, k: isize, vm: &VirtualMachine) -> PyResult<BigInt> {
            match k {
                ..0 => Err(vm.new_value_error("number of bits must be non-negative".to_owned())),
                0 => Ok(BigInt::zero()),
                mut k => {
                    let mut rng = self.rng.lock();
                    let mut gen_u32 = |k| {
                        let r = rng.next_u32();
                        if k < 32 { r >> (32 - k) } else { r }
                    };

                    let words = (k - 1) / 32 + 1;
                    let word_array = (0..words)
                        .map(|_| {
                            let word = gen_u32(k);
                            k = k.wrapping_sub(32);
                            word
                        })
                        .collect::<Vec<_>>();

                    let uint = BigUint::new(word_array);
                    // very unlikely but might as well check
                    let sign = if uint.is_zero() {
                        Sign::NoSign
                    } else {
                        Sign::Plus
                    };
                    Ok(BigInt::from_biguint(sign, uint))
                }
            }
        }

        #[pymethod]
        fn getstate(&self, vm: &VirtualMachine) -> PyTupleRef {
            let rng = self.rng.lock();
            vm.new_tuple(
                rng.get_state()
                    .iter()
                    .copied()
                    .chain([rng.get_index() as u32])
                    .map(|i| vm.ctx.new_int(i).into())
                    .collect::<Vec<PyObjectRef>>(),
            )
        }

        #[pymethod]
        fn setstate(&self, state: PyTupleRef, vm: &VirtualMachine) -> PyResult<()> {
            let state: &[_; mt19937::N + 1] = state
                .as_slice()
                .try_into()
                .map_err(|_| vm.new_value_error("state vector is the wrong size".to_owned()))?;
            let (index, state) = state.split_last().unwrap();
            let index: usize = index.try_to_value(vm)?;
            if index > mt19937::N {
                return Err(vm.new_value_error("invalid state".to_owned()));
            }
            let state: [u32; mt19937::N] = state
                .iter()
                .map(|i| i.try_to_value(vm))
                .process_results(|it| it.collect_array())?
                .unwrap();
            let mut rng = self.rng.lock();
            rng.set_state(&state);
            rng.set_index(index);
            Ok(())
        }
    }
}

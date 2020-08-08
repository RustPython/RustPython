//! Random module.

pub(crate) use _random::make_module;

#[pymodule]
mod _random {
    use crate::common::cell::PyMutex;
    use crate::function::OptionalOption;
    use crate::obj::objint::PyIntRef;
    use crate::obj::objtype::PyClassRef;
    use crate::pyobject::{BorrowValue, PyClassImpl, PyRef, PyResult, PyValue};
    use crate::VirtualMachine;
    use num_bigint::{BigInt, Sign};
    use num_traits::Signed;
    use rand::{rngs::StdRng, RngCore, SeedableRng};

    #[derive(Debug)]
    enum PyRng {
        Std(Box<StdRng>),
        MT(Box<mt19937::MT19937>),
    }

    impl Default for PyRng {
        fn default() -> Self {
            PyRng::Std(Box::new(StdRng::from_entropy()))
        }
    }

    impl RngCore for PyRng {
        fn next_u32(&mut self) -> u32 {
            match self {
                Self::Std(s) => s.next_u32(),
                Self::MT(m) => m.next_u32(),
            }
        }
        fn next_u64(&mut self) -> u64 {
            match self {
                Self::Std(s) => s.next_u64(),
                Self::MT(m) => m.next_u64(),
            }
        }
        fn fill_bytes(&mut self, dest: &mut [u8]) {
            match self {
                Self::Std(s) => s.fill_bytes(dest),
                Self::MT(m) => m.fill_bytes(dest),
            }
        }
        fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand::Error> {
            match self {
                Self::Std(s) => s.try_fill_bytes(dest),
                Self::MT(m) => m.try_fill_bytes(dest),
            }
        }
    }

    #[pyclass(name = "Random")]
    #[derive(Debug)]
    struct PyRandom {
        rng: PyMutex<PyRng>,
    }

    impl PyValue for PyRandom {
        fn class(vm: &VirtualMachine) -> PyClassRef {
            vm.class("_random", "Random")
        }
    }

    #[pyimpl(flags(BASETYPE))]
    impl PyRandom {
        #[pyslot(new)]
        fn new(cls: PyClassRef, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
            PyRandom {
                rng: PyMutex::default(),
            }
            .into_ref_with_type(vm, cls)
        }

        #[pymethod]
        fn random(&self) -> f64 {
            let mut rng = self.rng.lock();
            mt19937::gen_res53(&mut *rng)
        }

        #[pymethod]
        fn seed(&self, n: OptionalOption<PyIntRef>) {
            let new_rng = match n.flatten() {
                None => PyRng::default(),
                Some(n) => {
                    let (_, mut key) = n.borrow_value().abs().to_u32_digits();
                    if cfg!(target_endian = "big") {
                        key.reverse();
                    }
                    PyRng::MT(Box::new(mt19937::MT19937::new_with_slice_seed(&key)))
                }
            };

            *self.rng.lock() = new_rng;
        }

        #[pymethod]
        fn getrandbits(&self, k: usize) -> BigInt {
            let mut rng = self.rng.lock();
            let mut k = k;
            let mut gen_u32 = |k| rng.next_u32() >> (32 - k) as u32;

            if k <= 32 {
                return gen_u32(k).into();
            }

            let words = (k - 1) / 8 + 1;
            let mut wordarray = vec![0u32; words];

            let it = wordarray.iter_mut();
            #[cfg(target_endian = "big")]
            let it = it.rev();
            for word in it {
                *word = gen_u32(k);
                k -= 32;
            }

            BigInt::from_slice(Sign::NoSign, &wordarray)
        }
    }
}

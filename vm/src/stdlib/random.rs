//! Random module.

pub(crate) use _random::make_module;

#[pymodule]
mod _random {
    use crate::builtins::int::PyIntRef;
    use crate::builtins::pytype::PyTypeRef;
    use crate::common::lock::PyMutex;
    use crate::function::OptionalOption;
    use crate::pyobject::{BorrowValue, PyObjectRef, PyRef, PyResult, PyValue, StaticType};
    use crate::VirtualMachine;
    use num_bigint::{BigInt, Sign};
    use num_traits::{Signed, Zero};
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

    #[pyattr]
    #[pyclass(name = "Random")]
    #[derive(Debug)]
    struct PyRandom {
        rng: PyMutex<PyRng>,
    }

    impl PyValue for PyRandom {
        fn class(_vm: &VirtualMachine) -> &PyTypeRef {
            Self::static_type()
        }
    }

    #[pyimpl(flags(BASETYPE))]
    impl PyRandom {
        #[pyslot(new)]
        fn new(
            cls: PyTypeRef,
            // TODO: use x as the seed.
            _x: OptionalOption<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
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

        // TODO: n can be a float, str, bytes, or bytearray
        #[pymethod]
        fn seed(&self, n: OptionalOption<PyIntRef>) {
            let new_rng = match n.flatten() {
                None => PyRng::default(),
                Some(n) => {
                    let (_, mut key) = n.borrow_value().abs().to_u32_digits();
                    if cfg!(target_endian = "big") {
                        key.reverse();
                    }
                    let key = if key.is_empty() { &[0] } else { key.as_slice() };
                    PyRng::MT(Box::new(mt19937::MT19937::new_with_slice_seed(key)))
                }
            };

            *self.rng.lock() = new_rng;
        }

        #[pymethod]
        fn getrandbits(&self, k: usize, vm: &VirtualMachine) -> PyResult<BigInt> {
            if k == 0 {
                return Err(
                    vm.new_value_error("number of bits must be greater than zero".to_owned())
                );
            }

            let mut rng = self.rng.lock();
            let mut k = k;
            let mut gen_u32 = |k| {
                let r = rng.next_u32();
                if k < 32 {
                    r >> (32 - k)
                } else {
                    r
                }
            };

            if k <= 32 {
                return Ok(gen_u32(k).into());
            }

            let words = (k - 1) / 32 + 1;
            let wordarray = (0..words)
                .map(|_| {
                    let word = gen_u32(k);
                    k = k.wrapping_sub(32);
                    word
                })
                .collect::<Vec<_>>();

            let uint = num_bigint::BigUint::new(wordarray);
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

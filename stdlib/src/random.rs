//! Random module.

pub(crate) use _random::make_module;

#[pymodule]
mod _random {
    use std::cmp::Ordering;

    use crate::common::{int::BigInt, lock::PyMutex};
    use crate::vm::{
        builtins::{PyInt, PyTypeRef},
        function::OptionalOption,
        types::Constructor,
        PyObjectRef, PyPayload, PyResult, VirtualMachine,
    };
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
    #[derive(Debug, PyPayload)]
    struct PyRandom {
        rng: PyMutex<PyRng>,
    }

    impl Constructor for PyRandom {
        type Args = OptionalOption<PyObjectRef>;

        fn py_new(
            cls: PyTypeRef,
            // TODO: use x as the seed.
            _x: Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult {
            PyRandom {
                rng: PyMutex::default(),
            }
            .into_ref_with_type(vm, cls)
            .map(Into::into)
        }
    }

    #[pyclass(flags(BASETYPE), with(Constructor))]
    impl PyRandom {
        #[pymethod]
        fn random(&self) -> f64 {
            let mut rng = self.rng.lock();
            mt19937::gen_res53(&mut *rng)
        }

        #[pymethod]
        fn seed(&self, n: OptionalOption<PyObjectRef>, vm: &VirtualMachine) -> PyResult<()> {
            let new_rng = n
                .flatten()
                .map(|n| {
                    // Fallback to using hash if object isn't Int-like.
                    let (_, mut key) = match n.downcast::<PyInt>() {
                        Ok(n) => n.as_bigint().abs(),
                        Err(obj) => BigInt::from(obj.hash(vm)?).abs(),
                    }
                    .to_u32_digits();
                    if cfg!(target_endian = "big") {
                        key.reverse();
                    }
                    let key: &[u32] = if key.is_empty() { &[0] } else { key.as_slice() };
                    Ok(PyRng::MT(Box::new(mt19937::MT19937::new_with_slice_seed(
                        key,
                    ))))
                })
                .transpose()?
                .unwrap_or_default();

            *self.rng.lock() = new_rng;
            Ok(())
        }

        #[pymethod]
        fn getrandbits(&self, k: isize, vm: &VirtualMachine) -> PyResult<BigInt> {
            match k.cmp(&0) {
                Ordering::Less => {
                    Err(vm.new_value_error("number of bits must be non-negative".to_owned()))
                }
                Ordering::Equal => Ok(BigInt::zero()),
                Ordering::Greater => {
                    let mut rng = self.rng.lock();
                    Ok(BigInt::getrandbits(k as usize, || rng.next_u64()))
                }
            }
        }
    }
}

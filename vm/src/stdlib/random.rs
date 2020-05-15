//! Random module.

pub(crate) use _random::make_module;

#[pymodule]
mod _random {
    use crate::function::OptionalOption;
    use crate::obj::objint::PyIntRef;
    use crate::obj::objtype::PyClassRef;
    use crate::pyobject::{PyClassImpl, PyRef, PyResult, PyValue, ThreadSafe};
    use crate::VirtualMachine;
    use generational_arena::{self, Arena};
    use num_bigint::{BigInt, Sign};
    use num_traits::Signed;
    use rand::RngCore;
    use std::cell::RefCell;

    #[derive(Debug)]
    enum PyRng {
        Std(rand::rngs::ThreadRng),
        MT(Box<mt19937::MT19937>),
    }

    impl Default for PyRng {
        fn default() -> Self {
            PyRng::Std(rand::thread_rng())
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

    thread_local!(static RNG_HANDLES: RefCell<Arena<PyRng>> = RefCell::new(Arena::new()));

    #[derive(Debug)]
    struct RngHandle(generational_arena::Index);
    impl RngHandle {
        fn new(rng: PyRng) -> Self {
            let idx = RNG_HANDLES.with(|arena| arena.borrow_mut().insert(rng));
            RngHandle(idx)
        }
        fn exec<F, R>(&self, func: F) -> R
        where
            F: Fn(&mut PyRng) -> R,
        {
            RNG_HANDLES.with(|arena| {
                func(
                    arena
                        .borrow_mut()
                        .get_mut(self.0)
                        .expect("index was removed"),
                )
            })
        }
        fn replace(&self, rng: PyRng) {
            RNG_HANDLES.with(|arena| {
                *arena
                    .borrow_mut()
                    .get_mut(self.0)
                    .expect("index was removed") = rng
            })
        }
    }
    impl Drop for RngHandle {
        fn drop(&mut self) {
            RNG_HANDLES.with(|arena| arena.borrow_mut().remove(self.0));
        }
    }

    #[pyclass(name = "Random")]
    #[derive(Debug)]
    struct PyRandom {
        rng: RngHandle,
    }

    impl ThreadSafe for PyRandom {}

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
                rng: RngHandle::new(PyRng::default()),
            }
            .into_ref_with_type(vm, cls)
        }

        #[pymethod]
        fn random(&self) -> f64 {
            self.rng.exec(mt19937::gen_res53)
        }

        #[pymethod]
        fn seed(&self, n: OptionalOption<PyIntRef>) {
            let new_rng = match n.flat_option() {
                None => PyRng::default(),
                Some(n) => {
                    let (_, mut key) = n.as_bigint().abs().to_u32_digits();
                    if cfg!(target_endian = "big") {
                        key.reverse();
                    }
                    PyRng::MT(Box::new(mt19937::MT19937::new_with_slice_seed(&key)))
                }
            };

            self.rng.replace(new_rng);
        }

        #[pymethod]
        fn getrandbits(&self, k: usize) -> BigInt {
            self.rng.exec(|rng| {
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
            })
        }
    }
}

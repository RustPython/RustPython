//! Random module.

pub(crate) use _random::module_def;

#[pymodule]
mod _random {
    use crate::common::lock::PyMutex;
    use crate::vm::{
        Py, PyObjectRef, PyPayload, PyResult, VirtualMachine,
        builtins::{PyInt, PyTuple, PyTupleRef},
        types::{Constructor, Initializer},
    };
    use itertools::Itertools;
    use malachite_bigint::{BigInt, BigUint, Sign};
    use mt19937::MT19937;
    use num_traits::{Signed, Zero};
    use rand::{Rng, SeedableRng};
    use rustpython_vm::types::DefaultConstructor;

    #[pyattr]
    #[pyclass(name = "Random")]
    #[derive(Debug, PyPayload, Default)]
    struct PyRandom {
        rng: PyMutex<MT19937>,
    }

    #[derive(FromArgs)]
    struct SeedArgs {
        #[pyarg(positional, optional)]
        n: Option<PyObjectRef>,
    }

    impl DefaultConstructor for PyRandom {}

    impl Initializer for PyRandom {
        type Args = SeedArgs;

        fn init(zelf: &Py<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
            Self::seed(zelf, args, vm)
        }
    }

    #[pyclass(flags(BASETYPE), with(Constructor, Initializer))]
    impl PyRandom {
        #[pymethod]
        fn random(zelf: &Py<Self>) -> f64 {
            let mut rng = zelf.rng.lock();
            mt19937::gen_res53(&mut *rng)
        }

        #[pymethod]
        fn seed(zelf: &Py<Self>, args: SeedArgs, vm: &VirtualMachine) -> PyResult<()> {
            *zelf.rng.lock() = match args.n {
                Some(n) => {
                    // CPython uses the absolute value for integers, but casts
                    // hashes of other objects to the unsigned pointer width.
                    let value = match n.downcast::<PyInt>() {
                        Ok(n) => n.as_bigint().abs(),
                        Err(obj) => BigInt::from(obj.hash(vm)? as usize),
                    };
                    // Digits are already least-significant first on every target.
                    let (_, key) = value.to_u32_digits();
                    let key = if key.is_empty() { &[0] } else { key.as_slice() };
                    MT19937::new_with_slice_seed(key)
                }
                None => MT19937::from_rng(&mut rand::rng()),
            };
            Ok(())
        }

        #[pymethod]
        fn getrandbits(zelf: &Py<Self>, k: PyObjectRef, vm: &VirtualMachine) -> PyResult<BigInt> {
            let k_int = k.try_index(vm)?;
            let k_bigint = k_int.as_bigint();
            if k_bigint.is_negative() {
                return Err(vm.new_value_error("number of bits must be non-negative"));
            }
            let k: u64 = k_int
                .try_to_primitive(vm)
                .map_err(|_| vm.new_overflow_error("getrandbits: number of bits too large"))?;
            match k {
                0 => Ok(BigInt::zero()),
                1..=32 => Ok(BigInt::from(zelf.rng.lock().next_u32() >> (32 - k))),
                mut k => {
                    let words = word_count(k).ok_or_else(|| vm.no_memory_error())?;
                    let mut word_array = Vec::new();
                    word_array
                        .try_reserve_exact(words)
                        .map_err(|_| vm.no_memory_error())?;
                    let mut rng = zelf.rng.lock();
                    let mut gen_u32 = |k| {
                        let r = rng.next_u32();
                        if k < 32 { r >> (32 - k) } else { r }
                    };

                    for _ in 0..words {
                        word_array.push(gen_u32(k));
                        k = k.wrapping_sub(32);
                    }

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
        fn getstate(zelf: &Py<Self>, vm: &VirtualMachine) -> PyTupleRef {
            let rng = zelf.rng.lock();
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
        fn setstate(zelf: &Py<Self>, state: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            let state = state
                .downcast_ref::<PyTuple>()
                .ok_or_else(|| vm.new_type_error("state vector must be a tuple"))?;
            let state: &[_; mt19937::N + 1] = state
                .as_slice()
                .try_into()
                .map_err(|_| vm.new_value_error("state vector is the wrong size"))?;
            let (index, state) = state.split_last().unwrap();
            let state: [u32; mt19937::N] = state
                .iter()
                .map(|item| {
                    let value = item
                        .downcast_ref::<PyInt>()
                        .ok_or_else(|| vm.new_type_error("an integer is required"))?;
                    if value.as_bigint().is_negative() {
                        return Err(
                            vm.new_overflow_error("can't convert negative value to unsigned int")
                        );
                    }
                    let value = core::ffi::c_ulong::try_from(value.as_bigint()).map_err(|_| {
                        vm.new_overflow_error("Python int too large to convert to C unsigned long")
                    })?;
                    #[allow(clippy::unnecessary_cast)] // c_ulong is u32 on some platforms.
                    let value = value as u32;
                    Ok(value)
                })
                .process_results(|it| it.collect_array())?
                .unwrap();
            let index = index.try_index(vm)?;
            let index = core::ffi::c_long::try_from(index.as_bigint())
                .map_err(|_| vm.new_overflow_error("Python int too large to convert to C long"))?;
            if index < 0 || index > mt19937::N as core::ffi::c_long {
                return Err(vm.new_value_error("invalid state"));
            }
            let mut rng = zelf.rng.lock();
            rng.set_state(&state);
            rng.set_index(index as usize);
            Ok(())
        }
    }

    fn word_count(bits: u64) -> Option<usize> {
        let words = usize::try_from(bits.div_ceil(32)).ok()?;
        (words <= isize::MAX as usize / size_of::<u32>()).then_some(words)
    }

    #[cfg(test)]
    mod tests {
        use super::word_count;

        #[test]
        fn getrandbits_word_counts() {
            for (bits, words) in [
                (0, 0),
                (1, 1),
                (32, 1),
                (33, 2),
                (1 << 31, 1 << 26),
                (1 << 32, 1 << 27),
            ] {
                assert_eq!(word_count(bits), Some(words));
            }
            if usize::BITS == 32 {
                let max_words = isize::MAX as usize / size_of::<u32>();
                assert_eq!(word_count(max_words as u64 * 32), Some(max_words));
                assert_eq!(word_count(max_words as u64 * 32 + 1), None);
                assert_eq!(word_count(u64::MAX), None);
            } else {
                assert_eq!(word_count(u64::MAX), usize::try_from(1_u64 << 59).ok());
            }
        }
    }
}

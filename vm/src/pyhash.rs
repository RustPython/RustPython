use num_bigint::BigInt;
use num_traits::ToPrimitive;
use std::hash::{Hash, Hasher};

use crate::obj::objfloat;
use crate::pyobject::PyObjectRef;
use crate::pyobject::PyResult;
use crate::vm::VirtualMachine;

pub type PyHash = i64;
pub type PyUHash = u64;

/// Prime multiplier used in string and various other hashes.
pub const MULTIPLIER: PyHash = 1_000_003; // 0xf4243
/// Numeric hashes are based on reduction modulo the prime 2**_BITS - 1
pub const BITS: usize = 61;
pub const MODULUS: PyUHash = (1 << BITS) - 1;
pub const INF: PyHash = 314_159;
pub const NAN: PyHash = 0;
pub const IMAG: PyHash = MULTIPLIER;

// pub const CUTOFF: usize = 7;

pub fn hash_float(value: f64) -> PyHash {
    // cpython _Py_HashDouble
    if !value.is_finite() {
        return if value.is_infinite() {
            if value > 0.0 {
                INF
            } else {
                -INF
            }
        } else {
            NAN
        };
    }

    let frexp = objfloat::ufrexp(value);

    // process 28 bits at a time;  this should work well both for binary
    // and hexadecimal floating point.
    let mut m = frexp.0;
    let mut e = frexp.1;
    let mut x: PyUHash = 0;
    while m != 0.0 {
        x = ((x << 28) & MODULUS) | x >> (BITS - 28);
        m *= 268_435_456.0; // 2**28
        e -= 28;
        let y = m as PyUHash; // pull out integer part
        m -= y as f64;
        x += y;
        if x >= MODULUS {
            x -= MODULUS;
        }
    }

    // adjust for the exponent;  first reduce it modulo BITS
    const BITS32: i32 = BITS as i32;
    e = if e >= 0 {
        e % BITS32
    } else {
        BITS32 - 1 - ((-1 - e) % BITS32)
    };
    x = ((x << e) & MODULUS) | x >> (BITS32 - e);

    x as PyHash * value.signum() as PyHash
}

pub fn hash_value<T: Hash>(data: &T) -> PyHash {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    data.hash(&mut hasher);
    hasher.finish() as PyHash
}

pub fn hash_iter<'a, I: std::iter::Iterator<Item = &'a PyObjectRef>>(
    iter: I,
    vm: &VirtualMachine,
) -> PyResult<PyHash> {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for element in iter {
        let item_hash = vm._hash(&element)?;
        item_hash.hash(&mut hasher);
    }
    Ok(hasher.finish() as PyHash)
}

pub fn hash_iter_unordered<'a, I: std::iter::Iterator<Item = &'a PyObjectRef>>(
    iter: I,
    vm: &VirtualMachine,
) -> PyResult<PyHash> {
    let mut hash: PyHash = 0;
    for element in iter {
        let item_hash = vm._hash(element)?;
        // xor is commutative and hash should be independent of order
        hash ^= item_hash;
    }
    Ok(hash)
}

pub fn hash_bigint(value: &BigInt) -> PyHash {
    match value.to_i64() {
        Some(i64_value) => (i64_value % MODULUS as i64),
        None => (value % MODULUS).to_i64().unwrap(),
    }
}

#[pystruct_sequence(name = "sys.hash_info")]
#[derive(Debug)]
pub(crate) struct PyHashInfo {
    width: usize,
    modulus: PyUHash,
    inf: PyHash,
    nan: PyHash,
    imag: PyHash,
    algorithm: &'static str,
    hash_bits: usize,
    seed_bits: usize,
}
impl PyHashInfo {
    pub const INFO: Self = PyHashInfo {
        width: BITS,
        modulus: MODULUS,
        inf: INF,
        nan: NAN,
        imag: IMAG,
        algorithm: "siphash13",
        hash_bits: std::mem::size_of::<PyHash>() * 8,
        // internally hash_map::DefaultHasher uses 2 u64s as the seed, but
        // that's not guaranteed to be consistent across Rust releases
        // TODO: use something like the siphasher crate as our hash algorithm
        seed_bits: std::mem::size_of::<PyHash>() * 2 * 8,
    };
}

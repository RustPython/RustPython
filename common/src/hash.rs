use num_bigint::BigInt;
use num_complex::Complex64;
use num_traits::ToPrimitive;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::num::Wrapping;

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
pub const ALGO: &str = "siphasher13";
pub const HASH_BITS: usize = std::mem::size_of::<PyHash>() * 8;
// internally DefaultHasher uses 2 u64s as the seed, but
// that's not guaranteed to be consistent across Rust releases
// TODO: use something like the siphasher crate as our hash algorithm
pub const SEED_BITS: usize = std::mem::size_of::<PyHash>() * 2 * 8;

// pub const CUTOFF: usize = 7;

#[inline]
pub fn mod_int(value: i64) -> PyHash {
    value % MODULUS as i64
}

pub fn hash_value<T: Hash + ?Sized>(data: &T) -> PyHash {
    let mut hasher = DefaultHasher::new();
    data.hash(&mut hasher);
    mod_int(hasher.finish() as PyHash)
}

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

    let frexp = super::float_ops::ufrexp(value);

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

pub fn hash_complex(value: &Complex64) -> PyHash {
    let re_hash = hash_float(value.re);
    let im_hash = hash_float(value.im);
    let ret = Wrapping(re_hash) + Wrapping(im_hash) * Wrapping(IMAG);
    ret.0
}

pub fn hash_iter<'a, T: 'a, I, F, E>(iter: I, hashf: F) -> Result<PyHash, E>
where
    I: IntoIterator<Item = &'a T>,
    F: Fn(&'a T) -> Result<PyHash, E>,
{
    let mut hasher = DefaultHasher::new();
    for element in iter {
        let item_hash = hashf(element)?;
        item_hash.hash(&mut hasher);
    }
    Ok(mod_int(hasher.finish() as PyHash))
}

pub fn hash_iter_unordered<'a, T: 'a, I, F, E>(iter: I, hashf: F) -> Result<PyHash, E>
where
    I: IntoIterator<Item = &'a T>,
    F: Fn(&'a T) -> Result<PyHash, E>,
{
    let mut hash: PyHash = 0;
    for element in iter {
        let item_hash = hashf(element)?;
        // xor is commutative and hash should be independent of order
        hash ^= item_hash;
    }
    Ok(mod_int(hash))
}

pub fn hash_bigint(value: &BigInt) -> PyHash {
    value.to_i64().map_or_else(
        || {
            (value % MODULUS).to_i64().unwrap_or_else(||
            // guaranteed to be safe by mod
            unsafe { std::hint::unreachable_unchecked() })
        },
        mod_int,
    )
}

pub fn hash_str(value: &str) -> PyHash {
    hash_value(value.as_bytes())
}

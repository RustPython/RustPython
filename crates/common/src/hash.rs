use core::hash::{BuildHasher, Hash, Hasher};
use malachite_bigint::BigInt;
use num_traits::ToPrimitive;
use siphasher::sip::SipHasher24;

pub type PyHash = isize;
pub type PyUHash = usize;

/// A PyHash value used to represent a missing hash value, e.g. means "not yet computed" for
/// `str`'s hash cache
pub const SENTINEL: PyHash = -1;

/// Prime multiplier used in string and various other hashes.
pub const MULTIPLIER: PyHash = 1_000_003; // 0xf4243
/// Numeric hashes are based on reduction modulo the prime 2**_BITS - 1
pub const BITS: usize = if usize::BITS >= 64 { 61 } else { 31 };
pub const MODULUS: PyUHash = (1 << BITS) - 1;
pub const INF: PyHash = 314_159;
pub const NAN: PyHash = 0;
pub const IMAG: PyHash = MULTIPLIER;
pub const ALGO: &str = "siphash24";
// SipHash24 produces 64 bits before conversion to the platform hash width.
pub const HASH_BITS: usize = 64;
// SipHasher24 takes 2 u64s as a seed
pub const SEED_BITS: usize = core::mem::size_of::<u64>() * 2 * 8;

// pub const CUTOFF: usize = 7;

#[derive(Clone, Copy)]
pub struct HashSecret {
    k0: u64,
    k1: u64,
}

impl BuildHasher for HashSecret {
    type Hasher = SipHasher24;

    fn build_hasher(&self) -> Self::Hasher {
        SipHasher24::new_with_keys(self.k0, self.k1)
    }
}

impl HashSecret {
    #[must_use]
    pub fn new(seed: u32) -> Self {
        let mut buf = [0u8; 16];
        // CPython disables hash randomization with an all-zero secret.
        if seed != 0 {
            lcg_urandom(seed, &mut buf);
        }
        let (left, right) = buf.split_at(8);
        let k0 = u64::from_le_bytes(left.try_into().unwrap());
        let k1 = u64::from_le_bytes(right.try_into().unwrap());
        Self { k0, k1 }
    }

    /// Build a secret from explicit SipHash keys, bypassing seed derivation.
    /// Lets an embedder reproduce a fixed keying (e.g. a deterministic run) that
    /// [`new`](Self::new) cannot express through its `u32` seed.
    #[must_use]
    pub const fn from_keys(k0: u64, k1: u64) -> Self {
        Self { k0, k1 }
    }

    pub fn hash_value<T: Hash + ?Sized>(&self, data: &T) -> PyHash {
        fix_sentinel(mod_int(self.hash_one(data) as _))
    }

    pub fn hash_iter<'a, T: 'a, I, F, E>(&self, iter: I, hash_func: F) -> Result<PyHash, E>
    where
        I: IntoIterator<Item = &'a T>,
        F: Fn(&'a T) -> Result<PyHash, E>,
    {
        let mut hasher = self.build_hasher();
        for element in iter {
            let item_hash = hash_func(element)?;
            item_hash.hash(&mut hasher);
        }
        Ok(fix_sentinel(mod_int(hasher.finish() as i64)))
    }

    #[must_use]
    pub fn hash_bytes(&self, value: &[u8]) -> PyHash {
        if value.is_empty() {
            0
        } else {
            let mut hasher = self.build_hasher();
            // Py_HashBuffer hashes the buffer directly, without Rust's slice
            // length prefix or the modulus used for numeric hashes.
            hasher.write(value);
            fix_sentinel(hasher.finish() as PyHash)
        }
    }

    #[inline]
    #[must_use]
    pub fn hash_str(&self, value: &str) -> PyHash {
        self.hash_wtf8(value.into())
    }

    /// Hash CPython's canonical native-endian Unicode storage, including surrogates.
    #[inline]
    #[must_use]
    pub fn hash_wtf8(&self, value: &crate::wtf8::Wtf8) -> PyHash {
        if value.as_bytes().is_ascii() {
            return self.hash_bytes(value.as_bytes());
        }
        self.hash_non_ascii(value)
    }

    #[cold]
    fn hash_non_ascii(&self, value: &crate::wtf8::Wtf8) -> PyHash {
        let max = value.code_points().map(|cp| cp.to_u32()).max().unwrap_or(0);
        let mut hasher = self.build_hasher();
        if max <= 0xff {
            for cp in value.code_points() {
                hasher.write_u8(cp.to_u32() as u8);
            }
        } else if max <= 0xffff {
            for cp in value.code_points() {
                hasher.write(&(cp.to_u32() as u16).to_ne_bytes());
            }
        } else {
            for cp in value.code_points() {
                hasher.write(&cp.to_u32().to_ne_bytes());
            }
        }
        fix_sentinel(hasher.finish() as PyHash)
    }
}

#[inline]
#[must_use]
pub const fn hash_pointer(value: usize) -> PyHash {
    // TODO: 32bit?
    let hash = (value >> 4) | value;
    hash as _
}

#[inline]
#[must_use]
pub const fn hash_float(value: f64) -> Option<PyHash> {
    // cpython _Py_HashDouble
    if !value.is_finite() {
        return if value.is_infinite() {
            Some(if value > 0.0 { INF } else { -INF })
        } else {
            None
        };
    }

    let frexp = super::float_ops::decompose_float(value);

    // process 28 bits at a time;  this should work well both for binary
    // and hexadecimal floating point.
    let mut m = frexp.0;
    let mut e = frexp.1;
    let mut x: PyUHash = 0;

    #[expect(clippy::while_float, reason = "keep this loop like CPython does it")]
    while m != 0.0 {
        x = ((x << 28) & MODULUS) | (x >> (BITS - 28));
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
    x = ((x << e) & MODULUS) | (x >> (BITS32 - e));

    Some(fix_sentinel(x as PyHash * value.signum() as PyHash))
}

#[must_use]
pub fn hash_bigint(value: &BigInt) -> PyHash {
    let ret = if let Some(v) = value.to_i64() {
        mod_int(v)
    } else {
        // SAFETY:
        // MODULUS < isize::MAX, so the signed remainder fits in PyHash.
        unsafe { (value % MODULUS).to_isize().unwrap_unchecked() }
    };

    fix_sentinel(ret)
}

#[inline]
#[must_use]
pub const fn hash_usize(data: usize) -> PyHash {
    fix_sentinel((data % MODULUS) as PyHash)
}

#[inline(always)]
#[must_use]
pub const fn fix_sentinel(x: PyHash) -> PyHash {
    if x == SENTINEL { -2 } else { x }
}

#[inline]
#[must_use]
pub const fn mod_int(value: i64) -> PyHash {
    (value % MODULUS as i64) as PyHash
}

pub fn lcg_urandom(mut x: u32, buf: &mut [u8]) {
    for b in buf {
        x = x.wrapping_mul(214013);
        x = x.wrapping_add(2531011);
        *b = ((x >> 16) & 0xff) as u8;
    }
}

#[inline]
#[must_use]
pub const fn hash_object_id_raw(p: usize) -> PyHash {
    // TODO: Use commented logic when below issue resolved.
    // Ref: https://github.com/RustPython/RustPython/pull/3951#issuecomment-1193108966

    /* bottom 3 or 4 bits are likely to be 0; rotate y by 4 to avoid
    excessive hash collisions for dicts and sets */
    // p.rotate_right(4) as PyHash
    p as PyHash
}

#[inline]
#[must_use]
pub const fn hash_object_id(p: usize) -> PyHash {
    fix_sentinel(hash_object_id_raw(p))
}

#[must_use]
pub fn keyed_hash(key: u64, buf: &[u8]) -> u64 {
    let mut hasher = SipHasher24::new_with_keys(key, 0);
    buf.hash(&mut hasher);
    hasher.finish()
}

/// tuplehash: fold the element hashes of a tuple (xxHash-based).
///
/// The caller supplies each element's hash lazily; a hash computation may fail,
/// in which case the error short-circuits the fold.
pub fn hash_tuple<E>(
    element_hashes: impl IntoIterator<Item = Result<PyHash, E>>,
) -> Result<PyHash, E> {
    // CPython selects xxHash's constants and rotation by Py_hash_t width.
    #[cfg(target_pointer_width = "64")]
    const PRIME1: PyUHash = 11400714785074694791;
    #[cfg(target_pointer_width = "64")]
    const PRIME2: PyUHash = 14029467366897019727;
    #[cfg(target_pointer_width = "64")]
    const PRIME5: PyUHash = 2870177450012600261;
    #[cfg(target_pointer_width = "64")]
    const ROTATE: u32 = 31;
    #[cfg(target_pointer_width = "32")]
    const PRIME1: PyUHash = 2654435761;
    #[cfg(target_pointer_width = "32")]
    const PRIME2: PyUHash = 2246822519;
    #[cfg(target_pointer_width = "32")]
    const PRIME5: PyUHash = 374761393;
    #[cfg(target_pointer_width = "32")]
    const ROTATE: u32 = 13;

    let mut acc = PRIME5;
    let mut len: PyUHash = 0;

    for element_hash in element_hashes {
        let lane = element_hash? as PyUHash;
        acc = acc.wrapping_add(lane.wrapping_mul(PRIME2));
        acc = acc.rotate_left(ROTATE);
        acc = acc.wrapping_mul(PRIME1);
        len += 1;
    }

    acc = acc.wrapping_add(len ^ (PRIME5 ^ 3527539));

    let acc_py_hash = acc as PyHash;
    if acc_py_hash == -1 {
        return Ok(1546275796);
    }

    Ok(acc_py_hash)
}

/// frozenset_hash: order-independent XOR-fold of a frozenset's element hashes.
///
/// The entry hashes are fed in one at a time via [`FrozenSetHash::add`], so the
/// caller keeps ownership of the iteration (which may hold a lock and compute
/// each element hash fallibly). The fold is commutative, so element order does
/// not affect the result.
pub struct FrozenSetHash {
    // collections.abc.Set._hash uses sys.maxsize to mask this algorithm.
    hash: usize,
}

impl FrozenSetHash {
    #[must_use]
    pub fn new(len: usize) -> Self {
        // Factor in the number of active entries
        Self {
            hash: len.wrapping_add(1).wrapping_mul(1927868237),
        }
    }

    pub fn add(&mut self, element_hash: PyHash) {
        // Work to increase the bit dispersion for closely spaced hash values.
        // This is important because some use cases have many combinations of a
        // small number of elements with nearby hashes so that many distinct
        // combinations collapse to only a handful of distinct hash values.
        const fn shuffle_bits(h: usize) -> usize {
            ((h ^ 89869747) ^ (h.wrapping_shl(16))).wrapping_mul(3644798167)
        }
        // Xor-in shuffled bits from every entry's hash field because xor is
        // commutative and a frozenset hash should be independent of order.
        self.hash ^= shuffle_bits(element_hash as usize);
    }

    #[must_use]
    pub fn finish(self) -> PyHash {
        let mut hash = self.hash;
        // Disperse patterns arising in nested frozen-sets
        hash ^= (hash >> 11) ^ (hash >> 25);
        hash = hash.wrapping_mul(69069).wrapping_add(907133923);
        // -1 is reserved as an error code
        if hash == usize::MAX {
            hash = 590923713;
        }
        hash as PyHash
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_hash_uses_canonical_code_units() {
        let secret = HashSecret::new(42);
        for text in ["", "abc", "éÿ", "äú∑ℇ", "a𐀀b"] {
            let max = text.chars().map(u32::from).max().unwrap_or(0);
            let mut storage = alloc::vec::Vec::new();
            for ch in text.chars() {
                if max <= 0xff {
                    storage.push(ch as u8);
                } else if max <= 0xffff {
                    storage.extend_from_slice(&(ch as u16).to_ne_bytes());
                } else {
                    storage.extend_from_slice(&(ch as u32).to_ne_bytes());
                }
            }
            assert_eq!(secret.hash_str(text), secret.hash_bytes(&storage));
        }
        let surrogate = crate::wtf8::Wtf8Buf::from_wide(&[0xd800]);
        assert_eq!(
            secret.hash_wtf8(&surrogate),
            secret.hash_bytes(&0xd800u16.to_ne_bytes())
        );
    }

    #[test]
    fn native_integer_key_hash_matches_bigint() {
        for value in [0, 1, isize::MAX as usize, usize::MAX] {
            assert_eq!(hash_usize(value), hash_bigint(&BigInt::from(value)));
        }
    }

    #[test]
    fn frozenset_hash_matches_set_mixin_width() {
        for elements in [
            &[][..],
            &[0][..],
            &[-1, 1][..],
            &[1, 2, 3][..],
            &[PyHash::MIN, PyHash::MAX][..],
        ] {
            let mut hasher = FrozenSetHash::new(elements.len());
            let mask = usize::MAX as u128;
            let mut expected = (1927868237 * (elements.len() as u128 + 1)) & mask;
            for &element in elements {
                hasher.add(element);
                let h = element as u64 as u128;
                expected ^= (h ^ (h << 16) ^ 89869747) * 3644798167;
                expected &= mask;
            }
            expected ^= (expected >> 11) ^ (expected >> 25);
            expected = (expected * 69069 + 907133923) & mask;
            let expected = expected as usize as PyHash;
            let expected = if expected == -1 { 590923713 } else { expected };
            assert_eq!(hasher.finish(), expected);
        }
    }

    #[test]
    fn tuple_hash_uses_the_declared_hash_width() {
        for (elements, expected64, expected32) in [
            (&[][..], 5740354900026072187i64, 750394483),
            (&[0][..], -8753497827991233192, 1214856301),
            (&[0, 0][..], -8458139203682520985, -168982784),
        ] {
            let expected = if usize::BITS == 64 {
                expected64 as PyHash
            } else {
                expected32
            };
            let actual = hash_tuple(elements.iter().copied().map(Ok::<_, ()>)).unwrap();
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn from_keys_is_stable_and_seed_independent() {
        const K0: u64 = 0x0706_0504_0302_0100;
        const K1: u64 = 0x0f0e_0d0c_0b0a_0908;
        const LOCKED_DIGEST: PyHash = 424532722944612153u64 as PyHash;

        // Two secrets built from the same explicit keys hash identically, and
        // the digest does not depend on the seed-derivation path.
        let a = HashSecret::from_keys(K0, K1);
        let b = HashSecret::from_keys(K0, K1);
        assert_eq!(a.hash_str("hello"), b.hash_str("hello"));
        assert_eq!(
            a.hash_bytes(b"a fixed message"),
            b.hash_bytes(b"a fixed message")
        );

        // Explicit keys drive the SipHasher-2-4 directly. `keyed_hash` pins
        // k1 = 0, so a secret built with the same k0 and k1 = 0 must reproduce
        // its raw digest.
        let zero_k1 = HashSecret::from_keys(K0, 0);
        assert_eq!(keyed_hash(K0, b"payload"), zero_k1.hash_one(b"payload"));

        // Locked digest so an accidental keying change is caught.
        assert_eq!(a.hash_str("determinism"), LOCKED_DIGEST);
    }
}

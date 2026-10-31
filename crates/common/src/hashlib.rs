// spell-checker:ignore blake2b blake2s dklen fanout hmac opad pbkdf2 scrypt wasm32

//! VM-independent hashlib digest engine.
//!
//! The engine owns digest, HMAC, PBKDF2-HMAC, and scrypt state and reports
//! plain Rust results so interpreter and embedding layers can provide their
//! own object and exception adapters.

use md5::Md5;
use parking_lot::Mutex;
use sha1::Sha1;
use sha2::{Digest, Sha224, Sha256, Sha384, Sha512};
use sha3::{Sha3_224, Sha3_256, Sha3_384, Sha3_512};
use shake::{
    Shake128, Shake256,
    digest::{ExtendableOutput, Update, XofReader},
};

#[derive(Clone)]
enum HashState {
    Md5(Md5),
    Sha1(Sha1),
    Sha224(Sha224),
    Sha256(Sha256),
    Sha384(Sha384),
    Sha512(Sha512),
    Sha3_224(Sha3_224),
    Sha3_256(Sha3_256),
    Sha3_384(Sha3_384),
    Sha3_512(Sha3_512),
    Shake128(Shake128),
    Shake256(Shake256),
    Blake2b(blake2b_simd::State),
    Blake2s(blake2s_simd::State),
}

impl HashState {
    fn new(name: &str) -> Option<Self> {
        Some(match name {
            "md5" => Self::Md5(Md5::default()),
            "sha1" => Self::Sha1(Sha1::default()),
            "sha224" => Self::Sha224(Sha224::default()),
            "sha256" => Self::Sha256(Sha256::default()),
            "sha384" => Self::Sha384(Sha384::default()),
            "sha512" => Self::Sha512(Sha512::default()),
            "sha3_224" => Self::Sha3_224(Sha3_224::default()),
            "sha3_256" => Self::Sha3_256(Sha3_256::default()),
            "sha3_384" => Self::Sha3_384(Sha3_384::default()),
            "sha3_512" => Self::Sha3_512(Sha3_512::default()),
            "shake_128" => Self::Shake128(Shake128::default()),
            "shake_256" => Self::Shake256(Shake256::default()),
            "blake2b" => Self::Blake2b(blake2b_simd::Params::new().to_state()),
            "blake2s" => Self::Blake2s(blake2s_simd::Params::new().to_state()),
            _ => return None,
        })
    }

    fn update(&mut self, data: &[u8]) {
        macro_rules! update {
            ($state:expr) => {
                Update::update($state, data)
            };
        }
        match self {
            Self::Md5(state) => update!(state),
            Self::Sha1(state) => update!(state),
            Self::Sha224(state) => update!(state),
            Self::Sha256(state) => update!(state),
            Self::Sha384(state) => update!(state),
            Self::Sha512(state) => update!(state),
            Self::Sha3_224(state) => update!(state),
            Self::Sha3_256(state) => update!(state),
            Self::Sha3_384(state) => update!(state),
            Self::Sha3_512(state) => update!(state),
            Self::Shake128(state) => update!(state),
            Self::Shake256(state) => update!(state),
            Self::Blake2b(state) => {
                state.update(data);
            }
            Self::Blake2s(state) => {
                state.update(data);
            }
        }
    }

    fn digest(&self, length: usize) -> Vec<u8> {
        macro_rules! fixed {
            ($state:expr) => {
                Digest::finalize($state.clone()).to_vec()
            };
        }
        match self {
            Self::Md5(state) => fixed!(state),
            Self::Sha1(state) => fixed!(state),
            Self::Sha224(state) => fixed!(state),
            Self::Sha256(state) => fixed!(state),
            Self::Sha384(state) => fixed!(state),
            Self::Sha512(state) => fixed!(state),
            Self::Sha3_224(state) => fixed!(state),
            Self::Sha3_256(state) => fixed!(state),
            Self::Sha3_384(state) => fixed!(state),
            Self::Sha3_512(state) => fixed!(state),
            Self::Blake2b(state) => state.finalize().as_bytes().to_vec(),
            Self::Blake2s(state) => state.finalize().as_bytes().to_vec(),
            Self::Shake128(state) => {
                let mut out = vec![0; length];
                state.clone().finalize_xof().read(&mut out);
                out
            }
            Self::Shake256(state) => {
                let mut out = vec![0; length];
                state.clone().finalize_xof().read(&mut out);
                out
            }
        }
    }
}

type LockedHashState = Mutex<HashState>;

/// Size/alignment contract for the opaque, object-owned storage embedded in
/// a caller-owned `_HashState` payload.  The digest implementations are
/// fixed-size state machines; none owns heap memory or needs drop glue.
///
/// The capacity is stated in BYTES because that is what has to hold a state
/// machine whose size does not follow the target's pointer width.  Stating it
/// as a word count gave wasm32 half the room a 64-bit target got, and the
/// runtime check below then aborted the guest on the first `hashlib` digest.
pub const HASH_STATE_STORAGE_BYTES: usize = 512;
pub const HASH_STATE_STORAGE_WORDS: usize =
    HASH_STATE_STORAGE_BYTES / core::mem::size_of::<usize>();
pub const HASH_STATE_STORAGE_ALIGN: usize = 16;

// The state's size is a compile-time fact on every target, so the target that
// cannot hold it fails to build rather than trapping in the field.
const _: () = assert!(core::mem::size_of::<LockedHashState>() <= HASH_STATE_STORAGE_BYTES);
const _: () = assert!(core::mem::align_of::<LockedHashState>() <= HASH_STATE_STORAGE_ALIGN);

fn check_storage(storage: *mut usize, words: usize) {
    assert!(words >= HASH_STATE_STORAGE_WORDS);
    assert!(!storage.is_null());
    assert_eq!((storage as usize) % HASH_STATE_STORAGE_ALIGN, 0);
}

/// Initialize an object-owned opaque state buffer. Returns false for an
/// unsupported canonical digest name.
///
/// # Safety
///
/// `storage` must point to an aligned, writable, uninitialized buffer of at
/// least `words * size_of::<usize>()` bytes. The buffer must remain valid until
/// its successfully initialized state is passed to [`state_drop`].
pub unsafe fn state_init(storage: *mut usize, words: usize, name: &str) -> bool {
    check_storage(storage, words);
    let Some(state) = HashState::new(name) else {
        return false;
    };
    unsafe { storage.cast::<LockedHashState>().write(Mutex::new(state)) };
    true
}

/// Initialize a BLAKE2 state with the complete RFC 7693 parameter block.
/// The Python layer validates the exact argument ranges before calling;
/// the backend APIs then encode the fields in the algorithm-defined little-
/// endian parameter layout.
///
/// # Safety
///
/// `storage` must point to an aligned, writable, uninitialized buffer of at
/// least `words * size_of::<usize>()` bytes. The buffer must remain valid until
/// its successfully initialized state is passed to [`state_drop`].
#[allow(clippy::too_many_arguments)]
pub unsafe fn state_init_blake2(
    storage: *mut usize,
    words: usize,
    name: &str,
    digest_size: usize,
    key: &[u8],
    salt: &[u8],
    person: &[u8],
    fanout: u8,
    depth: u8,
    leaf_size: u32,
    node_offset: u64,
    node_depth: u8,
    inner_size: usize,
    last_node: bool,
) -> bool {
    check_storage(storage, words);
    let state = match name {
        "blake2b" => {
            let mut params = blake2b_simd::Params::new();
            params
                .hash_length(digest_size)
                .key(key)
                .salt(salt)
                .personal(person)
                .fanout(fanout)
                .max_depth(depth)
                .max_leaf_length(leaf_size)
                .node_offset(node_offset)
                .node_depth(node_depth)
                .inner_hash_length(inner_size)
                .last_node(last_node);
            HashState::Blake2b(params.to_state())
        }
        "blake2s" => {
            let mut params = blake2s_simd::Params::new();
            params
                .hash_length(digest_size)
                .key(key)
                .salt(salt)
                .personal(person)
                .fanout(fanout)
                .max_depth(depth)
                .max_leaf_length(leaf_size)
                .node_offset(node_offset)
                .node_depth(node_depth)
                .inner_hash_length(inner_size)
                .last_node(last_node);
            HashState::Blake2s(params.to_state())
        }
        _ => return false,
    };
    unsafe { storage.cast::<LockedHashState>().write(Mutex::new(state)) };
    true
}

/// # Safety
///
/// `storage` must contain a live state initialized by [`state_init`] or
/// [`state_init_blake2`] with the same `words` value, and it must not be
/// dropped while this operation runs.
pub unsafe fn state_update(storage: *mut usize, words: usize, data: &[u8]) {
    check_storage(storage, words);
    let state = unsafe { &*storage.cast::<LockedHashState>() };
    state.lock().update(data);
}

/// # Safety
///
/// `storage` must contain a live state initialized by [`state_init`] or
/// [`state_init_blake2`] with the same `words` value, and it must not be
/// dropped while this operation runs.
#[must_use]
pub unsafe fn state_digest(storage: *const usize, words: usize, length: usize) -> Vec<u8> {
    check_storage(storage.cast_mut(), words);
    let state = unsafe { &*storage.cast::<LockedHashState>() };
    state.lock().digest(length)
}

/// # Safety
///
/// `src` must contain a live initialized state, while `dst` must point to a
/// distinct aligned, writable, uninitialized buffer. Both buffers must hold at
/// least `words * size_of::<usize>()` bytes and remain valid for this call.
pub unsafe fn state_copy(src: *const usize, dst: *mut usize, words: usize) {
    check_storage(src.cast_mut(), words);
    check_storage(dst, words);
    let source = unsafe { &*src.cast::<LockedHashState>() };
    let cloned = source.lock().clone();
    unsafe { dst.cast::<LockedHashState>().write(Mutex::new(cloned)) };
}

/// # Safety
///
/// `storage` must contain a live state initialized by [`state_init`],
/// [`state_init_blake2`], or [`state_copy`] with the same `words` value. The
/// state must not be used again after this call.
pub unsafe fn state_drop(storage: *mut usize, words: usize) {
    check_storage(storage, words);
    unsafe { core::ptr::drop_in_place(storage.cast::<LockedHashState>()) };
}

#[derive(Clone)]
struct HmacState {
    inner: HashState,
    outer: HashState,
}

impl HmacState {
    fn new(name: &str, key: &[u8]) -> Option<Self> {
        let block_size = digest_block_size(name)?;
        let mut key = if key.len() > block_size {
            HashState::new(name)?.tap_update(key).digest(0)
        } else {
            key.to_vec()
        };
        key.resize(block_size, 0);
        let mut inner = HashState::new(name)?;
        let mut outer = HashState::new(name)?;
        let mut ipad = key.clone();
        let mut opad = key;
        for byte in &mut ipad {
            *byte ^= 0x36;
        }
        for byte in &mut opad {
            *byte ^= 0x5c;
        }
        inner.update(&ipad);
        outer.update(&opad);
        Some(Self { inner, outer })
    }

    fn update(&mut self, data: &[u8]) {
        self.inner.update(data);
    }

    fn digest(&self) -> Vec<u8> {
        let mut outer = self.outer.clone();
        outer.update(&self.inner.digest(0));
        outer.digest(0)
    }
}

/// PBKDF2-HMAC (RFC 8018), used by `_hashlib.pbkdf2_hmac`.
#[must_use]
pub fn compute_pbkdf2_hmac(
    name: &str,
    password: &[u8],
    salt: &[u8],
    iterations: usize,
    dklen: usize,
) -> Option<Vec<u8>> {
    if iterations == 0 || dklen == 0 {
        return None;
    }
    let digest_size = digest_output_size(name)?;
    let blocks = dklen.checked_add(digest_size - 1)? / digest_size;
    if blocks > u32::MAX as usize {
        return None;
    }
    let mut derived = Vec::with_capacity(blocks * digest_size);
    let mut first_input = Vec::with_capacity(salt.len() + 4);
    first_input.extend_from_slice(salt);
    for block in 1..=blocks {
        first_input.truncate(salt.len());
        first_input.extend_from_slice(&(block as u32).to_be_bytes());
        let mut hmac = HmacState::new(name, password)?;
        hmac.update(&first_input);
        let mut u = hmac.digest();
        let mut accumulator = u.clone();
        for _ in 1..iterations {
            let mut hmac = HmacState::new(name, password)?;
            hmac.update(&u);
            u = hmac.digest();
            for (out, byte) in accumulator.iter_mut().zip(&u) {
                *out ^= *byte;
            }
        }
        derived.extend_from_slice(&accumulator);
    }
    derived.truncate(dklen);
    Some(derived)
}

/// RFC 7914 scrypt, used by `_hashlib.scrypt`.
///
/// `_hashlib` performs argument and memory-limit validation before entering
/// this backend.  `Params::new` still enforces the algorithm's overflow
/// constraints; the raw `scrypt` KDF sizes the caller-owned output
/// independently of any password-hash facade length.
#[must_use]
pub fn compute_scrypt(
    password: &[u8],
    salt: &[u8],
    log_n: u8,
    r: u32,
    p: u32,
    dklen: usize,
) -> Option<Vec<u8>> {
    let params = scrypt::Params::new(log_n, r, p).ok()?;
    let mut output = vec![0; dklen];
    scrypt::scrypt(password, salt, &params, &mut output).ok()?;
    Some(output)
}

impl HashState {
    fn tap_update(mut self, data: &[u8]) -> Self {
        self.update(data);
        self
    }
}

#[must_use]
pub fn digest_block_size(name: &str) -> Option<usize> {
    Some(match name {
        "md5" | "sha1" | "sha224" | "sha256" | "blake2s" => 64,
        "sha384" | "sha512" | "blake2b" => 128,
        "sha3_224" => 144,
        "sha3_256" => 136,
        "sha3_384" => 104,
        "sha3_512" => 72,
        // RFC 2104 HMAC is not defined for XOF algorithms.
        "shake_128" | "shake_256" => return None,
        _ => return None,
    })
}

#[must_use]
pub fn digest_output_size(name: &str) -> Option<usize> {
    Some(match name {
        "md5" => 16,
        "sha1" => 20,
        "sha224" | "sha3_224" => 28,
        "sha256" | "sha3_256" | "blake2s" => 32,
        "sha384" | "sha3_384" => 48,
        "sha512" | "sha3_512" | "blake2b" => 64,
        "shake_128" | "shake_256" => return None,
        _ => return None,
    })
}

type LockedHmacState = Mutex<HmacState>;
/// [`HASH_STATE_STORAGE_BYTES`]'s counterpart: an HMAC carries two digest
/// states, so it is sized in bytes for the same reason.
pub const HMAC_STATE_STORAGE_BYTES: usize = 1024;
pub const HMAC_STATE_STORAGE_WORDS: usize =
    HMAC_STATE_STORAGE_BYTES / core::mem::size_of::<usize>();
pub const HMAC_STATE_STORAGE_ALIGN: usize = 16;

const _: () = assert!(core::mem::size_of::<LockedHmacState>() <= HMAC_STATE_STORAGE_BYTES);
const _: () = assert!(core::mem::align_of::<LockedHmacState>() <= HMAC_STATE_STORAGE_ALIGN);

fn check_hmac_storage(storage: *mut usize, words: usize) {
    assert!(words >= HMAC_STATE_STORAGE_WORDS);
    assert!(!storage.is_null());
    assert_eq!((storage as usize) % HMAC_STATE_STORAGE_ALIGN, 0);
}

/// # Safety
///
/// `storage` must point to an aligned, writable, uninitialized buffer of at
/// least `words * size_of::<usize>()` bytes. The buffer must remain valid until
/// its successfully initialized state is passed to [`hmac_state_drop`].
pub unsafe fn hmac_state_init(storage: *mut usize, words: usize, name: &str, key: &[u8]) -> bool {
    check_hmac_storage(storage, words);
    let Some(state) = HmacState::new(name, key) else {
        return false;
    };
    unsafe { storage.cast::<LockedHmacState>().write(Mutex::new(state)) };
    true
}

/// # Safety
///
/// `storage` must contain a live state initialized by [`hmac_state_init`] with
/// the same `words` value, and it must not be dropped while this operation
/// runs.
pub unsafe fn hmac_state_update(storage: *mut usize, words: usize, data: &[u8]) {
    check_hmac_storage(storage, words);
    unsafe { &*storage.cast::<LockedHmacState>() }
        .lock()
        .update(data);
}

/// # Safety
///
/// `storage` must contain a live state initialized by [`hmac_state_init`] with
/// the same `words` value, and it must not be dropped while this operation
/// runs.
#[must_use]
pub unsafe fn hmac_state_digest(storage: *const usize, words: usize) -> Vec<u8> {
    check_hmac_storage(storage.cast_mut(), words);
    unsafe { &*storage.cast::<LockedHmacState>() }
        .lock()
        .digest()
}

/// # Safety
///
/// `src` must contain a live initialized HMAC state, while `dst` must point to
/// a distinct aligned, writable, uninitialized buffer. Both buffers must hold
/// at least `words * size_of::<usize>()` bytes and remain valid for this call.
pub unsafe fn hmac_state_copy(src: *const usize, dst: *mut usize, words: usize) {
    check_hmac_storage(src.cast_mut(), words);
    check_hmac_storage(dst, words);
    let cloned = unsafe { &*src.cast::<LockedHmacState>() }.lock().clone();
    unsafe { dst.cast::<LockedHmacState>().write(Mutex::new(cloned)) };
}

/// # Safety
///
/// `storage` must contain a live state initialized by [`hmac_state_init`] or
/// [`hmac_state_copy`] with the same `words` value. The state must not be used
/// again after this call.
pub unsafe fn hmac_state_drop(storage: *mut usize, words: usize) {
    check_hmac_storage(storage, words);
    unsafe { core::ptr::drop_in_place(storage.cast::<LockedHmacState>()) };
}

#[must_use]
#[inline(never)]
pub fn compute_digest(name: &str, data: &[u8], length: usize) -> Option<Vec<u8>> {
    let digest = match name {
        "md5" => Md5::digest(data).to_vec(),
        "sha1" => Sha1::digest(data).to_vec(),
        "sha224" => Sha224::digest(data).to_vec(),
        "sha256" => Sha256::digest(data).to_vec(),
        "sha384" => Sha384::digest(data).to_vec(),
        "sha512" => Sha512::digest(data).to_vec(),
        "sha3_224" => Sha3_224::digest(data).to_vec(),
        "sha3_256" => Sha3_256::digest(data).to_vec(),
        "sha3_384" => Sha3_384::digest(data).to_vec(),
        "sha3_512" => Sha3_512::digest(data).to_vec(),
        "blake2b" => blake2b_simd::blake2b(data).as_bytes().to_vec(),
        "blake2s" => blake2s_simd::blake2s(data).as_bytes().to_vec(),
        "shake_128" => {
            let mut h = Shake128::default();
            h.update(data);
            let mut out = vec![0u8; length];
            h.finalize_xof().read(&mut out);
            out
        }
        "shake_256" => {
            let mut h = Shake256::default();
            h.update(data);
            let mut out = vec![0u8; length];
            h.finalize_xof().read(&mut out);
            out
        }
        _ => return None,
    };
    Some(digest)
}

#[cfg(test)]
mod tests {
    use super::{
        HASH_STATE_STORAGE_WORDS, HMAC_STATE_STORAGE_WORDS, compute_digest, compute_pbkdf2_hmac,
        compute_scrypt, hmac_state_digest, hmac_state_drop, hmac_state_init, hmac_state_update,
        state_copy, state_digest, state_drop, state_init, state_init_blake2, state_update,
    };

    fn hex(bytes: &[u8]) -> String {
        use core::fmt::Write;
        bytes.iter().fold(String::new(), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
    }

    #[test]
    fn unknown_names_are_rejected() {
        let mut state = HmacStorage([0usize; HMAC_STATE_STORAGE_WORDS + 1]);
        for name in ["foo", "", "shake_128", "FOO"] {
            let ok = unsafe {
                hmac_state_init(
                    aligned_mut_ptr(&mut state.0),
                    HMAC_STATE_STORAGE_WORDS,
                    name,
                    b"key",
                )
            };
            assert!(!ok, "hmac_state_init accepted {name:?}");
        }
        let mut hstate = HashStorage([0usize; HASH_STATE_STORAGE_WORDS + 1]);
        for name in ["foo", "", "FOO"] {
            let ok = unsafe {
                state_init(
                    aligned_mut_ptr(&mut hstate.0),
                    HASH_STATE_STORAGE_WORDS,
                    name,
                )
            };
            assert!(!ok, "state_init accepted {name:?}");
        }
    }

    struct HashStorage([usize; HASH_STATE_STORAGE_WORDS + 1]);

    struct HmacStorage([usize; HMAC_STATE_STORAGE_WORDS + 1]);

    fn aligned_ptr(words: &[usize]) -> *const usize {
        let address = words.as_ptr() as usize;
        ((address + 15) & !15) as *const usize
    }

    fn aligned_mut_ptr(words: &mut [usize]) -> *mut usize {
        aligned_ptr(words) as *mut usize
    }

    #[test]
    fn computes_fixed_length_digests() {
        assert_eq!(
            hex(&compute_digest("md5", b"abc", 0).unwrap()),
            "900150983cd24fb0d6963f7d28e17f72"
        );
        assert_eq!(
            hex(&compute_digest("sha256", b"abc", 0).unwrap()),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn computes_extendable_output_digests() {
        let digest = compute_digest("shake_128", b"abc", 8).unwrap();
        assert_eq!(digest.len(), 8);
        assert_eq!(hex(&digest), "5881092dd818bf5c");
    }

    #[test]
    fn rejects_unknown_algorithm() {
        assert!(compute_digest("not-a-hash", b"abc", 0).is_none());
    }

    #[test]
    fn incremental_state_updates_and_copies_independently() {
        let mut state = HashStorage([0usize; HASH_STATE_STORAGE_WORDS + 1]);
        let mut clone = HashStorage([0usize; HASH_STATE_STORAGE_WORDS + 1]);
        unsafe {
            assert!(state_init(
                aligned_mut_ptr(&mut state.0),
                HASH_STATE_STORAGE_WORDS,
                "sha256"
            ));
            state_update(
                aligned_mut_ptr(&mut state.0),
                HASH_STATE_STORAGE_WORDS,
                b"ab",
            );
            state_copy(
                aligned_ptr(&state.0),
                aligned_mut_ptr(&mut clone.0),
                HASH_STATE_STORAGE_WORDS,
            );
            state_update(
                aligned_mut_ptr(&mut state.0),
                HASH_STATE_STORAGE_WORDS,
                b"c",
            );
            state_update(
                aligned_mut_ptr(&mut clone.0),
                HASH_STATE_STORAGE_WORDS,
                b"d",
            );
            assert_eq!(
                state_digest(aligned_ptr(&state.0), HASH_STATE_STORAGE_WORDS, 0),
                compute_digest("sha256", b"abc", 0).unwrap()
            );
            assert_eq!(
                state_digest(aligned_ptr(&clone.0), HASH_STATE_STORAGE_WORDS, 0),
                compute_digest("sha256", b"abd", 0).unwrap()
            );
            state_drop(aligned_mut_ptr(&mut state.0), HASH_STATE_STORAGE_WORDS);
            state_drop(aligned_mut_ptr(&mut clone.0), HASH_STATE_STORAGE_WORDS);
        }
    }

    #[test]
    fn incremental_hmac_matches_rfc_4231_sha256() {
        let mut state = HmacStorage([0usize; HMAC_STATE_STORAGE_WORDS + 1]);
        unsafe {
            assert!(hmac_state_init(
                aligned_mut_ptr(&mut state.0),
                HMAC_STATE_STORAGE_WORDS,
                "sha256",
                &[0x0b; 20],
            ));
            hmac_state_update(
                aligned_mut_ptr(&mut state.0),
                HMAC_STATE_STORAGE_WORDS,
                b"Hi There",
            );
            assert_eq!(
                hex(&hmac_state_digest(
                    aligned_ptr(&state.0),
                    HMAC_STATE_STORAGE_WORDS,
                )),
                "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
            );
            hmac_state_drop(aligned_mut_ptr(&mut state.0), HMAC_STATE_STORAGE_WORDS);
        }
    }

    #[test]
    fn pbkdf2_hmac_matches_rfc_6070_sha1() {
        assert_eq!(
            hex(&compute_pbkdf2_hmac("sha1", b"password", b"salt", 2, 20).unwrap()),
            "ea6c014dc72d6f8ccd1ed92ace1d41f0d8de8957"
        );
    }

    #[test]
    fn scrypt_matches_rfc_7914() {
        assert_eq!(
            hex(&compute_scrypt(b"", b"", 4, 1, 1, 64).unwrap()),
            "77d6576238657b203b19ca42c18a0497f16b4844e3074ae8dfdffa3fede21442\
             fcd0069ded0948f8326a753a0fc81f17e8d3e0fb2e0d3628cf35e20c38d18906"
                .replace(' ', "")
        );
    }

    #[test]
    fn blake2_parameter_block_matches_cpython_vectors() {
        for (name, expected) in [
            ("blake2b", "920568b0c5873b2f0ab67bedb6cf1b2b"),
            ("blake2s", "bf2a8f7fe3c555012a6f8046e646bc75"),
        ] {
            let mut state = HashStorage([0usize; HASH_STATE_STORAGE_WORDS + 1]);
            unsafe {
                assert!(state_init_blake2(
                    aligned_mut_ptr(&mut state.0),
                    HASH_STATE_STORAGE_WORDS,
                    name,
                    16,
                    b"bar",
                    b"baz",
                    b"bing",
                    2,
                    3,
                    4,
                    5,
                    6,
                    7,
                    true,
                ));
                state_update(
                    aligned_mut_ptr(&mut state.0),
                    HASH_STATE_STORAGE_WORDS,
                    b"foo",
                );
                assert_eq!(
                    hex(&state_digest(
                        aligned_ptr(&state.0),
                        HASH_STATE_STORAGE_WORDS,
                        0,
                    )),
                    expected
                );
                state_drop(aligned_mut_ptr(&mut state.0), HASH_STATE_STORAGE_WORDS);
            }
        }
    }
}

/// Get `N` bytes of random data.
///
/// This function is mildly expensive to call, as it fetches random data
/// directly from the OS entropy source.
///
/// # Panics
///
/// Panics if the OS entropy source returns an error.
pub fn os_random<const N: usize>() -> [u8; N] {
    let mut buf = [0u8; N];
    getrandom::fill(&mut buf).unwrap();
    buf
}

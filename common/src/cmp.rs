use volatile::Volatile;

/// Compare 2 byte slices in a way that ensures that the timing of the operation can't be used to
/// glean any information about the data.
#[inline(never)]
#[cold]
pub fn timing_safe_cmp(a: &[u8], b: &[u8]) -> bool {
    // we use raw pointers here to keep faithful to the C implementation and
    // to try to avoid any optimizations rustc might do with slices
    let len_a = a.len();
    let a = a.as_ptr();
    let len_b = b.len();
    let b = b.as_ptr();
    /* The volatile type declarations make sure that the compiler has no
     * chance to optimize and fold the code in any way that may change
     * the timing.
     */
    let length: Volatile<usize>;
    let mut left: Volatile<*const u8>;
    let mut right: Volatile<*const u8>;
    let mut result: u8 = 0;

    /* loop count depends on length of b */
    length = Volatile::new(len_b);
    left = Volatile::new(std::ptr::null());
    right = Volatile::new(b);

    /* don't use else here to keep the amount of CPU instructions constant,
     * volatile forces re-evaluation
     *  */
    if len_a == length.read() {
        left.write(Volatile::new(a).read());
        result = 0;
    }
    if len_a != length.read() {
        left.write(b);
        result = 1;
    }

    for _ in 0..length.read() {
        let l = left.read();
        left.write(l.wrapping_add(1));
        let r = right.read();
        right.write(r.wrapping_add(1));
        // safety: the 0..length range will always be either:
        // * as long as the length of both a and b, if len_a and len_b are equal
        // * as long as b, and both `left` and `right` are b
        result |= unsafe { l.read_volatile() ^ r.read_volatile() };
    }

    result == 0
}

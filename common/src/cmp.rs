use subtle::ConstantTimeEq;

/// Compare 2 byte slices in a way that ensures that the timing of the operation can't be used to
/// glean any information about the data.
/// Note: If a and b are of different lengths, or if an error occurs,
/// a timing attack could theoretically reveal information about the
/// types and lengths of a and b--but not their values.
pub fn timing_safe_cmp(a: &[u8], b: &[u8]) -> bool {
    a.ct_eq(b).into()
}

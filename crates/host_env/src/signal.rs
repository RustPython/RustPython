#[must_use]
pub fn timeval_to_double(tv: &libc::timeval) -> f64 {
    tv.tv_sec as f64 + (tv.tv_usec as f64 / 1_000_000.0)
}

#[must_use]
pub fn double_to_timeval(val: f64) -> libc::timeval {
    libc::timeval {
        tv_sec: val.trunc() as _,
        tv_usec: (val.fract() * 1_000_000.0) as _,
    }
}

#[must_use]
pub fn itimerval_to_tuple(it: &libc::itimerval) -> (f64, f64) {
    (
        timeval_to_double(&it.it_value),
        timeval_to_double(&it.it_interval),
    )
}

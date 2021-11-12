pub(crate) use _statistics::make_module;

#[pymodule]
mod _statistics {

    #[pyfunction]
    pub unsafe extern fn _normal_dist_inv_cdf(...) {}
}


pub(crate) use _statistics::make_module;

#[pymodule]
mod _statistics {
    use crate::vm::{function::ArgIntoFloat, PyResult, VirtualMachine};

    // See https://github.com/python/cpython/blob/6846d6712a0894f8e1a91716c11dd79f42864216/Modules/_statisticsmodule.c#L28-L120
    #[allow(clippy::excessive_precision)]
    fn normal_dist_inv_cdf(p: f64, mu: f64, sigma: f64) -> Option<f64> {
        if p <= 0.0 || p >= 1.0 || sigma <= 0.0 {
            return None;
        }

        let q = p - 0.5;
        if q.abs() <= 0.425 {
            let r = 0.180625 - q * q;
            // Hash sum-55.8831928806149014439
            let num = (((((((2.5090809287301226727e+3 * r + 3.3430575583588128105e+4) * r
                + 6.7265770927008700853e+4)
                * r
                + 4.5921953931549871457e+4)
                * r
                + 1.3731693765509461125e+4)
                * r
                + 1.9715909503065514427e+3)
                * r
                + 1.3314166789178437745e+2)
                * r
                + 3.3871328727963666080e+0)
                * q;
            let den = ((((((5.2264952788528545610e+3 * r + 2.8729085735721942674e+4) * r
                + 3.9307895800092710610e+4)
                * r
                + 2.1213794301586595867e+4)
                * r
                + 5.3941960214247511077e+3)
                * r
                + 6.8718700749205790830e+2)
                * r
                + 4.2313330701600911252e+1)
                * r
                + 1.0;
            if den == 0.0 {
                return None;
            }
            let x = num / den;
            return Some(mu + (x * sigma));
        }
        let r = if q <= 0.0 { p } else { 1.0 - p };
        if r <= 0.0 || r >= 1.0 {
            return None;
        }
        let r = (-(r.ln())).sqrt();
        let num;
        let den;
        #[allow(clippy::excessive_precision)]
        if r <= 5.0 {
            let r = r - 1.6;
            // Hash sum-49.33206503301610289036
            num = ((((((7.74545014278341407640e-4 * r + 2.27238449892691845833e-2) * r
                + 2.41780725177450611770e-1)
                * r
                + 1.27045825245236838258e+0)
                * r
                + 3.64784832476320460504e+0)
                * r
                + 5.76949722146069140550e+0)
                * r
                + 4.63033784615654529590e+0)
                * r
                + 1.42343711074968357734e+0;
            den = ((((((1.05075007164441684324e-9 * r + 5.47593808499534494600e-4) * r
                + 1.51986665636164571966e-2)
                * r
                + 1.48103976427480074590e-1)
                * r
                + 6.89767334985100004550e-1)
                * r
                + 1.67638483018380384940e+0)
                * r
                + 2.05319162663775882187e+0)
                * r
                + 1.0;
        } else {
            let r = r - 5.0;
            // Hash sum-47.52583317549289671629
            num = ((((((2.01033439929228813265e-7 * r + 2.71155556874348757815e-5) * r
                + 1.24266094738807843860e-3)
                * r
                + 2.65321895265761230930e-2)
                * r
                + 2.96560571828504891230e-1)
                * r
                + 1.78482653991729133580e+0)
                * r
                + 5.46378491116411436990e+0)
                * r
                + 6.65790464350110377720e+0;
            den = ((((((2.04426310338993978564e-15 * r + 1.42151175831644588870e-7) * r
                + 1.84631831751005468180e-5)
                * r
                + 7.86869131145613259100e-4)
                * r
                + 1.48753612908506148525e-2)
                * r
                + 1.36929880922735805310e-1)
                * r
                + 5.99832206555887937690e-1)
                * r
                + 1.0;
        }
        if den == 0.0 {
            return None;
        }
        let mut x = num / den;
        if q < 0.0 {
            x = -x;
        }
        Some(mu + (x * sigma))
    }

    #[pyfunction]
    fn _normal_dist_inv_cdf(
        p: ArgIntoFloat,
        mu: ArgIntoFloat,
        sigma: ArgIntoFloat,
        vm: &VirtualMachine,
    ) -> PyResult<f64> {
        normal_dist_inv_cdf(*p, *mu, *sigma)
            .ok_or_else(|| vm.new_value_error("inv_cdf undefined for these parameters".to_owned()))
    }
}

/// Certificates and errors returned by the platform's native trust-store
/// loader. Certificate bytes stay independent of any TLS backend type.
pub struct LoadResult {
    pub certs: Vec<Vec<u8>>,
    pub errors: Vec<String>,
}

pub fn load() -> LoadResult {
    let result = rustls_native_certs::load_native_certs();
    LoadResult {
        certs: result
            .certs
            .into_iter()
            .map(|cert| cert.as_ref().to_vec())
            .collect(),
        errors: result
            .errors
            .into_iter()
            .map(|error| error.to_string())
            .collect(),
    }
}

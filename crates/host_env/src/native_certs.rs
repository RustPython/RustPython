/// Certificates and errors returned by the platform's native trust-store
/// loader. Certificate bytes stay independent of any TLS backend type.
pub struct LoadResult {
    pub certs: Vec<Vec<u8>>,
    pub errors: Vec<String>,
}

const DEFAULT_CERT_FILES: &[&str] = &[
    "/etc/ssl/certs/ca-certificates.crt",
    "/etc/pki/tls/certs/ca-bundle.crt",
    "/etc/ssl/ca-bundle.pem",
    "/etc/pki/tls/cacert.pem",
    "/etc/ssl/cert.pem",
    "/usr/local/share/certs/ca-root-nss.crt",
    "/usr/local/etc/openssl/cert.pem",
];

const DEFAULT_CERT_DIRS: &[&str] = &[
    "/etc/ssl/certs",
    "/etc/pki/tls/certs",
    "/system/etc/security/cacerts",
    "/usr/local/share/certs",
];

fn first_existing<'a>(candidates: &'a [&'a str], exists: impl Fn(&str) -> bool) -> Option<&'a str> {
    candidates.iter().copied().find(|path| exists(path))
}

/// Return the host's first available OpenSSL-style certificate file and directory.
/// Environment overrides are intentionally left to Python's `ssl` module.
pub fn default_verify_paths() -> (String, String) {
    if cfg!(windows) {
        return (String::new(), String::new());
    }

    let cafile = first_existing(DEFAULT_CERT_FILES, |path| crate::fs::is_file(path))
        .unwrap_or("/etc/ssl/cert.pem")
        .to_owned();
    let capath = first_existing(DEFAULT_CERT_DIRS, |path| crate::fs::is_dir(path))
        .unwrap_or("/etc/ssl/certs")
        .to_owned();
    (cafile, capath)
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

#[cfg(test)]
mod tests {
    use super::first_existing;

    #[test]
    fn first_existing_path_preserves_candidate_priority() {
        let candidates = ["missing", "first", "second"];
        assert_eq!(
            first_existing(&candidates, |path| path == "first" || path == "second"),
            Some("first")
        );
        assert_eq!(first_existing(&candidates, |_| false), None);
    }
}

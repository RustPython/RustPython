#[derive(Clone, Debug)]
pub struct DeterminismOptions {
    pub enabled: bool,
    pub hash_seed: u32,
    pub stdlib_whitelist: Vec<String>,
    pub stdlib_blacklist: Vec<String>,
    pub stdlib_hash: Option<String>,
    pub enable_softfloat: bool,
    pub enable_gas: bool,
}

impl DeterminismOptions {
    pub fn deterministic(hash_seed: Option<u32>) -> Self {
        let mut options = Self::default();
        options.enabled = true;
        options.hash_seed = hash_seed.unwrap_or(0);
        options
    }

    pub fn default_whitelist() -> Vec<String> {
        vec![
            "builtins",
            "types",
            "collections",
            "collections.abc",
            "abc",
            "enum",
            "dataclasses",
            "typing",
            "functools",
            "itertools",
            "operator",
            "re",
            "sre_compile",
            "sre_parse",
            "sre_constants",
            "_sre",
            "string",
            "codecs",
            "encodings",
            "unicodedata",
            "math",
            "keyword",
            "reprlib",
            "json",
            "copyreg",
            "base64",
            "binascii",
            "struct",
            "hashlib",
            "hmac",
            "warnings",
            "heapq",
            "bisect",
            "_collections",
            "_collections_abc",
            "_functools",
            "_abc",
            "_py_abc",
            "_struct",
            "_weakrefset",
            "_weakref",
            "_thread",
            "_json",
            "_hashlib",
            "_md5",
            "_sha1",
            "_sha256",
            "_sha512",
            "_sha3",
            "_blake2",
            "_bisect",
            "_heapq",
            "_warnings",
            "_operator",
            "pvm_host",
            "pvm_sdk",
            "pvm_sdk.pvm_time",
            "pvm_sdk.pvm_random",
            "pvm_sdk.pvm_sys",
            "pvm_time",
            "pvm_random",
            "pvm_sys",
        ]
        .into_iter()
        .map(|item| item.to_owned())
        .collect()
    }

    pub fn default_blacklist() -> Vec<String> {
        vec![
            "time",
            "datetime",
            "random",
            "secrets",
            "uuid",
            "os",
            "sys",
            "socket",
            "ssl",
            "subprocess",
            "ctypes",
            "threading",
            "multiprocessing",
            "signal",
            "select",
            "asyncio",
            "pathlib",
            "glob",
            "tempfile",
            "shutil",
            "zipfile",
            "inspect",
            "traceback",
        ]
        .into_iter()
        .map(|item| item.to_owned())
        .collect()
    }
}

impl Default for DeterminismOptions {
    fn default() -> Self {
        Self {
            enabled: false,
            hash_seed: 0,
            stdlib_whitelist: Self::default_whitelist(),
            stdlib_blacklist: Self::default_blacklist(),
            stdlib_hash: None,
            enable_softfloat: false,
            enable_gas: false,
        }
    }
}

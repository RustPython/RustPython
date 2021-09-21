fn main() {
    #[allow(clippy::unusual_byte_groupings)]
    if let Ok(v) = std::env::var("DEP_OPENSSL_VERSION_NUMBER") {
        println!("cargo:rustc-env=OPENSSL_API_VERSION={}", v);
        // cfg setup from openssl crate's build script
        let version = u64::from_str_radix(&v, 16).unwrap();
        if version >= 0x1_00_01_00_0 {
            println!("cargo:rustc-cfg=ossl101");
        }
        if version >= 0x1_00_02_00_0 {
            println!("cargo:rustc-cfg=ossl102");
        }
        if version >= 0x1_01_00_00_0 {
            println!("cargo:rustc-cfg=ossl110");
        }
        if version >= 0x1_01_00_07_0 {
            println!("cargo:rustc-cfg=ossl110g");
        }
        if version >= 0x1_01_01_00_0 {
            println!("cargo:rustc-cfg=ossl111");
        }
    }
    if let Ok(v) = std::env::var("DEP_OPENSSL_CONF") {
        for conf in v.split(',') {
            println!("cargo:rustc-cfg=osslconf=\"{}\"", conf);
        }
    }
}

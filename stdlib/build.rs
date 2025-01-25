fn main() {
    println!(r#"cargo::rustc-check-cfg=cfg(osslconf, values("OPENSSL_NO_COMP"))"#);

    #[allow(clippy::unusual_byte_groupings)]
    let ossl_vers = [
        (0x1_00_01_00_0, "ossl101"),
        (0x1_00_02_00_0, "ossl102"),
        (0x1_01_00_00_0, "ossl110"),
        (0x1_01_00_07_0, "ossl110g"),
        (0x1_01_00_08_0, "ossl110h"),
        (0x1_01_01_00_0, "ossl111"),
        (0x1_01_01_04_0, "ossl111d"),
        (0x3_00_00_00_0, "ossl300"),
        (0x3_01_00_00_0, "ossl310"),
        (0x3_02_00_00_0, "ossl320"),
        (0x3_03_00_00_0, "ossl330"),
    ];

    for (_, cfg) in ossl_vers {
        println!("cargo::rustc-check-cfg=cfg({cfg})");
    }

    #[allow(clippy::unusual_byte_groupings)]
    if let Ok(v) = std::env::var("DEP_OPENSSL_VERSION_NUMBER") {
        println!("cargo:rustc-env=OPENSSL_API_VERSION={v}");
        // cfg setup from openssl crate's build script
        let version = u64::from_str_radix(&v, 16).unwrap();
        for (ver, cfg) in ossl_vers {
            if version >= ver {
                println!("cargo:rustc-cfg={cfg}");
            }
        }
    }
    if let Ok(v) = std::env::var("DEP_OPENSSL_CONF") {
        for conf in v.split(',') {
            println!("cargo:rustc-cfg=osslconf=\"{conf}\"");
        }
    }
}

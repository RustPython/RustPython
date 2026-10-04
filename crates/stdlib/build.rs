// spell-checker:ignore ossl osslconf

fn main() {
    println!(r#"cargo::rustc-check-cfg=cfg(osslconf, values("OPENSSL_NO_COMP"))"#);
    println!(r#"cargo::rustc-check-cfg=cfg(openssl_vendored)"#);

    #[allow(
        clippy::unusual_byte_groupings,
        reason = "hex groups follow OpenSSL version field boundaries"
    )]
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

    #[cfg(feature = "ssl-openssl")]
    {
        // Cargo `DEP_*` metadata from openssl-sys, not process host environment.
        #[allow(
            clippy::disallowed_methods,
            clippy::unusual_byte_groupings,
            reason = "build scripts read Cargo DEP_* vars; OpenSSL version hex is grouped by field"
        )]
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
        #[allow(
            clippy::disallowed_methods,
            reason = "build scripts read Cargo DEP_* vars"
        )]
        if let Ok(v) = std::env::var("DEP_OPENSSL_CONF") {
            for conf in v.split(',') {
                println!("cargo:rustc-cfg=osslconf=\"{conf}\"");
            }
        }
        // it's possible for openssl-sys to link against the system openssl under certain conditions,
        // so let the ssl module know to only perform a probe if we're actually vendored
        #[allow(
            clippy::disallowed_methods,
            reason = "build scripts read Cargo DEP_* vars"
        )]
        if std::env::var("DEP_OPENSSL_VENDORED").is_ok_and(|s| s == "1") {
            println!("cargo::rustc-cfg=openssl_vendored")
        }
    }
}

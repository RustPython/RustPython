fn main() {
    let target = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
    let capi_enabled = std::env::var_os("CARGO_FEATURE_CAPI").is_some();

    match target.as_str() {
        "linux" if capi_enabled => {
            println!("cargo:rustc-link-arg-bin=rustpython=-Wl,--export-dynamic");
        }
        "macos" if capi_enabled => {
            println!("cargo:rustc-link-arg-bin=rustpython=-Wl,-export_dynamic");
        }
        "windows" => {
            if capi_enabled {
                println!("cargo:rerun-if-changed=capi-exports.def");
                let exports =
                    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("capi-exports.def");
                let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap();
                match target_env.as_str() {
                    "msvc" => println!(
                        "cargo:rustc-link-arg-bin=rustpython=/DEF:{}",
                        exports.display()
                    ),
                    "gnu" => println!("cargo:rustc-link-arg-bin=rustpython={}", exports.display()),
                    _ => panic!("Unsupported Windows C API linker environment: {target_env}"),
                }
            }
            println!("cargo:rerun-if-changed=logo.ico");
            let mut res = winresource::WindowsResource::new();
            if std::path::Path::new("logo.ico").exists() {
                res.set_icon("logo.ico");
            } else {
                println!("cargo:warning=logo.ico not found, skipping icon embedding");
                return;
            }
            res.compile()
                .map_err(|e| {
                    println!("cargo:warning=Failed to compile Windows resources: {e}");
                })
                .ok();
        }
        _ => {}
    }
}

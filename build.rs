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

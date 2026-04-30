fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();

    if target_os != "windows" {
        println!("cargo:rustc-link-arg-bin=rustpython=-Wl,-export_dynamic");
    }

    if target_os == "windows" {
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
}

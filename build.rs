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
                export_capi_symbols_for_windows_exe();
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

fn export_capi_symbols_for_windows_exe() {
    let exports_def_filename = "rustpython-capi-exports.def";
    println!("cargo:rerun-if-changed={exports_def_filename}");

    let exports = std::fs::read_to_string(exports_def_filename)
        .expect("Failed to read exports definition file");

    for export in exports
        .lines()
        .skip(1)
        .filter(|line| !line.trim().is_empty())
    {
        println!("cargo:rustc-link-arg-bin=rustpython=/INCLUDE:{export}");
    }

    println!("cargo:rustc-link-arg-bin=rustpython=/DEF:{exports_def_filename}",);
}

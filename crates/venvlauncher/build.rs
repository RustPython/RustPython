//! Build script for venvlauncher
//!
//! Sets the Windows subsystem to GUI for venvwlauncher variants.
//! Only MSVC toolchain is supported on Windows (same as CPython).

fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();

    // Only apply on Windows with MSVC toolchain
    if target_os == "windows" && target_env == "msvc" {
        let exe_name = std::env::var("CARGO_BIN_NAME").unwrap_or_default();

        // venvwlauncher and venvwlaunchert should be Windows GUI applications
        // (no console window)
        if exe_name.contains("venvw") {
            println!("cargo:rustc-link-arg=/SUBSYSTEM:WINDOWS");
            println!("cargo:rustc-link-arg=/ENTRY:mainCRTStartup");
        }
    }
}

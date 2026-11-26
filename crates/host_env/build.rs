//! Like CPython's `HAVE_ALTZONE`, it detects the presence of `altzone` in `time.h` at build time.

#![allow(
    clippy::disallowed_methods,
    reason = "build scripts cannot use rustpython-host_env"
)]

use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rustc-check-cfg=cfg(has_altzone)");

    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let target = env::var("TARGET").unwrap_or_default();

    link_wasi_emulated_signal(&target);

    if target_env == "msvc" || target_arch == "wasm32" {
        return;
    }

    let host = env::var("HOST").unwrap_or_default();
    // cc::Build resolves CC_<target>, CC_<target_with_underscores>, TARGET_CC, then CC.
    if host != target && !has_target_c_compiler(&target) {
        return;
    }

    if probe_altzone() {
        println!("cargo:rustc-cfg=has_altzone");
    }
}

/// WASI targets with a vendored `libwasi-emulated-signal.a`.
///
/// `src/signal.rs` declares wasi-libc's `signal`, `raise`, `__SIG_IGN` and
/// `__SIG_ERR` on every WASI target. wasi-libc ships them in a separate
/// library that is neither in `libc.a` nor in Rust's self-contained WASI
/// sysroot, so this crate links its own copy. Linking it here rather than
/// from `.cargo/config.toml` reaches every crate that depends on
/// `rustpython-host_env`: Cargo never reads a dependency's config file.
/// A WASI target missing from this list fails to link with undefined
/// symbols until its copy is vendored.
const WASI_EMULATED_SIGNAL_TARGETS: &[&str] = &["wasm32-wasip1", "wasm32-wasip2"];

/// Link the vendored wasi-libc signal emulation for `target`, if vendored.
///
/// The `static` kind bundles the archive into this crate's rlib, so the
/// final link of a dependent crate needs no search path of its own.
fn link_wasi_emulated_signal(target: &str) {
    if !WASI_EMULATED_SIGNAL_TARGETS.contains(&target) {
        return;
    }
    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let vendor_dir = manifest_dir.join("vendor").join(target);
    println!(
        "cargo:rerun-if-changed={}",
        vendor_dir.join("libwasi-emulated-signal.a").display()
    );
    println!("cargo:rustc-link-search=native={}", vendor_dir.display());
    println!("cargo:rustc-link-lib=static=wasi-emulated-signal");
}

/// Whether any compiler env var that `cc::Build` would consult for the target is set.
fn has_target_c_compiler(target: &str) -> bool {
    let underscored = target.replace(['-', '.'], "_");
    env::var_os(format!("CC_{target}")).is_some()
        || env::var_os(format!("CC_{underscored}")).is_some()
        || env::var_os("TARGET_CC").is_some()
        || env::var_os("CC").is_some()
}

/// Check corresponding to `AC_TRY_COMPILE(... altzone ...)` in CPython's `configure`.
fn probe_altzone() -> bool {
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    let src = out_dir.join("probe_altzone.c");
    let obj = out_dir.join("probe_altzone.o");

    if fs::write(
        &src,
        "#include <time.h>\nint main(void) { return (int)altzone; }\n",
    )
    .is_err()
    {
        return false;
    }

    let Ok(compiler) = cc::Build::new().try_get_compiler() else {
        return false;
    };

    let mut cmd = compiler.to_command();
    if compiler.is_like_msvc() {
        cmd.arg("/c").arg(&src).arg(format!("/Fo{}", obj.display()));
    } else {
        cmd.arg("-c").arg(&src).arg("-o").arg(&obj);
    }

    match cmd.output() {
        Ok(output) => output.status.success(),
        Err(_) => false,
    }
}

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

    if target_env == "msvc" || target_arch == "wasm32" {
        return;
    }

    let host = env::var("HOST").unwrap_or_default();
    let target = env::var("TARGET").unwrap_or_default();
    // cc::Build resolves CC_<target>, CC_<target_with_underscores>, TARGET_CC, then CC.
    if host != target && !has_target_c_compiler(&target) {
        return;
    }

    if probe_altzone() {
        println!("cargo:rustc-cfg=has_altzone");
    }
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

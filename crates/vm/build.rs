#![allow(
    clippy::disallowed_methods,
    reason = "build scripts cannot use rustpython-host_env"
)]

use itertools::Itertools;
use std::{
    env,
    io::{self, prelude::*},
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

fn main() {
    let frozen_libs = if cfg!(feature = "freeze-stdlib") {
        "Lib/*/*.py"
    } else {
        "Lib/python_builtins/*.py"
    };
    for entry in glob::glob(frozen_libs).expect("Lib/ exists?").flatten() {
        let display = entry.display();
        println!("cargo:rerun-if-changed={display}");
    }
    println!("cargo:rerun-if-changed=../../Lib/importlib/_bootstrap.py");

    println!("cargo:rustc-env=RUSTPYTHON_GIT_HASH={}", git_hash());
    println!(
        "cargo:rustc-env=RUSTPYTHON_GIT_TIMESTAMP={}",
        git_timestamp()
    );
    println!("cargo:rustc-env=RUSTPYTHON_GIT_TAG={}", git_tag());
    println!("cargo:rustc-env=RUSTPYTHON_GIT_BRANCH={}", git_branch());
    println!("cargo:rustc-env=RUSTC_VERSION={}", rustc_version());

    let release_level = option_env!("RUSTPYTHON_RELEASE_LEVEL").unwrap_or("alpha");
    println!("cargo:rustc-env=RUSTPYTHON_RELEASE_LEVEL={release_level}");
    let release_serial = option_env!("RUSTPYTHON_RELEASE_SERIAL").unwrap_or("0");
    println!("cargo:rustc-env=RUSTPYTHON_RELEASE_SERIAL={release_serial}");

    println!(
        "cargo:rustc-env=RUSTPYTHON_TARGET_TRIPLE={}",
        env::var("TARGET").unwrap()
    );

    let mut env_path = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    env_path.push("env_vars.rs");
    let mut f = std::fs::File::create(env_path).unwrap();
    write!(
        f,
        "sysvars! {{ {} }}",
        std::env::vars_os().format_with(", ", |(k, v), f| f(&format_args!("{k:?} => {v:?}")))
    )
    .unwrap();
}

fn git_hash() -> String {
    git(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|_| "0000000".into())
}

fn git_timestamp() -> String {
    git(&["log", "-1", "--format=%ct"]).unwrap_or_else(|_| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .to_string()
    })
}

fn git_tag() -> String {
    git(&["describe", "--all", "--always", "--dirty"])
        .unwrap_or_else(|_| "heads/unknown-branch".into())
}

fn git_branch() -> String {
    git(&["name-rev", "--name-only", "HEAD"]).unwrap_or_else(|_| "unknown-branch".into())
}

fn git(args: &[&str]) -> io::Result<String> {
    command("git", args)
}

fn rustc_version() -> String {
    let rustc = env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    command(rustc, &["-V"]).unwrap_or_else(|_| "rustc [unknown]".into())
}

fn command(cmd: impl AsRef<std::ffi::OsStr>, args: &[&str]) -> io::Result<String> {
    Command::new(&cmd).args(args).output().and_then(|output| {
        // TODO: Switch to exit_ok()? when stable.
        if !output.status.success() {
            Err(io::Error::other(format!(
                "command '{}' exited with status {}",
                cmd.as_ref().to_string_lossy(),
                output.status
            )))?
        }

        str::from_utf8(&output.stdout)
            .map(|s| s.trim().to_owned())
            .map_err(io::Error::other)
    })
}

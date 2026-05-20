#![allow(
    clippy::disallowed_methods,
    reason = "build scripts cannot use rustpython-host_env"
)]

use chrono::{Local, prelude::DateTime};
use core::time::Duration;
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

    // = 3.14.0alpha
    python_version(3, 14, 0, "alpha", 0);

    println!("cargo:rustc-env=RUSTPYTHON_GIT_HASH={}", git_hash());
    println!(
        "cargo:rustc-env=RUSTPYTHON_GIT_TIMESTAMP={}",
        git_timestamp()
    );
    println!("cargo:rustc-env=RUSTPYTHON_GIT_TAG={}", git_tag());
    println!("cargo:rustc-env=RUSTPYTHON_GIT_BRANCH={}", git_branch());
    println!(
        "cargo:rustc-env=RUSTPYTHON_GIT_IDENTIFIER={}",
        git_identifier()
    );
    println!("cargo:rustc-env=RUSTPYTHON_BUILD_INFO={}", get_build_info());
    println!("cargo:rustc-env=RUSTC_VERSION={}", rustc_version());

    let release_level = option_env!("RUSTPYTHON_RELEASE_LEVEL").unwrap_or("alpha");
    println!("cargo:rustc-env=RUSTPYTHON_RELEASE_LEVEL={release_level}");
    let release_level_n = release_to_n(release_level);
    println!("cargo:rustc-env=RUSTPYTHON_RELEASE_LEVEL_N={release_level_n}");
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

#[must_use]
fn get_build_info() -> String {
    // See: https://reproducible-builds.org/docs/timestamps/
    let revision = git_hash();
    let separator = if revision.is_empty() { "" } else { ":" };
    let identifier = git_identifier();

    format!(
        "{id}{sep}{revision}, {date:.20}, {time:.9}",
        id = if identifier.is_empty() {
            "default"
        } else {
            &identifier
        },
        sep = separator,
        revision = revision,
        date = get_git_date(),
        time = get_git_time(),
    )
}

fn git_identifier() -> String {
    let tag = git_tag();
    if tag.is_empty() || tag.eq_ignore_ascii_case("undefined") {
        git_branch()
    } else {
        tag
    }
}

fn get_git_timestamp_datetime() -> DateTime<Local> {
    let timestamp = git_timestamp().parse::<u64>().unwrap_or_default();
    let datetime = UNIX_EPOCH + Duration::from_secs(timestamp);
    datetime.into()
}

#[must_use]
fn get_git_date() -> String {
    let datetime = get_git_timestamp_datetime();

    datetime.format("%b %e %Y").to_string()
}

#[must_use]
fn get_git_time() -> String {
    let datetime = get_git_timestamp_datetime();

    datetime.format("%H:%M:%S").to_string()
}

fn rustc_version() -> String {
    let rustc = env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    command(rustc, &["-V"]).unwrap_or_else(|_| "rustc [unknown]".into())
}

fn python_version(major: usize, minor: usize, micro: usize, release: &str, serial: usize) {
    println!("cargo:rustc-env=MAJOR_CPY={major}");
    println!("cargo:rustc-env=MINOR_CPY={minor}");
    println!("cargo:rustc-env=MICRO_CPY={micro}");
    println!("cargo:rustc-env=RELEASE_LEVEL_CPY={release}");
    println!(
        "cargo:rustc-env=RELEASE_LEVEL_N_CPY={}",
        release_to_n(release)
    );
    println!("cargo:rustc-env=SERIAL_CPY={serial}");

    println!("cargo:rustc-env=WINVER_CPY={major}.{minor}",);

    let cpy_version = format!("{major}.{minor}.{micro}.{release}");

    let (left, right) = get_version(&cpy_version);
    println!("cargo:rustc-env=RUSTPYTHON_VERSION_LEFT={left}");
    println!("cargo:rustc-env=RUSTPYTHON_VERSION_RIGHT={right}");
}

#[must_use]
fn get_version(cpy_version: &str) -> (String, String) {
    // Windows: include MSC v. for compatibility with ctypes.util.find_library
    // MSC v.1929 = VS 2019, version 14+ makes find_msvcrt() return None
    let msc_info = cfg_select! {
        windows => {{
            // Include both RustPython identifier and MSC v. for compatibility
            if cfg!(target_pointer_width = "64") {
                " MSC v.1929 64 bit (AMD64)"
            } else {
                " MSC v.1929 32 bit (Intel)"
            }
        }},
        _ => "",
    };

    // `left` and `right` are split by \n like PyPy. Passing a string with a newline to rustc
    // truncates everything from the newline onward, so we have to manually combine them later.
    let left = format!("{:.80} ({:.80})", cpy_version, get_build_info());
    let right = format!(
        "[RustPython {} with {:.80}{}]",
        env!("CARGO_PKG_VERSION"),
        rustc_version(),
        msc_info,
    );
    (left, right)
}

fn release_to_n(release: &str) -> usize {
    match release {
        "alpha" => 0xA,
        "beta" => 0xB,
        "candidate" => 0xC,
        "final" => 0xD,
        _ => panic!("`release` must be one of: 'alpha', 'beta', 'candidate', 'final'"),
    }
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

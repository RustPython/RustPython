use itertools::Itertools;
use std::env;
use std::io::prelude::*;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rustc-env=RUSTPYTHON_GIT_HASH={}", git_hash());
    println!(
        "cargo:rustc-env=RUSTPYTHON_GIT_TIMESTAMP={}",
        git_timestamp()
    );
    println!("cargo:rustc-env=RUSTPYTHON_GIT_TAG={}", git_tag());
    println!("cargo:rustc-env=RUSTPYTHON_GIT_BRANCH={}", git_branch());
    println!(
        "cargo:rustc-env=RUSTC_VERSION={}",
        rustc_version::version().unwrap()
    );

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
        std::env::vars_os().format_with(", ", |(k, v), f| f(&format_args!("{:?} => {:?}", k, v)))
    )
    .unwrap();
}

fn git_hash() -> String {
    git(&["rev-parse", "--short", "HEAD"])
}

fn git_timestamp() -> String {
    git(&["log", "-1", "--format=%ct"])
}

fn git_tag() -> String {
    git(&["describe", "--all", "--always", "--dirty"])
}

fn git_branch() -> String {
    git(&["name-rev", "--name-only", "HEAD"])
}

fn git(args: &[&str]) -> String {
    command("git", args)
}

fn command(cmd: &str, args: &[&str]) -> String {
    match Command::new(cmd).args(args).output() {
        Ok(output) => match String::from_utf8(output.stdout) {
            Ok(s) => s,
            Err(err) => format!("(output error: {})", err),
        },
        Err(err) => format!("(command error: {})", err),
    }
}

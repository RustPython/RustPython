use std::process::Command;

fn main() {
    println!("cargo:rustc-env=RUSTPYTHON_GIT_HASH={}", git_hash());
    println!(
        "cargo:rustc-env=RUSTPYTHON_GIT_TIMESTAMP={}",
        git_timestamp()
    );
    println!("cargo:rustc-env=RUSTPYTHON_GIT_BRANCH={}", git_branch());
}

fn git_hash() -> String {
    git(&["rev-parse", "HEAD"])
}

fn git_timestamp() -> String {
    git(&["log", "-1", "--format=%cd"])
}

fn git_branch() -> String {
    git(&["rev-parse", "--abbrev-ref", "HEAD"])
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

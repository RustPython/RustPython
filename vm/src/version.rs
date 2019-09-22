/* Several function to retrieve version information.
 */

pub fn get_version() -> String {
    format!(
        "{} {:?} {}",
        get_version_number(),
        get_build_info(),
        get_compiler()
    )
}

pub fn get_version_number() -> String {
    format!(
        "{}.{}.{}{}",
        env!("CARGO_PKG_VERSION_MAJOR"),
        env!("CARGO_PKG_VERSION_MINOR"),
        env!("CARGO_PKG_VERSION_PATCH"),
        option_env!("CARGO_PKG_VERSION_PRE").unwrap_or("")
    )
}

pub fn get_compiler() -> String {
    let rustc_version = rustc_version_runtime::version_meta();
    format!("rustc {}", rustc_version.semver)
}

pub fn get_build_info() -> (String, String) {
    let git_hash = get_git_revision();
    // See: https://reproducible-builds.org/docs/timestamps/
    let git_timestamp = option_env!("RUSTPYTHON_GIT_TIMESTAMP")
        .unwrap_or("")
        .to_string();
    (git_hash, git_timestamp)
}

pub fn get_git_revision() -> String {
    option_env!("RUSTPYTHON_GIT_HASH").unwrap_or("").to_string()
}

pub fn get_git_tag() -> String {
    option_env!("RUSTPYTHON_GIT_TAG").unwrap_or("").to_string()
}

pub fn get_git_branch() -> String {
    option_env!("RUSTPYTHON_GIT_BRANCH")
        .unwrap_or("")
        .to_string()
}

/* Several function to retrieve version information.
 */

const MAJOR: usize = 3;
const MINOR: usize = 5;
const MICRO: usize = 0;
const RELEASELEVEL: &str = "alpha";
const SERIAL: usize = 0;

#[pystruct_sequence(name = "version_info")]
#[derive(Default, Debug)]
pub struct VersionInfo {
    major: usize,
    minor: usize,
    micro: usize,
    releaselevel: &'static str,
    serial: usize,
}

pub fn get_version() -> String {
    format!(
        "{} {:?} {}",
        get_version_number(),
        get_build_info(),
        get_compiler()
    )
}

pub fn get_version_info() -> VersionInfo {
    VersionInfo {
        major: MAJOR,
        minor: MINOR,
        micro: MICRO,
        releaselevel: RELEASELEVEL,
        serial: SERIAL,
    }
}

pub fn get_version_number() -> String {
    format!("{}.{}.{}{}", MAJOR, MINOR, MICRO, RELEASELEVEL)
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

pub fn get_git_identifier() -> String {
    let git_tag = get_git_tag();
    let git_branch = get_git_branch();

    if git_tag.is_empty() || git_tag == "undefined" {
        git_branch
    } else {
        git_tag
    }
}

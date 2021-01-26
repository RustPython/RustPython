/* Several function to retrieve version information.
 */

use crate::pyobject::PyStructSequence;
use chrono::prelude::DateTime;
use chrono::Local;
use std::time::{Duration, UNIX_EPOCH};

const MAJOR: usize = 3;
const MINOR: usize = 9;
const MICRO: usize = 0;
const RELEASELEVEL: &str = "alpha";
const RELEASELEVEL_N: usize = 0xA;
const SERIAL: usize = 0;

pub const VERSION_HEX: usize =
    (MAJOR << 24) | (MINOR << 16) | (MICRO << 8) | (RELEASELEVEL_N << 4) | SERIAL;

#[pyclass(module = "sys", name = "version_info")]
#[derive(Default, Debug, PyStructSequence)]
pub struct VersionInfo {
    major: usize,
    minor: usize,
    micro: usize,
    releaselevel: &'static str,
    serial: usize,
}

pub fn get_version() -> String {
    format!(
        "{:.80} ({:.80}) \n[{:.80}]",
        get_version_number(),
        get_build_info(),
        get_compiler()
    )
}

#[pyimpl(with(PyStructSequence))]
impl VersionInfo {
    pub const VERSION: VersionInfo = VersionInfo {
        major: MAJOR,
        minor: MINOR,
        micro: MICRO,
        releaselevel: RELEASELEVEL,
        serial: SERIAL,
    };
    #[pyslot]
    fn tp_new(
        _cls: crate::builtins::pytype::PyTypeRef,
        _args: crate::function::FuncArgs,
        vm: &crate::VirtualMachine,
    ) -> crate::pyobject::PyResult {
        Err(vm.new_type_error("cannot create 'sys.version_info' instances".to_owned()))
    }
}

pub fn get_version_number() -> String {
    format!("{}.{}.{}{}", MAJOR, MINOR, MICRO, RELEASELEVEL)
}

pub fn get_compiler() -> String {
    let rustc_version = rustc_version_runtime::version_meta();
    format!("rustc {}", rustc_version.semver)
}

pub fn get_build_info() -> String {
    // See: https://reproducible-builds.org/docs/timestamps/
    let git_revision = get_git_revision();
    let separator = if git_revision.is_empty() { "" } else { ":" };

    let git_identifier = get_git_identifier();

    format!(
        "{id}{sep}{revision}, {date:.20}, {time:.9}",
        id = if git_identifier.is_empty() {
            "default".to_owned()
        } else {
            git_identifier
        },
        sep = separator,
        revision = git_revision,
        date = get_git_date(),
        time = get_git_time(),
    )
}

pub fn get_git_revision() -> String {
    option_env!("RUSTPYTHON_GIT_HASH").unwrap_or("").to_owned()
}

pub fn get_git_tag() -> String {
    option_env!("RUSTPYTHON_GIT_TAG").unwrap_or("").to_owned()
}

pub fn get_git_branch() -> String {
    option_env!("RUSTPYTHON_GIT_BRANCH")
        .unwrap_or("")
        .to_owned()
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

fn get_git_timestamp_datetime() -> DateTime<Local> {
    let timestamp = option_env!("RUSTPYTHON_GIT_TIMESTAMP")
        .unwrap_or("")
        .to_owned();
    let timestamp = timestamp.parse::<u64>().unwrap_or(0);

    let datetime = UNIX_EPOCH + Duration::from_secs(timestamp);

    datetime.into()
}

pub fn get_git_date() -> String {
    let datetime = get_git_timestamp_datetime();

    datetime.format("%b %e %Y").to_string()
}

pub fn get_git_time() -> String {
    let datetime = get_git_timestamp_datetime();

    datetime.format("%H:%M:%S").to_string()
}

pub fn get_git_datetime() -> String {
    let date = get_git_date();
    let time = get_git_time();

    format!("{} {}", date, time)
}

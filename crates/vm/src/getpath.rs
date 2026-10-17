//! Path configuration for RustPython (ref: Modules/getpath.py)
//!
//! This module implements Python path calculation logic following getpath.py.
//! It uses landmark-based search to locate prefix, exec_prefix, and stdlib directories.
//!
//! The main entry point is `init_path_config()` which computes Paths from Settings.

use crate::vm::{Paths, Settings};
use std::env;
use std::path::{Path, PathBuf};

// Platform-specific landmarks (ref: getpath.py PLATFORM CONSTANTS)

#[cfg(not(windows))]
mod platform {
    use crate::version;

    pub(super) const BUILDDIR_TXT: &str = "pybuilddir.txt";
    pub(super) const BUILD_LANDMARK: &str = "Modules/Setup.local";
    pub(super) const VENV_LANDMARK: &str = "pyvenv.cfg";
    pub(super) const BUILDSTDLIB_LANDMARK: &str = "Lib/os.py";

    pub(super) fn stdlib_subdir() -> String {
        format!("lib/python{}.{}", version::MAJOR, version::MINOR)
    }

    pub(super) fn stdlib_landmarks() -> [String; 2] {
        let subdir = stdlib_subdir();
        [format!("{subdir}/os.py"), format!("{subdir}/os.pyc")]
    }

    pub(super) fn platstdlib_landmark() -> String {
        format!(
            "lib/python{}.{}/lib-dynload",
            version::MAJOR,
            version::MINOR
        )
    }

    pub(super) fn zip_landmark() -> String {
        format!("lib/python{}{}.zip", version::MAJOR, version::MINOR)
    }
}

#[cfg(windows)]
mod platform {
    use crate::version;

    pub(super) const BUILDDIR_TXT: &str = "pybuilddir.txt";
    pub(super) const BUILD_LANDMARK: &str = "Modules\\Setup.local";
    pub(super) const VENV_LANDMARK: &str = "pyvenv.cfg";
    pub(super) const BUILDSTDLIB_LANDMARK: &str = "Lib\\os.py";
    pub(super) const STDLIB_SUBDIR: &str = "Lib";

    pub(super) fn stdlib_landmarks() -> [String; 2] {
        ["Lib\\os.py".into(), "Lib\\os.pyc".into()]
    }

    pub(super) fn platstdlib_landmark() -> String {
        "DLLs".into()
    }

    pub(super) fn zip_landmark() -> String {
        format!("python{}{}.zip", version::MAJOR, version::MINOR)
    }
}

// Helper functions (ref: getpath.py HELPER FUNCTIONS)

/// Search upward from a directory for landmark files/directories
/// Returns the directory where a landmark was found
fn search_up<P, F>(start: P, landmarks: &[&str], test: F) -> Option<PathBuf>
where
    P: AsRef<Path>,
    F: Fn(&Path) -> bool,
{
    let mut current = start.as_ref().to_path_buf();
    loop {
        for landmark in landmarks {
            let path = current.join(landmark);
            if test(&path) {
                return Some(current);
            }
        }
        if !current.pop() {
            return None;
        }
    }
}

/// Search upward for a file landmark
fn search_up_file<P: AsRef<Path>>(start: P, landmarks: &[&str]) -> Option<PathBuf> {
    search_up(start, landmarks, |p| p.is_file())
}

/// Search upward for a directory landmark
#[cfg(not(windows))]
fn search_up_dir<P: AsRef<Path>>(start: P, landmarks: &[&str]) -> Option<PathBuf> {
    search_up(start, landmarks, |p| p.is_dir())
}

// Path computation functions

/// Compute path configuration from Settings
///
/// This function should be called before interpreter initialization.
/// It returns a Paths struct with all computed path values.
pub fn init_path_config(settings: &Settings) -> Paths {
    let mut paths = Paths::default();

    // Step 0: Get executable path
    let executable = get_executable_path();
    let real_executable = executable
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();

    // Step 1: Check for PYTHONEXECUTABLE / __PYVENV_LAUNCHER__
    // When set, these are used as sys.executable and when searching for venvs.
    // The argv0 path is kept as the base executable for prefix calculation.
    let exe_dir = if let Some(override_exe) = env_executable_override() {
        paths.executable.clone_from(&override_exe);
        paths.base_executable = real_executable;
        PathBuf::from(&override_exe).parent().map(PathBuf::from)
    } else {
        paths.executable = real_executable;
        executable
            .as_ref()
            .and_then(|p| p.parent().map(PathBuf::from))
    };

    // A readable pyvenv.cfg establishes the environment even without a home key.
    let (venv_prefix, configured_home) = detect_venv(exe_dir.as_ref());
    // Explicit PYTHONHOME takes precedence over the configuration's home key,
    // including its influence on _base_executable.
    let explicit_home = settings.home.as_deref().filter(|home| !home.is_empty());
    let home_dir = if explicit_home.is_some() {
        None
    } else {
        configured_home
    };
    let search_dir = home_dir.clone().or(exe_dir);
    #[cfg(unix)]
    let search_dir = if venv_prefix.is_some()
        && home_dir.is_none()
        && explicit_home.is_none()
        && paths.base_executable.is_empty()
    {
        // A no-home environment uses the target installation for landmarks,
        // while _base_executable retains the invoked executable spelling.
        executable
            .as_deref()
            .and_then(resolve_executable_symlinks)
            .and_then(|resolved| resolved.parent().map(PathBuf::from))
            .or(search_dir)
    } else {
        search_dir
    };
    let build_prefix = explicit_home
        .is_none()
        .then(|| detect_build_directory(search_dir.as_ref()))
        .flatten();
    let home_prefixes = explicit_home.map(|home| {
        home.split_once(if cfg!(windows) { ';' } else { ':' })
            .unwrap_or((home, home))
    });
    paths.base_prefix = home_prefixes
        .map(|(prefix, _)| prefix)
        .filter(|prefix| !prefix.is_empty())
        .map_or_else(
            || calculate_prefix(search_dir.as_ref(), build_prefix.as_ref()),
            str::to_owned,
        );
    paths.base_exec_prefix = home_prefixes
        .map(|(_, exec_prefix)| exec_prefix)
        .filter(|exec_prefix| !exec_prefix.is_empty())
        .map_or_else(
            || calculate_exec_prefix(search_dir.as_ref(), &paths.base_prefix),
            str::to_owned,
        );
    // The venv overlays public prefixes, not the base library search paths.
    if let Some(venv) = venv_prefix {
        paths.prefix = venv.to_string_lossy().into_owned();
        paths.exec_prefix.clone_from(&paths.prefix);
    } else {
        paths.prefix.clone_from(&paths.base_prefix);
        paths.exec_prefix.clone_from(&paths.base_exec_prefix);
    }
    if paths.base_executable.is_empty() {
        paths.base_executable = calculate_base_executable(executable.as_ref(), home_dir.as_ref());
    }
    paths.module_search_paths =
        build_module_search_paths(settings, &paths.base_prefix, &paths.base_exec_prefix);
    paths.stdlib_dir = calculate_stdlib_dir(&paths.base_prefix);

    paths
}

/// Identify an uninstalled RustPython source library independently of the
/// executable location (Cargo permits targets outside the source tree).
#[cfg(windows)]
pub(crate) fn source_directory(paths: &Paths) -> Option<&Path> {
    let library = Path::new(paths.stdlib_dir.as_deref()?);
    let source = library.parent()?;
    (library.file_name()? == "Lib" && source.join("crates/vm/Cargo.toml").is_file())
        .then_some(source)
}

/// Get default prefix value used when landmark search fails.
///
/// A compile-time `RUSTPYTHON_PREFIX` always wins. Otherwise POSIX uses the
/// conventional install prefix, while Windows has no meaningful compile-time
/// prefix and falls back to the executable's directory (ref: getpath.py).
///
/// A bare drive root must never be returned on Windows: pip walks up from
/// `<prefix>/Lib/site-packages` and would otherwise probe the drive root for
/// writability, which fails for standard users (see issue #8246).
fn default_prefix(exe_dir: Option<&PathBuf>) -> String {
    if let Some(prefix) = std::option_env!("RUSTPYTHON_PREFIX") {
        return prefix.to_owned();
    }

    if cfg!(windows) {
        if let Some(dir) = exe_dir {
            return dir.to_string_lossy().into_owned();
        }
        // Executable directory is unknown; use a valid absolute root as a last
        // resort rather than a drive-relative bare "C:".
        "C:\\".to_owned()
    } else {
        "/usr/local".to_owned()
    }
}

/// Detect virtual environment by looking for pyvenv.cfg
/// Returns (venv_prefix, home_dir from pyvenv.cfg)
fn detect_venv(exe_dir: Option<&PathBuf>) -> (Option<PathBuf>, Option<PathBuf>) {
    // Try exe_dir/../pyvenv.cfg first (standard venv layout: venv/bin/python)
    if let Some(dir) = exe_dir
        && let Some(venv_dir) = dir.parent()
    {
        let cfg = venv_dir.join(platform::VENV_LANDMARK);
        if let Some(home) = parse_pyvenv_home(&cfg) {
            return (Some(venv_dir.to_path_buf()), home.map(PathBuf::from));
        }
    }

    // Try exe_dir/pyvenv.cfg (alternative layout)
    if let Some(dir) = exe_dir {
        let cfg = dir.join(platform::VENV_LANDMARK);
        if let Some(home) = parse_pyvenv_home(&cfg) {
            return (Some(dir.clone()), home.map(PathBuf::from));
        }
    }

    (None, None)
}

/// Detect if running from a build directory
fn detect_build_directory(exe_dir: Option<&PathBuf>) -> Option<PathBuf> {
    let dir = exe_dir?;

    // Check for pybuilddir.txt (indicates build directory)
    if dir.join(platform::BUILDDIR_TXT).exists() {
        return Some(dir.clone());
    }

    // Check for Modules/Setup.local (build landmark)
    if dir.join(platform::BUILD_LANDMARK).exists() {
        return Some(dir.clone());
    }

    // Search up for Lib/os.py (build stdlib landmark)
    search_up_file(dir, &[platform::BUILDSTDLIB_LANDMARK])
}

/// Calculate prefix by searching for landmarks
fn calculate_prefix(exe_dir: Option<&PathBuf>, build_prefix: Option<&PathBuf>) -> String {
    // 1. If build directory detected, use it
    if let Some(bp) = build_prefix {
        return bp.to_string_lossy().into_owned();
    }

    if let Some(dir) = exe_dir {
        // 2. Search for ZIP landmark
        let zip = platform::zip_landmark();
        if let Some(prefix) = search_up_file(dir, &[&zip]) {
            return prefix.to_string_lossy().into_owned();
        }

        // 3. Search for stdlib landmarks (os.py)
        let landmarks = platform::stdlib_landmarks();
        let refs: Vec<&str> = landmarks.iter().map(|s| s.as_str()).collect();
        if let Some(prefix) = search_up_file(dir, &refs) {
            return prefix.to_string_lossy().into_owned();
        }
    }

    // 4. Fallback to default
    default_prefix(exe_dir)
}

/// Calculate exec_prefix
fn calculate_exec_prefix(exe_dir: Option<&PathBuf>, prefix: &str) -> String {
    #[cfg(windows)]
    {
        // Windows: exec_prefix == prefix
        let _ = exe_dir; // silence unused warning
        prefix.to_owned()
    }

    #[cfg(not(windows))]
    {
        // Prefer the platform library under prefix before executable ancestors.
        let landmark = platform::platstdlib_landmark();
        if Path::new(prefix).join(&landmark).is_dir() {
            return prefix.to_owned();
        }
        if let Some(dir) = exe_dir
            && let Some(exec_prefix) = search_up_dir(dir, &[&landmark])
        {
            return exec_prefix.to_string_lossy().into_owned();
        }
        // Fallback: same as prefix
        prefix.to_owned()
    }
}

/// Resolve final-component links without resolving directory aliases (ref: getpath.c).
#[cfg(unix)]
fn resolve_executable_symlinks(executable: &Path) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;

    let mut path = executable.to_path_buf();
    // CPython rejects 40 successful hops, matching the Linux 4.2 kernel limit.
    for _ in 0..40 {
        let Ok(target) = std::fs::read_link(&path) else {
            return Some(path);
        };
        if target.is_absolute() {
            path = target;
            continue;
        }

        // Match getpath.c's raw dirname join, then normalize lexically without
        // resolving directory symlinks or losing a double-leading-slash root.
        let bytes = path.as_os_str().as_bytes();
        let end = bytes.iter().rposition(|&b| b == b'/');
        let dirname = &bytes[..end.unwrap_or(bytes.len())];
        let mut joined = PathBuf::from(std::ffi::OsStr::from_bytes(dirname));
        // Filesystem text uses UTF-8/surrogateescape; count characters, not bytes.
        let multiple_chars =
            core::str::from_utf8(dirname).map_or(dirname.len() > 1, |s| s.chars().nth(1).is_some());
        if multiple_chars && !dirname.ends_with(b"/") {
            joined.as_mut_os_string().push("/");
        }
        joined.as_mut_os_string().push(target.as_os_str());
        let bytes = joined.as_os_str().as_bytes();
        let double_root = bytes.starts_with(b"//") && !bytes.starts_with(b"///");
        path.clear();
        for component in joined.components() {
            if component == std::path::Component::ParentDir
                && !path.as_os_str().is_empty()
                && !path.ends_with("..")
            {
                path.pop();
            } else if component != std::path::Component::CurDir {
                path.push(component.as_os_str());
            }
        }
        if double_root {
            path = Path::new("//").join(path.strip_prefix("/").ok()?);
        }
    }
    None
}

/// Calculate base_executable
fn calculate_base_executable(executable: Option<&PathBuf>, home_dir: Option<&PathBuf>) -> String {
    if let (Some(exe), Some(home)) = (executable, home_dir)
        && let Some(exe_name) = exe.file_name()
    {
        // Like getpath.py, resolve the executable's final links once, without
        // following creator metadata or another environment's configuration.
        #[cfg(unix)]
        if let Some(resolved) = resolve_executable_symlinks(exe)
            && resolved.as_os_str() != exe.as_os_str()
        {
            return resolved.to_string_lossy().into_owned();
        }

        let base = home.join(exe_name);
        #[cfg(not(windows))]
        if !base.is_file() {
            for name in [
                format!("python{}", crate::version::MAJOR),
                format!("python{}.{}", crate::version::MAJOR, crate::version::MINOR),
            ] {
                let candidate = home.join(name);
                if candidate.is_file() {
                    return candidate.to_string_lossy().into_owned();
                }
            }
        }
        return base.to_string_lossy().into_owned();
    }

    // Otherwise, base_executable == executable
    executable
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Calculate stdlib_dir (sys._stdlib_dir)
/// Returns None if the stdlib directory doesn't exist
fn calculate_stdlib_dir(prefix: &str) -> Option<String> {
    let stdlib_dir = Path::new(prefix).join(cfg_select! {
        windows => platform::STDLIB_SUBDIR,
        _ => platform::stdlib_subdir(),
    });

    if stdlib_dir.is_dir() {
        Some(stdlib_dir.to_string_lossy().into_owned())
    } else {
        None
    }
}

/// Add compiled-in library paths after environment entries and the default ZIP.
/// Preserve an embedding hook's custom search order by appending instead when
/// the calculated paths have already been replaced or modified.
pub fn add_build_time_paths(settings: &Settings, paths: &mut Paths, extra: Vec<String>) {
    let calculated =
        build_module_search_paths(settings, &paths.base_prefix, &paths.base_exec_prefix);
    let index = if paths.module_search_paths == calculated {
        settings.path_list.len() + 1
    } else {
        paths.module_search_paths.len()
    };
    paths.module_search_paths.splice(index..index, extra);
}

/// Build the complete module_search_paths (sys.path)
fn build_module_search_paths(settings: &Settings, prefix: &str, exec_prefix: &str) -> Vec<String> {
    let mut paths = Vec::new();

    // 1. PYTHONPATH/RUSTPYTHONPATH from settings
    paths.extend(settings.path_list.iter().cloned());

    // 2. ZIP file path
    let zip_path = PathBuf::from(prefix).join(platform::zip_landmark());
    paths.push(zip_path.to_string_lossy().into_owned());

    // 3. stdlib and platstdlib directories
    #[cfg(not(windows))]
    {
        // POSIX: stdlib first, then lib-dynload
        let stdlib_dir = PathBuf::from(prefix).join(platform::stdlib_subdir());
        paths.push(stdlib_dir.to_string_lossy().into_owned());

        let platstdlib = PathBuf::from(exec_prefix).join(platform::platstdlib_landmark());
        paths.push(platstdlib.to_string_lossy().into_owned());
    }

    #[cfg(windows)]
    {
        // Windows: DLLs first, then Lib
        let platstdlib = PathBuf::from(exec_prefix).join(platform::platstdlib_landmark());
        paths.push(platstdlib.to_string_lossy().into_owned());

        let stdlib_dir = PathBuf::from(prefix).join(platform::STDLIB_SUBDIR);
        paths.push(stdlib_dir.to_string_lossy().into_owned());
    }

    paths
}

/// `PYTHONEXECUTABLE` takes precedence over `__PYVENV_LAUNCHER__`.
/// An empty value is ignored.
fn env_executable_override() -> Option<String> {
    for name in ["PYTHONEXECUTABLE", "__PYVENV_LAUNCHER__"] {
        if let Ok(value) = crate::host_env::os::var(name)
            && !value.is_empty()
        {
            return Some(value);
        }
    }
    None
}

/// Get the current executable path
fn get_executable_path() -> Option<PathBuf> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let exec_arg = env::args_os().next()?;
        crate::host_env::fs::which(exec_arg)
    }
    #[cfg(target_arch = "wasm32")]
    {
        let exec_arg = env::args().next()?;
        Some(PathBuf::from(exec_arg))
    }
}

/// Parse pyvenv.cfg and extract the 'home' key value
// Outer None means unreadable; Some(None) is a valid configuration without home.
fn parse_pyvenv_home(pyvenv_cfg: &Path) -> Option<Option<String>> {
    #[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
    let content = crate::host_env::fs::read_to_string(pyvenv_cfg).ok()?;
    #[cfg(all(target_arch = "wasm32", not(target_os = "wasi")))]
    let content = std::fs::read_to_string(pyvenv_cfg).ok()?;

    for line in content.lines() {
        if let Some((key, value)) = line.split_once('=')
            && key.trim().eq_ignore_ascii_case("home")
        {
            return Some(Some(value.trim().to_string()));
        }
    }

    Some(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_time_paths_follow_configured_paths_and_zip() {
        for configured in [vec![], vec!["first".to_owned(), "second".to_owned()]] {
            let settings = Settings {
                path_list: configured,
                ..Settings::default()
            };
            let calculated = build_module_search_paths(&settings, "prefix", "exec-prefix");
            let mut paths = Paths {
                base_prefix: "prefix".to_owned(),
                base_exec_prefix: "exec-prefix".to_owned(),
                module_search_paths: calculated.clone(),
                ..Paths::default()
            };
            let extra = vec!["build-first".to_owned(), "build-second".to_owned()];
            add_build_time_paths(&settings, &mut paths, extra.clone());
            let boundary = settings.path_list.len() + 1;
            let mut expected = calculated[..boundary].to_vec();
            expected.extend(extra);
            expected.extend_from_slice(&calculated[boundary..]);
            assert_eq!(paths.module_search_paths, expected);
        }
    }

    #[test]
    fn build_time_paths_preserve_custom_search_order() {
        let settings = Settings {
            path_list: vec!["configured".to_owned()],
            ..Settings::default()
        };
        let mut modified = build_module_search_paths(&settings, "prefix", "exec-prefix");
        modified.push("custom".to_owned());
        for custom in [vec![], modified] {
            let mut paths = Paths {
                base_prefix: "prefix".to_owned(),
                base_exec_prefix: "exec-prefix".to_owned(),
                module_search_paths: custom.clone(),
                ..Paths::default()
            };
            add_build_time_paths(&settings, &mut paths, vec!["build".to_owned()]);
            let mut expected = custom;
            expected.push("build".to_owned());
            assert_eq!(paths.module_search_paths, expected);
        }
    }

    #[test]
    fn init_path_config_basic() {
        let settings = Settings::default();
        let paths = init_path_config(&settings);
        // Just verify it doesn't panic and returns valid paths
        assert!(!paths.prefix.is_empty());
    }

    #[test]
    fn search_up() {
        // Test with a path that doesn't have any landmarks
        let result = search_up_file(
            crate::host_env::os::temp_dir(),
            &["nonexistent_landmark_xyz"],
        );
        assert!(result.is_none());
    }

    #[test]
    fn default_prefix_basic() {
        let prefix = default_prefix(None);
        assert!(!prefix.is_empty());
    }
}

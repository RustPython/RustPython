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

    pub const BUILDDIR_TXT: &str = "pybuilddir.txt";
    pub const BUILD_LANDMARK: &str = "Modules/Setup.local";
    pub const VENV_LANDMARK: &str = "pyvenv.cfg";
    pub const BUILDSTDLIB_LANDMARK: &str = "Lib/os.py";

    pub fn stdlib_subdir() -> String {
        format!("lib/python{}.{}", version::MAJOR, version::MINOR)
    }

    pub fn stdlib_landmarks() -> [String; 2] {
        let subdir = stdlib_subdir();
        [format!("{}/os.py", subdir), format!("{}/os.pyc", subdir)]
    }

    pub fn platstdlib_landmark() -> String {
        format!(
            "lib/python{}.{}/lib-dynload",
            version::MAJOR,
            version::MINOR
        )
    }

    pub fn zip_landmark() -> String {
        format!("lib/python{}{}.zip", version::MAJOR, version::MINOR)
    }
}

#[cfg(windows)]
mod platform {
    use crate::version;

    pub const BUILDDIR_TXT: &str = "pybuilddir.txt";
    pub const BUILD_LANDMARK: &str = "Modules\\Setup.local";
    pub const VENV_LANDMARK: &str = "pyvenv.cfg";
    pub const BUILDSTDLIB_LANDMARK: &str = "Lib\\os.py";
    pub const STDLIB_SUBDIR: &str = "Lib";

    pub fn stdlib_landmarks() -> [String; 2] {
        ["Lib\\os.py".into(), "Lib\\os.pyc".into()]
    }

    pub fn platstdlib_landmark() -> String {
        "DLLs".into()
    }

    pub fn zip_landmark() -> String {
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
    paths.executable = executable
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();

    let exe_dir = executable
        .as_ref()
        .and_then(|p| p.parent().map(PathBuf::from));

    // Step 1: Check for __PYVENV_LAUNCHER__ environment variable
    if let Ok(launcher) = env::var("__PYVENV_LAUNCHER__") {
        paths.base_executable = launcher;
    }

    // Step 2: Check for venv (pyvenv.cfg) and get 'home'
    let (venv_prefix, home_dir) = detect_venv(&exe_dir);
    let search_dir = home_dir.clone().or(exe_dir.clone());

    // Step 3: Check for build directory
    let build_prefix = detect_build_directory(&search_dir);

    // Step 4: Calculate prefix via landmark search
    // When in venv, search_dir is home_dir, so this gives us the base Python's prefix
    let calculated_prefix = calculate_prefix(&search_dir, &build_prefix);

    // Step 5: Set prefix and base_prefix
    if venv_prefix.is_some() {
        // In venv: prefix = venv directory, base_prefix = original Python's prefix
        paths.prefix = venv_prefix
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| calculated_prefix.clone());
        paths.base_prefix = calculated_prefix;
    } else {
        // Not in venv: prefix == base_prefix
        paths.prefix = calculated_prefix.clone();
        paths.base_prefix = calculated_prefix;
    }

    // Step 6: Calculate exec_prefix
    paths.exec_prefix = if venv_prefix.is_some() {
        // In venv: exec_prefix = prefix (venv directory)
        paths.prefix.clone()
    } else {
        calculate_exec_prefix(&search_dir, &paths.prefix)
    };
    paths.base_exec_prefix = paths.base_prefix.clone();

    // Step 7: Calculate base_executable (if not already set by __PYVENV_LAUNCHER__)
    if paths.base_executable.is_empty() {
        paths.base_executable = calculate_base_executable(executable.as_ref(), &home_dir);
    }

    // Step 8: Build module_search_paths
    paths.module_search_paths =
        build_module_search_paths(settings, &paths.prefix, &paths.exec_prefix);

    paths
}

/// Get default prefix value
fn default_prefix() -> String {
    std::option_env!("RUSTPYTHON_PREFIX")
        .map(String::from)
        .unwrap_or_else(|| {
            if cfg!(windows) {
                "C:".to_owned()
            } else {
                "/usr/local".to_owned()
            }
        })
}

/// Detect virtual environment by looking for pyvenv.cfg
/// Returns (venv_prefix, home_dir from pyvenv.cfg)
fn detect_venv(exe_dir: &Option<PathBuf>) -> (Option<PathBuf>, Option<PathBuf>) {
    // Try exe_dir/../pyvenv.cfg first (standard venv layout: venv/bin/python)
    if let Some(dir) = exe_dir
        && let Some(venv_dir) = dir.parent()
    {
        let cfg = venv_dir.join(platform::VENV_LANDMARK);
        if cfg.exists()
            && let Some(home) = parse_pyvenv_home(&cfg)
        {
            return (Some(venv_dir.to_path_buf()), Some(PathBuf::from(home)));
        }
    }

    // Try exe_dir/pyvenv.cfg (alternative layout)
    if let Some(dir) = exe_dir {
        let cfg = dir.join(platform::VENV_LANDMARK);
        if cfg.exists()
            && let Some(home) = parse_pyvenv_home(&cfg)
        {
            return (Some(dir.clone()), Some(PathBuf::from(home)));
        }
    }

    (None, None)
}

/// Detect if running from a build directory
fn detect_build_directory(exe_dir: &Option<PathBuf>) -> Option<PathBuf> {
    let dir = exe_dir.as_ref()?;

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
fn calculate_prefix(exe_dir: &Option<PathBuf>, build_prefix: &Option<PathBuf>) -> String {
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
    default_prefix()
}

/// Calculate exec_prefix
fn calculate_exec_prefix(exe_dir: &Option<PathBuf>, prefix: &str) -> String {
    #[cfg(windows)]
    {
        // Windows: exec_prefix == prefix
        let _ = exe_dir; // silence unused warning
        prefix.to_owned()
    }

    #[cfg(not(windows))]
    {
        // POSIX: search for lib-dynload directory
        if let Some(dir) = exe_dir {
            let landmark = platform::platstdlib_landmark();
            if let Some(exec_prefix) = search_up_dir(dir, &[&landmark]) {
                return exec_prefix.to_string_lossy().into_owned();
            }
        }
        // Fallback: same as prefix
        prefix.to_owned()
    }
}

/// Calculate base_executable
fn calculate_base_executable(executable: Option<&PathBuf>, home_dir: &Option<PathBuf>) -> String {
    // If in venv and we have home, construct base_executable from home
    if let (Some(exe), Some(home)) = (executable, home_dir)
        && let Some(exe_name) = exe.file_name()
    {
        let base = home.join(exe_name);
        return base.to_string_lossy().into_owned();
    }

    // Otherwise, base_executable == executable
    executable
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
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

/// Get the current executable path
fn get_executable_path() -> Option<PathBuf> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let exec_arg = env::args_os().next()?;
        which::which(exec_arg).ok()
    }
    #[cfg(target_arch = "wasm32")]
    {
        let exec_arg = env::args().next()?;
        Some(PathBuf::from(exec_arg))
    }
}

/// Parse pyvenv.cfg and extract the 'home' key value
fn parse_pyvenv_home(pyvenv_cfg: &Path) -> Option<String> {
    let content = std::fs::read_to_string(pyvenv_cfg).ok()?;

    for line in content.lines() {
        if let Some((key, value)) = line.split_once('=')
            && key.trim().to_lowercase() == "home"
        {
            return Some(value.trim().to_string());
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_init_path_config() {
        let settings = Settings::default();
        let paths = init_path_config(&settings);
        // Just verify it doesn't panic and returns valid paths
        assert!(!paths.prefix.is_empty());
    }

    #[test]
    fn test_search_up() {
        // Test with a path that doesn't have any landmarks
        let result = search_up_file(std::env::temp_dir(), &["nonexistent_landmark_xyz"]);
        assert!(result.is_none());
    }

    #[test]
    fn test_default_prefix() {
        let prefix = default_prefix();
        assert!(!prefix.is_empty());
    }
}

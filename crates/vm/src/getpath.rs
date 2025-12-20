//! Path configuration for RustPython (ref: Modules/getpath.py)
//!
//! This module provides path calculation logic but implemented directly in Rust.
//!
//! The main entry point is `init_path_config()` which computes Paths from Settings.

use std::env;
use std::path::{Path, PathBuf};

use crate::version;
use crate::vm::{Paths, Settings};

/// Compute path configuration from Settings
///
/// This function should be called before interpreter initialization.
/// It returns a Paths struct with all computed path values.
pub fn init_path_config(settings: &Settings) -> Paths {
    let mut paths = Paths::default();

    // 1. Compute executable path
    paths.executable = get_executable_path()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();

    let exe_path = if paths.executable.is_empty() {
        None
    } else {
        Some(PathBuf::from(&paths.executable))
    };

    // 2. Compute base_executable (for venv support)
    paths.base_executable =
        compute_base_executable(exe_path.as_deref()).unwrap_or_else(|| paths.executable.clone());

    // 3. Compute prefix paths
    let (prefix, base_prefix) = compute_prefixes(exe_path.as_deref());
    paths.prefix = prefix.unwrap_or_else(default_prefix);
    paths.base_prefix = base_prefix.unwrap_or_else(|| paths.prefix.clone());
    paths.exec_prefix = paths.prefix.clone();
    paths.base_exec_prefix = paths.base_prefix.clone();

    // 4. Build module_search_paths
    paths.module_search_paths = compute_module_search_paths(settings, &paths.base_prefix);

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

/// Compute base_executable from executable path
fn compute_base_executable(exe_path: Option<&Path>) -> Option<String> {
    let exe_path = exe_path?;

    // Check for __PYVENV_LAUNCHER__ environment variable (like getpath.c env_to_dict)
    if let Ok(launcher) = env::var("__PYVENV_LAUNCHER__") {
        return Some(launcher);
    }

    // Check if we're in a venv
    if let Some(venv_home) = get_venv_home(exe_path) {
        // venv_home is the bin directory containing the base Python
        let home_path = PathBuf::from(&venv_home);
        let exe_name = exe_path.file_name()?;
        let base_exe = home_path.join(exe_name);
        return Some(base_exe.to_string_lossy().into_owned());
    }

    // Not in venv: base_executable == executable
    Some(exe_path.to_string_lossy().into_owned())
}

/// Compute prefix and base_prefix from executable path
fn compute_prefixes(exe_path: Option<&Path>) -> (Option<String>, Option<String>) {
    let Some(exe_path) = exe_path else {
        return (None, None);
    };
    let Some(exe_dir) = exe_path.parent() else {
        return (None, None);
    };

    // Check if we're in a venv
    if let Some(venv_home) = get_venv_home(exe_path) {
        // prefix is the venv directory (parent of bin/)
        let prefix = exe_dir.parent().map(|p| p.to_string_lossy().into_owned());

        // base_prefix is parent of venv_home (the original Python's prefix)
        let home_path = PathBuf::from(&venv_home);
        let base_prefix = home_path.parent().map(|p| p.to_string_lossy().into_owned());

        return (prefix, base_prefix);
    }

    // Not in venv: prefix == base_prefix
    let prefix = exe_dir.parent().map(|p| p.to_string_lossy().into_owned());
    (prefix.clone(), prefix)
}

/// Build the complete module_search_paths (sys.path)
fn compute_module_search_paths(settings: &Settings, base_prefix: &str) -> Vec<String> {
    let mut paths = Vec::new();

    // 1. Add paths from path_list (PYTHONPATH/RUSTPYTHONPATH)
    paths.extend(settings.path_list.iter().cloned());

    // 2. Add zip stdlib path
    let platlibdir = "lib";
    let zip_name = format!("rustpython{}{}", version::MAJOR, version::MINOR);
    let zip_path = PathBuf::from(base_prefix).join(platlibdir).join(&zip_name);
    paths.push(zip_path.to_string_lossy().into_owned());

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

/// Get the 'home' value from pyvenv.cfg if running in a virtual environment
///
/// pyvenv.cfg is located in the parent directory of the bin directory
/// (e.g., venv/pyvenv.cfg when executable is venv/bin/rustpython)
pub fn get_venv_home(exe_path: &Path) -> Option<String> {
    let exe_dir = exe_path.parent()?;
    let venv_dir = exe_dir.parent()?;
    let pyvenv_cfg = venv_dir.join("pyvenv.cfg");

    if !pyvenv_cfg.exists() {
        return None;
    }

    parse_pyvenv_home(&pyvenv_cfg)
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
}

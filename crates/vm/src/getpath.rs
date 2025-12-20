//! Path configuration for RustPython (ref: Modules/getpath.py)
//!
//! This module provides path calculation logic but implemented directly in Rust.
//!
//! The main entry point is `init_path_config()` which should be called
//! before interpreter initialization to populate Settings with path info.

use std::env;
use std::path::{Path, PathBuf};

use crate::version;
use crate::vm::Settings;

/// Initialize path configuration in Settings (like getpath.py)
///
/// This function should be called before interpreter initialization.
/// It computes executable, base_executable, prefix, and module_search_paths.
pub fn init_path_config(settings: &mut Settings) {
    // Skip if already configured
    if settings.module_search_paths_set {
        return;
    }

    // 1. Compute executable path
    if settings.executable.is_none() {
        settings.executable = get_executable_path().map(|p| p.to_string_lossy().into_owned());
    }

    let exe_path = settings
        .executable
        .as_ref()
        .map(PathBuf::from)
        .or_else(get_executable_path);

    // 2. Compute base_executable (for venv support)
    if settings.base_executable.is_none() {
        settings.base_executable = compute_base_executable(exe_path.as_deref());
    }

    // 3. Compute prefix paths (with fallbacks to ensure all values are set)
    let (prefix, base_prefix) = compute_prefixes(exe_path.as_deref());
    let default_prefix = || {
        std::option_env!("RUSTPYTHON_PREFIX")
            .map(String::from)
            .unwrap_or_else(|| if cfg!(windows) { "C:" } else { "/usr/local" }.to_owned())
    };

    if settings.prefix.is_none() {
        settings.prefix = Some(prefix.clone().unwrap_or_else(default_prefix));
    }
    if settings.base_prefix.is_none() {
        settings.base_prefix = Some(
            base_prefix
                .clone()
                .or_else(|| prefix.clone())
                .unwrap_or_else(default_prefix),
        );
    }
    if settings.exec_prefix.is_none() {
        settings.exec_prefix = settings.prefix.clone();
    }
    if settings.base_exec_prefix.is_none() {
        settings.base_exec_prefix = settings.base_prefix.clone();
    }

    // 4. Build module_search_paths (use settings.base_prefix which is now guaranteed to be set)
    settings.module_search_paths =
        compute_module_search_paths(settings, settings.base_prefix.as_deref());
    settings.module_search_paths_set = true;
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
        if base_exe.exists() {
            return Some(base_exe.to_string_lossy().into_owned());
        }
        // Fallback: just return the home directory path with exe name
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

    let exe_dir = match exe_path.parent() {
        Some(d) => d,
        None => return (None, None),
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
fn compute_module_search_paths(settings: &Settings, base_prefix: Option<&str>) -> Vec<String> {
    let mut paths = Vec::new();

    // 1. Add paths from path_list (PYTHONPATH/RUSTPYTHONPATH)
    paths.extend(settings.path_list.iter().cloned());

    // 2. Add zip stdlib path
    if let Some(base_prefix) = base_prefix {
        let platlibdir = "lib";
        let zip_name = format!("rustpython{}{}", version::MAJOR, version::MINOR);
        let zip_path = PathBuf::from(base_prefix).join(platlibdir).join(&zip_name);
        paths.push(zip_path.to_string_lossy().into_owned());
    }

    paths
}

/// Get the zip stdlib path to add to sys.path
///
/// Returns a path like `/usr/local/lib/rustpython313` or
/// `/path/to/venv/lib/rustpython313` for virtual environments.
pub fn get_zip_stdlib_path() -> Option<String> {
    // ZIP_LANDMARK pattern: {platlibdir}/{impl_name}{VERSION_MAJOR}{VERSION_MINOR}
    let platlibdir = "lib";
    let zip_name = format!("rustpython{}{}", version::MAJOR, version::MINOR);

    let base_prefix = get_base_prefix()?;
    let zip_path = base_prefix.join(platlibdir).join(&zip_name);

    Some(zip_path.to_string_lossy().into_owned())
}

/// Get the base prefix directory
///
/// For installed Python: parent of the bin directory
/// For venv: the 'home' value from pyvenv.cfg
fn get_base_prefix() -> Option<PathBuf> {
    let exe_path = get_executable_path()?;
    let exe_dir = exe_path.parent()?;

    // Check if we're in a venv by looking for pyvenv.cfg
    if let Some(venv_home) = get_venv_home(&exe_path) {
        // venv_home is the directory containing the base Python
        // Go up one level to get the prefix (e.g., /usr/local from /usr/local/bin)
        let home_path = PathBuf::from(&venv_home);
        if let Some(parent) = home_path.parent() {
            return Some(parent.to_path_buf());
        }
        return Some(home_path);
    }

    // Not in venv: go up from bin/ to get prefix
    // e.g., /usr/local/bin/rustpython -> /usr/local
    exe_dir.parent().map(|p| p.to_path_buf())
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
    fn test_zip_stdlib_path_format() {
        // Just verify it returns something and doesn't panic
        let _path = get_zip_stdlib_path();
    }
}

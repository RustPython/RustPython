//! RustPython venv launcher
//!
//! A lightweight launcher that reads pyvenv.cfg and delegates execution
//! to the actual Python interpreter. This mimics CPython's venvlauncher.c.
//! Windows only.

#[cfg(not(windows))]
compile_error!("venvlauncher is only supported on Windows");

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code as u8),
        Err(e) => {
            eprintln!("venvlauncher error: {}", e);
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<u32, Box<dyn core::error::Error>> {
    // 1. Get own executable path
    let exe_path = env::current_exe()?;
    let exe_name = exe_path
        .file_name()
        .ok_or("Failed to get executable name")?
        .to_string_lossy();

    // 2. Determine target executable name based on launcher name
    // pythonw.exe / venvwlauncher -> pythonw.exe (GUI, no console)
    // python.exe / venvlauncher -> python.exe (console)
    let exe_name_lower = exe_name.to_lowercase();
    let target_exe = if exe_name_lower.contains("pythonw") || exe_name_lower.contains("venvw") {
        "pythonw.exe"
    } else {
        "python.exe"
    };

    // 3. Find pyvenv.cfg
    // The launcher is in Scripts/ directory, pyvenv.cfg is in parent (venv root)
    let scripts_dir = exe_path.parent().ok_or("Failed to get Scripts directory")?;
    let venv_dir = scripts_dir.parent().ok_or("Failed to get venv directory")?;
    let cfg_path = venv_dir.join("pyvenv.cfg");

    if !cfg_path.exists() {
        return Err(format!("pyvenv.cfg not found: {}", cfg_path.display()).into());
    }

    // 4. Parse home= from pyvenv.cfg
    let home = read_home(&cfg_path)?;

    // 5. Locate python executable in home directory
    let python_path = PathBuf::from(&home).join(target_exe);
    if !python_path.exists() {
        return Err(format!("Python not found: {}", python_path.display()).into());
    }

    // 6. Set __PYVENV_LAUNCHER__ environment variable
    // This tells Python it was launched from a venv
    // SAFETY: We are in a single-threaded context (program entry point)
    unsafe {
        env::set_var("__PYVENV_LAUNCHER__", &exe_path);
    }

    // 7. Launch Python with same arguments
    let args: Vec<String> = env::args().skip(1).collect();
    launch_process(&python_path, &args)
}

/// Parse the `home=` value from pyvenv.cfg
fn read_home(cfg_path: &Path) -> Result<String, Box<dyn core::error::Error>> {
    let content = fs::read_to_string(cfg_path)?;

    for line in content.lines() {
        let line = line.trim();
        // Skip comments and empty lines
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Look for "home = <path>" or "home=<path>"
        if let Some(rest) = line.strip_prefix("home") {
            let rest = rest.trim_start();
            if let Some(value) = rest.strip_prefix('=') {
                return Ok(value.trim().to_string());
            }
        }
    }

    Err("'home' key not found in pyvenv.cfg".into())
}

/// Launch the Python process and wait for it to complete
fn launch_process(exe: &Path, args: &[String]) -> Result<u32, Box<dyn core::error::Error>> {
    use std::process::Command;

    let status = Command::new(exe).args(args).status()?;

    Ok(status.code().unwrap_or(1) as u32)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_read_home() {
        let temp_dir = std::env::temp_dir();
        let cfg_path = temp_dir.join("test_pyvenv.cfg");

        let mut file = fs::File::create(&cfg_path).unwrap();
        writeln!(file, "home = C:\\Python314").unwrap();
        writeln!(file, "include-system-site-packages = false").unwrap();
        writeln!(file, "version = 3.14.0").unwrap();

        let home = read_home(&cfg_path).unwrap();
        assert_eq!(home, "C:\\Python314");

        fs::remove_file(&cfg_path).unwrap();
    }

    #[test]
    fn test_read_home_no_spaces() {
        let temp_dir = std::env::temp_dir();
        let cfg_path = temp_dir.join("test_pyvenv2.cfg");

        let mut file = fs::File::create(&cfg_path).unwrap();
        writeln!(file, "home=C:\\Python313").unwrap();

        let home = read_home(&cfg_path).unwrap();
        assert_eq!(home, "C:\\Python313");

        fs::remove_file(&cfg_path).unwrap();
    }

    #[test]
    fn test_read_home_with_comments() {
        let temp_dir = std::env::temp_dir();
        let cfg_path = temp_dir.join("test_pyvenv3.cfg");

        let mut file = fs::File::create(&cfg_path).unwrap();
        writeln!(file, "# This is a comment").unwrap();
        writeln!(file, "home = D:\\RustPython").unwrap();

        let home = read_home(&cfg_path).unwrap();
        assert_eq!(home, "D:\\RustPython");

        fs::remove_file(&cfg_path).unwrap();
    }
}

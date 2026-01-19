const CRATE_ROOT: &str = "../..";

fn main() {
    process_python_libs(format!("{CRATE_ROOT}/vm/Lib/python_builtins/*").as_str());
    process_python_libs(format!("{CRATE_ROOT}/vm/Lib/core_modules/*").as_str());

    #[cfg(feature = "freeze-stdlib")]
    if cfg!(windows) {
        process_python_libs(format!("{CRATE_ROOT}/Lib/**/*").as_str());
    } else {
        process_python_libs("./Lib/**/*");
    }

    if cfg!(windows) {
        // On Windows, the Lib entry can be either:
        // 1. A text file containing the relative path (git without symlink support)
        // 2. A proper symlink (git with symlink support)
        // We handle both cases to resolve to the actual Lib directory.
        let lib_path = if let Ok(real_path) = std::fs::read_to_string("Lib") {
            // Case 1: Text file containing relative path
            std::path::PathBuf::from(real_path.trim())
        } else {
            // Case 2: Symlink or directory - canonicalize directly
            std::path::PathBuf::from("Lib")
        };

        if let Ok(canonicalized_path) = std::fs::canonicalize(&lib_path) {
            // Strip the extended path prefix (\\?\) that canonicalize adds on Windows
            let path_str = canonicalized_path.to_str().unwrap();
            let path_str = path_str.strip_prefix(r"\\?\").unwrap_or(path_str);
            println!("cargo:rustc-env=win_lib_path={path_str}");
        }
    }
}

// remove *.pyc files and add *.py to watch list
fn process_python_libs(pattern: &str) {
    let glob = glob::glob(pattern).unwrap_or_else(|e| panic!("failed to glob {pattern:?}: {e}"));
    for entry in glob.flatten() {
        if entry.is_dir() {
            continue;
        }
        let display = entry.display();
        if display.to_string().ends_with(".pyc") {
            if std::fs::remove_file(&entry).is_err() {
                println!("cargo:warning=failed to remove {display}")
            }
            continue;
        }
        println!("cargo:rerun-if-changed={display}");
    }
}

fn main() {
    process_python_libs("../Lib/python_builtins/*");

    #[cfg(not(feature = "stdlib"))]
    process_python_libs("../Lib/core_modules/*");

    #[cfg(feature = "stdlib")]
    process_python_libs("../../Lib/**/*");

    if cfg!(windows) {
        if let Ok(real_path) = std::fs::read_to_string("Lib") {
            println!("rustc-env:win_lib_path={real_path:?}");
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

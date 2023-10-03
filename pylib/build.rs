fn main() {
    process_python_libs("../vm/Lib/python_builtins/*");

    #[cfg(not(feature = "stdlib"))]
    process_python_libs("../vm/Lib/core_modules/*");
    #[cfg(feature = "freeze-stdlib")]
    if cfg!(windows) {
        process_python_libs("../Lib/**/*");
    } else {
        process_python_libs("./Lib/**/*");
    }

    if cfg!(windows) {
        if let Ok(real_path) = std::fs::read_to_string("Lib") {
            let canonicalized_path = std::fs::canonicalize(real_path)
                .expect("failed to resolve RUSTPYTHONPATH during build time");
            println!(
                "cargo:rustc-env=win_lib_path={}",
                canonicalized_path.to_str().unwrap()
            );
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

fn main() {
    rerun_if_changed("../Lib/python_builtins/*");

    #[cfg(not(feature = "stdlib"))]
    rerun_if_changed("../Lib/core_modules/*");

    #[cfg(feature = "stdlib")]
    rerun_if_changed("../../Lib/**/*");

    if cfg!(windows) {
        if let Ok(real_path) = std::fs::read_to_string("Lib") {
            println!("rustc-env:win_lib_path={:?}", real_path);
        }
    }
}

fn rerun_if_changed(pattern: &str) {
    let glob =
        glob::glob(pattern).unwrap_or_else(|e| panic!("failed to glob {:?}: {}", pattern, e));
    for entry in glob.flatten() {
        if entry.is_dir() {
            continue;
        }
        let display = entry.display();
        if display.to_string().ends_with(".pyc") {
            continue;
        }
        println!("cargo:rerun-if-changed={}", display);
    }
}

fn main() {
    #[cfg(feature = "compiled-bytecode")]
    for entry in glob::glob("../../Lib/**/*")
        .expect("Lib/ exists?")
        .flatten()
    {
        if entry.is_dir() {
            continue;
        }
        let display = entry.display();
        if display.to_string().ends_with(".pyc") {
            continue;
        }
        println!("cargo:rerun-if-changed={}", display);
    }
    if cfg!(windows) {
        if let Ok(real_path) = std::fs::read_to_string("Lib") {
            println!("rustc-env:win_lib_path={:?}", real_path);
        }
    }
}

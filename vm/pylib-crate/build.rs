fn main() {
    if cfg!(windows) {
        if let Ok(real_path) = std::fs::read_to_string("Lib") {
            println!("rustc-env:win_lib_path={:?}", real_path);
        }
    }
}

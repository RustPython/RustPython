fn main() {
    let mut lib_path = std::env::current_dir().unwrap();
    lib_path.push("Lib");
    println!("cargo:rustc-env=RUSTPYTHON_LIB_DIR={}", lib_path.display());
}

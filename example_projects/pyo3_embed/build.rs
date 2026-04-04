fn main() {
    println!(
        "cargo:rustc-link-arg=/Users/basschoenmaeckers/repo/RustPython/target/debug/librustpython_capi.a"
    );
    println!("cargo:rustc-link-lib=framework=CoreFoundation");
}

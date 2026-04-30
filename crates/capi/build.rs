fn main() {
    println!("cargo:rerun-if-changed=csrc/pyobject_callmethodobjargs.c");
    cc::Build::new()
        .file("csrc/pyobject_callmethodobjargs.c")
        .compile("rustpython_capi_shims");
}

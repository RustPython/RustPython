fn main() {
    println!("cargo:rerun-if-changed=Python.asdl");
    println!("cargo:rerun-if-changed=asdl_rs.py");
    // println!("cargo:rerun-if-changed=../scripts/update_asdl.sh");

    let ast_gen = "./src/ast_gen.rs";
    let stdlib_ast_gen = "../vm/src/stdlib/ast/gen.rs";

    std::process::Command::new("python3")
        .args([
            "./asdl_rs.py",
            "-D",
            ast_gen,
            "-M",
            stdlib_ast_gen,
            "./Python.asdl",
        ])
        .output()
        .expect("failed to execute update_asdl");

    std::process::Command::new("rustfmt")
        .args([ast_gen, stdlib_ast_gen])
        .status()
        .expect("failed to execute rustfmt");
}

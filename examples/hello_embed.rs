use rustpython_vm as vm;

fn main() -> vm::embedding::Result<()> {
    vm::Interpreter::without_stdlib(Default::default())
        .enter(|vm| vm.exec(r#"print("Hello World!")"#))
}

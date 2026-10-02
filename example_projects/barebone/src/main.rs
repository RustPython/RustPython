use rustpython_vm::Interpreter;

pub fn main() {
    let interp = Interpreter::without_stdlib(Default::default());
    let value = interp.enter(|vm| {
        let builtins = vm.import("builtins")?;
        let max = builtins.get_attr("max")?;
        let value = max.call(&[vm.new_int(5)?, vm.new_int(10)?])?;
        builtins
            .get_attr("print")?
            .call(&[vm.new_str("python print")?, value.clone()])?;
        value.repr()
    });
    let result = value.map(|value| println!("Rust repr: {value}"));
    interp.run(|_vm| result).expect("no native workers remain");
}

use rustpython_vm::Interpreter;

pub fn main() {
    let interp = Interpreter::without_stdlib(Default::default());
    let value = interp.enter(|vm| {
        let max = vm.builtins.get_attr("max", vm)?;
        let value = max.call((vm.ctx.new_int(5), vm.ctx.new_int(10)), vm)?;
        vm.print((vm.ctx.new_str("python print"), value.clone()))?;
        Ok(value)
    });
    match value {
        Ok(value) => println!("Rust repr: {:?}", value),
        Err(err) => {
            interp.finalize(err);
        }
    }
}

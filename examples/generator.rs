use rustpython::InterpreterBuilderExt;
use rustpython_vm as vm;
use std::process::ExitCode;
use vm::Interpreter;

fn py_main(interp: &Interpreter) -> vm::embedding::Result<()> {
    let generator = interp.enter(|vm| {
        vm.exec("def gen():\n    yield from range(10)")?;
        Ok(vm.eval("gen()")?.unbind())
    })?;

    loop {
        let value = interp.enter(|vm| {
            vm.bind(&generator)?
                .next()?
                .map(|value| value.to_i64())
                .transpose()
        })?;
        match value {
            Some(value) => println!("{value}"),
            None => break,
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    let interp = vm::Interpreter::builder(Default::default())
        .init_stdlib()
        .build();
    let result = py_main(&interp);
    vm::host_env::os::exit_code(interp.run(|_vm| result).expect("no native workers remain"))
}

use rustpython_vm as vm;
use std::process::ExitCode;
use vm::{
    Interpreter, PyResult,
    builtins::PyIntRef,
    protocol::{PyIter, PyIterReturn},
};

fn py_main(interp: &Interpreter) -> vm::PyResult<()> {
    let generator = interp.enter(|vm| {
        let scope = vm.new_scope_with_builtins();
        let generator = vm.run_block_expr(
            scope,
            r#"
def gen():
    for i in range(10):
        yield i

gen()
"#,
        )?;
        Ok(generator)
    })?;

    loop {
        let r = interp.enter(|vm| {
            let v = match PyIter::new(generator.clone()).next(vm)? {
                PyIterReturn::Return(obj) => {
                    PyIterReturn::Return(obj.try_into_value::<PyIntRef>(vm)?)
                }
                PyIterReturn::StopIteration(x) => PyIterReturn::StopIteration(x),
            };
            PyResult::Ok(v)
        })?;
        match r {
            PyIterReturn::Return(value) => println!("{value}"),
            PyIterReturn::StopIteration(_) => break,
        }
    }

    Ok(())
}

fn main() -> ExitCode {
    let builder = vm::Interpreter::builder(Default::default());
    let defs = rustpython_stdlib::stdlib_module_defs(&builder.ctx);
    let interp = builder.add_native_modules(&defs).build();
    let result = py_main(&interp);
    vm::common::os::exit_code(interp.run(|_vm| result))
}

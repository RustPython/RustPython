use rustpython_vm as vm;
use std::process::ExitCode;
use vm::{
    builtins::PyIntRef,
    protocol::{PyIter, PyIterReturn},
    Interpreter, PyResult,
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
    let interp = vm::Interpreter::with_init(Default::default(), |vm| {
        vm.add_native_modules(rustpython_stdlib::get_module_inits());
    });
    let result = py_main(&interp);
    ExitCode::from(interp.run(|_vm| result))
}

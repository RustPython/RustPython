use rustpython_vm as vm;
use vm::{
    builtins::PyIntRef,
    protocol::{PyIter, PyIterReturn},
    Interpreter, PyResult,
};

fn py_main(interp: &Interpreter) -> vm::PyResult<()> {
    let generator = interp.enter(|vm| {
        let scope = vm.new_scope_with_builtins();
        let _ = vm.run_code_string(
            scope.clone(),
            r#"
def gen():
    for i in range(10):
        yield i

generator = gen()
"#,
            "".to_owned(),
        )?;
        let generator = scope.globals.get_item("generator", vm)?;
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
            PyIterReturn::Return(value) => println!("{}", value),
            PyIterReturn::StopIteration(_) => break,
        }
    }

    Ok(())
}

fn main() {
    let interp = vm::Interpreter::with_init(Default::default(), |vm| {
        vm.add_native_modules(rustpython_stdlib::get_module_inits());
    });
    let result = py_main(&interp);
    std::process::exit(interp.run(|_vm| result));
}

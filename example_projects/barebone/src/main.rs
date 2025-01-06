use rustpython::InterpreterConfig;
use rustpython_vm::stdlib::get_module_inits;
use rustpython_vm::{Interpreter, PyResult, Settings, VirtualMachine};
use std::env;
use std::env::split_paths;

pub fn main() {
    env_logger::init();
    // let mut stdlib = get_module_inits();
    // let interp = Interpreter::with_init(Default::default(), |vm: &mut VirtualMachine| {
    //     vm.add_native_module("_ast", stdlib.remove("_ast").unwrap())
    //
    // });
    let mut settings = Settings::default();
    settings.path_list.extend(get_paths("RUSTPYTHONPATH"));
    let mut config = InterpreterConfig::new().init_stdlib().settings(settings);

    let interp = config.interpreter();
    let value = interp.enter(|vm| {
        // import ast
        // let module = vm.import("ast", 0);
        // module

        vm.run_code_string(
            vm.new_scope_with_builtins(),
            "
import enum
@enum._simple_enum(enum.IntFlag, boundary=enum.KEEP)
class RegexFlag:
    NOFLAG = 0
    DEBUG = 1
RegexFlag.NOFLAG & RegexFlag.DEBUG
",
            "<string>".to_string(),
        )
    });
    match value {
        Ok(value) => println!("Rust repr: {:?}", value),
        Err(err) => {
            interp.finalize(Some(err));
        }
    }
}

/// Helper function to retrieve a sequence of paths from an environment variable.
fn get_paths(env_variable_name: &str) -> impl Iterator<Item = String> + '_ {
    env::var_os(env_variable_name)
        .into_iter()
        .flat_map(move |paths| {
            split_paths(&paths)
                .map(|path| {
                    path.into_os_string()
                        .into_string()
                        .unwrap_or_else(|_| panic!("{env_variable_name} isn't valid unicode"))
                })
                .collect::<Vec<_>>()
        })
}

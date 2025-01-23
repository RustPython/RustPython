use rustpython::InterpreterConfig;
use rustpython_vm::Settings;
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
    settings.write_bytecode = false;
    settings.path_list.extend(get_paths("RUSTPYTHONPATH"));
    let config = InterpreterConfig::new().init_stdlib().settings(settings);
    let interp = config.interpreter();
    let value = interp.enter(|vm| {
        // import ast
        // let module = vm.import("ast", 0);
        // module

        let source = r#"
import ast
a = ast.parse("""
print(0)

import ast

class Node:
    pass

class ClassDef(Node):
    a = 'a'

def f():
    print(ClassDef.a)

b = 'b'
c = (1,2,)
e = ''
e = '' ''
e = f''
e = '' f'' '' f'a'
print(0, 'a', f'a{b}', 1.0, b'33333', True, False, None, ..., c, d := 42)
""")
# print(ast.dump(a, indent=4))
compile(a, '<string>', 'exec')
"#;
        let r = vm.run_code_string(vm.new_scope_with_builtins(), source, "<string>".to_string());
        _ = vm.unwrap_pyresult(r);

        let s = vm.new_scope_with_builtins();
        let () = vm.unwrap_pyresult(s.locals.mapping().ass_subscript(
            &vm.ctx.new_str("b"),
            Some(vm.ctx.new_str(source).into()),
            vm,
        ));
        _ = vm.unwrap_pyresult(vm.run_code_string(
            s.clone(),
            r#"import ast
a = ast.unparse(ast.parse(b))"#,
            "<string>".to_string(),
        ));
        s.locals.mapping().subscript(&vm.ctx.new_str("a"), vm)
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

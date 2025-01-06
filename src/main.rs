use rustpython::InterpreterConfig;
use rustpython_vm::Settings;
use std::env;
use std::env::split_paths;
use std::process::ExitCode;

pub fn main() -> ExitCode {
    let mut settings = Settings::default();
    settings.write_bytecode = false;
    settings.path_list.extend(get_paths("RUSTPYTHONPATH"));
    let mut config = InterpreterConfig::new().init_stdlib().settings(settings);
    let interp = config.interpreter();
    let exitcode = interp.run(move |vm| {
        let scope = vm.new_scope_with_builtins();
        vm.run_code_string(
            scope,
            r#"
import enum
@enum._simple_enum(enum.IntFlag, boundary=enum.KEEP)
class RegexFlag:
    NOFLAG = 0
    DEBUG = 1
print(RegexFlag.NOFLAG & RegexFlag.DEBUG)
#import ast
#print(dir(ast))
"#,
            "<main>".to_string(),
        )
        .map(|_| ())
    });

    ExitCode::from(exitcode)
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

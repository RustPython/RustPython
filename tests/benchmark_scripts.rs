// cspell:ignore pyperformance nqueens pidigits
use rustpython::{InterpreterBuilder, InterpreterBuilderExt};
use std::{fs, path::Path};

const PYPERFORMANCE_BENCHMARKS: &[&str] = &[
    "base64.py",
    "decimal_factorial.py",
    "deepcopy.py",
    "deltablue.py",
    "float.py",
    "gc_collect.py",
    "gc_traversal.py",
    "generators.py",
    "json_dumps.py",
    "nqueens.py",
    "pickle.py",
    "pidigits.py",
    "regex_effbot.py",
    "richards.py",
    "unpickle.py",
];

#[test]
fn pyperformance_benchmarks_run_in_rustpython() {
    let benchmark_dir = Path::new("benches/benchmarks");

    for name in PYPERFORMANCE_BENCHMARKS {
        let path = benchmark_dir.join(name);
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
        let source_path = path.to_string_lossy();

        InterpreterBuilder::new()
            .init_stdlib()
            .interpreter()
            .enter(|vm| {
                let code = vm
                    .compile(&source, rustpython::vm::compiler::Mode::Exec, &*source_path)
                    .unwrap_or_else(|err| panic!("failed to compile {}: {err}", path.display()));
                let scope = vm.new_scope_with_builtins();
                if let Err(err) = vm.run_code_obj(code, scope) {
                    vm.print_exception(err);
                    panic!("failed to execute {}", path.display());
                }
            });
    }
}

#[test]
fn pyperformance_benchmarks_restore_interpreter_state() {
    let source = r#"
import decimal
import pickle
import sys

missing = object()
pickle_module = sys.modules.get("pickle", missing)
pickle_accelerator = sys.modules.get("_pickle", missing)
decimal_precision = decimal.getcontext().prec

for path in (
    "benches/benchmarks/pickle.py",
    "benches/benchmarks/unpickle.py",
    "benches/benchmarks/decimal_factorial.py",
):
    with open(path) as benchmark_file:
        benchmark_source = benchmark_file.read()
    exec(compile(benchmark_source, path, "exec"), {})

assert sys.modules.get("pickle", missing) is pickle_module
assert sys.modules.get("_pickle", missing) is pickle_accelerator
assert decimal.getcontext().prec == decimal_precision
"#;

    InterpreterBuilder::new()
        .init_stdlib()
        .interpreter()
        .enter(|vm| {
            if let Err(err) = vm.run_simple_string(source) {
                vm.print_exception(err);
                panic!("pyperformance benchmarks leaked interpreter state");
            }
        });
}

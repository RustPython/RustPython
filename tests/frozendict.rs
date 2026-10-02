#![allow(clippy::tests_outside_test_module)]

use rustpython::{InterpreterBuilder, InterpreterBuilderExt};

fn run_suite(name: &str, source: &str) {
    InterpreterBuilder::new()
        .init_stdlib()
        .interpreter()
        .enter(|vm| {
            let scope = vm.new_scope_with_builtins();
            scope
                .globals
                .set_item("suite_name", vm.ctx.new_str(name).into(), vm)
                .unwrap();
            scope
                .globals
                .set_item("suite_source", vm.ctx.new_str(source).into(), vm)
                .unwrap();
            let runner = r#"
import sys
import types
import unittest

module = types.ModuleType(suite_name)
sys.modules[suite_name] = module
exec(compile(suite_source, suite_name + '.py', 'exec'), module.__dict__)
suite = unittest.defaultTestLoader.loadTestsFromModule(module)
result = unittest.TextTestRunner(verbosity=2).run(suite)
assert result.wasSuccessful(), 'frozendict regression suite failed'
assert not result.skipped, 'frozendict regressions must all execute'
"#;
            if let Err(err) = vm.run_string(scope, runner, "frozendict-test-runner") {
                vm.print_exception(&err);
                panic!("{name} failed");
            }
        });
}

#[test]
fn frozen_mapping_contract() {
    run_suite(
        "frozendict_contract",
        include_str!("../extra_tests/frozendict_contract.py"),
    );
}

#[test]
fn frozen_global_namespaces() {
    run_suite(
        "frozendict_globals",
        include_str!("../extra_tests/frozendict_globals.py"),
    );
}

#[test]
fn frozen_mapping_edgecases() {
    run_suite(
        "frozendict_edgecases",
        include_str!("../extra_tests/frozendict_edgecases.py"),
    );
}

#[test]
fn frozen_type_namespaces() {
    run_suite(
        "frozendict_type_namespace",
        include_str!("../extra_tests/frozendict_type_namespace.py"),
    );
}

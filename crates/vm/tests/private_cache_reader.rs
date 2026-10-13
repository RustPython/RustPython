//! Direct embedding API format boundary. Python behavior is tested in snippets.

#![allow(clippy::tests_outside_test_module)]

use rustpython_vm::{AsObject, Interpreter, builtins::PyCode};

include!("fixtures/private_cache_bytes.rs");

#[test]
fn foreign_formats_are_rejected_before_bootstrap_lookup() {
    Interpreter::without_stdlib(Default::default()).enter(|vm| {
        for magic in [3627u16, 3666, 0xDA15] {
            let mut bytes = PRIVATE_PYC.to_vec();
            bytes[..2].copy_from_slice(&magic.to_le_bytes());
            let error = PyCode::from_pyc(&bytes, None, None, None, vm).unwrap_err();
            assert!(error.fast_isinstance(vm.ctx.exceptions.value_error));
        }
    });
}

#[cfg(feature = "importlib")]
#[test]
fn private_format_loads_and_runs_with_external_bootstrap() {
    Interpreter::without_stdlib(Default::default()).enter(|vm| {
        assert_eq!(
            &PRIVATE_PYC[..4],
            &rustpython_vm::version::PYC_MAGIC_NUMBER_BYTES
        );
        let code = PyCode::from_pyc(PRIVATE_PYC, None, None, None, vm).unwrap();
        assert_eq!(code.arg_count, 0);
        let scope = vm.new_scope_with_main().unwrap();
        vm.run_pyc_bytes(PRIVATE_PYC, scope)
            .unwrap_or_else(|error| {
                vm.print_exception(&error);
                panic!("private embedded pyc execution failed");
            });
    });
}

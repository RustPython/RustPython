#![cfg(test)]
#![cfg(all(feature = "compiler", feature = "gc"))]

// Keep collection isolated from other interpreter fixtures in this process.
use rustpython_vm::{AsObject, Interpreter, PyPayload, builtins::PyValueError, types::Constructor};

/// Collects the cycle from exception arguments through a marker back to the exception.
#[test]
fn argument_cycle_releases_marker() {
    Interpreter::without_stdlib(Default::default()).enter(|vm| {
        let scope = vm.new_scope_with_builtins();
        let source = "class Marker: pass\nmarker = Marker()\n";
        let code = vm
            .compile(
                source,
                rustpython_vm::compiler::Mode::Exec,
                "<argument-cycle>",
            )
            .map_err(|error| error.into_pyexception(vm, Some(source)))
            .unwrap();
        vm.run_code_obj(code, scope.clone()).unwrap();
        let argument = scope.globals.get_item("marker", vm).unwrap();
        let weak = argument.downgrade(None, vm).unwrap();
        let exception = PyValueError::py_new(
            vm.ctx.exceptions.value_error,
            vec![argument.clone()].into(),
            vm,
        )
        .unwrap()
        .into_ref(&vm.ctx);
        argument
            .set_attr("exception", exception.clone(), vm)
            .unwrap();
        assert!(
            argument
                .get_attr("exception", vm)
                .unwrap()
                .is(exception.as_object())
        );
        scope.globals.del_item("marker", vm).unwrap();
        drop(argument);
        drop(exception);
        assert!(weak.upgrade().is_some());
        vm.state.gc.collect_force(2);
        assert!(weak.upgrade().is_none());
    });
}

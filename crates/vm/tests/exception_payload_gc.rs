#![cfg(test)]
#![cfg(all(feature = "compiler", feature = "gc"))]

// Keep the collection regression in its own process: collect_force may return
// without collecting when another interpreter holds the global collector lock.
use rustpython_vm::{
    Interpreter, PyPayload,
    builtins::{PyBaseException, PyValueError},
    class::PySubclass,
    types::Constructor,
};

#[test]
fn owned_wrapper_cycles_release_argument_objects() {
    Interpreter::without_stdlib(Default::default()).enter(|vm| {
        let scope = vm.new_scope_with_builtins();
        let source = "class Marker: pass\nmarker = Marker()\n";
        let code = vm
            .compile(source, rustpython_vm::compiler::Mode::Exec, "<cycle>")
            .map_err(|error| error.into_pyexception(vm, Some(source)))
            .unwrap();
        vm.run_code_obj(code, scope.clone()).unwrap();
        let argument = scope.globals.get_item("marker", vm).unwrap();
        let weak = argument.downgrade(None, vm).unwrap();
        let exception =
            PyValueError::py_new(vm.ctx.exceptions.value_error, vec![argument].into(), vm)
                .unwrap()
                .into_ref(&vm.ctx);
        exception
            .as_base()
            .as_base()
            .set_cause(Some(exception.clone().upcast::<PyBaseException>()));
        exception
            .as_base()
            .as_base()
            .set_context(Some(exception.clone().upcast::<PyBaseException>()));
        scope.globals.del_item("marker", vm).unwrap();
        drop(exception);
        vm.state.gc.collect_force(2);
        assert!(weak.upgrade().is_none());
    });
}

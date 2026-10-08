#![allow(clippy::tests_outside_test_module)]

use rustpython::{InterpreterBuilder, InterpreterBuilderExt};

#[test]
fn context_cache_does_not_outlive_its_context() {
    InterpreterBuilder::new()
        .init_stdlib()
        .interpreter()
        .enter(|vm| {
            let source = r#"
import contextvars

for factory_name in ('new', 'copy'):
    for _ in range(100):
        var = contextvars.ContextVar('var', default='default')
        empty = contextvars.Context()
        ctx = contextvars.Context() if factory_name == 'new' else empty.copy()
        ctx.run(var.set, 'cached')
        del ctx
        ctx = contextvars.Context() if factory_name == 'new' else empty.copy()
        assert list(ctx.items()) == []
        assert ctx.run(var.get) == 'default'
        assert ctx.run(var.get, 'explicit default') == 'explicit default'

        missing = contextvars.ContextVar('missing')
        ctx.run(missing.set, 'cached')
        del ctx
        ctx = contextvars.Context() if factory_name == 'new' else empty.copy()
        try:
            ctx.run(missing.get)
        except LookupError:
            pass
        else:
            raise AssertionError('an empty context returned a destroyed context value')
"#;
            if let Err(err) = vm.run_simple_string(source) {
                vm.print_exception(&err);
                panic!("context cache lifetime regression failed");
            }
        });
}

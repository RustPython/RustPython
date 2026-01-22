//! _asyncio module - provides native asyncio support
//!
//! This module uses lazy attribute loading to avoid circular import issues.
//! When attributes like Future or Task are accessed, they are imported from
//! the Python asyncio modules on demand.

use crate::vm::{
    Context, Py, PyResult, VirtualMachine,
    builtins::{PyModule, PyModuleDef, PyModuleSlots},
    common::static_cell,
    compiler,
};

pub(crate) const MODULE_NAME: &str = "_asyncio";

static_cell! {
    static DEF: PyModuleDef;
}

/// Returns the module definition for multi-phase initialization.
pub(crate) fn module_def(ctx: &Context) -> &'static PyModuleDef {
    DEF.get_or_init(|| PyModuleDef {
        name: ctx.intern_str(MODULE_NAME),
        doc: Some(ctx.intern_str("Accelerator module for asyncio")),
        methods: &[],
        slots: PyModuleSlots {
            create: None,
            exec: Some(exec_module),
        },
    })
}

/// Exec phase: Set up the module __getattr__ for lazy attribute loading.
/// We don't import asyncio here to avoid circular import issues.
fn exec_module(vm: &VirtualMachine, module: &Py<PyModule>) -> PyResult<()> {
    // Define __getattr__ function that lazily loads attributes from asyncio
    let getattr_code = r#"
def __getattr__(name):
    import sys
    _asyncio = sys.modules['_asyncio']

    # Mapping of _asyncio attributes to their sources
    if name == 'Future':
        from asyncio.futures import _PyFuture
        setattr(_asyncio, 'Future', _PyFuture)
        return _PyFuture
    elif name == 'Task':
        from asyncio.tasks import _PyTask
        setattr(_asyncio, 'Task', _PyTask)
        return _PyTask
    elif name == 'current_task':
        from asyncio.tasks import _py_current_task
        setattr(_asyncio, 'current_task', _py_current_task)
        return _py_current_task
    elif name == '_register_task':
        from asyncio.tasks import _py_register_task
        setattr(_asyncio, '_register_task', _py_register_task)
        return _py_register_task
    elif name == '_register_eager_task':
        from asyncio.tasks import _py_register_eager_task
        setattr(_asyncio, '_register_eager_task', _py_register_eager_task)
        return _py_register_eager_task
    elif name == '_unregister_task':
        from asyncio.tasks import _py_unregister_task
        setattr(_asyncio, '_unregister_task', _py_unregister_task)
        return _py_unregister_task
    elif name == '_unregister_eager_task':
        from asyncio.tasks import _py_unregister_eager_task
        setattr(_asyncio, '_unregister_eager_task', _py_unregister_eager_task)
        return _py_unregister_eager_task
    elif name == '_enter_task':
        from asyncio.tasks import _py_enter_task
        setattr(_asyncio, '_enter_task', _py_enter_task)
        return _py_enter_task
    elif name == '_leave_task':
        from asyncio.tasks import _py_leave_task
        setattr(_asyncio, '_leave_task', _py_leave_task)
        return _py_leave_task
    elif name == '_swap_current_task':
        from asyncio.tasks import _py_swap_current_task
        setattr(_asyncio, '_swap_current_task', _py_swap_current_task)
        return _py_swap_current_task
    elif name == '_scheduled_tasks':
        from asyncio.tasks import _scheduled_tasks
        setattr(_asyncio, '_scheduled_tasks', _scheduled_tasks)
        return _scheduled_tasks
    elif name == '_eager_tasks':
        from asyncio.tasks import _eager_tasks
        setattr(_asyncio, '_eager_tasks', _eager_tasks)
        return _eager_tasks
    elif name == '_current_tasks':
        from asyncio.tasks import _current_tasks
        setattr(_asyncio, '_current_tasks', _current_tasks)
        return _current_tasks
    elif name == '_get_running_loop':
        from asyncio.events import _py__get_running_loop
        setattr(_asyncio, '_get_running_loop', _py__get_running_loop)
        return _py__get_running_loop
    elif name == '_set_running_loop':
        from asyncio.events import _py__set_running_loop
        setattr(_asyncio, '_set_running_loop', _py__set_running_loop)
        return _py__set_running_loop
    elif name == 'get_running_loop':
        from asyncio.events import _py_get_running_loop
        setattr(_asyncio, 'get_running_loop', _py_get_running_loop)
        return _py_get_running_loop
    elif name == 'get_event_loop':
        from asyncio.events import _py_get_event_loop
        setattr(_asyncio, 'get_event_loop', _py_get_event_loop)
        return _py_get_event_loop

    raise AttributeError(f"module '_asyncio' has no attribute '{name}'")
"#;

    // Execute the code in the module's namespace
    let code = vm
        .compile(getattr_code, compiler::Mode::Exec, "<_asyncio>".to_owned())
        .map_err(|e| vm.new_syntax_error(&e, Some(getattr_code)))?;

    let scope = vm.new_scope_with_builtins();
    vm.run_code_obj(code, scope.clone())?;

    // Get the __getattr__ function and set it on the module
    if let Ok(getattr_func) = scope.globals.get_item("__getattr__", vm) {
        module.set_attr("__getattr__", getattr_func, vm)?;
    }

    Ok(())
}

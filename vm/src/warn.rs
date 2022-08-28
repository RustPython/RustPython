use crate::{
    builtins::{PyDict, PyStrRef, PyType, PyTypeRef},
    AsObject, Py, PyObjectRef, PyResult, VirtualMachine,
};

pub fn py_warn(
    category: &Py<PyType>,
    message: String,
    stack_level: usize,
    vm: &VirtualMachine,
) -> PyResult<()> {
    // TODO: use rust warnings module
    if let Ok(module) = vm.import("warnings", None, 0) {
        if let Ok(func) = module.get_attr("warn", vm) {
            let _ = vm.invoke(&func, (message, category.to_owned(), stack_level));
        }
    }
    Ok(())
}

pub fn warn(
    message: PyStrRef,
    category: Option<PyTypeRef>,
    stack_level: isize,
    source: Option<PyObjectRef>,
    vm: &VirtualMachine,
) -> PyResult<()> {
    let (filename, lineno, module, registry) = setup_context(stack_level, vm)?;
    warn_explicit(
        category, message, filename, lineno, module, registry, None, source, vm,
    )
}

#[allow(clippy::too_many_arguments)]
fn warn_explicit(
    category: Option<PyTypeRef>,
    message: PyStrRef,
    _filename: PyStrRef,
    _lineno: usize,
    _module: PyObjectRef,
    _registry: PyObjectRef,
    _source_line: Option<PyObjectRef>,
    _source: Option<PyObjectRef>,
    vm: &VirtualMachine,
) -> PyResult<()> {
    // TODO: Implement correctly
    let category = if let Some(category) = category {
        if !category.fast_issubclass(vm.ctx.exceptions.warning) {
            return Err(vm.new_type_error(format!(
                "category must be a Warning subclass, not '{}'",
                category.class().name()
            )));
        }
        category
    } else {
        vm.ctx.exceptions.user_warning.to_owned()
    };
    let stderr = crate::stdlib::sys::PyStderr(vm);
    writeln!(stderr, "{}: {}", category.name(), message.as_str(),);
    Ok(())
}

/// filename, module, and registry are new refs, globals is borrowed
/// Returns `Ok` on success, or `Err` on error (no new refs)
fn setup_context(
    mut stack_level: isize,
    vm: &VirtualMachine,
) -> PyResult<
    // filename, lineno, module, registry
    (PyStrRef, usize, PyObjectRef, PyObjectRef),
> {
    let __warningregistry__ = "__warningregistry__";
    let __name__ = "__name__";

    let current_frame = vm.current_frame();
    let mut f = current_frame.as_deref().cloned();

    // Stack level comparisons to Python code is off by one as there is no
    // warnings-related stack level to avoid.

    if let Some(frame) = current_frame {
        if stack_level <= 0 || frame.is_internal_frame() {
            while let Some(tmp) = frame.clone().f_back(vm) {
                stack_level -= 1;
                if stack_level > 0 {
                    break;
                }
                f = Some(tmp);
            }
        } else {
            stack_level -= 1;
            if stack_level <= 0 {
                f = frame.next_external_frame(vm);

                while let Some(frame) = f.clone() {
                    stack_level -= 1;
                    if stack_level > 0 {
                        break;
                    }
                    f = frame.next_external_frame(vm);
                }
            }
        }
    }

    let (globals, filename, lineno) = if let Some(f) = f {
        (f.globals.clone(), f.code.source_path, f.f_lineno())
    } else {
        (vm.current_globals().clone(), vm.ctx.intern_str("sys"), 1)
    };

    let registry = if let Ok(registry) = globals.get_item(__warningregistry__, vm) {
        registry
    } else {
        let registry = PyDict::new_ref(&vm.ctx);
        globals.set_item(__warningregistry__, registry.clone().into(), vm)?;
        registry.into()
    };

    // Setup module.
    let module = globals
        .get_item(__name__, vm)
        .unwrap_or_else(|_| vm.new_pyobj("<string>"));
    Ok((filename.to_owned(), lineno, module, registry))
}

use crate::{
    builtins::{PyDict, PyStrRef, PyType, PyTypeRef},
    frame::FrameRef,
    AsObject, Py, PyObjectRef, PyResult, VirtualMachine,
};
use std::ops::Deref;

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
pub fn setup_context(
    mut stack_level: isize,
    vm: &VirtualMachine,
) -> PyResult<
    // filename, lineno, module, registry
    (PyStrRef, usize, PyObjectRef, PyObjectRef),
> {
    let __warningregistry__ = "__warningregistry__";
    let __name__ = "__name__";

    // for return
    let mut globals = vm.current_globals().clone();
    let mut filename = vm.ctx.intern_str("sys");
    let mut lineno = 1;

    let current_frame = vm.current_frame();
    let mut f: FrameRef;
    if current_frame.is_some() {
        // SAFETY: it's safe
        f = current_frame.as_ref().unwrap().deref().clone();
        // Stack level comparisons to Python code is off by one as there is no
        // warnings-related stack level to avoid.
        if stack_level <= 0 || f.is_internal_frame() {
            while let Some(tmp) = f.clone().f_back(vm) {
                stack_level -= 1;
                if stack_level > 0 {
                    break;
                }
                f = tmp;
            }
        } else {
            let current_frame = f.clone().f_back(vm);
            if let Some(tmp) = current_frame {
                loop {
                    stack_level -= 1;
                    if stack_level > 0 {
                        break;
                    }
                    f = tmp.next_external_frame(vm);
                }
            }
        }

        globals = f.globals.clone();
        filename = f.code.source_path;
        lineno = f.f_lineno();
    }

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

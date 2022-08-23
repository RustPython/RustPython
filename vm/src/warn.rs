use crate::{
    builtins::{PyDict, PyStrRef, PyType, PyTypeRef},
    frame::FrameRef,
    AsObject, Py, PyObjectRef, PyResult, VirtualMachine,
};
use core::slice::Iter;

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

fn is_internal_frame(frame: Option<&FrameRef>, vm: &VirtualMachine) -> bool {
    if let Some(frame) = frame {
        let code = &frame.code;
        let filename = code.source_path;

        let contains = filename.as_str().contains("importlib");
        if contains {
            let contains = filename.as_str().contains("_bootstrap");
            contains
        } else {
            false
        }
    } else {
        false
    }
}

fn next_external_frame(
    frame: Option<&FrameRef>,
    frames: Iter<FrameRef>,
    vm: &VirtualMachine,
) -> Option<&FrameRef> {
    loop {
        frame = frames.next();
        if frame.is_some() && is_internal_frame(frame, vm) {
            break frame;
        }
    }
}

// filename, module, and registry are new refs, globals is borrowed
// Returns 0 on error (no new refs), 1 on success
fn setup_context(
    stack_level: isize,
    vm: &VirtualMachine,
) -> PyResult<
    // filename, lineno, module, registry
    (PyStrRef, usize, PyObjectRef, PyObjectRef),
> {
    let __warningregistry__ = "__warningregistry__";
    let __name__ = "__name__";

    // Setup globals, filename and lineno.
    let frames = vm.frames.borrow().iter();
    let mut frame = frames.next();
    // Stack level comparisons to Python code is off by one as there is no
    // warnings-related stack level to avoid.
    if stack_level <= 0 || is_internal_frame(frame, vm) {
        loop {
            if stack_level -= 1 > 0 && frame.is_some() {
                frame = frames.next();
            } else {
                break;
            }
        }
    } else {
        loop {
            if stack_level -= 1 > 0 && frame.is_some() {
                frame = next_external_frame(frame, frames, vm);
            } else {
                break;
            }
        }
    };

    let (globals, filename, lineno) = if let Some(f) = frame {
        // TODO:
        let lineno = 1;
        let filename = f.code.source_path;
        (f.globals.clone(), filename, lineno)
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

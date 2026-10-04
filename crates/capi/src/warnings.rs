use crate::util::{CStrExt, FfiPtrExt};
use crate::{PyObject, pystate::with_vm};
use core::ffi::{c_char, c_int};
use rustpython_vm::builtins::{PyType, PyTypeRef};
use rustpython_vm::warn::{warn, warn_explicit};
use rustpython_vm::{AsObject, PyResult};

fn resolve_warning_category(
    vm: &rustpython_vm::VirtualMachine,
    category: *mut PyObject,
) -> PyResult<PyTypeRef> {
    if category.is_null() {
        return Ok(vm.ctx.exceptions.runtime_warning.to_owned());
    };

    let category = unsafe { category.assume_borrowed_and_cast::<PyType>(vm) }?.to_owned();
    if !category.fast_issubclass(vm.ctx.exceptions.warning) {
        return Err(vm.new_type_error(format!(
            "category must be a Warning subclass, not '{}'",
            category.class().name()
        )));
    }

    Ok(category)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_WarnEx(
    category: *mut PyObject,
    message: *const c_char,
    stack_level: isize,
) -> c_int {
    with_vm(|vm| {
        let message = unsafe { message.try_as_str(vm) }?;

        let category = resolve_warning_category(vm, category)?;

        warn(
            vm.ctx.new_str(message).into(),
            Some(category),
            stack_level,
            None,
            vm,
        )
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_WarnExplicit(
    category: *mut PyObject,
    message: *const c_char,
    filename: *const c_char,
    lineno: c_int,
    module: *const c_char,
    registry: *mut PyObject,
) -> c_int {
    with_vm(|vm| {
        let message = unsafe { message.try_as_str(vm) }?;
        let filename = unsafe { filename.try_as_str(vm) }?;

        let module =
            unsafe { module.try_as_str_opt(vm) }?.map(|module| vm.ctx.new_str(module).into());

        let category = resolve_warning_category(vm, category)?;

        let registry = unsafe { registry.assume_borrowed_or_opt() }
            .map_or_else(|| vm.ctx.none(), |registry| registry.to_owned());

        let lineno = usize::try_from(lineno)
            .map_err(|_| vm.new_system_error("lineno must be non-negative"))?;

        warn_explicit(
            Some(category),
            vm.ctx.new_str(message).into(),
            vm.ctx.new_str(filename),
            lineno,
            module,
            registry,
            None,
            None,
            vm,
        )
    })
}

#[cfg(test)]
mod tests {
    use pyo3::exceptions::{PyRuntimeWarning, PyUserWarning};
    use pyo3::prelude::*;
    use pyo3::types::PyType;

    #[test]
    fn warn_ex_works() {
        Python::attach(|py| {
            let category = py.get_type::<PyRuntimeWarning>();
            PyErr::warn(py, &category, c"warn ex message", 1).unwrap();
        })
    }

    #[test]
    fn warn_explicit_works() {
        Python::attach(|py| {
            let category = py.get_type::<PyUserWarning>();
            PyErr::warn_explicit(
                py,
                &category,
                c"warn explicit message",
                c"warnings_test.py",
                7,
                Some(c"warnings_test"),
                None,
            )
            .unwrap();
        })
    }

    #[test]
    fn warn_ex_rejects_non_warning_category() {
        Python::attach(|py| {
            let not_warning = py.get_type::<PyType>();
            let err = PyErr::warn(py, &not_warning, c"not warning", 1).unwrap_err();
            assert!(err.is_instance_of::<pyo3::exceptions::PyTypeError>(py));
        })
    }
}

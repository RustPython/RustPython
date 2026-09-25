pub(crate) use _warnings::module_def;

use crate::{Py, PyResult, VirtualMachine, builtins::PyType};

pub fn warn(
    category: &Py<PyType>,
    message: String,
    stack_level: usize,
    vm: &VirtualMachine,
) -> PyResult<()> {
    crate::warn::warn(
        vm.new_pyobj(message),
        Some(category.to_owned()),
        isize::try_from(stack_level).unwrap_or(isize::MAX),
        None,
        vm,
    )
}

#[pymodule]
mod _warnings {
    use crate::{
        AsObject, PyObject, PyObjectRef, PyResult, VirtualMachine,
        builtins::{PyDictRef, PyListRef, PyStrRef, PyTupleRef, PyTypeRef},
        convert::TryFromObject,
        function::OptionalArg,
    };

    #[pyattr]
    fn filters(vm: &VirtualMachine) -> PyListRef {
        vm.state.warnings.filters.to_owned()
    }

    #[pyattr]
    fn _defaultaction(vm: &VirtualMachine) -> PyStrRef {
        vm.state.warnings.default_action.to_owned()
    }

    #[pyattr]
    fn _onceregistry(vm: &VirtualMachine) -> PyDictRef {
        vm.state.warnings.once_registry.to_owned()
    }

    #[pyattr]
    fn _warnings_context(vm: &VirtualMachine) -> PyObjectRef {
        if let Some(ctx) = vm.state.warnings.context_var.get() {
            return ctx.clone();
        }
        // _warnings is initialized before _contextvars may be importable.
        // Retry until ContextVar can be created; do not cache a None fallback.
        let created = vm
            .import("_contextvars", 0)
            .ok()
            .and_then(|m| m.get_attr("ContextVar", vm).ok())
            .and_then(|cv_cls| cv_cls.call(("_warnings_context",), vm).ok());
        match created {
            Some(cv) => match vm.state.warnings.context_var.set(cv.clone()) {
                Ok(()) => cv,
                Err(_) => vm.state.warnings.context_var.get().cloned().unwrap_or(cv),
            },
            None => vm.ctx.none(),
        }
    }

    #[pyfunction]
    fn _acquire_lock(vm: &VirtualMachine) {
        vm.state.warnings.acquire_lock();
    }

    #[pyfunction]
    fn _release_lock(vm: &VirtualMachine) -> PyResult<()> {
        if !vm.state.warnings.release_lock() {
            return Err(vm.new_runtime_error("cannot release un-acquired lock"));
        }
        Ok(())
    }

    #[pyfunction]
    fn _filters_mutated_lock_held(vm: &VirtualMachine) {
        vm.state.warnings.filters_mutated();
    }

    #[derive(FromArgs)]
    struct WarnArgs {
        #[pyarg(any)]
        message: PyObjectRef,
        #[pyarg(any, optional)]
        category: OptionalArg<PyObjectRef>,
        #[pyarg(any, default = 1)]
        stacklevel: i32,
        #[pyarg(any, optional)]
        source: OptionalArg<PyObjectRef>,
        #[pyarg(named, optional, py_default = "<unrepresentable>")]
        skip_file_prefixes: OptionalArg<PyTupleRef>,
    }

    /// Validate and resolve the category argument, matching get_category() in C.
    fn get_category(
        message: &PyObject,
        category: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyTypeRef>> {
        let cat_obj = match category {
            Some(c) if !vm.is_none(&c) => c,
            _ => {
                return Ok(if message.fast_isinstance(vm.ctx.exceptions.warning) {
                    Some(message.class().to_owned())
                } else {
                    None // will default to UserWarning in warn_explicit
                });
            }
        };

        let cat = PyTypeRef::try_from_object(vm, cat_obj.clone()).map_err(|_| {
            vm.new_type_error(format!(
                "category must be a Warning subclass, not '{}'",
                cat_obj.class().name()
            ))
        })?;

        if !cat.fast_issubclass(vm.ctx.exceptions.warning) {
            return Err(vm.new_type_error(format!(
                "category must be a Warning subclass, not '{}'",
                cat.class().name()
            )));
        }

        Ok(Some(cat))
    }

    #[pyfunction]
    fn warn(args: WarnArgs, vm: &VirtualMachine) -> PyResult<()> {
        let level = args.stacklevel as isize;

        let category = get_category(&args.message, args.category.into_option(), vm)?;

        // Validate skip_file_prefixes: each element must be a str
        let skip_prefixes = args.skip_file_prefixes.into_option();
        if let Some(ref prefixes) = skip_prefixes {
            for item in prefixes.iter() {
                if !item.class().is(vm.ctx.types.str_type) {
                    return Err(vm.new_type_error("skip_file_prefixes must be a tuple of strs"));
                }
            }
        }

        crate::warn::warn_with_skip(
            args.message,
            category,
            level,
            args.source.into_option(),
            skip_prefixes.as_deref(),
            vm,
        )
    }

    #[derive(FromArgs)]
    struct WarnExplicitArgs {
        #[pyarg(any)]
        message: PyObjectRef,
        #[pyarg(any)]
        category: PyObjectRef,
        #[pyarg(any)]
        filename: PyStrRef,
        #[pyarg(any)]
        lineno: usize,
        #[pyarg(any, optional, py_default = "<unrepresentable>")]
        module: OptionalArg<PyObjectRef>,
        #[pyarg(any, optional)]
        registry: OptionalArg<PyObjectRef>,
        #[pyarg(any, optional)]
        module_globals: OptionalArg<PyObjectRef>,
        #[pyarg(any, optional)]
        source: OptionalArg<PyObjectRef>,
    }

    #[pyfunction]
    fn warn_explicit(args: WarnExplicitArgs, vm: &VirtualMachine) -> PyResult<()> {
        let registry = args.registry.into_option().unwrap_or_else(|| vm.ctx.none());

        let module = args.module.into_option();

        let source_line = if let Some(mg) = args.module_globals.into_option() {
            if vm.is_none(&mg) {
                None
            } else if !mg.class().is(vm.ctx.types.dict_type) {
                return Err(vm.new_type_error(format!(
                    "module_globals must be a dict, not '{}'",
                    mg.class().name()
                )));
            } else {
                crate::warn::get_source_line(&mg, args.lineno, vm)?
            }
        } else {
            None
        };

        let category = if vm.is_none(&args.category) {
            None
        } else {
            Some(
                PyTypeRef::try_from_object(vm, args.category)
                    .map_err(|_| vm.new_type_error("category must be a Warning subclass"))?,
            )
        };

        crate::warn::warn_explicit(
            category,
            args.message,
            args.filename,
            args.lineno,
            module,
            registry,
            source_line,
            args.source.into_option(),
            vm,
        )
    }
}

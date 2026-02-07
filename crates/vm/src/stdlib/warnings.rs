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
        AsObject, PyObjectRef, PyResult, VirtualMachine,
        builtins::{PyDictRef, PyListRef, PyStrRef, PyTupleRef, PyTypeRef},
        convert::TryFromObject,
        function::OptionalArg,
    };

    #[pyattr]
    fn filters(vm: &VirtualMachine) -> PyListRef {
        vm.state.warnings.filters.clone()
    }

    #[pyattr]
    fn _defaultaction(vm: &VirtualMachine) -> PyStrRef {
        vm.state.warnings.default_action.clone()
    }

    #[pyattr]
    fn _onceregistry(vm: &VirtualMachine) -> PyDictRef {
        vm.state.warnings.once_registry.clone()
    }

    #[pyattr]
    fn _warnings_context(vm: &VirtualMachine) -> PyObjectRef {
        vm.state
            .warnings
            .context_var
            .get_or_init(|| {
                // Try to create a real ContextVar if _contextvars is available.
                // During early startup it may not be importable yet, in which
                // case we fall back to None.  This is safe because
                // context_aware_warnings defaults to False.
                if let Ok(contextvars) = vm.import("_contextvars", 0)
                    && let Ok(cv_cls) = contextvars.get_attr("ContextVar", vm)
                    && let Ok(cv) = cv_cls.call(("_warnings_context",), vm)
                {
                    cv
                } else {
                    vm.ctx.none()
                }
            })
            .clone()
    }

    #[pyfunction]
    fn _acquire_lock(vm: &VirtualMachine) {
        vm.state.warnings.acquire_lock();
    }

    #[pyfunction]
    fn _release_lock(vm: &VirtualMachine) -> PyResult<()> {
        if !vm.state.warnings.release_lock() {
            return Err(vm.new_runtime_error("cannot release un-acquired lock".to_owned()));
        }
        Ok(())
    }

    #[pyfunction]
    fn _filters_mutated_lock_held(vm: &VirtualMachine) {
        vm.state.warnings.filters_mutated();
    }

    #[derive(FromArgs)]
    struct WarnArgs {
        #[pyarg(positional)]
        message: PyObjectRef,
        #[pyarg(any, optional)]
        category: OptionalArg<PyObjectRef>,
        #[pyarg(any, optional)]
        stacklevel: OptionalArg<i32>,
        #[pyarg(named, optional)]
        source: OptionalArg<PyObjectRef>,
        #[pyarg(named, optional)]
        skip_file_prefixes: OptionalArg<PyTupleRef>,
    }

    /// Validate and resolve the category argument, matching get_category() in C.
    fn get_category(
        message: &PyObjectRef,
        category: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyTypeRef>> {
        let cat_obj = match category {
            Some(c) if !vm.is_none(&c) => c,
            _ => {
                if message.fast_isinstance(vm.ctx.exceptions.warning) {
                    return Ok(Some(message.class().to_owned()));
                } else {
                    return Ok(None); // will default to UserWarning in warn_explicit
                }
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
        let level = args.stacklevel.unwrap_or(1) as isize;

        let category = get_category(&args.message, args.category.into_option(), vm)?;

        // Validate skip_file_prefixes: each element must be a str
        let skip_prefixes = args.skip_file_prefixes.into_option();
        if let Some(ref prefixes) = skip_prefixes {
            for item in prefixes.iter() {
                if !item.class().is(vm.ctx.types.str_type) {
                    return Err(
                        vm.new_type_error("skip_file_prefixes must be a tuple of strs".to_owned())
                    );
                }
            }
        }

        crate::warn::warn_with_skip(
            args.message,
            category,
            level,
            args.source.into_option(),
            skip_prefixes,
            vm,
        )
    }

    #[derive(FromArgs)]
    struct WarnExplicitArgs {
        #[pyarg(positional)]
        message: PyObjectRef,
        #[pyarg(positional)]
        category: PyObjectRef,
        #[pyarg(positional)]
        filename: PyStrRef,
        #[pyarg(positional)]
        lineno: usize,
        #[pyarg(any, optional)]
        module: OptionalArg<PyObjectRef>,
        #[pyarg(any, optional)]
        registry: OptionalArg<PyObjectRef>,
        #[pyarg(any, optional)]
        module_globals: OptionalArg<PyObjectRef>,
        #[pyarg(named, optional)]
        source: OptionalArg<PyObjectRef>,
    }

    #[pyfunction]
    fn warn_explicit(args: WarnExplicitArgs, vm: &VirtualMachine) -> PyResult<()> {
        let registry = args.registry.into_option().unwrap_or_else(|| vm.ctx.none());

        let module = args.module.into_option();

        // Validate module_globals: must be None or a dict
        if let Some(ref mg) = args.module_globals.into_option()
            && !vm.is_none(mg)
            && !mg.class().is(vm.ctx.types.dict_type)
        {
            return Err(vm.new_type_error("module_globals must be a dict".to_owned()));
        }

        let category =
            if vm.is_none(&args.category) {
                None
            } else {
                Some(PyTypeRef::try_from_object(vm, args.category).map_err(|_| {
                    vm.new_type_error("category must be a Warning subclass".to_owned())
                })?)
            };

        crate::warn::warn_explicit(
            category,
            args.message,
            args.filename,
            args.lineno,
            module,
            registry,
            None, // source_line
            args.source.into_option(),
            vm,
        )
    }
}

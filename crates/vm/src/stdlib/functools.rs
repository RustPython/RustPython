pub(crate) use _functools::module_def;

#[pymodule]
mod _functools {
    use crate::{
        Py, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
        builtins::{PyBoundMethod, PyDict, PyGenericAlias, PyTuple, PyType, PyTypeRef},
        common::lock::PyRwLock,
        function::{FuncArgs, KwArgs, OptionalOption},
        object::AsObject,
        protocol::PyIter,
        pyclass,
        recursion::ReprGuard,
        types::{Callable, Constructor, GetDescriptor, Representable},
    };
    use indexmap::IndexMap;

    #[derive(FromArgs)]
    struct ReduceArgs {
        function: PyObjectRef,
        iterator: PyIter,
        #[pyarg(any, optional, name = "initial")]
        initial: OptionalOption<PyObjectRef>,
    }

    #[pyfunction]
    fn reduce(args: ReduceArgs, vm: &VirtualMachine) -> PyResult {
        let ReduceArgs {
            function,
            iterator,
            initial,
        } = args;
        let mut iter = iterator.iter_without_hint(vm)?;
        // OptionalOption distinguishes between:
        // - Missing: no argument provided → use first element from iterator
        // - Present(None): explicitly passed None → use None as initial value
        // - Present(Some(v)): passed a value → use that value
        let start_value = if let Some(val) = initial.into_option() {
            // initial was provided (could be None or Some value)
            val.unwrap_or_else(|| vm.ctx.none())
        } else {
            // initial was not provided at all
            iter.next().transpose()?.ok_or_else(|| {
                let exc_type = vm.ctx.exceptions.type_error.to_owned();
                vm.new_exception_msg(
                    exc_type,
                    "reduce() of empty sequence with no initial value".to_owned(),
                )
            })?
        };

        let mut accumulator = start_value;
        for next_obj in iter {
            accumulator = function.call((accumulator, next_obj?), vm)?
        }
        Ok(accumulator)
    }

    // Placeholder singleton for partial arguments
    // The singleton is stored as _instance on the type class
    #[pyattr]
    #[allow(non_snake_case)]
    fn Placeholder(vm: &VirtualMachine) -> PyObjectRef {
        let placeholder = PyPlaceholderType.into_pyobject(vm);
        // Store the singleton on the type class for slot_new to find
        let typ = placeholder.class();
        typ.set_attr(vm.ctx.intern_str("_instance"), placeholder.clone());
        placeholder
    }

    #[pyattr]
    #[pyclass(name = "_PlaceholderType", module = "functools")]
    #[derive(Debug, PyPayload)]
    pub struct PyPlaceholderType;

    impl Constructor for PyPlaceholderType {
        type Args = FuncArgs;

        fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            if !args.args.is_empty() || !args.kwargs.is_empty() {
                return Err(vm.new_type_error("_PlaceholderType takes no arguments".to_owned()));
            }
            // Return the singleton stored on the type class
            if let Some(instance) = cls.get_attr(vm.ctx.intern_str("_instance")) {
                return Ok(instance);
            }
            // Fallback: create a new instance (shouldn't happen for base type after module init)
            Ok(PyPlaceholderType.into_pyobject(vm))
        }

        fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
            // This is never called because we override slot_new
            Ok(PyPlaceholderType)
        }
    }

    #[pyclass(with(Constructor, Representable))]
    impl PyPlaceholderType {
        #[pymethod]
        fn __reduce__(&self) -> &'static str {
            "Placeholder"
        }

        #[pymethod]
        fn __init_subclass__(_cls: PyTypeRef, vm: &VirtualMachine) -> PyResult<()> {
            Err(vm.new_type_error("cannot subclass '_PlaceholderType'".to_owned()))
        }
    }

    impl Representable for PyPlaceholderType {
        #[inline]
        fn repr_str(_zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            Ok("Placeholder".to_owned())
        }
    }

    fn is_placeholder(obj: &PyObjectRef) -> bool {
        &*obj.class().name() == "_PlaceholderType"
    }

    fn count_placeholders(args: &[PyObjectRef]) -> usize {
        args.iter().filter(|a| is_placeholder(a)).count()
    }

    #[pyattr]
    #[pyclass(name = "partial", module = "functools")]
    #[derive(Debug, PyPayload)]
    pub struct PyPartial {
        inner: PyRwLock<PyPartialInner>,
    }

    #[derive(Debug)]
    struct PyPartialInner {
        func: PyObjectRef,
        args: PyRef<PyTuple>,
        keywords: PyRef<PyDict>,
        phcount: usize,
    }

    #[pyclass(
        with(Constructor, Callable, GetDescriptor, Representable),
        flags(BASETYPE, HAS_DICT)
    )]
    impl PyPartial {
        #[pygetset]
        fn func(&self) -> PyObjectRef {
            self.inner.read().func.clone()
        }

        #[pygetset]
        fn args(&self) -> PyRef<PyTuple> {
            self.inner.read().args.clone()
        }

        #[pygetset]
        fn keywords(&self) -> PyRef<PyDict> {
            self.inner.read().keywords.clone()
        }

        #[pymethod]
        fn __reduce__(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult {
            let inner = zelf.inner.read();
            let partial_type = zelf.class();

            // Get __dict__ if it exists and is not empty
            let dict_obj = match zelf.as_object().dict() {
                Some(dict) if !dict.is_empty() => dict.into(),
                _ => vm.ctx.none(),
            };

            let state = vm.ctx.new_tuple(vec![
                inner.func.clone(),
                inner.args.clone().into(),
                inner.keywords.clone().into(),
                dict_obj,
            ]);
            Ok(vm
                .ctx
                .new_tuple(vec![
                    partial_type.to_owned().into(),
                    vm.ctx.new_tuple(vec![inner.func.clone()]).into(),
                    state.into(),
                ])
                .into())
        }

        #[pymethod]
        fn __setstate__(zelf: &Py<Self>, state: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            let state_tuple = state
                .downcast::<PyTuple>()
                .map_err(|_| vm.new_type_error("argument to __setstate__ must be a tuple"))?;

            if state_tuple.len() != 4 {
                return Err(vm.new_type_error(format!(
                    "expected 4 items in state, got {}",
                    state_tuple.len()
                )));
            }

            let func = &state_tuple[0];
            let args = &state_tuple[1];
            let kwds = &state_tuple[2];
            let dict = &state_tuple[3];

            if !func.is_callable() {
                return Err(vm.new_type_error("invalid partial state"));
            }

            // Validate that args is a tuple (or subclass)
            if !args.fast_isinstance(vm.ctx.types.tuple_type) {
                return Err(vm.new_type_error("invalid partial state"));
            }
            // Always convert to base tuple, even if it's a subclass
            let args_tuple = match args.clone().downcast::<PyTuple>() {
                Ok(tuple) if tuple.class().is(vm.ctx.types.tuple_type) => tuple,
                _ => {
                    // It's a tuple subclass, convert to base tuple
                    let elements: Vec<PyObjectRef> = args.try_to_value(vm)?;
                    vm.ctx.new_tuple(elements)
                }
            };

            let keywords_dict = if kwds.is(&vm.ctx.none) {
                vm.ctx.new_dict()
            } else {
                // Always convert to base dict, even if it's a subclass
                let dict = kwds
                    .clone()
                    .downcast::<PyDict>()
                    .map_err(|_| vm.new_type_error("invalid partial state"))?;
                if dict.class().is(vm.ctx.types.dict_type) {
                    // It's already a base dict
                    dict
                } else {
                    // It's a dict subclass, convert to base dict
                    let new_dict = vm.ctx.new_dict();
                    for (key, value) in dict {
                        new_dict.set_item(&*key, value, vm)?;
                    }
                    new_dict
                }
            };

            // Validate no trailing placeholders
            let args_slice = args_tuple.as_slice();
            if !args_slice.is_empty() && is_placeholder(args_slice.last().unwrap()) {
                return Err(vm.new_type_error("trailing Placeholders are not allowed".to_owned()));
            }
            let phcount = count_placeholders(args_slice);

            // Actually update the state
            let mut inner = zelf.inner.write();
            inner.func = func.clone();
            // Handle args - use the already validated tuple
            inner.args = args_tuple;

            // Handle keywords - keep the original type
            inner.keywords = keywords_dict;
            inner.phcount = phcount;

            // Update __dict__ if provided
            let Some(instance_dict) = zelf.as_object().dict() else {
                return Ok(());
            };

            if dict.is(&vm.ctx.none) {
                // If dict is None, clear the instance dict
                instance_dict.clear();
                return Ok(());
            }

            let dict_obj = dict
                .clone()
                .downcast::<PyDict>()
                .map_err(|_| vm.new_type_error("invalid partial state"))?;

            // Clear existing dict and update with new values
            instance_dict.clear();
            for (key, value) in dict_obj {
                instance_dict.set_item(&*key, value, vm)?;
            }

            Ok(())
        }

        #[pyclassmethod]
        fn __class_getitem__(
            cls: PyTypeRef,
            args: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyGenericAlias {
            PyGenericAlias::from_args(cls, args, vm)
        }
    }

    impl Constructor for PyPartial {
        type Args = FuncArgs;

        fn py_new(
            _cls: &crate::Py<crate::builtins::PyType>,
            args: Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult<Self> {
            let (func, args_slice) = args
                .args
                .split_first()
                .ok_or_else(|| vm.new_type_error("partial expected at least 1 argument, got 0"))?;

            if !func.is_callable() {
                return Err(vm.new_type_error("the first argument must be callable"));
            }

            // Check for placeholders in kwargs
            for (key, value) in &args.kwargs {
                if is_placeholder(value) {
                    return Err(vm.new_type_error(format!(
                        "Placeholder cannot be passed as a keyword argument to partial(). \
                         Did you mean partial(..., {}=Placeholder, ...)(value)?",
                        key
                    )));
                }
            }

            // Handle nested partial objects
            let (final_func, final_args, final_keywords) =
                if let Some(partial) = func.downcast_ref::<Self>() {
                    let inner = partial.inner.read();
                    let stored_args = inner.args.as_slice();

                    // Merge placeholders: replace placeholders in stored_args with new args
                    let mut merged_args = Vec::with_capacity(stored_args.len() + args_slice.len());
                    let mut new_args_iter = args_slice.iter();

                    for stored_arg in stored_args {
                        if is_placeholder(stored_arg) {
                            // Replace placeholder with next new arg, or keep placeholder
                            if let Some(new_arg) = new_args_iter.next() {
                                merged_args.push(new_arg.clone());
                            } else {
                                merged_args.push(stored_arg.clone());
                            }
                        } else {
                            merged_args.push(stored_arg.clone());
                        }
                    }
                    // Append remaining new args
                    merged_args.extend(new_args_iter.cloned());

                    (inner.func.clone(), merged_args, inner.keywords.clone())
                } else {
                    (func.clone(), args_slice.to_vec(), vm.ctx.new_dict())
                };

            // Trailing placeholders are not allowed
            if !final_args.is_empty() && is_placeholder(final_args.last().unwrap()) {
                return Err(vm.new_type_error("trailing Placeholders are not allowed".to_owned()));
            }

            let phcount = count_placeholders(&final_args);

            // Add new keywords
            for (key, value) in args.kwargs {
                final_keywords.set_item(vm.ctx.intern_str(key.as_str()), value, vm)?;
            }

            Ok(Self {
                inner: PyRwLock::new(PyPartialInner {
                    func: final_func,
                    args: vm.ctx.new_tuple(final_args),
                    keywords: final_keywords,
                    phcount,
                }),
            })
        }
    }

    impl Callable for PyPartial {
        type Args = FuncArgs;

        fn call(zelf: &Py<Self>, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            // Clone and release lock before calling Python code to prevent deadlock
            let (func, stored_args, keywords, phcount) = {
                let inner = zelf.inner.read();
                (
                    inner.func.clone(),
                    inner.args.clone(),
                    inner.keywords.clone(),
                    inner.phcount,
                )
            };

            // Check if we have enough args to fill placeholders
            if phcount > 0 && args.args.len() < phcount {
                return Err(vm.new_type_error(format!(
                    "missing positional arguments in 'partial' call; expected at least {}, got {}",
                    phcount,
                    args.args.len()
                )));
            }

            // Build combined args, replacing placeholders
            let mut combined_args = Vec::with_capacity(stored_args.len() + args.args.len());
            let mut new_args_iter = args.args.iter();

            for stored_arg in stored_args.as_slice() {
                if is_placeholder(stored_arg) {
                    // Replace placeholder with next new arg
                    if let Some(new_arg) = new_args_iter.next() {
                        combined_args.push(new_arg.clone());
                    } else {
                        // This shouldn't happen if phcount check passed
                        combined_args.push(stored_arg.clone());
                    }
                } else {
                    combined_args.push(stored_arg.clone());
                }
            }
            // Append remaining new args
            combined_args.extend(new_args_iter.cloned());

            // Merge keywords from self.keywords and args.kwargs
            let mut final_kwargs = IndexMap::new();

            // Add keywords from self.keywords
            for (key, value) in &*keywords {
                let key_str = key
                    .downcast::<crate::builtins::PyStr>()
                    .map_err(|_| vm.new_type_error("keywords must be strings"))?;
                final_kwargs.insert(key_str.as_str().to_owned(), value);
            }

            // Add keywords from args.kwargs (these override self.keywords)
            for (key, value) in args.kwargs {
                final_kwargs.insert(key, value);
            }

            func.call(FuncArgs::new(combined_args, KwArgs::new(final_kwargs)), vm)
        }
    }

    impl GetDescriptor for PyPartial {
        fn descr_get(
            zelf: PyObjectRef,
            obj: Option<PyObjectRef>,
            _cls: Option<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult {
            let obj = match obj {
                Some(obj) if !vm.is_none(&obj) => obj,
                _ => return Ok(zelf),
            };
            Ok(PyBoundMethod::new(obj, zelf).into_ref(&vm.ctx).into())
        }
    }

    impl Representable for PyPartial {
        #[inline]
        fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
            // Check for recursive repr
            let obj = zelf.as_object();
            if let Some(_guard) = ReprGuard::enter(vm, obj) {
                // Clone and release lock before calling Python code to prevent deadlock
                let (func, args, keywords) = {
                    let inner = zelf.inner.read();
                    (
                        inner.func.clone(),
                        inner.args.clone(),
                        inner.keywords.clone(),
                    )
                };

                let func_repr = func.repr(vm)?;
                let mut parts = vec![func_repr.as_str().to_owned()];

                for arg in args.as_slice() {
                    parts.push(arg.repr(vm)?.as_str().to_owned());
                }

                for (key, value) in &*keywords {
                    // For string keys, use them directly without quotes
                    let key_part = if let Ok(s) = key.clone().downcast::<crate::builtins::PyStr>() {
                        s.as_str().to_owned()
                    } else {
                        // For non-string keys, convert to string using __str__
                        key.str(vm)?.as_str().to_owned()
                    };
                    let value_str = value.repr(vm)?;
                    parts.push(format!(
                        "{key_part}={value_str}",
                        value_str = value_str.as_str()
                    ));
                }

                let qualname = zelf.class().__qualname__(vm);
                let qualname_str = qualname
                    .downcast::<crate::builtins::PyStr>()
                    .map(|s| s.as_str().to_owned())
                    .unwrap_or_else(|_| zelf.class().name().to_owned());
                let module = zelf.class().__module__(vm);

                let qualified_name = match module.downcast::<crate::builtins::PyStr>() {
                    Ok(module_str) => {
                        let module_name = module_str.as_str();
                        match module_name {
                            "builtins" | "" => qualname_str,
                            _ => format!("{module_name}.{qualname_str}"),
                        }
                    }
                    Err(_) => qualname_str,
                };

                Ok(format!(
                    "{qualified_name}({parts})",
                    parts = parts.join(", ")
                ))
            } else {
                Ok("...".to_owned())
            }
        }
    }
}

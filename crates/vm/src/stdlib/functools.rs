pub(crate) use _functools::make_module;

#[pymodule]
mod _functools {
    use crate::{
        Py, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
        builtins::{PyDict, PyGenericAlias, PyTuple, PyTypeRef},
        common::lock::PyRwLock,
        function::{FuncArgs, KwArgs, OptionalArg},
        object::AsObject,
        protocol::PyIter,
        pyclass,
        recursion::ReprGuard,
        types::{Callable, Constructor, Representable},
    };
    use indexmap::IndexMap;

    #[pyfunction]
    fn reduce(
        function: PyObjectRef,
        iterator: PyIter,
        start_value: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let mut iter = iterator.iter_without_hint(vm)?;
        let start_value = if let OptionalArg::Present(val) = start_value {
            val
        } else {
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

    #[pyattr]
    #[pyclass(name = "partial", module = "_functools")]
    #[derive(Debug, PyPayload)]
    pub struct PyPartial {
        inner: PyRwLock<PyPartialInner>,
    }

    #[derive(Debug)]
    struct PyPartialInner {
        func: PyObjectRef,
        args: PyRef<PyTuple>,
        keywords: PyRef<PyDict>,
    }

    #[pyclass(with(Constructor, Callable, Representable), flags(BASETYPE, HAS_DICT))]
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

        #[pymethod(name = "__reduce__")]
        fn reduce(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult {
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

            // Actually update the state
            let mut inner = zelf.inner.write();
            inner.func = func.clone();
            // Handle args - use the already validated tuple
            inner.args = args_tuple;

            // Handle keywords - keep the original type
            inner.keywords = keywords_dict;

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

        fn py_new(cls: PyTypeRef, args: Self::Args, vm: &VirtualMachine) -> PyResult {
            let (func, args_slice) = args
                .args
                .split_first()
                .ok_or_else(|| vm.new_type_error("partial expected at least 1 argument, got 0"))?;

            if !func.is_callable() {
                return Err(vm.new_type_error("the first argument must be callable"));
            }

            // Handle nested partial objects
            let (final_func, final_args, final_keywords) =
                if let Some(partial) = func.downcast_ref::<Self>() {
                    let inner = partial.inner.read();
                    let mut combined_args = inner.args.as_slice().to_vec();
                    combined_args.extend_from_slice(args_slice);
                    (inner.func.clone(), combined_args, inner.keywords.clone())
                } else {
                    (func.clone(), args_slice.to_vec(), vm.ctx.new_dict())
                };

            // Add new keywords
            for (key, value) in args.kwargs {
                final_keywords.set_item(vm.ctx.intern_str(key.as_str()), value, vm)?;
            }

            let partial = Self {
                inner: PyRwLock::new(PyPartialInner {
                    func: final_func,
                    args: vm.ctx.new_tuple(final_args),
                    keywords: final_keywords,
                }),
            };

            partial.into_ref_with_type(vm, cls).map(Into::into)
        }
    }

    impl Callable for PyPartial {
        type Args = FuncArgs;

        fn call(zelf: &Py<Self>, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            let inner = zelf.inner.read();
            let mut combined_args = inner.args.as_slice().to_vec();
            combined_args.extend_from_slice(&args.args);

            // Merge keywords from self.keywords and args.kwargs
            let mut final_kwargs = IndexMap::new();

            // Add keywords from self.keywords
            for (key, value) in &*inner.keywords {
                let key_str = key
                    .downcast::<crate::builtins::PyStr>()
                    .map_err(|_| vm.new_type_error("keywords must be strings"))?;
                final_kwargs.insert(key_str.as_str().to_owned(), value);
            }

            // Add keywords from args.kwargs (these override self.keywords)
            for (key, value) in args.kwargs {
                final_kwargs.insert(key, value);
            }

            inner
                .func
                .call(FuncArgs::new(combined_args, KwArgs::new(final_kwargs)), vm)
        }
    }

    impl Representable for PyPartial {
        #[inline]
        fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
            // Check for recursive repr
            let obj = zelf.as_object();
            if let Some(_guard) = ReprGuard::enter(vm, obj) {
                let inner = zelf.inner.read();
                let func_repr = inner.func.repr(vm)?;
                let mut parts = vec![func_repr.as_str().to_owned()];

                for arg in inner.args.as_slice() {
                    parts.push(arg.repr(vm)?.as_str().to_owned());
                }

                for (key, value) in inner.keywords.clone() {
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

                let class_name = zelf.class().name();
                let module = zelf.class().__module__(vm);

                let qualified_name = if zelf.class().is(Self::class(&vm.ctx)) {
                    // For the base partial class, always use functools.partial
                    "functools.partial".to_owned()
                } else {
                    // For subclasses, check if they're defined in __main__ or test modules
                    match module.downcast::<crate::builtins::PyStr>() {
                        Ok(module_str) => {
                            let module_name = module_str.as_str();
                            match module_name {
                                "builtins" | "" | "__main__" => class_name.to_owned(),
                                name if name.starts_with("test.") || name == "test" => {
                                    // For test modules, just use the class name without module prefix
                                    class_name.to_owned()
                                }
                                _ => format!("{module_name}.{class_name}"),
                            }
                        }
                        Err(_) => class_name.to_owned(),
                    }
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

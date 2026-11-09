pub(crate) use _functools::module_def;

#[pymodule]
mod _functools {
    use crate::{
        Context, Py, PyObject, PyObjectRef, PyPayload, PyResult, VirtualMachine,
        builtins::{
            PyBoundMethod, PyDict, PyDictRef, PyGenericAlias, PyStr, PyTuple, PyType, PyTypeRef,
            object,
        },
        common::{hash::PyHash, lock::PyRwLock},
        function::{
            ArgIntoBool, Callee, Either, FuncArgs, KwArgs, OptionalOption, PosArgs,
            PyComparisonValue, PySetterValue,
        },
        object::{AsObject, PyAtomicRef, TraverseFn},
        protocol::PyIter,
        pyclass,
        recursion::ReprGuard,
        types::{Callable, Constructor, GetDescriptor, PyComparisonOp, Representable},
    };
    use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use parking_lot::lock_api::RawReentrantMutex as GenericRawReentrantMutex;
    use rustpython_common::wtf8::Wtf8Buf;

    /// Reentrant raw mutex used to guard the LRU cache's lookup/insert critical
    /// sections. Defined locally (rather than reusing `stdlib::_thread::RawRMutex`)
    /// because that type is only available when the `threading` feature is enabled,
    /// while `parking_lot` itself is always a dependency.
    type RawRMutex = GenericRawReentrantMutex<parking_lot::RawMutex, parking_lot::RawThreadId>;

    #[derive(FromArgs)]
    struct ReduceArgs {
        #[pyarg(positional)]
        function: PyObjectRef,
        #[pyarg(positional)]
        iterable: PyIter,
        #[pyarg(any, optional, py_default = "functools._initial_missing")]
        initial: OptionalOption<PyObjectRef>,
    }

    #[pyfunction]
    fn reduce(args: ReduceArgs, vm: &VirtualMachine) -> PyResult {
        let ReduceArgs {
            function,
            iterable,
            initial,
        } = args;
        let mut iter = iterable.iter(vm)?;
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
                vm.new_type_error("reduce() of empty iterable with no initial value")
            })?
        };

        let mut accumulator = start_value;
        for next_obj in iter {
            accumulator = function.call((accumulator, next_obj?), vm)?
        }
        Ok(accumulator)
    }

    #[derive(FromArgs)]
    struct CmpToKeyArgs {
        mycmp: PyObjectRef,
    }

    #[pyfunction]
    fn cmp_to_key(args: CmpToKeyArgs) -> PyKeyWrapper {
        PyKeyWrapper {
            cmp: args.mycmp,
            object: PyRwLock::new(None),
        }
    }

    #[derive(FromArgs)]
    struct KeyWrapperCallArgs {
        obj: PyObjectRef,
    }

    #[pyclass(
        no_attr,
        name = "KeyWrapper",
        module = "functools",
        unhashable = true,
        traverse = "manual"
    )]
    #[derive(Debug, PyPayload)]
    struct PyKeyWrapper {
        cmp: PyObjectRef,
        object: PyRwLock<Option<PyObjectRef>>,
    }

    // SAFETY: Each owned reference is visited once, without cloning it.
    unsafe impl crate::object::Traverse for PyKeyWrapper {
        fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
            self.cmp.traverse(tracer_fn);
            self.object.traverse(tracer_fn);
        }

        fn clear_refs(&mut self, out: &mut Vec<PyObjectRef>) {
            out.push(core::mem::replace(&mut self.cmp, Context::genesis().none()));
            if let Some(object) = self.object.get_mut().take() {
                out.push(object);
            }
        }
    }

    #[pyclass(with(Callable), flags(IMMUTABLETYPE, DISALLOW_INSTANTIATION))]
    impl Py<PyKeyWrapper> {
        #[pygetset]
        fn obj(&self, vm: &VirtualMachine) -> PyObjectRef {
            self.object.read().clone().unwrap_or_else(|| vm.ctx.none())
        }

        #[pygetset(setter)]
        fn set_obj(&self, value: PySetterValue) {
            let value = match value {
                PySetterValue::Assign(v) => Some(v),
                PySetterValue::Delete => None,
            };
            let _old = core::mem::replace(&mut *self.object.write(), value);
        }

        #[pygetset]
        fn __text_signature__(&self) -> &'static str {
            "(obj)"
        }

        #[pyslot]
        fn slot_richcompare(
            zelf: &PyObject,
            other: &PyObject,
            op: PyComparisonOp,
            vm: &VirtualMachine,
        ) -> PyResult<Either<PyObjectRef, PyComparisonValue>> {
            let Some(zelf) = zelf.downcast_ref::<PyKeyWrapper>() else {
                return Err(vm.new_type_error(format!(
                    "unexpected payload for {}",
                    op.method_name(&vm.ctx).as_str()
                )));
            };
            let Some(other) = other.downcast_ref::<PyKeyWrapper>() else {
                return Err(vm.new_type_error("other argument must be K instance"));
            };
            let x = zelf
                .object
                .read()
                .clone()
                .ok_or_else(|| vm.new_attribute_error("object"))?;
            let y = other
                .object
                .read()
                .clone()
                .ok_or_else(|| vm.new_attribute_error("object"))?;
            let res = zelf.cmp.call((x, y), vm)?;
            res.rich_compare(vm.ctx.new_int(0).into(), op, vm)
                .map(Either::A)
        }
    }

    impl Callable for PyKeyWrapper {
        type Args = KeyWrapperCallArgs;

        fn call(zelf: &Py<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult {
            Ok(Self {
                cmp: zelf.cmp.clone(),
                object: PyRwLock::new(Some(args.obj)),
            }
            .into_pyobject(vm))
        }
    }

    // Keep identity in interpreter state, not on the shared static type.
    #[pyattr]
    #[allow(non_snake_case)]
    fn Placeholder(vm: &VirtualMachine) -> PyObjectRef {
        let cell = &vm.state.functools_placeholder;
        if let Some(placeholder) = cell.get() {
            return placeholder.clone();
        }
        // Allocate outside the cell initializer: allocation may trigger GC.
        // Another thread can publish first; always return the published winner.
        let placeholder = PyPlaceholderType.into_pyobject(vm);
        let _ = cell.set(placeholder);
        cell.get().expect("Placeholder was initialized").clone()
    }

    #[pyattr]
    #[pyclass(name = "_PlaceholderType", module = "functools")]
    #[derive(Debug, PyPayload)]
    pub(super) struct PyPlaceholderType;

    impl Constructor for PyPlaceholderType {
        type Args = FuncArgs;

        fn slot_new(_cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            if !args.args.is_empty() || !args.kwargs.is_empty() {
                return Err(vm.new_type_error("_PlaceholderType takes no arguments"));
            }
            Ok(Placeholder(vm))
        }

        fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
            // This is never called because we override slot_new
            Ok(Self)
        }
    }

    #[pyclass(with(Constructor, Representable))]
    impl Py<PyPlaceholderType> {
        #[pymethod]
        fn __reduce__(&self) -> &'static str {
            "Placeholder"
        }

        #[pymethod]
        fn __init_subclass__(_cls: PyTypeRef, vm: &VirtualMachine) -> PyResult<()> {
            Err(vm.new_type_error("cannot subclass '_PlaceholderType'"))
        }
    }

    impl Representable for PyPlaceholderType {
        #[inline]
        fn repr_str(_zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            Ok("Placeholder".to_owned())
        }
    }

    fn is_placeholder(obj: &PyObject) -> bool {
        obj.downcast_ref::<PyPlaceholderType>().is_some()
    }

    fn count_placeholders(args: &[PyObjectRef]) -> usize {
        args.iter().filter(|a| is_placeholder(a)).count()
    }

    #[pyattr]
    #[pyclass(name = "partial", module = "functools", traverse = "manual")]
    #[derive(Debug, PyPayload)]
    pub(super) struct PyPartial {
        #[pymember]
        func: PyAtomicRef<PyObject>,
        #[pymember]
        args: PyAtomicRef<PyTuple>,
        #[pymember]
        keywords: PyAtomicRef<PyDict>,
        phcount: AtomicUsize,
    }

    // SAFETY: Each owned reference is visited once, without cloning it.
    unsafe impl crate::object::Traverse for PyPartial {
        fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
            self.func.traverse(tracer_fn);
            self.args.traverse(tracer_fn);
            self.keywords.traverse(tracer_fn);
        }

        fn clear_refs(&mut self, out: &mut Vec<PyObjectRef>) {
            // __setstate__ can make func point directly back to this partial.
            // The args tuple and keywords dict have their own cycle clearing.
            out.push(self.func.store(Context::genesis().none()));
        }
    }

    #[pyclass(
        with(Constructor, Callable, GetDescriptor, Representable),
        flags(BASETYPE, HAS_DICT, HAS_WEAKREF)
    )]
    impl PyPartial {
        #[pyslot]
        fn vectorcall(
            zelf: &PyObject,
            args: Vec<PyObjectRef>,
            nargs: usize,
            kwnames: Option<&[PyObjectRef]>,
            vm: &VirtualMachine,
        ) -> PyResult {
            let zelf = zelf.downcast_ref::<Self>().unwrap();
            // Keep the call's state alive before keyword lookups can execute
            // __hash__/__eq__ and replace it through __setstate__.
            let func = zelf.func.load_owned();
            let stored_args = zelf.args.load_owned();
            let keywords = zelf.keywords.load_owned();
            let phcount = zelf.phcount.load(Ordering::Relaxed);

            // Check if we have enough args to fill placeholders
            if nargs < phcount {
                return Err(vm.new_type_error(format!(
                    "missing positional arguments in 'partial' call; expected at least {phcount}, got {nargs}"
                )));
            }

            // Build combined args, replacing placeholders
            let mut combined_args = Vec::with_capacity(stored_args.as_slice().len() + args.len());
            let mut new_args_iter = args[..nargs].iter();

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
            let combined_nargs = combined_args.len();

            let kwnames = kwnames.unwrap_or_default();
            if func.vectorcall_func().is_none() {
                // FuncArgs callables retain their existing keyword binding path.
                // In particular, do not introduce vectorcall's hash callbacks
                // when the wrapped callable does not support vectorcall.
                let mut kwargs = KwArgs::default();
                for (key, value) in keywords.items_vec() {
                    let key = key
                        .downcast_ref::<PyStr>()
                        .ok_or_else(|| vm.new_type_error("keywords must be strings"))?;
                    kwargs.insert(key.as_wtf8().to_owned(), value);
                }
                for (key, value) in kwnames.iter().zip(&args[nargs..]) {
                    let key = key
                        .downcast_ref::<PyStr>()
                        .ok_or_else(|| vm.new_type_error("keywords must be strings"))?;
                    kwargs.insert(key.as_wtf8().to_owned(), value.clone());
                }
                return func.call(FuncArgs::new(combined_args, kwargs), vm);
            }
            if keywords.is_empty() {
                combined_args.extend_from_slice(&args[nargs..]);
                return func.vectorcall(combined_args, combined_nargs, Some(kwnames), vm);
            }

            // Like partial_vectorcall, copy stored keywords only when an incoming
            // key replaces one. New keys remain a tail, preserving their identity
            // and avoiding extra hash callbacks from an intermediate dictionary.
            let mut merged_keywords = None;
            let mut tail_names = Vec::new();
            let mut tail_values = Vec::new();
            for (key, value) in kwnames.iter().zip(&args[nargs..]) {
                let hash = PyDict::hash_or_unhashable(&**key, vm)?;
                if keywords.entries.contains(vm, &**key, hash)? {
                    let merged = merged_keywords.get_or_insert_with(|| keywords.copy());
                    merged.inner_setitem(&**key, value.clone(), vm)?;
                } else {
                    tail_names.push(key.clone());
                    tail_values.push(value.clone());
                }
            }

            let merged = merged_keywords.as_ref().unwrap_or(&keywords);
            let mut combined_names = Vec::with_capacity(merged.__len__() + tail_names.len());
            for (key, value) in merged.items_vec() {
                key.downcast_ref::<PyStr>()
                    .ok_or_else(|| vm.new_type_error("keywords must be strings"))?;
                combined_names.push(key);
                combined_args.push(value);
            }
            combined_names.extend(tail_names);
            combined_args.extend(tail_values);

            func.vectorcall(combined_args, combined_nargs, Some(&combined_names), vm)
        }

        #[pygetset]
        fn __dict__(zelf: &Py<Self>, vm: &VirtualMachine) -> PyDictRef {
            zelf.as_object()
                .instance_dict()
                .map_or_else(|| vm.ctx.new_dict(), |d| d.get_or_insert(vm))
        }

        #[pygetset(setter)]
        fn set___dict__(
            zelf: &Py<Self>,
            value: PySetterValue,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            object::object_generic_set_dict(zelf.as_object().to_owned(), value, vm)
        }

        #[pymethod]
        fn __reduce__(zelf: &Py<Self>, vm: &VirtualMachine) -> PyObjectRef {
            let func = zelf.func.load_owned();
            let args = zelf.args.load_owned();
            let keywords = zelf.keywords.load_owned();
            let partial_type = zelf.class();

            // Preserve the difference between an absent and an allocated empty dict.
            let dict_obj = match zelf.as_object().dict() {
                Some(dict) => dict.into(),
                _ => vm.ctx.none(),
            };

            let state =
                vm.ctx
                    .new_tuple(vec![func.clone(), args.into(), keywords.into(), dict_obj]);
            vm.ctx
                .new_tuple(vec![
                    partial_type.to_owned().into(),
                    vm.ctx.new_tuple(vec![func]).into(),
                    state.into(),
                ])
                .into()
        }

        #[pymethod]
        fn __setstate__(zelf: &Py<Self>, object: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            let invalid_state = || vm.new_type_error("invalid partial state");
            let state = object.downcast::<PyTuple>().map_err(|_| invalid_state())?;
            let [func, args, keywords, dict] = state.as_slice() else {
                return Err(invalid_state());
            };
            if !func.is_callable() {
                return Err(invalid_state());
            }
            let args = args
                .clone()
                .downcast::<PyTuple>()
                .map_err(|_| invalid_state())?;
            let keywords = if vm.is_none(keywords) {
                None
            } else {
                Some(
                    keywords
                        .clone()
                        .downcast::<PyDict>()
                        .map_err(|_| invalid_state())?,
                )
            };
            let dict = if vm.is_none(dict) {
                None
            } else {
                Some(
                    dict.clone()
                        .downcast::<PyDict>()
                        .map_err(|_| invalid_state())?,
                )
            };

            // Reject invalid state before tuple conversion can execute Python code
            // or any of the partial's existing fields are replaced.
            if args
                .as_slice()
                .last()
                .is_some_and(|arg| is_placeholder(arg))
            {
                return Err(vm.new_type_error("trailing Placeholders are not allowed"));
            }
            let phcount = count_placeholders(args.as_slice());
            let args = if args.class().is(vm.ctx.types.tuple_type) {
                args
            } else {
                let elements: Vec<PyObjectRef> = args.as_object().try_to_value(vm)?;
                vm.ctx.new_tuple(elements)
            };
            let keywords = match keywords {
                None => vm.ctx.new_dict(),
                Some(dict) if dict.class().is(vm.ctx.types.dict_type) => dict,
                // Copy the stored entries, including their hashes, without invoking
                // a dict subclass's protocol or rehashing its keys.
                Some(dict) => dict.copy().into_ref(&vm.ctx),
            };

            zelf.func.store(func.clone());
            zelf.args.store(args);
            zelf.keywords.store(keywords);
            zelf.phcount.store(phcount, Ordering::Relaxed);
            // The supplied instance dictionary is installed by identity. Clearing
            // and updating the old dictionary would mutate external references,
            // lose an absent dictionary, and erase a state dictionary aliased to it.
            if let Some(instance_dict) = zelf.as_object().instance_dict() {
                instance_dict.set(dict);
            }

            Ok(())
        }

        #[pyclassmethod]
        fn __class_getitem__(
            cls: PyTypeRef,
            object: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyGenericAlias> {
            PyGenericAlias::from_args(cls, object, vm)
        }
    }

    #[derive(FromArgs)]
    pub(crate) struct PartialSig {
        #[pyarg(positional)]
        func: PyObjectRef,
        #[pyarg(flatten)]
        args: PosArgs<PyObjectRef>,
        #[pyarg(flatten)]
        keywords: KwArgs<PyObjectRef, crate::function::NameKeywords>,
    }

    impl Constructor for PyPartial {
        type Args = PartialSig;

        fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            let args = args.bind_for(vm, Callee::of::<Self>(vm))?;
            let payload = Self::py_new(&cls, args, vm)?;
            // An absent instance dictionary permits nested partial flattening;
            // allocating even an empty dictionary changes that behavior.
            payload
                .into_ref_with_type_lazy_dict(vm, cls)
                .map(Into::into)
        }

        fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            let PartialSig {
                func,
                args,
                keywords,
            } = args;
            let args_slice = args.as_ref();

            if !func.is_callable() {
                return Err(vm.new_type_error("the first argument must be callable"));
            }

            // Check the arguments supplied to this constructor before flattening:
            // a trailing placeholder is invalid even if the inner partial would
            // supply a later non-placeholder argument.
            if args_slice.last().is_some_and(|arg| is_placeholder(arg)) {
                return Err(vm.new_type_error("trailing Placeholders are not allowed"));
            }
            for value in keywords.values() {
                if is_placeholder(value) {
                    return Err(
                        vm.new_type_error("Placeholder cannot be passed as a keyword argument")
                    );
                }
            }

            // Handle nested partial objects
            let (final_func, final_args, final_keywords) = if let Some(partial) = func
                .downcast_ref::<Self>()
                .filter(|_| !func.has_instance_dict())
            {
                let stored_args_obj = partial.args.load_owned();
                let stored_func = partial.func.load_owned();
                let stored_keywords = partial.keywords.load_owned();
                let stored_args = stored_args_obj.as_slice();

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

                // Each partial owns its keyword mapping. Sharing the inner
                // mapping would make both construction and later mutation of
                // the outer partial change the inner partial's calls.
                (
                    stored_func,
                    merged_args,
                    stored_keywords.copy().into_ref(&vm.ctx),
                )
            } else {
                (func, args_slice.to_vec(), vm.ctx.new_dict())
            };

            let phcount = count_placeholders(&final_args);

            // Add new keywords
            for (key, value) in keywords {
                final_keywords.set_item(vm.ctx.intern_str(key), value, vm)?;
            }

            Ok(Self {
                func: PyAtomicRef::from(final_func),
                args: PyAtomicRef::from(vm.ctx.new_tuple(final_args)),
                keywords: PyAtomicRef::from(final_keywords),
                phcount: AtomicUsize::new(phcount),
            })
        }
    }

    impl Callable for PyPartial {
        type Args = FuncArgs;

        fn call(zelf: &Py<Self>, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            let nargs = args.args.len();
            let mut values = args.args;
            let mut kwnames = Vec::with_capacity(args.kwargs.len());
            for (key, value) in args.kwargs {
                kwnames.push(vm.ctx.new_str(key).into());
                values.push(value);
            }
            Self::vectorcall(zelf.as_object(), values, nargs, Some(&kwnames), vm)
        }
    }

    impl GetDescriptor for PyPartial {
        fn descr_get(
            zelf: &PyObject,
            obj: Option<&PyObject>,
            _cls: Option<&PyObject>,
            vm: &VirtualMachine,
        ) -> PyResult {
            let obj = match obj {
                Some(obj) if !vm.is_none(obj) => obj,
                _ => return Ok(zelf.to_owned()),
            };
            Ok(PyBoundMethod::new(obj.to_owned(), zelf.to_owned())
                .into_ref(&vm.ctx)
                .into())
        }
    }

    impl Representable for PyPartial {
        #[inline]
        fn repr_wtf8(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<Wtf8Buf> {
            // Check for recursive repr
            let obj = zelf.as_object();
            if let Some(_guard) = ReprGuard::enter(vm, obj) {
                // Clone and release lock before calling Python code to prevent deadlock
                let func = zelf.func.load_owned();
                let args = zelf.args.load_owned();
                let keywords = zelf.keywords.load_owned();

                let qualname = zelf.class().__qualname__(vm);
                let qualname_wtf8 = qualname
                    .downcast_ref::<crate::builtins::PyStr>()
                    .map_or_else(
                        || Wtf8Buf::from(zelf.class().name().to_owned()),
                        |s| s.as_wtf8().to_owned(),
                    );
                let module = zelf.class().__module__(vm)?;

                let mut result = Wtf8Buf::new();
                if let Ok(module_str) = module.downcast::<crate::builtins::PyStr>() {
                    let module_name = module_str.as_wtf8();
                    if module_name != "builtins" && !module_name.is_empty() {
                        result.push_wtf8(module_name);
                        result.push_char('.');
                    }
                }
                result.push_wtf8(&qualname_wtf8);
                result.push_char('(');
                result.push_wtf8(func.repr(vm)?.as_wtf8());

                for arg in args.as_slice() {
                    result.push_str(", ");
                    result.push_wtf8(arg.repr(vm)?.as_wtf8());
                }

                for (key, value) in &*keywords {
                    result.push_str(", ");
                    let key_str = if let Ok(s) = key.clone().downcast::<crate::builtins::PyStr>() {
                        s
                    } else {
                        key.str(vm)?
                    };
                    result.push_wtf8(key_str.as_wtf8());
                    result.push_char('=');
                    result.push_wtf8(value.repr(vm)?.as_wtf8());
                }

                result.push_char(')');
                Ok(result)
            } else {
                Ok(Wtf8Buf::from("..."))
            }
        }
    }

    /// RAII guard that releases a [`RawRMutex`] acquired with `lock()`, even on early
    /// return through `?`. The mutex is reentrant, so a nested call from the same
    /// thread (e.g. via a custom `__hash__`/`__eq__` invoked while probing the cache)
    /// re-enters instead of deadlocking.
    struct RMutexGuard<'a>(&'a RawRMutex);

    impl<'a> RMutexGuard<'a> {
        fn acquire(mu: &'a RawRMutex) -> Self {
            mu.lock();
            Self(mu)
        }
    }

    impl Drop for RMutexGuard<'_> {
        fn drop(&mut self) {
            // SAFETY: this guard is only constructed right after a matching `lock()`
            // call on the same mutex, so the current thread holds (at least one level
            // of) the lock.
            unsafe { self.0.unlock() };
        }
    }

    #[pyclass(no_attr, name = "_lru_list_elem", module = "functools", traverse)]
    #[derive(Debug, PyPayload)]
    struct PyLruListElem {
        #[pytraverse(skip)]
        hash: PyHash,
        result: PyObjectRef,
    }

    #[pyclass(flags(IMMUTABLETYPE, DISALLOW_INSTANTIATION))]
    impl PyLruListElem {}

    // Native implementation of `functools._lru_cache_wrapper`, mirroring CPython's
    // `_functools` accelerator so `functools.lru_cache` doesn't fall back to the much
    // slower pure-Python implementation in `Lib/functools.py`.
    #[pyattr]
    #[pyclass(name = "_lru_cache_wrapper", module = "functools", traverse = "manual")]
    #[derive(PyPayload)]
    pub(super) struct PyLruCacheWrapper {
        /// The wrapped user function.
        func: PyObjectRef,
        /// `None` means unbounded; `Some(0)` means "never cache".
        maxsize: Option<usize>,
        typed: bool,
        /// Sentinel marking the boundary between positional and keyword arguments
        /// within a cache key tuple. Fixed for the lifetime of the wrapper, matching
        /// `_make_key`'s `kwd_mark` default argument in `Lib/functools.py`.
        keyword_marker: PyObjectRef,
        /// The namedtuple type used to build `cache_info()` results.
        cache_info_type: PyObjectRef,
        hits: AtomicU64,
        misses: AtomicU64,
        /// Entries are kept in least-to-most-recently-used order: a hit moves its
        /// entry to the end atomically, so the front of the dict is always the next
        /// eviction candidate. Moving an entry preserves the original key and does
        /// not invoke additional equality callbacks.
        /// Unbounded caches store results directly, without an LRU entry wrapper.
        cache: PyDictRef,
        /// Coarse-grained reentrant lock guarding the cache lookup/insert critical
        /// sections (not held while calling the user function), matching CPython.
        lock: RawRMutex,
    }

    // SAFETY: Each owned reference is visited once; the instance dict is
    // traversed separately by the object layer.
    unsafe impl crate::object::Traverse for PyLruCacheWrapper {
        fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
            self.func.traverse(tracer_fn);
            self.keyword_marker.traverse(tracer_fn);
            self.cache_info_type.traverse(tracer_fn);
            self.cache.traverse(tracer_fn);
        }
    }

    impl core::fmt::Debug for PyLruCacheWrapper {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.pad("lru_cache_wrapper")
        }
    }

    #[derive(FromArgs)]
    #[from_args(legacy, fname = "lru_cache")]
    pub(super) struct LruCacheWrapperArgs {
        #[pyarg(any)]
        user_function: PyObjectRef,
        #[pyarg(any)]
        maxsize: PyObjectRef,
        #[pyarg(any)]
        typed: ArgIntoBool,
        #[pyarg(any)]
        cache_info_type: PyObjectRef,
    }

    impl Constructor for PyLruCacheWrapper {
        type Args = LruCacheWrapperArgs;

        fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            if !args.user_function.is_callable() {
                return Err(vm.new_type_error("the first argument must be callable"));
            }
            // The binder has already converted typed's truthiness. Only now
            // invoke __index__, after the callable check, as in CPython's OOpO
            // constructor. Keep maxsize raw until then to preserve that order.
            let maxsize = if vm.is_none(&args.maxsize) {
                None
            } else {
                let size = args
                    .maxsize
                    .try_index_opt(vm)
                    .ok_or_else(|| vm.new_type_error("maxsize should be integer or None"))??
                    .try_to_primitive::<isize>(vm)?;
                Some(size.max(0) as usize)
            };
            Ok(Self {
                func: args.user_function,
                maxsize,
                typed: args.typed.into_bool(),
                keyword_marker: vm
                    .ctx
                    .new_base_object(vm.ctx.types.object_type.to_owned(), None),
                cache_info_type: args.cache_info_type,
                hits: AtomicU64::new(0),
                misses: AtomicU64::new(0),
                cache: vm.ctx.new_dict(),
                lock: RawRMutex::INIT,
            })
        }
    }

    impl PyLruCacheWrapper {
        /// Build the cache key for a call, following `functools._make_key`.
        fn make_key(&self, args: &FuncArgs, vm: &VirtualMachine) -> PyObjectRef {
            // CPython deliberately uses scalar keys only for exact int and str.
            // In particular, (1.0,) and (True,) must not alias the scalar key 1.
            if !self.typed && args.kwargs.is_empty() && args.args.len() == 1 {
                let arg = &args.args[0];
                if arg.class().is(vm.ctx.types.int_type) || arg.class().is(vm.ctx.types.str_type) {
                    return arg.clone();
                }
            }
            let mut elements: Vec<PyObjectRef> = args.args.clone();
            if !args.kwargs.is_empty() {
                elements.push(self.keyword_marker.clone());
                for (name, value) in &args.kwargs {
                    elements.push(vm.ctx.new_str(name.clone()).into());
                    elements.push(value.to_owned());
                }
            }
            if self.typed {
                elements.extend(args.args.iter().map(|a| a.class().to_owned().into()));
                if !args.kwargs.is_empty() {
                    elements.extend(
                        (&args.kwargs)
                            .into_iter()
                            .map(|(_, v)| v.class().to_owned().into()),
                    );
                }
            }
            vm.ctx.new_tuple(elements).into()
        }
    }

    #[pyclass(
        with(Constructor, Callable, GetDescriptor),
        flags(HAS_DICT, HAS_WEAKREF, METHOD_DESCRIPTOR)
    )]
    impl Py<PyLruCacheWrapper> {
        #[pymethod]
        fn cache_info(&self, vm: &VirtualMachine) -> PyResult {
            let hits = self.hits.load(Ordering::Relaxed);
            let misses = self.misses.load(Ordering::Relaxed);
            let currsize = self.cache.__len__();
            let maxsize: PyObjectRef = match self.maxsize {
                Some(n) => vm.ctx.new_int(n).into(),
                None => vm.ctx.none(),
            };
            self.cache_info_type
                .call((hits, misses, maxsize, currsize), vm)
        }

        #[pymethod]
        fn cache_clear(&self) {
            let _guard = RMutexGuard::acquire(&self.lock);
            self.hits.store(0, Ordering::Relaxed);
            self.misses.store(0, Ordering::Relaxed);
            // Keep the dict identity: an equality callback can clear the cache
            // while a lookup is in progress. DictContent restarts that lookup
            // against the empty table and releases entries outside its own lock.
            self.cache.clear();
        }

        #[pymethod]
        fn __reduce__(zelf: &Self, vm: &VirtualMachine) -> PyResult {
            zelf.as_object().get_attr("__qualname__", vm)
        }

        #[pymethod]
        fn __copy__(zelf: PyObjectRef, _args: PosArgs) -> PyObjectRef {
            zelf
        }

        #[pymethod]
        fn __deepcopy__(zelf: PyObjectRef, _args: PosArgs) -> PyObjectRef {
            zelf
        }

        #[pyclassmethod]
        fn __class_getitem__(
            cls: PyTypeRef,
            args: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyGenericAlias> {
            PyGenericAlias::from_args(cls, args, vm)
        }
    }

    impl Callable for PyLruCacheWrapper {
        type Args = FuncArgs;

        fn call(zelf: &Py<Self>, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            if zelf.maxsize == Some(0) {
                zelf.misses.fetch_add(1, Ordering::Relaxed);
                return zelf.func.call(args, vm);
            }

            let key = zelf.make_key(&args, vm);
            let hash = key.hash(vm)?;

            {
                let _guard = RMutexGuard::acquire(&zelf.lock);
                let cache = &zelf.cache;
                let value = if zelf.maxsize.is_some() {
                    cache.entries.move_to_end(vm, key.as_object(), hash)?
                } else {
                    cache.entries.get_reentrant(vm, key.as_object(), hash)?
                };
                if let Some(value) = value {
                    let result = if zelf.maxsize.is_some() {
                        let Some(elem) = value.downcast_ref::<PyLruListElem>() else {
                            return Err(vm.new_type_error("lru cache entry is corrupted"));
                        };
                        elem.result.clone()
                    } else {
                        value
                    };
                    zelf.hits.fetch_add(1, Ordering::Relaxed);
                    return Ok(result);
                }
                zelf.misses.fetch_add(1, Ordering::Relaxed);
                // Lock released here (guard dropped) before calling into user code, so
                // other threads may compute the same miss concurrently.
            }

            let result = zelf.func.call(args, vm)?;

            {
                let _guard = RMutexGuard::acquire(&zelf.lock);
                let cache = &zelf.cache;
                if let Some(maxsize) = zelf.maxsize {
                    // Only bounded caches preserve a result inserted by a
                    // recursive call while the user function was running.
                    // Revalidate equality callbacks here too, without moving
                    // an existing entry or changing its recency.
                    if cache
                        .entries
                        .get_reentrant(vm, key.as_object(), hash)?
                        .is_none()
                    {
                        let entry = PyLruListElem {
                            hash,
                            result: result.clone(),
                        }
                        .into_pyobject(vm);
                        let _evicted = if cache.__len__() >= maxsize {
                            if let Some((oldest_key, oldest_value)) = cache.into_iter().next() {
                                let Some(elem) = oldest_value.downcast_ref::<PyLruListElem>()
                                else {
                                    return Err(vm.new_type_error("lru cache entry is corrupted"));
                                };
                                // Pop can run equality for colliding keys. A
                                // callback may clear the cache before it finds
                                // the eviction candidate, leaving no entry.
                                if cache
                                    .entries
                                    .pop_reentrant(vm, &*oldest_key, elem.hash)?
                                    .is_none()
                                {
                                    return Ok(result);
                                }
                                Some((oldest_key, oldest_value))
                            } else {
                                None
                            }
                        } else {
                            None
                        };
                        if let Err(error) =
                            cache.set_item_known_hash(key.as_object(), hash, entry, vm)
                        {
                            // CPython releases the failed new entry before the
                            // evicted key/result. Any of their finalizers can
                            // reenter and inspect or repopulate the cache.
                            drop(key);
                            drop(result);
                            return Err(error);
                        }
                        // Neither keys nor results may be finalized between
                        // eviction and insertion into the now-consistent cache.
                    }
                } else {
                    // An unbounded cache always installs the newly computed
                    // result, including after a recursive call for this key.
                    cache.set_item_known_hash(key.as_object(), hash, result.clone(), vm)?;
                }
            }

            Ok(result)
        }
    }

    impl GetDescriptor for PyLruCacheWrapper {
        fn descr_get(
            zelf: &PyObject,
            obj: Option<&PyObject>,
            _cls: Option<&PyObject>,
            vm: &VirtualMachine,
        ) -> PyResult {
            let obj = match obj {
                Some(obj) if !vm.is_none(obj) => obj,
                _ => return Ok(zelf.to_owned()),
            };
            Ok(PyBoundMethod::new(obj.to_owned(), zelf.to_owned())
                .into_ref(&vm.ctx)
                .into())
        }
    }
}

pub(crate) use _functools::module_def;

#[pymodule]
mod _functools {
    use crate::{
        Py, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
        builtins::{
            PyBoundMethod, PyDict, PyDictRef, PyGenericAlias, PyTuple, PyType, PyTypeRef, object,
        },
        common::lock::PyRwLock,
        function::{FuncArgs, KwArgs, OptionalOption, PySetterValue},
        object::AsObject,
        protocol::PyIter,
        pyclass,
        recursion::ReprGuard,
        types::{Callable, Constructor, GetDescriptor, Representable},
    };
    use core::sync::atomic::{AtomicU64, Ordering};
    use parking_lot::lock_api::RawReentrantMutex as GenericRawReentrantMutex;
    use rustpython_common::wtf8::Wtf8Buf;

    /// Reentrant raw mutex used to guard the LRU cache's lookup/insert critical
    /// sections. Defined locally (rather than reusing `stdlib::_thread::RawRMutex`)
    /// because that type is only available when the `threading` feature is enabled,
    /// while `parking_lot` itself is always a dependency.
    type RawRMutex = GenericRawReentrantMutex<parking_lot::RawMutex, parking_lot::RawThreadId>;

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
        let mut iter = iterator.iter(vm)?;
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
    pub(super) struct PyPlaceholderType;

    impl Constructor for PyPlaceholderType {
        type Args = FuncArgs;

        fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            if !args.args.is_empty() || !args.kwargs.is_empty() {
                return Err(vm.new_type_error("_PlaceholderType takes no arguments"));
            }
            // Return the singleton stored on the type class
            if let Some(instance) = cls.get_attr(vm.ctx.intern_str("_instance")) {
                return Ok(instance);
            }
            // Fallback: create a new instance (shouldn't happen for base type after module init)
            Ok(Self.into_pyobject(vm))
        }

        fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
            // This is never called because we override slot_new
            Ok(Self)
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
            Err(vm.new_type_error("cannot subclass '_PlaceholderType'"))
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
    pub(super) struct PyPartial {
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
        flags(BASETYPE, HAS_DICT, HAS_WEAKREF)
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
            vm.ctx
                .new_tuple(vec![
                    partial_type.to_owned().into(),
                    vm.ctx.new_tuple(vec![inner.func.clone()]).into(),
                    state.into(),
                ])
                .into()
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
                return Err(vm.new_type_error("trailing Placeholders are not allowed"));
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
        ) -> PyResult<PyGenericAlias> {
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
                         Did you mean partial(..., {key}=Placeholder, ...)(value)?"
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
                return Err(vm.new_type_error("trailing Placeholders are not allowed"));
            }

            let phcount = count_placeholders(&final_args);

            // Add new keywords
            for (key, value) in args.kwargs {
                final_keywords.set_item(vm.ctx.intern_str(key), value, vm)?;
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
            let mut final_kwargs = crate::function::KwArgsMap::default();

            // Add keywords from self.keywords
            for (key, value) in &*keywords {
                // `expect_str()` would panic on surrogate keys; keep them as WTF-8.
                let key_str = key
                    .downcast_ref::<crate::builtins::PyStr>()
                    .ok_or_else(|| vm.new_type_error("keywords must be strings"))?;
                final_kwargs.insert(key_str.as_wtf8().to_owned(), value);
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
        fn repr_wtf8(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<Wtf8Buf> {
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

                let qualname = zelf.class().__qualname__(vm);
                let qualname_wtf8 = qualname
                    .downcast_ref::<crate::builtins::PyStr>()
                    .map_or_else(
                        || Wtf8Buf::from(zelf.class().name().to_owned()),
                        |s| s.as_wtf8().to_owned(),
                    );
                let module = zelf.class().__module__(vm);

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

    /// Native implementation of `functools._lru_cache_wrapper`, mirroring CPython's
    /// `_functools` accelerator so `functools.lru_cache` doesn't fall back to the much
    /// slower pure-Python implementation in `Lib/functools.py`.
    #[pyattr]
    #[pyclass(name = "_lru_cache_wrapper", module = "functools")]
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
        /// entry to the end by removing and reinserting it, so the front of the dict
        /// is always the next eviction candidate. Ordering is only maintained (at the
        /// cost of the extra reinsert on a hit) when `maxsize` is `Some`.
        cache: PyRwLock<PyDictRef>,
        /// Coarse-grained reentrant lock guarding the cache lookup/insert critical
        /// sections (not held while calling the user function), matching CPython.
        lock: RawRMutex,
    }

    impl core::fmt::Debug for PyLruCacheWrapper {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.pad("lru_cache_wrapper")
        }
    }

    #[derive(FromArgs)]
    pub(super) struct LruCacheWrapperArgs {
        #[pyarg(positional)]
        function: PyObjectRef,
        #[pyarg(positional)]
        maxsize: Option<isize>,
        #[pyarg(positional)]
        typed: bool,
        #[pyarg(positional)]
        cache_info_type: PyObjectRef,
    }

    impl Constructor for PyLruCacheWrapper {
        type Args = LruCacheWrapperArgs;

        fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            if !args.function.is_callable() {
                return Err(vm.new_type_error("the first argument must be callable"));
            }
            // Negative maxsize is normalized to 0 by `functools.lru_cache` before
            // reaching here, but clamp defensively to match that behavior exactly.
            let maxsize = args.maxsize.map(|n| n.max(0) as usize);
            Ok(Self {
                func: args.function,
                maxsize,
                typed: args.typed,
                keyword_marker: vm
                    .ctx
                    .new_base_object(vm.ctx.types.object_type.to_owned(), None),
                cache_info_type: args.cache_info_type,
                hits: AtomicU64::new(0),
                misses: AtomicU64::new(0),
                cache: PyRwLock::new(vm.ctx.new_dict()),
                lock: RawRMutex::INIT,
            })
        }
    }

    #[pyclass(
        with(Constructor, Callable, GetDescriptor),
        flags(HAS_DICT, HAS_WEAKREF)
    )]
    impl PyLruCacheWrapper {
        /// Build the cache key for a call, following `functools._make_key`.
        fn make_key(&self, args: &FuncArgs, vm: &VirtualMachine) -> PyObjectRef {
            let mut elements: Vec<PyObjectRef> = args.args.clone();
            if !args.kwargs.is_empty() {
                elements.push(self.keyword_marker.clone());
                for (name, value) in &args.kwargs {
                    elements.push(vm.ctx.new_str(name.clone()).into());
                    elements.push(value.clone());
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

        /// Refresh `key`'s recency by moving it to the end of `cache`'s insertion
        /// order (removing then reinserting it). `cache` itself provides the
        /// hashing/equality, so this needs no separate key-comparison machinery.
        fn touch_key(
            key: &PyObjectRef,
            value: &PyObjectRef,
            cache: &PyDictRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            cache.del_item(key.as_object(), vm)?;
            cache.set_item(key.as_object(), value.clone(), vm)?;
            Ok(())
        }

        /// Evict the least-recently-used entry (the first item in `cache`'s
        /// insertion order) if `cache` grew past `maxsize`.
        fn evict_if_full(maxsize: usize, cache: &PyDictRef, vm: &VirtualMachine) -> PyResult<()> {
            if cache.__len__() <= maxsize {
                return Ok(());
            }
            let oldest = cache.into_iter().next();
            if let Some((oldest_key, _)) = oldest {
                cache.del_item(oldest_key.as_object(), vm)?;
            }
            Ok(())
        }

        #[pymethod]
        fn cache_info(&self, vm: &VirtualMachine) -> PyResult {
            let hits = self.hits.load(Ordering::Relaxed);
            let misses = self.misses.load(Ordering::Relaxed);
            let currsize = self.cache.read().__len__();
            let maxsize: PyObjectRef = match self.maxsize {
                Some(n) => vm.ctx.new_int(n).into(),
                None => vm.ctx.none(),
            };
            self.cache_info_type
                .call((hits, misses, maxsize, currsize), vm)
        }

        #[pymethod]
        fn cache_clear(&self, vm: &VirtualMachine) {
            let _guard = RMutexGuard::acquire(&self.lock);
            *self.cache.write() = vm.ctx.new_dict();
            self.hits.store(0, Ordering::Relaxed);
            self.misses.store(0, Ordering::Relaxed);
        }

        #[pymethod]
        fn __reduce__(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult {
            zelf.as_object().get_attr("__qualname__", vm)
        }

        #[pymethod]
        fn __copy__(zelf: PyObjectRef) -> PyObjectRef {
            zelf
        }

        #[pymethod]
        fn __deepcopy__(zelf: PyObjectRef, _memo: PyObjectRef) -> PyObjectRef {
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

            {
                let _guard = RMutexGuard::acquire(&zelf.lock);
                let cache = zelf.cache.read().clone();
                if let Some(value) = cache.get_item_opt(key.as_object(), vm)? {
                    zelf.hits.fetch_add(1, Ordering::Relaxed);
                    if zelf.maxsize.is_some() {
                        Self::touch_key(&key, &value, &cache, vm)?;
                    }
                    return Ok(value);
                }
                zelf.misses.fetch_add(1, Ordering::Relaxed);
                // Lock released here (guard dropped) before calling into user code, so
                // other threads may compute the same miss concurrently.
            }

            let result = zelf.func.call(args, vm)?;

            {
                let _guard = RMutexGuard::acquire(&zelf.lock);
                let cache = zelf.cache.read().clone();
                // A reentrant or concurrent call may have already inserted this key
                // (e.g. recursive lookups, or another thread finishing first); in that
                // case keep the existing entry instead of creating orphan eviction
                // links (see CPython issue gh-35780).
                if !cache.contains_key(key.as_object(), vm) {
                    cache.set_item(key.as_object(), result.clone(), vm)?;
                    if let Some(maxsize) = zelf.maxsize {
                        Self::evict_if_full(maxsize, &cache, vm)?;
                    }
                }
            }

            Ok(result)
        }
    }

    impl GetDescriptor for PyLruCacheWrapper {
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
}

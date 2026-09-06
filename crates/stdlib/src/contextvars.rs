pub(crate) use _contextvars::module_def;

use crate::vm::PyRef;
use _contextvars::PyContext;
use core::cell::RefCell;

thread_local! {
    // TODO: Vec doesn't seem to match copy behavior
    static CONTEXTS: RefCell<Vec<PyRef<PyContext>>> = RefCell::default();
}

#[pymodule]
mod _contextvars {
    use crate::vm::{
        AsObject, Py, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine, atomic_func,
        builtins::{PyGenericAlias, PyList, PyStr, PyType, PyTypeRef},
        class::StaticType,
        common::{
            hash::PyHash,
            lock::{LazyLock, PyMutex},
            wtf8::Wtf8Buf,
        },
        function::{FuncArgs, OptionalArg},
        protocol::{PyMappingMethods, PySequenceMethods},
        types::{AsMapping, AsSequence, Constructor, Hashable, Iterable, Representable},
    };
    use core::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
    use indexmap::IndexMap;

    // TODO: Real hamt implementation
    type Hamt = IndexMap<PyRef<ContextVar>, PyObjectRef, rapidhash::quality::RandomState>;

    #[pyclass(no_attr, name = "Hamt", module = "contextvars")]
    #[derive(Debug, PyPayload)]
    pub(crate) struct HamtObject {
        hamt: PyMutex<Hamt>,
    }

    #[pyclass]
    impl HamtObject {}

    impl Default for HamtObject {
        fn default() -> Self {
            Self {
                hamt: PyMutex::new(Hamt::default()),
            }
        }
    }

    #[derive(Debug)]
    struct ContextInner {
        idx: AtomicUsize,
        vars: PyRef<HamtObject>,
        // PyObject *ctx_weakreflist;
        entered: AtomicBool,
    }

    #[pyattr]
    #[pyclass(name = "Context")]
    #[derive(Debug, PyPayload)]
    pub(crate) struct PyContext {
        // not to confuse with vm::Context
        inner: ContextInner,
    }

    impl PyContext {
        fn empty(vm: &VirtualMachine) -> Self {
            Self {
                inner: ContextInner {
                    idx: AtomicUsize::new(usize::MAX),
                    vars: HamtObject::default().into_ref(&vm.ctx),
                    entered: AtomicBool::new(false),
                },
            }
        }

        fn borrow_vars(&self) -> impl core::ops::DerefMut<Target = Hamt> + '_ {
            self.inner.vars.hamt.lock()
        }

        fn borrow_vars_mut(&self) -> impl core::ops::DerefMut<Target = Hamt> + '_ {
            self.inner.vars.hamt.lock()
        }

        fn enter(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<()> {
            // A context is entered by one thread at a time, so the check and the
            // claim have to be a single step.
            if zelf
                .inner
                .entered
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
            {
                return Err(vm.new_runtime_error(format!(
                    "cannot enter context: {} is already entered",
                    zelf.as_object().repr(vm)?
                )));
            }

            super::CONTEXTS.with_borrow_mut(|ctxs| {
                zelf.inner.idx.store(ctxs.len(), Ordering::Relaxed);
                ctxs.push(zelf.to_owned());
            });

            Ok(())
        }

        fn exit(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<()> {
            if !zelf.inner.entered.load(Ordering::Acquire) {
                return Err(vm.new_runtime_error(format!(
                    "cannot exit context: {} is not entered",
                    zelf.as_object().repr(vm)?
                )));
            }

            super::CONTEXTS.with_borrow_mut(|ctxs| {
                ctxs.pop_if(|ctx| ctx.get_id() == zelf.get_id())
                    .map(drop)
                    .ok_or_else(|| {
                        vm.new_runtime_error(
                            "cannot exit context: thread state references a different context object"
                        )
                    })
            })?;
            zelf.inner.entered.store(false, Ordering::Release);

            Ok(())
        }

        fn current(vm: &VirtualMachine) -> PyRef<Self> {
            super::CONTEXTS.with_borrow_mut(|ctxs| {
                if let Some(ctx) = ctxs.last() {
                    ctx.clone()
                } else {
                    let ctx = Self::empty(vm);
                    ctx.inner.idx.store(0, Ordering::Relaxed);
                    ctx.inner.entered.store(true, Ordering::Release);
                    let ctx = ctx.into_ref(&vm.ctx);
                    ctxs.push(ctx);
                    ctxs[0].clone()
                }
            })
        }

        fn contains(&self, needle: &Py<ContextVar>) -> bool {
            let vars = self.borrow_vars();
            vars.get(needle).is_some()
        }

        fn get_inner(&self, needle: &Py<ContextVar>) -> Option<PyObjectRef> {
            let vars = self.borrow_vars();
            vars.get(needle).map(|o| o.to_owned())
        }
    }

    fn context_check_key_type<'a>(
        key: &'a crate::vm::PyObject,
        vm: &VirtualMachine,
    ) -> PyResult<&'a Py<ContextVar>> {
        match key.downcast_ref::<ContextVar>() {
            Some(var) => Ok(var),
            None => Err(vm.new_type_error(format!(
                "a ContextVar key was expected, got {}",
                key.repr(vm)?
            ))),
        }
    }

    #[pyclass(with(Constructor, AsMapping, AsSequence, Iterable))]
    impl PyContext {
        #[pymethod]
        fn run(zelf: &Py<Self>, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            let (callable, rest) = args
                .args
                .split_first()
                .ok_or_else(|| vm.new_type_error("run() missing 1 required positional argument"))?;
            let rest = FuncArgs {
                args: rest.to_vec(),
                kwargs: args.kwargs,
            };
            Self::enter(zelf, vm)?;
            let result = callable.call(rest, vm);
            Self::exit(zelf, vm)?;
            result
        }

        #[pymethod]
        fn copy(&self, vm: &VirtualMachine) -> Self {
            // Deep copy the vars - clone the underlying Hamt data, not just the PyRef
            let vars_copy = HamtObject {
                hamt: PyMutex::new(self.inner.vars.hamt.lock().clone()),
            };
            Self {
                inner: ContextInner {
                    idx: AtomicUsize::new(usize::MAX),
                    vars: vars_copy.into_ref(&vm.ctx),
                    entered: AtomicBool::new(false),
                },
            }
        }

        fn __getitem__(
            &self,
            var: PyRef<ContextVar>,
            vm: &VirtualMachine,
        ) -> PyResult<PyObjectRef> {
            let item = self.borrow_vars().get(&*var).map(|item| item.to_owned());
            item.ok_or_else(|| vm.new_key_error(var.into()))
        }

        fn __len__(&self) -> usize {
            self.borrow_vars().len()
        }

        #[pymethod]
        fn get(
            &self,
            key: PyObjectRef,
            default: OptionalArg<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<Option<PyObjectRef>> {
            let key = context_check_key_type(&key, vm)?;
            let found = self.get_inner(key);
            if found.is_some() {
                Ok(found)
            } else {
                Ok(default.into_option())
            }
        }

        // TODO: wrong return type
        #[pymethod]
        fn keys(zelf: &Py<Self>) -> Vec<PyObjectRef> {
            let vars = zelf.borrow_vars();
            vars.keys().map(|key| key.to_owned().into()).collect()
        }

        // TODO: wrong return type
        #[pymethod]
        fn values(zelf: PyRef<Self>) -> Vec<PyObjectRef> {
            let vars = zelf.borrow_vars();
            vars.values().map(|value| value.to_owned()).collect()
        }

        // TODO: wrong return type
        #[pymethod]
        fn items(zelf: PyRef<Self>, vm: &VirtualMachine) -> Vec<PyObjectRef> {
            let vars = zelf.borrow_vars();
            vars.iter()
                .map(|(k, v)| vm.ctx.new_tuple(vec![k.clone().into(), v.clone()]).into())
                .collect()
        }
    }

    impl Constructor for PyContext {
        type Args = FuncArgs;

        fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            if !args.args.is_empty() || !args.kwargs.is_empty() {
                return Err(vm.new_type_error("Context() does not accept any arguments"));
            }
            Self::empty(vm).into_ref_with_type(vm, cls).map(Into::into)
        }

        fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
            unreachable!("use slot_new")
        }
    }

    impl AsMapping for PyContext {
        fn as_mapping() -> &'static PyMappingMethods {
            static AS_MAPPING: PyMappingMethods = PyMappingMethods {
                length: atomic_func!(|mapping, _vm| Ok(
                    PyContext::mapping_downcast(mapping).__len__()
                )),
                subscript: atomic_func!(|mapping, needle, vm| {
                    let needle = context_check_key_type(needle, vm)?;
                    PyContext::mapping_downcast(mapping)
                        .get_inner(needle)
                        .ok_or_else(|| vm.new_key_error(needle.to_owned().into()))
                }),
                ass_subscript: None,
            };
            &AS_MAPPING
        }
    }

    impl AsSequence for PyContext {
        fn as_sequence() -> &'static PySequenceMethods {
            static AS_SEQUENCE: LazyLock<PySequenceMethods> = LazyLock::new(|| PySequenceMethods {
                contains: atomic_func!(|seq, target, vm| {
                    let target = context_check_key_type(target, vm)?;
                    Ok(PyContext::sequence_downcast(seq).contains(target))
                }),
                ..PySequenceMethods::NOT_IMPLEMENTED
            });
            &AS_SEQUENCE
        }
    }

    impl Iterable for PyContext {
        fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            let vars = zelf.borrow_vars();
            let keys: Vec<PyObjectRef> = vars.keys().map(|k| k.clone().into()).collect();
            let list = vm.ctx.new_list(keys);
            <PyList as Iterable>::iter(list, vm)
        }
    }

    #[pyattr]
    #[pyclass(name, traverse)]
    #[derive(PyPayload)]
    struct ContextVar {
        #[pytraverse(skip)]
        name: String,
        default: Option<PyObjectRef>,
        #[pytraverse(skip)]
        cached: PyMutex<Option<ContextVarCache>>,
        #[pytraverse(skip)]
        cached_id: AtomicUsize, // cached_tsid in CPython
        #[pytraverse(skip)]
        hash: AtomicI64,
    }

    impl core::fmt::Debug for ContextVar {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.debug_struct("ContextVar").finish()
        }
    }

    impl PartialEq for ContextVar {
        fn eq(&self, other: &Self) -> bool {
            core::ptr::eq(self, other)
        }
    }
    impl Eq for ContextVar {}

    #[derive(Debug)]
    struct ContextVarCache {
        object: PyObjectRef, // value; cached in CPython
        idx: usize,          // Context index; cached_tsver in CPython
    }

    impl ContextVar {
        fn delete(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<()> {
            let cached = zelf.cached.lock().take();
            drop(cached);

            let ctx = PyContext::current(vm);

            let removed = ctx.borrow_vars_mut().swap_remove(zelf);
            let existed = removed.is_some();
            drop(removed);
            if !existed {
                // TODO:
                // PyErr_SetObject(PyExc_LookupError, (PyObject *)var);
                return Err(vm.new_lookup_error(zelf.as_object().repr(vm)?.as_wtf8().to_owned()));
            }

            Ok(())
        }

        // contextvar_set
        fn set_inner(zelf: &Py<Self>, value: PyObjectRef, vm: &VirtualMachine) {
            let ctx = PyContext::current(vm);

            let replaced = ctx.borrow_vars_mut().insert(zelf.to_owned(), value.clone());
            drop(replaced);

            // cached and cached_id are one snapshot: another thread's get()
            // must not observe a new id with an old value, or the reverse.
            let cache = ContextVarCache {
                object: value,
                idx: ctx.inner.idx.load(Ordering::Relaxed),
            };
            let mut cached = zelf.cached.lock();
            zelf.cached_id.store(ctx.get_id(), Ordering::SeqCst);
            let replaced = cached.replace(cache);
            drop(cached);
            drop(replaced);
        }

        fn generate_hash(zelf: &Py<Self>, name_hash: PyHash) -> PyHash {
            let pointer_hash = crate::common::hash::hash_pointer(zelf.as_object().get_id());
            let hash = pointer_hash ^ name_hash;
            if hash == -1 { -2 } else { hash }
        }
    }

    #[pyclass(with(Constructor, Hashable, Representable))]
    impl ContextVar {
        #[pygetset]
        fn name(&self) -> String {
            self.name.clone()
        }

        #[pymethod]
        fn get(
            zelf: &Py<Self>,
            default: OptionalArg<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<Option<PyObjectRef>> {
            // The replaced cache entry comes back out so that dropping it, which
            // can run a __del__ that calls back in, happens with no lock held.
            let (found, replaced) = super::CONTEXTS.with_borrow(|ctxs| {
                let Some(ctx) = ctxs.last() else {
                    return (None, None);
                };
                let mut cached = zelf.cached.lock();
                if let Some(cached) = &*cached
                    && zelf.cached_id.load(Ordering::SeqCst) == ctx.get_id()
                    && cached.idx + 1 == ctxs.len()
                {
                    return (Some(cached.object.clone()), None);
                }
                let Some(obj) = ctx.borrow_vars().get(zelf).map(|obj| obj.to_owned()) else {
                    return (None, None);
                };
                zelf.cached_id.store(ctx.get_id(), Ordering::SeqCst);

                let replaced = cached.replace(ContextVarCache {
                    object: obj.clone(),
                    idx: ctxs.len() - 1,
                });

                (Some(obj), replaced)
            });
            drop(replaced);

            let value = if let Some(value) = found {
                value
            } else if let Some(default) = default.into_option() {
                default
            } else if let Some(default) = zelf.default.as_ref() {
                default.clone()
            } else {
                return Err(vm.new_lookup_error(zelf.as_object().repr(vm)?.as_wtf8().to_owned()));
            };
            Ok(Some(value))
        }

        #[pymethod]
        fn set(zelf: &Py<Self>, value: PyObjectRef, vm: &VirtualMachine) -> PyRef<ContextToken> {
            let ctx = PyContext::current(vm);

            let old_value = ctx.borrow_vars().get(zelf).map(|v| v.to_owned());
            let token = ContextToken {
                ctx,
                var: zelf.to_owned(),
                old_value,
                used: false.into(),
            };

            // ctx.vars borrow must be released
            Self::set_inner(zelf, value, vm);

            token.into_ref(&vm.ctx)
        }

        #[pymethod]
        fn reset(zelf: &Py<Self>, token: PyRef<ContextToken>, vm: &VirtualMachine) -> PyResult<()> {
            if token.used.load(Ordering::Acquire) {
                return Err(vm.new_runtime_error(format!(
                    "{} has already been used once",
                    token.as_object().repr(vm)?
                )));
            }

            if !zelf.is(&token.var) {
                return Err(vm.new_value_error(format!(
                    "{} was created by a different ContextVar",
                    token.var.as_object().repr(vm)?
                )));
            }

            let ctx = PyContext::current(vm);
            if !ctx.is(&token.ctx) {
                return Err(vm.new_value_error(format!(
                    "{} was created in a different Context",
                    token.var.as_object().repr(vm)?
                )));
            }

            token.used.store(true, Ordering::Release);

            if let Some(old_value) = &token.old_value {
                Self::set_inner(zelf, old_value.clone(), vm);
            } else {
                Self::delete(zelf, vm)?;
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

    impl Constructor for ContextVar {
        type Args = FuncArgs;

        fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            let mut args = args;
            if args.args.len() != 1 {
                return Err(vm.new_type_error(format!(
                    "ContextVar() takes exactly 1 argument ({} given)",
                    args.args.len()
                )));
            }
            let default = args.take_keyword("default");
            if let Some((name, _)) = args.kwargs.first() {
                return Err(vm.new_type_error(format!(
                    "ContextVar() got an unexpected keyword argument '{name}'"
                )));
            }

            let name = args.args.swap_remove(0);
            let name = name
                .downcast::<PyStr>()
                .map_err(|_| vm.new_type_error("context variable name must be a str"))?;
            let name_hash = name.as_object().hash(vm)?;
            let name = name.to_string();

            let var = Self {
                name,
                default,
                cached_id: 0.into(),
                cached: PyMutex::new(None),
                hash: AtomicI64::new(0),
            };
            let py_var = var.into_ref_with_type(vm, cls)?;
            let hash = Self::generate_hash(&py_var, name_hash);
            py_var.hash.store(hash, Ordering::Relaxed);
            Ok(py_var.into())
        }

        fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
            unreachable!("use slot_new")
        }
    }

    impl core::hash::Hash for ContextVar {
        #[inline]
        fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
            self.hash.load(Ordering::Relaxed).hash(state)
        }
    }

    impl Hashable for ContextVar {
        #[inline]
        fn hash(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<PyHash> {
            Ok(zelf.hash.load(Ordering::Relaxed))
        }
    }

    impl Representable for ContextVar {
        #[inline]
        fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
            let name = zelf.name.as_str();
            let id = zelf.get_id();

            Ok(if let Some(arg) = zelf.default.as_ref() {
                let default = arg.str(vm).ok();
                format!("<ContextVar name='{name}' default={default:?} at {id:#x}>",)
            } else {
                format!("<ContextVar name='{name}' at {id:#x}>")
            })
        }
    }

    #[pyattr]
    #[pyclass(name = "Token")]
    #[derive(Debug, PyPayload)]
    struct ContextToken {
        ctx: PyRef<PyContext>,          // tok_ctx in CPython
        var: PyRef<ContextVar>,         // tok_var in CPython
        old_value: Option<PyObjectRef>, // tok_oldval in CPython
        used: AtomicBool,
    }

    #[pyclass(with(Constructor, Representable))]
    impl ContextToken {
        #[pygetset]
        fn var(&self, _vm: &VirtualMachine) -> PyRef<ContextVar> {
            self.var.clone()
        }

        #[pygetset]
        fn old_value(&self, _vm: &VirtualMachine) -> PyObjectRef {
            match &self.old_value {
                Some(value) => value.clone(),
                None => ContextTokenMissing::static_type().to_owned().into(),
            }
        }

        #[pyclassmethod]
        fn __class_getitem__(
            cls: PyTypeRef,
            args: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyGenericAlias> {
            PyGenericAlias::from_args(cls, args, vm)
        }

        #[pymethod]
        fn __enter__(zelf: PyRef<Self>) -> PyRef<Self> {
            zelf
        }

        #[pymethod]
        fn __exit__(
            zelf: &Py<Self>,
            _ty: PyObjectRef,
            _val: PyObjectRef,
            _tb: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            ContextVar::reset(&zelf.var, zelf.to_owned(), vm)
        }
    }

    impl Constructor for ContextToken {
        type Args = FuncArgs;

        fn slot_new(_cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            Err(vm.new_runtime_error("Tokens can only be created by ContextVars"))
        }

        fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
            unimplemented!("use slot_new")
        }
    }

    impl Representable for ContextToken {
        #[inline]
        fn repr_wtf8(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<Wtf8Buf> {
            let used = if zelf.used.load(Ordering::Acquire) {
                " used"
            } else {
                ""
            };
            let var = Representable::repr_wtf8(&zelf.var, vm)?;
            let ptr = zelf.as_object().get_id() as *const u8;
            let mut result = Wtf8Buf::from(format!("<Token{used} var="));
            result.push_wtf8(&var);
            result.push_str(&format!(" at {ptr:p}>"));
            Ok(result)
        }
    }

    #[pyclass(no_attr, name = "Token.MISSING")]
    #[derive(Debug, PyPayload)]
    pub(super) struct ContextTokenMissing {}

    #[pyclass(with(Representable))]
    impl ContextTokenMissing {}

    impl Representable for ContextTokenMissing {
        fn repr_str(_zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            Ok("<Token.MISSING>".to_owned())
        }
    }

    #[pyfunction]
    fn copy_context(vm: &VirtualMachine) -> PyContext {
        PyContext::current(vm).copy(vm)
    }

    // Set Token.MISSING attribute
    pub(crate) fn module_exec(
        vm: &VirtualMachine,
        module: &Py<crate::vm::builtins::PyModule>,
    ) -> PyResult<()> {
        __module_exec(vm, module);

        let token_type = module.get_attr("Token", vm)?;
        token_type.set_attr("MISSING", ContextTokenMissing::static_type().to_owned(), vm)?;

        Ok(())
    }
}

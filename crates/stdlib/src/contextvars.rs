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
        builtins::{PyGenericAlias, PyList, PyStrRef, PyType, PyTypeRef},
        class::PyClassDef,
        class::StaticType,
        common::{
            hash::PyHash,
            lock::{LazyLock, PyMutex},
            wtf8::Wtf8Buf,
        },
        function::{ArgCallable, FuncArgs, OptionalArg},
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

    #[pyclass(with(Constructor, AsMapping, AsSequence, Iterable))]
    impl PyContext {
        #[pymethod]
        fn run(
            zelf: &Py<Self>,
            callable: ArgCallable,
            args: FuncArgs,
            vm: &VirtualMachine,
        ) -> PyResult {
            Self::enter(zelf, vm)?;
            let result = callable.invoke(args, vm);
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
            key: PyRef<ContextVar>,
            default: OptionalArg<PyObjectRef>,
        ) -> Option<PyObjectRef> {
            let found = self.get_inner(&key);
            if found.is_some() {
                found
            } else {
                default.into_option()
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
        type Args = ();
        fn py_new(_cls: &Py<PyType>, _args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            Ok(Self::empty(vm))
        }
    }

    impl AsMapping for PyContext {
        fn as_mapping() -> &'static PyMappingMethods {
            static AS_MAPPING: PyMappingMethods = PyMappingMethods {
                length: atomic_func!(|mapping, _vm| Ok(
                    PyContext::mapping_downcast(mapping).__len__()
                )),
                subscript: atomic_func!(|mapping, needle, vm| {
                    let needle = needle.try_to_value(vm)?;
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
                    let target = target.try_to_value(vm)?;
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

        // contextvar_set in CPython
        fn set_inner(zelf: &Py<Self>, value: PyObjectRef, vm: &VirtualMachine) {
            let ctx = PyContext::current(vm);

            let replaced = ctx.borrow_vars_mut().insert(zelf.to_owned(), value.clone());
            drop(replaced);

            zelf.cached_id.store(ctx.get_id(), Ordering::SeqCst);

            let cache = ContextVarCache {
                object: value,
                idx: ctx.inner.idx.load(Ordering::Relaxed),
            };
            let replaced = zelf.cached.lock().replace(cache);
            drop(replaced);
        }

        fn generate_hash(zelf: &Py<Self>, vm: &VirtualMachine) -> PyHash {
            let name_hash = vm.state.hash_secret.hash_str(&zelf.name);
            let pointer_hash = crate::common::hash::hash_pointer(zelf.as_object().get_id());
            pointer_hash ^ name_hash
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

    #[derive(FromArgs)]
    struct ContextVarOptions {
        #[pyarg(positional)]
        name: PyStrRef,
        #[pyarg(any, optional)]
        default: OptionalArg<PyObjectRef>,
    }

    impl Constructor for ContextVar {
        type Args = ContextVarOptions;

        fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            let args: Self::Args = args.bind_for(vm, Self::NAME)?;
            let var = Self {
                name: args.name.to_string(),
                default: args.default.into_option(),
                cached_id: 0.into(),
                cached: PyMutex::new(None),
                hash: AtomicI64::new(0),
            };
            let py_var = var.into_ref_with_type(vm, cls)?;

            let hash = Self::generate_hash(&py_var, vm);
            py_var.hash.store(hash, Ordering::Relaxed);
            Ok(py_var.into())
        }

        fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
            unimplemented!("use slot_new")
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

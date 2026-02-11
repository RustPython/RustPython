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
        class::StaticType,
        common::hash::PyHash,
        function::{ArgCallable, FuncArgs, OptionalArg},
        protocol::{PyMappingMethods, PySequenceMethods},
        types::{AsMapping, AsSequence, Constructor, Hashable, Iterable, Representable},
    };
    use core::{
        cell::{Cell, RefCell, UnsafeCell},
        sync::atomic::Ordering,
    };
    use crossbeam_utils::atomic::AtomicCell;
    use indexmap::IndexMap;
    use rustpython_common::lock::LazyLock;

    // TODO: Real hamt implementation
    type Hamt = IndexMap<PyRef<ContextVar>, PyObjectRef, ahash::RandomState>;

    #[pyclass(no_attr, name = "Hamt", module = "contextvars")]
    #[derive(Debug, PyPayload)]
    pub(crate) struct HamtObject {
        hamt: RefCell<Hamt>,
    }

    #[pyclass]
    impl HamtObject {}

    impl Default for HamtObject {
        fn default() -> Self {
            Self {
                hamt: RefCell::new(Hamt::default()),
            }
        }
    }

    unsafe impl Sync for HamtObject {}

    #[derive(Debug)]
    struct ContextInner {
        idx: Cell<usize>,
        vars: PyRef<HamtObject>,
        // PyObject *ctx_weakreflist;
        entered: Cell<bool>,
    }

    unsafe impl Sync for ContextInner {}

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
                    idx: Cell::new(usize::MAX),
                    vars: HamtObject::default().into_ref(&vm.ctx),
                    entered: Cell::new(false),
                },
            }
        }

        fn borrow_vars(&self) -> impl core::ops::Deref<Target = Hamt> + '_ {
            self.inner.vars.hamt.borrow()
        }

        fn borrow_vars_mut(&self) -> impl core::ops::DerefMut<Target = Hamt> + '_ {
            self.inner.vars.hamt.borrow_mut()
        }

        fn enter(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<()> {
            if zelf.inner.entered.get() {
                let msg = format!(
                    "cannot enter context: {} is already entered",
                    zelf.as_object().repr(vm)?
                );
                return Err(vm.new_runtime_error(msg));
            }

            super::CONTEXTS.with_borrow_mut(|ctxs| {
                zelf.inner.idx.set(ctxs.len());
                ctxs.push(zelf.to_owned());
            });
            zelf.inner.entered.set(true);

            Ok(())
        }

        fn exit(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<()> {
            if !zelf.inner.entered.get() {
                let msg = format!(
                    "cannot exit context: {} is not entered",
                    zelf.as_object().repr(vm)?
                );
                return Err(vm.new_runtime_error(msg));
            }

            super::CONTEXTS.with_borrow_mut(|ctxs| {
                let err_msg =
                    "cannot exit context: thread state references a different context object";
                ctxs.pop_if(|ctx| ctx.get_id() == zelf.get_id())
                    .map(drop)
                    .ok_or_else(|| vm.new_runtime_error(err_msg))
            })?;
            zelf.inner.entered.set(false);

            Ok(())
        }

        fn current(vm: &VirtualMachine) -> PyRef<Self> {
            super::CONTEXTS.with_borrow_mut(|ctxs| {
                if let Some(ctx) = ctxs.last() {
                    ctx.clone()
                } else {
                    let ctx = Self::empty(vm);
                    ctx.inner.idx.set(0);
                    ctx.inner.entered.set(true);
                    let ctx = ctx.into_ref(&vm.ctx);
                    ctxs.push(ctx);
                    ctxs[0].clone()
                }
            })
        }

        fn contains(&self, needle: &Py<ContextVar>) -> PyResult<bool> {
            let vars = self.borrow_vars();
            Ok(vars.get(needle).is_some())
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
                hamt: RefCell::new(self.inner.vars.hamt.borrow().clone()),
            };
            Self {
                inner: ContextInner {
                    idx: Cell::new(usize::MAX),
                    vars: vars_copy.into_ref(&vm.ctx),
                    entered: Cell::new(false),
                },
            }
        }

        fn __getitem__(
            &self,
            var: PyRef<ContextVar>,
            vm: &VirtualMachine,
        ) -> PyResult<PyObjectRef> {
            let vars = self.borrow_vars();
            let item = vars
                .get(&*var)
                .ok_or_else(|| vm.new_key_error(var.into()))?;
            Ok(item.to_owned())
        }

        fn __len__(&self) -> usize {
            self.borrow_vars().len()
        }

        #[pymethod]
        fn get(
            &self,
            key: PyRef<ContextVar>,
            default: OptionalArg<PyObjectRef>,
        ) -> PyResult<Option<PyObjectRef>> {
            let found = self.get_inner(&key);
            let result = if let Some(found) = found {
                Some(found.to_owned())
            } else {
                default.into_option()
            };
            Ok(result)
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
                    let found = PyContext::mapping_downcast(mapping).get_inner(needle);
                    if let Some(found) = found {
                        Ok(found.to_owned())
                    } else {
                        Err(vm.new_key_error(needle.to_owned().into()))
                    }
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
                    PyContext::sequence_downcast(seq).contains(target)
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
        cached: AtomicCell<Option<ContextVarCache>>,
        #[pytraverse(skip)]
        cached_id: core::sync::atomic::AtomicUsize, // cached_tsid in CPython
        #[pytraverse(skip)]
        hash: UnsafeCell<PyHash>,
    }

    impl core::fmt::Debug for ContextVar {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.debug_struct("ContextVar").finish()
        }
    }

    unsafe impl Sync for ContextVar {}

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
            zelf.cached.store(None);

            let ctx = PyContext::current(vm);

            let mut vars = ctx.borrow_vars_mut();
            if vars.swap_remove(zelf).is_none() {
                // TODO:
                // PyErr_SetObject(PyExc_LookupError, (PyObject *)var);
                let msg = zelf.as_object().repr(vm)?.as_str().to_owned();
                return Err(vm.new_lookup_error(msg));
            }

            Ok(())
        }

        // contextvar_set in CPython
        fn set_inner(zelf: &Py<Self>, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            let ctx = PyContext::current(vm);

            let mut vars = ctx.borrow_vars_mut();
            vars.insert(zelf.to_owned(), value.clone());

            zelf.cached_id.store(ctx.get_id(), Ordering::SeqCst);

            let cache = ContextVarCache {
                object: value,
                idx: ctx.inner.idx.get(),
            };
            zelf.cached.store(Some(cache));

            Ok(())
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
            let found = super::CONTEXTS.with_borrow(|ctxs| {
                let ctx = ctxs.last()?;
                let cached_ptr = zelf.cached.as_ptr();
                debug_assert!(!cached_ptr.is_null());
                if let Some(cached) = unsafe { &*cached_ptr }
                    && zelf.cached_id.load(Ordering::SeqCst) == ctx.get_id()
                    && cached.idx + 1 == ctxs.len()
                {
                    return Some(cached.object.clone());
                }
                let vars = ctx.borrow_vars();
                let obj = vars.get(zelf)?;
                zelf.cached_id.store(ctx.get_id(), Ordering::SeqCst);

                // TODO: ensure cached is not changed
                let _removed = zelf.cached.swap(Some(ContextVarCache {
                    object: obj.clone(),
                    idx: ctxs.len() - 1,
                }));

                Some(obj.clone())
            });

            let value = if let Some(value) = found {
                value
            } else if let Some(default) = default.into_option() {
                default
            } else if let Some(default) = zelf.default.as_ref() {
                default.clone()
            } else {
                let msg = zelf.as_object().repr(vm)?;
                return Err(vm.new_lookup_error(msg.as_str().to_owned()));
            };
            Ok(Some(value))
        }

        #[pymethod]
        fn set(
            zelf: &Py<Self>,
            value: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<ContextToken>> {
            let ctx = PyContext::current(vm);

            let old_value = ctx.borrow_vars().get(zelf).map(|v| v.to_owned());
            let token = ContextToken {
                ctx: ctx.to_owned(),
                var: zelf.to_owned(),
                old_value,
                used: false.into(),
            };

            // ctx.vars borrow must be released
            Self::set_inner(zelf, value, vm)?;

            Ok(token.into_ref(&vm.ctx))
        }

        #[pymethod]
        fn reset(zelf: &Py<Self>, token: PyRef<ContextToken>, vm: &VirtualMachine) -> PyResult<()> {
            if token.used.get() {
                let msg = format!("{} has already been used once", token.as_object().repr(vm)?);
                return Err(vm.new_runtime_error(msg));
            }

            if !zelf.is(&token.var) {
                let msg = format!(
                    "{} was created by a different ContextVar",
                    token.var.as_object().repr(vm)?
                );
                return Err(vm.new_value_error(msg));
            }

            let ctx = PyContext::current(vm);
            if !ctx.is(&token.ctx) {
                let msg = format!(
                    "{} was created in a different Context",
                    token.var.as_object().repr(vm)?
                );
                return Err(vm.new_value_error(msg));
            }

            token.used.set(true);

            if let Some(old_value) = &token.old_value {
                Self::set_inner(zelf, old_value.clone(), vm)?;
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
        ) -> PyGenericAlias {
            PyGenericAlias::from_args(cls, args, vm)
        }
    }

    #[derive(FromArgs)]
    struct ContextVarOptions {
        #[pyarg(positional)]
        #[allow(dead_code)] // TODO: RUSTPYTHON
        name: PyStrRef,
        #[pyarg(any, optional)]
        #[allow(dead_code)] // TODO: RUSTPYTHON
        default: OptionalArg<PyObjectRef>,
    }

    impl Constructor for ContextVar {
        type Args = ContextVarOptions;

        fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            let args: Self::Args = args.bind(vm)?;
            let var = Self {
                name: args.name.to_string(),
                default: args.default.into_option(),
                cached_id: 0.into(),
                cached: AtomicCell::new(None),
                hash: UnsafeCell::new(0),
            };
            let py_var = var.into_ref_with_type(vm, cls)?;

            unsafe {
                // SAFETY: py_var is not exposed to python memory model yet
                *py_var.hash.get() = Self::generate_hash(&py_var, vm)
            };
            Ok(py_var.into())
        }

        fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
            unimplemented!("use slot_new")
        }
    }

    impl core::hash::Hash for ContextVar {
        #[inline]
        fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
            unsafe { *self.hash.get() }.hash(state)
        }
    }

    impl Hashable for ContextVar {
        #[inline]
        fn hash(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<PyHash> {
            Ok(unsafe { *zelf.hash.get() })
        }
    }

    impl Representable for ContextVar {
        #[inline]
        fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
            // unimplemented!("<ContextVar name={{}} default={{}} at {{}}")
            Ok(format!(
                "<ContextVar name={} default={:?} at {:#x}>",
                zelf.name.as_str(),
                zelf.default
                    .as_ref()
                    .and_then(|default| default.str(vm).ok()),
                zelf.get_id()
            ))
        }
    }

    #[pyattr]
    #[pyclass(name = "Token")]
    #[derive(Debug, PyPayload)]
    struct ContextToken {
        ctx: PyRef<PyContext>,          // tok_ctx in CPython
        var: PyRef<ContextVar>,         // tok_var in CPython
        old_value: Option<PyObjectRef>, // tok_oldval in CPython
        used: Cell<bool>,
    }

    unsafe impl Sync for ContextToken {}

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
        ) -> PyGenericAlias {
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
        fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
            let used = if zelf.used.get() { " used" } else { "" };
            let var = Representable::repr_str(&zelf.var, vm)?;
            let ptr = zelf.as_object().get_id() as *const u8;
            Ok(format!("<Token{used} var={var} at {ptr:p}>"))
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

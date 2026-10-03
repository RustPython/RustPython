//! Native PEP 810 deferred bindings. Dictionaries themselves always expose raw values.

use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    builtins::{PyCode, PyModule, PySet, PyStr, PyStrRef, PyTuple, PyType},
    class::PyClassImpl,
    common::lock::{OnceCell, PyMutex},
    frame,
    types::{Constructor, GetAttr, Representable},
};
use core::sync::atomic::{AtomicBool, Ordering};
use std::collections::{HashMap, HashSet};

pub(crate) struct LazyImportsState {
    pub(crate) modules: PyRef<PySet>,
    pub(crate) default_callback: OnceCell<PyObjectRef>,
    pub(crate) all: AtomicBool,
    filter: PyMutex<Option<PyObjectRef>>,
    // These names are not Python objects, and this lock is never held across Python calls.
    pending: PyMutex<HashMap<String, HashSet<String>>>,
    resolving: PyMutex<HashSet<usize>>,
}

impl LazyImportsState {
    pub(crate) fn new(ctx: &Context) -> Self {
        Self {
            modules: PySet::default().into_ref(ctx),
            default_callback: OnceCell::new(),
            all: AtomicBool::new(false),
            filter: PyMutex::default(),
            pending: PyMutex::default(),
            resolving: PyMutex::default(),
        }
    }

    pub(crate) fn filter(&self) -> Option<PyObjectRef> {
        self.filter.lock().clone()
    }

    pub(crate) fn set_filter(&self, filter: Option<PyObjectRef>) {
        // A removed callable's finalizer can re-enter this API.
        let old = core::mem::replace(&mut *self.filter.lock(), filter);
        drop(old);
    }

    fn register(&self, name: &Py<PyStr>) {
        let Some(name) = name.to_str() else {
            return;
        };
        let mut pending = self.pending.lock();
        let mut full = name;
        while let Some((parent, child)) = full.rsplit_once('.') {
            pending
                .entry(parent.to_owned())
                .or_default()
                .insert(child.to_owned());
            full = parent;
        }
    }
}

#[pyclass(module = false, name = "lazy_import", traverse)]
#[derive(Debug)]
pub(crate) struct PyLazyImport {
    builtins: PyObjectRef,
    name: PyStrRef,
    // None is an import, a tuple is an import with fromlist, and a string is one member.
    fromlist: Option<PyObjectRef>,
    declaration: Option<PyRef<PyCode>>,
    #[pytraverse(skip)]
    instruction: u32,
}

impl PyPayload for PyLazyImport {
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.lazy_import_type
    }
}

impl PyLazyImport {
    fn new(builtins: PyObjectRef, name: PyStrRef, fromlist: Option<PyObjectRef>) -> Self {
        let frame = frame::current_thread_iframe();
        let (declaration, instruction) = if frame.is_null() {
            (None, 0)
        } else {
            // The thread's active frame is live for this call; retain its code, not the frame.
            let frame = unsafe { &*frame };
            (Some(frame.code().to_owned()), frame.get_lasti())
        };
        Self {
            builtins,
            name,
            fromlist,
            declaration,
            instruction,
        }
    }

    fn display_name(&self) -> String {
        match &self.fromlist {
            Some(attr) if attr.downcast_ref::<PyStr>().is_some() => {
                format!("{}.{}", self.name, attr.downcast_ref::<PyStr>().unwrap())
            }
            Some(_) => format!("{}...", self.name),
            None => self.name.to_string(),
        }
    }
}

#[pyclass(with(Constructor, GetAttr, Representable))]
impl Py<PyLazyImport> {
    /// Resolve this import and return its value without changing a containing dictionary.
    #[pymethod]
    fn resolve(&self, vm: &VirtualMachine) -> PyResult {
        resolve(self, vm)
    }
}

impl Constructor for PyLazyImport {
    type Args = crate::function::FuncArgs;
    fn py_new(_cls: &Py<PyType>, _args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
        Err(vm.new_type_error("cannot create 'lazy_import' instances"))
    }
}

impl GetAttr for PyLazyImport {
    fn getattro(zelf: &Py<Self>, name: &Py<PyStr>, vm: &VirtualMachine) -> PyResult {
        if let Some(value) = zelf.as_object().generic_getattr_opt(name, None, vm)? {
            return Ok(value);
        }
        Err(vm.new_attribute_error(format!(
            "cannot access attribute '{}' on unresolved lazy import '{}'",
            name,
            zelf.display_name()
        )))
    }
}

impl Representable for PyLazyImport {
    fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
        Ok(format!("<lazy_import '{}'>", zelf.display_name()))
    }
}

pub(crate) fn init_type(ctx: &'static Context) {
    PyLazyImport::extend_class(ctx, ctx.types.lazy_import_type);
}

pub(crate) fn is_module_level(vm: &VirtualMachine) -> bool {
    let frame = frame::current_thread_iframe();
    if frame.is_null() {
        return false;
    }
    let frame = unsafe { &*frame };
    frame.globals().as_object().is(frame.locals.as_object(vm))
}

fn optional_item(
    mapping: &PyObject,
    name: &Py<PyStr>,
    vm: &VirtualMachine,
) -> PyResult<Option<PyObjectRef>> {
    match mapping.get_item(name, vm) {
        Ok(value) => Ok(Some(value)),
        Err(exc) if exc.fast_isinstance(vm.ctx.exceptions.key_error) => Ok(None),
        Err(exc) => Err(exc),
    }
}

pub(crate) fn import_name(
    name: &Py<PyStr>,
    fromlist: PyObjectRef,
    level: usize,
    policy: u32,
    vm: &VirtualMachine,
) -> PyResult {
    if policy & 2 != 0 {
        return vm.import_from(name, fromlist, level);
    }
    let explicit = policy & 1 != 0;
    let globals = vm.current_globals();
    let mut lazy =
        explicit || (vm.state.lazy_imports.all.load(Ordering::Acquire) && is_module_level(vm));
    if !lazy
        && is_module_level(vm)
        && let Some(names) = optional_item(
            globals.as_object(),
            vm.ctx.intern_str("__lazy_modules__"),
            vm,
        )?
    {
        let absolute =
            crate::import::absolute_import_name(name, Some(globals.as_object()), level as i32, vm)?;
        lazy = names
            .sequence_unchecked()
            .contains(absolute.as_object(), vm)?;
    }
    if !lazy {
        return vm.import_from(name, fromlist, level);
    }
    let builtins = frame::current_builtins().unwrap_or_else(|| vm.builtins.dict().into());
    let callback = optional_item(&builtins, vm.ctx.intern_str("__lazy_import__"), vm)?
        .ok_or_else(|| vm.new_import_error("__lazy_import__ not found", name.to_owned()))?;
    if vm
        .state
        .lazy_imports
        .default_callback
        .get()
        .is_some_and(|default| callback.is(default))
    {
        return create(
            name,
            globals.as_object(),
            Some(fromlist),
            level as i32,
            builtins,
            vm,
        );
    }
    let locals: PyObjectRef = vm.current_locals()?.into();
    callback.call(
        (name.to_owned(), globals, locals, fromlist, level, builtins),
        vm,
    )
}

pub(crate) fn create(
    name: &Py<PyStr>,
    globals: &PyObject,
    fromlist: Option<PyObjectRef>,
    level: i32,
    builtins: PyObjectRef,
    vm: &VirtualMachine,
) -> PyResult {
    let absolute = crate::import::absolute_import_name(name, Some(globals), level, vm)?;
    if !is_module_level(vm) {
        return Err(vm.new_exception_msg(
            vm.ctx.exceptions.syntax_error.to_owned(),
            "'lazy import' is only allowed at module level".into(),
        ));
    }
    if let Some(filter) = vm.state.lazy_imports.filter() {
        let globals_dict = crate::builtins::PyAnyDictRef::from_object(globals)
            .ok_or_else(|| vm.new_type_error("globals must be a dict or a frozendict"))?;
        let importer = globals_dict
            .inner_getitem_opt(identifier!(vm, __name__), vm)?
            .unwrap_or_else(|| vm.ctx.none());
        let from = fromlist.clone().unwrap_or_else(|| vm.ctx.none());
        if !filter
            .call((importer, absolute.clone(), from), vm)?
            .try_to_bool(vm)?
        {
            return crate::import::import_module_level(name, Some(globals), fromlist, level, vm);
        }
    }
    let fromlist = match fromlist.filter(|value| !vm.is_none(value)) {
        Some(value) if value.downcast_ref::<PyStr>().is_some() => {
            Some(PyTuple::new_ref(vec![value], &vm.ctx).into())
        }
        Some(value) if value.downcast_ref::<PyTuple>().is_some() => Some(value),
        Some(_) => {
            return Err(
                vm.new_type_error("lazy_import: fromlist must be None, a string, or a tuple")
            );
        }
        None => None,
    };
    let deferred =
        PyLazyImport::new(builtins, absolute.clone(), fromlist.clone()).into_ref(&vm.ctx);
    let state = &vm.state.lazy_imports;
    state.modules.add(absolute.clone().into(), vm)?;
    if let Some(names) = fromlist
        .as_ref()
        .and_then(|value| value.downcast_ref::<PyTuple>())
        && !names.as_slice().is_empty()
    {
        for name in names.as_slice() {
            let name = name
                .downcast_ref::<PyStr>()
                .ok_or_else(|| vm.new_type_error("fromlist items must be str"))?;
            let fullname = vm.ctx.new_str(format!("{absolute}.{name}"));
            state.modules.add(fullname.clone().into(), vm)?;
            state.register(&fullname);
        }
    } else {
        state.register(&absolute);
    }
    Ok(deferred.into())
}

pub(crate) fn import_from(
    deferred: &Py<PyLazyImport>,
    name: &Py<PyStr>,
    vm: &VirtualMachine,
) -> PyResult {
    if let Some(module) = crate::import::get_imported_module(&deferred.name, vm)?
        && let Some(module) = module.downcast_ref::<PyModule>()
        && let Some(value) = module.dict().get_item_opt(name, vm)?
    {
        return Ok(value);
    }
    let from = match &deferred.fromlist {
        Some(attr) if attr.downcast_ref::<PyStr>().is_some() => vm.ctx.new_str(format!(
            "{}.{}",
            deferred.name,
            attr.downcast_ref::<PyStr>().unwrap()
        )),
        None => match deferred.name.to_str().and_then(|name| name.split_once('.')) {
            Some((top, _)) => vm.ctx.new_str(top),
            None => deferred.name.clone(),
        },
        _ => deferred.name.clone(),
    };
    Ok(PyLazyImport::new(
        deferred.builtins.clone(),
        from,
        Some(name.to_owned().into()),
    )
    .into_ref(&vm.ctx)
    .into())
}

struct ResolvingGuard<'a> {
    state: &'a LazyImportsState,
    identity: usize,
}

impl Drop for ResolvingGuard<'_> {
    fn drop(&mut self) {
        self.state.resolving.lock().remove(&self.identity);
    }
}

pub(crate) fn resolve(deferred: &Py<PyLazyImport>, vm: &VirtualMachine) -> PyResult {
    let _lock = crate::stdlib::_imp::ImportLockGuard::acquire(vm);
    let identity = deferred.as_object().get_id();
    if !vm.state.lazy_imports.resolving.lock().insert(identity) {
        let exc = vm.new_exception_msg(
            vm.ctx.exceptions.import_cycle_error.to_owned(),
            format!(
                "cannot import name '{}' (most likely due to a circular import)",
                deferred.display_name()
            )
            .into(),
        );
        exc.as_object()
            .set_attr("name", deferred.name.clone(), vm)?;
        return Err(exc);
    }
    let _resolving = ResolvingGuard {
        state: &vm.state.lazy_imports,
        identity,
    };
    let result = (|| {
        let callback = optional_item(&deferred.builtins, vm.ctx.intern_str("__import__"), vm)?
            .ok_or_else(|| vm.new_import_error("__import__ not found", deferred.name.clone()))?;
        let fromlist = match &deferred.fromlist {
            Some(value) if value.downcast_ref::<PyStr>().is_some() => {
                PyTuple::new_ref(vec![value.clone()], &vm.ctx).into()
            }
            Some(value) => value.clone(),
            None => vm.ctx.none(),
        };
        let globals =
            frame::current_globals().map_or_else(|| vm.ctx.none(), |g| g.as_object().to_owned());
        let module = callback
            .call(
                (
                    deferred.name.clone(),
                    globals.clone(),
                    globals,
                    fromlist,
                    0i32,
                ),
                vm,
            )
            .inspect_err(|exc| crate::import::remove_importlib_frames(vm, exc))?;
        if let Some(attr) = deferred
            .fromlist
            .as_ref()
            .and_then(|v| v.downcast_ref::<PyStr>())
        {
            crate::import::import_from_attribute(&module, attr, vm)
        } else {
            Ok(module)
        }
    })();
    result.inspect_err(|exc| {
        if let Some(code) = &deferred.declaration {
            let cause = vm.new_import_error(
                format!(
                    "lazy import of '{}' raised an exception during resolution",
                    deferred.display_name()
                ),
                deferred.name.clone(),
            );
            let offset = deferred.instruction.saturating_sub(1) as usize;
            if let Some((location, _)) = code.locations.get(offset) {
                let globals = vm.ctx.new_dict();
                let frame = frame::FrameObject::new_ref(
                    code.clone(),
                    crate::scope::Scope::new(None, globals),
                    deferred.builtins.clone(),
                    &[],
                    None,
                    false,
                    vm,
                );
                frame.set_lasti(deferred.instruction);
                let tb = crate::builtins::PyTraceback::new(
                    None,
                    frame,
                    (offset * 2) as i32,
                    location.line,
                )
                .into_ref(&vm.ctx);
                cause.set_traceback(Some(tb));
            }
            exc.set_cause(Some(cause));
        }
    })
}

pub(crate) fn resolve_value(value: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    match value.downcast_ref_if_exact::<PyLazyImport>(vm) {
        Some(deferred) => resolve(deferred, vm),
        None => Ok(value),
    }
}

pub(crate) fn try_load_submodule(
    module: &Py<PyModule>,
    name: &Py<PyStr>,
    vm: &VirtualMachine,
) -> PyResult<Option<PyObjectRef>> {
    let Some(module_name) = module.dict().get_item_opt(identifier!(vm, __name__), vm)? else {
        return Ok(None);
    };
    let Some(module_name) = module_name
        .downcast_ref::<PyStr>()
        .and_then(|name| name.to_str())
    else {
        return Ok(None);
    };
    let Some(child) = name.to_str() else {
        return Ok(None);
    };
    let pending = vm
        .state
        .lazy_imports
        .pending
        .lock()
        .get(module_name)
        .is_some_and(|children| children.contains(child));
    if !pending {
        return Ok(None);
    }
    let fullname = vm.ctx.new_str(format!("{module_name}.{child}"));
    let loader = vm.importlib.get_attr("_find_and_load_lazy_submodule", vm)?;
    let imported = loader
        .call((fullname, vm.import_func.clone()), vm)
        .inspect_err(|exc| crate::import::remove_importlib_frames(vm, exc))?;
    if vm.is_none(&imported) {
        return Ok(None);
    }
    if let Some(children) = vm.state.lazy_imports.pending.lock().get_mut(module_name) {
        children.remove(child);
    }
    module.dict().set_item(name, imported.clone(), vm)?;
    Ok(Some(imported))
}

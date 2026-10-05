//! Native PEP 810 deferred bindings. Dictionaries themselves always expose raw values.

use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    builtins::{
        PyBaseException, PyCode, PyDict, PyModule, PySet, PyStr, PyStrRef, PyTuple, PyType,
    },
    class::PyClassImpl,
    common::lock::{OnceCell, PyMutex},
    frame,
    types::{Constructor, GetAttr, Representable},
};
use core::sync::atomic::{AtomicBool, Ordering};
use std::collections::HashMap;

type PendingChildren = HashMap<String, Option<PyRef<PyLazyImport>>>;

pub(crate) struct LazyImportsState {
    pub(crate) modules: PyRef<PySet>,
    pub(crate) default_callback: OnceCell<PyObjectRef>,
    pub(crate) all: AtomicBool,
    filter: PyMutex<Option<PyObjectRef>>,
    // Removed declarations must be dropped after releasing this lock.
    pending: PyMutex<HashMap<String, PendingChildren>>,
}

impl LazyImportsState {
    pub(crate) fn new(ctx: &Context) -> Self {
        Self {
            modules: PySet::default().into_ref(ctx),
            default_callback: OnceCell::new(),
            all: AtomicBool::new(false),
            filter: PyMutex::default(),
            pending: PyMutex::default(),
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

    fn register(
        &self,
        name: &Py<PyStr>,
        source: Option<&Py<PyLazyImport>>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let Some(name) = name.to_str() else {
            return Ok(());
        };
        let mut full = name;
        while let Some((parent, child)) = full.rsplit_once('.') {
            let cached = raw_imported_module(&vm.ctx.new_str(full), vm)?;
            let value = if cached.as_ref().is_some_and(|value| !vm.is_none(value)) {
                None
            } else {
                source.map(ToOwned::to_owned)
            };
            let replaced = {
                let mut pending = self.pending.lock();
                let children = pending.entry(parent.to_owned()).or_default();
                if source.is_some() {
                    children.insert(child.to_owned(), value)
                } else {
                    children.entry(child.to_owned()).or_insert(value);
                    None
                }
            };
            drop(replaced);
            full = parent;
        }
        Ok(())
    }
}

#[pyclass(module = false, name = "lazy_import", traverse)]
#[derive(Debug)]
pub(crate) struct PyLazyImport {
    builtins: PyObjectRef,
    name: PyStrRef,
    // None is an import, a tuple is an import with fromlist, and a string is one member.
    fromlist: Option<PyObjectRef>,
    // Attribute projections retain the placeholder returned by IMPORT_NAME.
    source: Option<PyRef<Self>>,
    declaration: Option<PyRef<PyCode>>,
    #[pytraverse(skip)]
    instruction: u32,
    #[pytraverse(skip)]
    active: AtomicBool,
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
            source: None,
            declaration,
            instruction,
            active: AtomicBool::new(false),
        }
    }

    fn projection(source: &Py<Self>, name: &Py<PyStr>) -> Self {
        let mut projection = Self::new(
            source.builtins.clone(),
            source.name.clone(),
            Some(name.to_owned().into()),
        );
        projection.source = Some(source.to_owned());
        projection
    }

    fn root_and_attrs(&self) -> (&Self, Vec<PyStrRef>) {
        let mut root = self;
        let mut attrs = Vec::new();
        while let Some(source) = &root.source {
            let attr = root
                .fromlist
                .as_ref()
                .unwrap()
                .downcast_ref::<PyStr>()
                .unwrap();
            attrs.push(attr.to_owned());
            root = source;
        }
        attrs.reverse();
        (root, attrs)
    }

    fn has_fromlist(&self) -> bool {
        self.fromlist.as_ref().is_some_and(|from| {
            from.downcast_ref::<PyTuple>()
                .is_none_or(|t| !t.as_slice().is_empty())
        })
    }

    fn path(&self) -> String {
        let (root, attrs) = self.root_and_attrs();
        let mut name = if root.has_fromlist() {
            root.name.to_string()
        } else {
            root.name
                .to_string_lossy()
                .split('.')
                .next()
                .unwrap()
                .to_owned()
        };
        for attr in attrs {
            name.push('.');
            name.push_str(&attr.to_string_lossy());
        }
        name
    }

    fn display_name(&self) -> String {
        if self.source.is_some() {
            self.path()
        } else if self.has_fromlist() {
            format!("{}...", self.name)
        } else {
            self.name.to_string()
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
    callback.call((name.to_owned(), globals, locals, fromlist, level), vm)
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
    let fromlist = match fromlist.filter(|value| !vm.is_none(value)) {
        Some(value) if value.downcast_ref::<PyStr>().is_some() => {
            Some(PyTuple::new_ref(vec![value], &vm.ctx).into())
        }
        value => value,
    };
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
    if let Some(value) = &fromlist {
        let Some(names) = value.downcast_ref::<PyTuple>() else {
            return Err(
                vm.new_type_error("lazy_import: fromlist must be None, a string, or a tuple")
            );
        };
        for name in names.as_slice() {
            if name.downcast_ref::<PyStr>().is_none() {
                return Err(vm.new_type_error(format!(
                    "Item in ``from list'' must be str, not {:.200}",
                    name.class().name()
                )));
            }
        }
    }
    let deferred =
        PyLazyImport::new(builtins, absolute.clone(), fromlist.clone()).into_ref(&vm.ctx);
    let state = &vm.state.lazy_imports;
    let existing = track_module(&absolute, vm)?;
    if let Some(names) = fromlist
        .as_ref()
        .and_then(|value| value.downcast_ref::<PyTuple>())
        && !names.as_slice().is_empty()
    {
        for name in names.as_slice() {
            let name = name.downcast_ref::<PyStr>().unwrap();
            // Cached attributes are concrete or tracked by their own placeholder.
            if let Some(module) = existing.as_ref().and_then(|m| m.downcast_ref::<PyModule>())
                && module.dict().get_item_opt(name, vm)?.is_some()
            {
                continue;
            }
            let fullname = vm.ctx.new_str(format!("{absolute}.{name}"));
            track_module(&fullname, vm)?;
            state.register(&fullname, None, vm)?;
        }
    } else {
        state.register(&absolute, Some(&deferred), vm)?;
    }
    Ok(deferred.into())
}

pub(crate) fn import_from(
    deferred: &Py<PyLazyImport>,
    name: &Py<PyStr>,
    vm: &VirtualMachine,
) -> PyResult {
    if deferred.source.is_none()
        && deferred.has_fromlist()
        && let Some(value) = loaded_attr(&deferred.name, name, vm)?
    {
        return Ok(value);
    }
    Ok(PyLazyImport::projection(deferred, name)
        .into_ref(&vm.ctx)
        .into())
}

struct ResolvingGuard<'a> {
    vm: &'a VirtualMachine,
    identity: usize,
}

impl Drop for ResolvingGuard<'_> {
    fn drop(&mut self) {
        self.vm
            .lazy_imports_resolving
            .borrow_mut()
            .remove(&self.identity);
    }
}

pub(crate) fn is_resolving(deferred: &Py<PyLazyImport>, vm: &VirtualMachine) -> bool {
    vm.lazy_imports_resolving
        .borrow()
        .contains(&deferred.as_object().get_id())
}

fn raw_imported_module(name: &Py<PyStr>, vm: &VirtualMachine) -> PyResult<Option<PyObjectRef>> {
    let modules = vm.sys_module.dict().get_item("modules", vm)?;
    optional_item(&modules, name, vm)
}

// Tracking must not invoke module/spec descriptors or convert a flag to bool.
fn track_module(name: &Py<PyStr>, vm: &VirtualMachine) -> PyResult<Option<PyObjectRef>> {
    let module = raw_imported_module(name, vm)?;
    let loaded = if let Some(module) = module.as_ref().and_then(|m| m.downcast_ref::<PyModule>()) {
        let spec = module
            .dict()
            .get_item_opt(vm.ctx.intern_str("__spec__"), vm)?;
        match spec {
            None => true,
            Some(spec) if vm.is_none(&spec) => true,
            Some(spec) if spec.has_inline_values() => {
                let initializing = match spec.dict() {
                    Some(dict) => dict.get_item_opt(vm.ctx.intern_str("_initializing"), vm)?,
                    None => None,
                };
                initializing.is_none_or(|flag| flag.is(&vm.ctx.false_value))
            }
            Some(_) => false,
        }
    } else {
        module.as_ref().is_some_and(|module| !vm.is_none(module))
    };
    if !loaded {
        vm.state
            .lazy_imports
            .modules
            .add(name.to_owned().into(), vm)?;
    }
    Ok(module)
}

// Attribute caching may inspect __spec__, but never waits for module imports.
fn loaded_attr(
    name: &Py<PyStr>,
    attr: &Py<PyStr>,
    vm: &VirtualMachine,
) -> PyResult<Option<PyObjectRef>> {
    let lookup = || -> PyResult<Option<PyObjectRef>> {
        let modules = vm.sys_module.dict().get_item("modules", vm)?;
        let Some(module) = optional_item(&modules, name, vm)? else {
            return Ok(None);
        };
        let Some(module) = module.downcast_ref::<PyModule>() else {
            return Ok(None);
        };
        if crate::import::is_module_initializing(module.as_object(), vm)? {
            return Ok(None);
        }
        if !vm.sys_module.dict().get_item("modules", vm)?.is(&modules)
            || !optional_item(&modules, name, vm)?.is_some_and(|m| m.is(module))
        {
            return Ok(None);
        }
        Ok(module
            .dict()
            .get_item_opt(attr, vm)?
            .filter(|value| value.downcast_ref_if_exact::<PyLazyImport>(vm).is_none()))
    };
    match lookup() {
        Err(exc) if exc.fast_isinstance(vm.ctx.exceptions.exception_type) => Ok(None),
        result => result,
    }
}

fn add_declaration_context(
    deferred: &PyLazyImport,
    exc: &Py<PyBaseException>,
    vm: &VirtualMachine,
) {
    let Some(code) = &deferred.declaration else {
        return;
    };
    let offset = deferred.instruction.saturating_sub(1) as usize;
    let location = code.locations.get(offset).map(|(location, _)| location);
    if exc.__cause__().is_some() || exc.__context__().is_some() || exc.__suppress_context__() {
        let note = vm.ctx.new_str(format!(
            "lazy import of '{}' declared in {} at {}:{}",
            deferred.display_name(),
            code.obj_name,
            code.source_path(),
            location.map_or(0, |location| location.line.get()),
        ));
        // Adding diagnostic context must not replace the original resolution error.
        let _ = (|| -> PyResult<()> {
            if let Some(notes) =
                vm.get_attribute_opt(exc.as_object(), vm.ctx.intern_str("__notes__"))?
                && notes.sequence_unchecked().contains(note.as_object(), vm)?
            {
                return Ok(());
            }
            exc.add_note(note, vm)
        })();
        return;
    }
    let cause = vm.new_import_error(
        format!(
            "lazy import of '{}' raised an exception during resolution",
            deferred.display_name()
        ),
        deferred.name.clone(),
    );
    if let Some(location) = location {
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
        let tb = crate::builtins::PyTraceback::new(None, frame, (offset * 2) as i32, location.line)
            .into_ref(&vm.ctx);
        cause.set_traceback(Some(tb));
    }
    exc.set_cause(Some(cause));
}

fn resolve_impl(
    deferred: &Py<PyLazyImport>,
    vm: &VirtualMachine,
) -> PyResult<(PyObjectRef, Option<PyObjectRef>)> {
    let (root, attrs) = deferred.root_and_attrs();
    if is_resolving(deferred, vm) {
        let exc = vm.new_exception_msg(
            vm.ctx.exceptions.import_cycle_error.to_owned(),
            format!(
                "cannot import name '{}' (most likely due to a circular import)",
                deferred.display_name()
            )
            .into(),
        );
        exc.as_object().set_attr("name", root.name.clone(), vm)?;
        return Err(exc);
    }
    vm.with_recursion("while resolving a lazy import", || {
        let identity = deferred.as_object().get_id();
        vm.lazy_imports_resolving.borrow_mut().insert(identity);
        let _resolving = ResolvingGuard { vm, identity };
        let result = (|| -> PyResult<(PyObjectRef, Option<PyObjectRef>)> {
            let callback = optional_item(&root.builtins, vm.ctx.intern_str("__import__"), vm)?
                .ok_or_else(|| vm.new_import_error("__import__ not found", root.name.clone()))?;
            let default_import = callback.is(&vm.import_func);
            let fromlist = match attrs.first().filter(|_| root.has_fromlist()) {
                Some(first) => PyTuple::new_ref(vec![first.clone().into()], &vm.ctx).into(),
                None => root.fromlist.clone().unwrap_or_else(|| vm.ctx.none()),
            };
            let mut name = root.name.clone();
            if attrs.is_empty()
                && !root.has_fromlist()
                && default_import
                && root.builtins.is(vm.builtins.dict().as_object())
                && let Some(fullname) = root.name.to_str()
                && let Some(dot) = fullname.find('.')
            {
                let mut regular = true;
                let mut complete = true;
                for (dot, _) in fullname.match_indices('.') {
                    let prefix = vm.ctx.new_str(&fullname[..dot]);
                    let Some(cached) = loaded_attr(&prefix, identifier!(vm, __name__), vm)? else {
                        complete = false;
                        break;
                    };
                    regular = loaded_attr(&prefix, vm.ctx.intern_str("__path__"), vm)?.is_some()
                        && cached
                            .downcast_ref::<PyStr>()
                            .is_some_and(|s| s.as_wtf8() == prefix.as_wtf8());
                    if !regular {
                        break;
                    }
                }
                let loaded = loaded_attr(&name, identifier!(vm, __name__), vm)?;
                if regular && (!complete || loaded.is_none()) {
                    name = vm.ctx.new_str(&fullname[..dot]);
                }
            }
            let globals = frame::current_globals()
                .map_or_else(|| vm.ctx.none(), |g| g.as_object().to_owned());
            let import = |name: &PyStrRef| {
                callback
                    .call(
                        (
                            name.clone(),
                            globals.clone(),
                            globals.clone(),
                            fromlist.clone(),
                            0i32,
                        ),
                        vm,
                    )
                    .inspect_err(|exc| crate::import::remove_importlib_frames(vm, exc))
            };
            let mut module = import(&name)?;
            if !name.is(&root.name) {
                root.active.store(true, Ordering::Relaxed);
                let package = match module.downcast_ref_if_exact::<PyModule>(vm) {
                    Some(module) => module
                        .dict()
                        .get_item_opt(vm.ctx.intern_str("__path__"), vm)?
                        .is_some(),
                    None => false,
                };
                if !package {
                    module = import(&root.name)?;
                }
            } else if default_import {
                clear_submodule(&name, false, vm)?;
            }
            let mut value = resolve_value(module, vm)?;
            let imported_module = if default_import && value.downcast_ref::<PyModule>().is_some() {
                Some(value.clone())
            } else {
                None
            };
            for attr in attrs {
                value =
                    resolve_value(crate::import::import_from_attribute(&value, &attr, vm)?, vm)?;
            }
            vm.state
                .lazy_imports
                .modules
                .discard(vm.ctx.new_str(deferred.path()).into(), vm)?;
            Ok((value, imported_module))
        })();
        result.inspect_err(|exc| add_declaration_context(deferred, exc, vm))
    })
}

pub(crate) fn resolve(deferred: &Py<PyLazyImport>, vm: &VirtualMachine) -> PyResult {
    resolve_impl(deferred, vm).map(|(value, _)| value)
}

pub(crate) fn resolve_value(value: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    match value.downcast_ref_if_exact::<PyLazyImport>(vm) {
        Some(deferred) => resolve(deferred, vm),
        None => Ok(value),
    }
}

pub(crate) fn reify(
    deferred: &Py<PyLazyImport>,
    name: &Py<PyStr>,
    namespace: &PyObject,
    vm: &VirtualMachine,
) -> PyResult {
    let (value, imported_module) = resolve_impl(deferred, vm)?;
    if let Some(dict) = namespace.downcast_ref_if_exact::<PyDict>(vm) {
        let replaced = dict.replace_if_identity(name, deferred.as_object(), value.clone(), vm)?;
        if !replaced
            && deferred.source.is_some()
            && let Some(child) = imported_module
        {
            let (root, _) = deferred.root_and_attrs();
            if let Some((parent_name, child_name)) =
                root.name.to_str().and_then(|s| s.rsplit_once('.'))
                && name.to_str() == Some(child_name)
                && let Some(parent) = raw_imported_module(&vm.ctx.new_str(parent_name), vm)?
                && let Some(parent) = parent.downcast_ref::<PyModule>()
                && parent.dict().as_object().is(namespace)
            {
                dict.replace_if_identity(name, &child, value.clone(), vm)?;
            }
        }
    } else if namespace
        .mapping_unchecked()
        .slots()
        .ass_subscript
        .load()
        .is_some()
        && optional_item(namespace, name, vm)?.is_some_and(|current| current.is(deferred))
    {
        namespace.set_item(name, value.clone(), vm)?;
    }
    Ok(value)
}

pub(crate) fn reify_value(
    value: PyObjectRef,
    name: &Py<PyStr>,
    namespace: &PyObject,
    vm: &VirtualMachine,
) -> PyResult {
    match value.downcast_ref_if_exact::<PyLazyImport>(vm) {
        Some(deferred) => reify(deferred, name, namespace, vm),
        None => Ok(value),
    }
}

pub(crate) fn resolved_dict_item(
    dict: &Py<PyDict>,
    name: &Py<PyStr>,
    vm: &VirtualMachine,
) -> PyResult<Option<PyObjectRef>> {
    let Some(value) = dict.get_item_opt(name, vm)? else {
        return Ok(None);
    };
    if let Some(deferred) = value.downcast_ref_if_exact::<PyLazyImport>(vm) {
        if is_resolving(deferred, vm) {
            return Ok(None);
        }
        return reify(deferred, name, dict.as_object(), vm).map(Some);
    }
    Ok(Some(value))
}

fn load_child(source: &Py<PyLazyImport>, name: &Py<PyStr>, vm: &VirtualMachine) -> PyResult {
    vm.with_recursion("while resolving a lazy import", || {
        let mut child = PyLazyImport::new(source.builtins.clone(), name.to_owned(), None);
        child.declaration = None;
        if let Some(name) = name.to_str() {
            for attr in name.split('.').skip(1) {
                child = PyLazyImport::projection(&child.into_ref(&vm.ctx), &vm.ctx.new_str(attr));
                child.declaration = None;
            }
        }
        child.declaration.clone_from(&source.declaration);
        child.instruction = source.instruction;
        let result = resolve(&child.into_ref(&vm.ctx), vm)?;
        if let Some(module) = result.downcast_ref::<PyModule>()
            && source.name.as_wtf8() != name.as_wtf8()
            && module
                .dict()
                .get_item_opt(vm.ctx.intern_str("__path__"), vm)?
                .is_none()
        {
            return load_child(source, &source.name, vm);
        }
        Ok(result)
    })
}

// Keep pending names while their parent is initializing, without retaining
// completed declarations and their namespace mappings indefinitely.
pub(crate) fn clear_submodule(name: &Py<PyStr>, bind: bool, vm: &VirtualMachine) -> PyResult<()> {
    let Some((parent_name, child)) = name.to_str().and_then(|name| name.rsplit_once('.')) else {
        return Ok(());
    };
    let state = &vm.state.lazy_imports;
    if !state
        .pending
        .lock()
        .get(parent_name)
        .is_some_and(|children| children.contains_key(child))
    {
        return Ok(());
    }
    if let Some(loaded) = raw_imported_module(name, vm)? {
        match crate::import::is_module_initializing(&loaded, vm) {
            Ok(true) => return Ok(()),
            Err(exc) if exc.fast_isinstance(vm.ctx.exceptions.exception_type) => return Ok(()),
            Err(exc) => return Err(exc),
            Ok(false) => {}
        }
    }
    let parent = raw_imported_module(&vm.ctx.new_str(parent_name), vm)?;
    let initializing = match &parent {
        Some(parent) => crate::import::is_module_initializing(parent, vm)?,
        None => false,
    };
    if bind
        && !initializing
        && let Some(parent) = parent
            .as_ref()
            .and_then(|p| p.downcast_ref_if_exact::<PyModule>(vm))
        && let Some(value) = raw_imported_module(name, vm)?
        && !vm.is_none(&value)
    {
        parent
            .dict()
            .setdefault(vm.ctx.new_str(child).into(), value, vm)?;
    }
    let removed = {
        let mut pending = state.pending.lock();
        if let Some(children) = pending.get_mut(parent_name) {
            if initializing {
                children.get_mut(child).and_then(Option::take)
            } else {
                children.remove(child).flatten()
            }
        } else {
            None
        }
    };
    drop(removed);
    Ok(())
}

pub(crate) fn try_load_submodule(
    module: &Py<PyModule>,
    name: &Py<PyStr>,
    suppress: bool,
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
    let source = vm
        .state
        .lazy_imports
        .pending
        .lock()
        .get(module_name)
        .and_then(|children| children.get(child).cloned());
    let Some(source) = source else {
        return Ok(None);
    };
    let fullname = vm.ctx.new_str(format!("{module_name}.{child}"));
    let imported = (|| {
        if let Some(source) = &source
            && !suppress
            && (source.active.load(Ordering::Relaxed)
                || module
                    .dict()
                    .get_item_opt(identifier!(vm, __getattr__), vm)?
                    .is_none())
        {
            load_child(source, &fullname, vm)
        } else {
            if let Some(loaded) = raw_imported_module(&fullname, vm)?
                && !vm.is_none(&loaded)
            {
                return Ok(loaded);
            }
            crate::import::find_and_load(&fullname, "_find_and_load_lazy_submodule", vm)
        }
    })();
    let imported = match imported {
        Ok(imported) => imported,
        Err(exc) => {
            if let Ok(Some(loaded)) = raw_imported_module(&fullname, vm)
                && !vm.is_none(&loaded)
            {
                let _ = PyDict::hash_or_unhashable(name, vm).and_then(|hash| {
                    module
                        .dict()
                        .entries
                        .delete_if(vm, name, hash, |value| Ok(value.is(&loaded)))
                });
                let mut pending = vm.state.lazy_imports.pending.lock();
                pending
                    .entry(module_name.to_owned())
                    .or_default()
                    .entry(child.to_owned())
                    .or_insert_with(|| source.clone());
            }
            crate::import::remove_importlib_frames(vm, &exc);
            return Err(exc);
        }
    };
    if vm.is_none(&imported) {
        return Ok(None);
    }
    let initializing = match crate::import::is_module_initializing(&imported, vm) {
        Ok(initializing) => initializing,
        Err(exc) if exc.fast_isinstance(vm.ctx.exceptions.exception_type) => false,
        Err(exc) => return Err(exc),
    };
    if !initializing {
        module.dict().set_item(name, imported.clone(), vm)?;
        let removed = vm
            .state
            .lazy_imports
            .pending
            .lock()
            .get_mut(module_name)
            .and_then(|children| children.remove(child));
        drop(removed);
    }
    Ok(Some(imported))
}

use super::{PyDict, PyDictRef, PyStr, PyStrRef, PyType, PyTypeRef, PyUtf8Str};
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    builtins::{PyStrInterned, pystr::AsPyStr},
    class::PyClassImpl,
    convert::ToPyObject,
    function::{FuncArgs, PyMethodDef, PySetterValue},
    import::{
        get_spec_file_origin, is_possibly_shadowing_path, is_spec_initializing,
        is_stdlib_module_name,
    },
    types::{GetAttr, Initializer, Representable},
};
use rustpython_common::wtf8::wtf8_concat;

#[pyclass(module = false, name = "module")]
#[derive(Debug)]
pub struct PyModuleDef {
    // pub index: usize,
    pub name: &'static PyStrInterned,
    pub doc: Option<&'static PyStrInterned>,
    // pub size: isize,
    pub methods: &'static [PyMethodDef],
    pub slots: PyModuleSlots,
    // traverse: traverse_proc
    // clear: inquiry
    // free: free_func
}

pub(crate) type ModuleCreate =
    fn(&VirtualMachine, &PyObject, &'static PyModuleDef) -> PyResult<PyRef<PyModule>>;
pub(crate) type ModuleExec = fn(&VirtualMachine, &Py<PyModule>) -> PyResult<()>;

#[derive(Default)]
pub struct PyModuleSlots {
    pub create: Option<ModuleCreate>,
    pub exec: Option<ModuleExec>,
}

impl core::fmt::Debug for PyModuleSlots {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PyModuleSlots")
            .field("create", &self.create.is_some())
            .field("exec", &self.exec.is_some())
            .finish()
    }
}

impl PyModuleDef {
    /// Create a module from this definition (Phase 1 of multi-phase init).
    ///
    /// This performs:
    /// 1. Create module object (using create slot if provided)
    /// 2. Initialize module dict from def
    /// 3. Add methods to module
    ///
    /// Does NOT add to sys.modules or call exec slot.
    pub fn create_module(&'static self, vm: &VirtualMachine) -> PyResult<PyRef<PyModule>> {
        use crate::PyPayload;

        // Create module (use create slot if provided, else default creation)
        let module = if let Some(create) = self.slots.create {
            // Custom module creation
            let spec = vm.ctx.new_str(self.name.as_str());
            create(vm, spec.as_object(), self)?
        } else {
            // Default module creation
            PyModule::from_def(self).into_ref(&vm.ctx)
        };

        // Initialize module dict and methods
        PyModule::__init_dict_from_def(vm, &module)?;
        module.__init_methods(vm)?;

        Ok(module)
    }

    /// Execute the module's exec slot (Phase 2 of multi-phase init).
    ///
    /// Calls the exec slot if present. Returns Ok(()) if no exec slot.
    pub fn exec_module(&'static self, vm: &VirtualMachine, module: &Py<PyModule>) -> PyResult<()> {
        if let Some(exec) = self.slots.exec {
            exec(vm, module)?;
        }
        Ok(())
    }
}

#[pyclass(module = false, name = "module", traverse = "manual")]
// The dict lives in the object extension, not the payload. The offset is
// the dict cell at the front of that extension.
#[pymember(name = "__dict__", offset = ::rustpython_vm::object::dict_member_offset())]
#[derive(Debug)]
pub struct PyModule {
    // PyObject *md_dict;
    pub def: Option<&'static PyModuleDef>,
    native_state: crate::object::PyAtomicRef<Option<PyObject>>,
    // weaklist
    // for logging purposes after md_dict is cleared
    pub name: Option<&'static PyStrInterned>,
}

// SAFETY: Native state is the only owned Python reference in the payload.
// The module dictionary is visited separately by the object extension.
unsafe impl crate::object::Traverse for PyModule {
    fn traverse(&self, tracer: &mut crate::object::TraverseFn<'_>) {
        self.native_state.traverse(tracer);
    }

    fn clear(&mut self, out: &mut Vec<PyObjectRef>) {
        if let Some(state) = self.native_state.store(None) {
            out.push(state);
        }
    }
}

impl PyPayload for PyModule {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.module_type
    }
}

#[derive(FromArgs)]
pub struct ModuleInitArgs {
    name: PyStrRef,
    #[pyarg(any, default)]
    doc: Option<PyObjectRef>,
}

impl PyModule {
    #[expect(
        clippy::new_without_default,
        reason = "avoid a misleading Default implementation"
    )]
    #[must_use]
    pub const fn new() -> Self {
        Self {
            def: None,
            native_state: crate::object::PyAtomicRef::new_empty(),
            name: None,
        }
    }

    #[must_use]
    pub const fn from_def(def: &'static PyModuleDef) -> Self {
        Self {
            def: Some(def),
            native_state: crate::object::PyAtomicRef::new_empty(),
            name: Some(def.name),
        }
    }

    /// Interpreter-local native state, retained and traversed with the module.
    #[must_use]
    pub fn native_state(&self) -> Option<PyObjectRef> {
        self.native_state.load_owned()
    }

    /// Replace native state without exposing it in the Python module dictionary.
    pub fn set_native_state(&self, state: PyObjectRef) {
        drop(self.native_state.store(Some(state)));
    }

    pub fn __init_dict_from_def(vm: &VirtualMachine, module: &Py<Self>) -> PyResult<()> {
        let doc = module.def.unwrap().doc.map(|doc| doc.as_object());
        module.init_dict(module.name.unwrap(), doc, vm)
    }
}

impl Py<PyModule> {
    pub fn __init_methods(&self, vm: &VirtualMachine) -> PyResult<()> {
        debug_assert!(self.def.is_some());
        for method in self.def.unwrap().methods {
            let func = method
                .to_function()
                .with_module(self.name.unwrap())
                .with_module_object(self.to_owned().into())
                .into_ref(&vm.ctx);
            vm.__module_set_attr(self, vm.ctx.intern_str(method.name), func)?;
        }
        Ok(())
    }

    pub(crate) fn getattr_inner(
        &self,
        name: &Py<PyStr>,
        suppress_cycle: bool,
        vm: &VirtualMachine,
    ) -> PyResult {
        let mut attr = match self.as_object().generic_getattr_opt(name, None, vm) {
            Err(exc) if exc.fast_isinstance(vm.ctx.exceptions.attribute_error) => None,
            result => result?,
        };
        if attr.is_none() {
            if let Some(value) =
                crate::lazy_import::try_load_submodule(self, name, suppress_cycle, vm)?
            {
                return Ok(value);
            }
            // A concurrent import may have bound a child and removed its pending entry.
            if self.class().is(vm.ctx.types.module_type)
                || self
                    .class()
                    .get_attr(vm.ctx.intern_str(name.as_wtf8()))
                    .is_none_or(|descriptor| descriptor.class().slots().descr_get.load().is_none())
            {
                attr = self.dict().get_item_opt(name, vm)?;
            }
        }
        if let Some(attr) = attr {
            if let Some(deferred) =
                attr.downcast_ref_if_exact::<crate::lazy_import::PyLazyImport>(vm)
            {
                if name.to_str() != Some("__getattr__")
                    && let Some(getattr) = crate::lazy_import::resolved_dict_item(
                        &self.dict(),
                        identifier!(vm, __getattr__),
                        vm,
                    )?
                {
                    match getattr.call((name.to_owned(),), vm) {
                        Ok(value) => return Ok(value),
                        Err(exc) if exc.fast_isinstance(vm.ctx.exceptions.attribute_error) => {}
                        Err(exc) => return Err(exc),
                    }
                }
                if suppress_cycle && crate::lazy_import::is_resolving(deferred, vm) {
                    return Err(vm.new_attribute_error(format!("module has no attribute '{name}'")));
                }
                return crate::lazy_import::reify(deferred, name, self.dict().as_object(), vm);
            }
            return Ok(attr);
        }
        if let Some(getattr) =
            crate::lazy_import::resolved_dict_item(&self.dict(), identifier!(vm, __getattr__), vm)?
        {
            return getattr.call((name.to_owned(),), vm);
        }
        if suppress_cycle {
            return Err(vm.new_attribute_error(format!("module has no attribute '{name}'")));
        }
        let dict = self.dict();

        // Get the raw __name__ object (may be a str subclass)
        let mod_name_obj = dict.inner_getitem_opt(identifier!(vm, __name__), vm)?;
        let mod_name = mod_name_obj
            .as_ref()
            .and_then(|n| n.downcast_ref::<PyUtf8Str>());

        // If __name__ is not set or not a string, use a simpler error message
        let Some(mod_display) = mod_name.map(|s| s.as_str()) else {
            return Err(vm.new_attribute_error(format!("module has no attribute '{name}'")));
        };

        let spec = dict.inner_getitem_opt(vm.ctx.intern_str("__spec__"), vm)?;

        let origin = get_spec_file_origin(spec.as_deref(), vm)?;

        let is_possibly_shadowing = origin
            .as_ref()
            .is_some_and(|o| is_possibly_shadowing_path(o, vm));
        // Use the ORIGINAL __name__ object for stdlib check (may raise TypeError
        // if __name__ is an unhashable str subclass)
        let is_possibly_shadowing_stdlib = if is_possibly_shadowing {
            if let Some(ref mod_name) = mod_name_obj {
                is_stdlib_module_name(mod_name, vm)?
            } else {
                false
            }
        } else {
            false
        };

        if is_possibly_shadowing_stdlib {
            let origin = origin.as_ref().unwrap();
            Err(vm.new_attribute_error(format!(
                "module '{mod_display}' has no attribute '{name}' \
                 (consider renaming '{origin}' since it has the same \
                 name as the standard library module named '{mod_display}' \
                 and prevents importing that standard library module)"
            )))
        } else {
            let is_initializing = is_spec_initializing(spec.as_deref(), vm)?;
            if is_initializing {
                if is_possibly_shadowing {
                    let origin = origin.as_ref().unwrap();
                    Err(vm.new_attribute_error(format!(
                        "module '{mod_display}' has no attribute '{name}' \
                         (consider renaming '{origin}' if it has the same name \
                         as a library you intended to import)"
                    )))
                } else if let Some(ref origin) = origin {
                    Err(vm.new_attribute_error(format!(
                        "partially initialized module '{mod_display}' from '{origin}' \
                         has no attribute '{name}' \
                         (most likely due to a circular import)"
                    )))
                } else {
                    Err(vm.new_attribute_error(format!(
                        "partially initialized module '{mod_display}' \
                         has no attribute '{name}' \
                         (most likely due to a circular import)"
                    )))
                }
            } else {
                // Check for uninitialized submodule
                let submodule_initializing = is_uninitialized_submodule(spec.as_deref(), name, vm)?;
                if submodule_initializing {
                    Err(vm.new_attribute_error(format!(
                        "cannot access submodule '{name}' of module '{mod_display}' \
                         (most likely due to a circular import)"
                    )))
                } else {
                    Err(vm.new_attribute_error(format!(
                        "module '{mod_display}' has no attribute '{name}'"
                    )))
                }
            }
        }
    }

    // TODO: to be replaced by the commented-out dict method above once dictoffset land
    pub fn dict(&self) -> PyDictRef {
        self.as_object().dict().unwrap()
    }

    // TODO: should be on PyModule, not Py<PyModule>
    pub(crate) fn init_dict(
        &self,
        name: &Py<PyStr>,
        doc: Option<&PyObject>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let dict = self.dict();
        dict.inner_setitem(identifier!(vm, __name__), name.to_owned().into(), vm)?;
        dict.inner_setitem(identifier!(vm, __doc__), doc.to_pyobject(vm), vm)?;
        dict.inner_setitem(vm.ctx.intern_str("__package__"), vm.ctx.none(), vm)?;
        dict.inner_setitem(vm.ctx.intern_str("__loader__"), vm.ctx.none(), vm)?;
        dict.inner_setitem(vm.ctx.intern_str("__spec__"), vm.ctx.none(), vm)
    }

    pub fn get_attr<'a>(&self, attr_name: impl AsPyStr<'a>, vm: &VirtualMachine) -> PyResult {
        let attr_name = attr_name.as_pystr(&vm.ctx);
        self.getattr_inner(attr_name, false, vm)
    }

    pub fn set_attr<'a>(
        &self,
        attr_name: impl AsPyStr<'a>,
        attr_value: impl Into<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        self.as_object().set_attr(attr_name, attr_value, vm)
    }
}

#[pyclass(
    with(GetAttr, Initializer, Representable),
    flags(BASETYPE, HAS_DICT, HAS_WEAKREF)
)]
impl PyModule {
    fn load_dict(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyDictRef> {
        zelf.as_object()
            .get_attr(identifier!(vm, __dict__), vm)?
            .downcast::<PyDict>()
            .map_err(|_| vm.new_type_error("<module>.__dict__ is not a dictionary"))
    }

    #[pyslot]
    fn slot_new(cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        Self::new().into_ref_with_type(vm, cls).map(Into::into)
    }

    #[pymethod]
    fn __dir__(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult {
        // First check if __dict__ attribute exists and is actually a dictionary
        let dict = Self::load_dict(zelf, vm)?;
        // PEP 562: honor a module-level __dir__ if one is defined
        if let Some(dir_func) =
            crate::lazy_import::resolved_dict_item(&dict, identifier!(vm, __dir__), vm)?
        {
            return dir_func.call((), vm);
        }
        let attrs = dict.into_iter().map(|(k, _v)| k).collect();
        Ok(vm.ctx.new_list(attrs).into())
    }

    #[pygetset]
    fn __annotate__(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let dict = Self::load_dict(zelf, vm)?;
        // Get __annotate__ from dict; if not present, insert None and return it
        // See: module_get_annotate()
        if let Some(annotate) = dict.inner_getitem_opt(identifier!(vm, __annotate__), vm)? {
            Ok(annotate)
        } else {
            let none = vm.ctx.none();
            dict.inner_setitem(identifier!(vm, __annotate__), none.clone(), vm)?;
            Ok(none)
        }
    }

    #[pygetset(setter)]
    fn set___annotate__(
        zelf: &Py<Self>,
        value: PySetterValue,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        match value {
            PySetterValue::Assign(value) => {
                let dict = Self::load_dict(zelf, vm)?;
                if !vm.is_none(&value) && !value.is_callable() {
                    return Err(vm.new_type_error("__annotate__ must be callable or None"));
                }
                dict.inner_setitem(identifier!(vm, __annotate__), value.clone(), vm)?;
                // Clear __annotations__ if value is not None
                if !vm.is_none(&value) {
                    dict.inner_pop_item(identifier!(vm, __annotations__), vm)?;
                }
                Ok(())
            }
            PySetterValue::Delete => Err(vm.new_type_error("cannot delete __annotate__ attribute")),
        }
    }

    #[pygetset]
    fn __annotations__(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let dict = Self::load_dict(zelf, vm)?;

        // Check if __annotations__ is already in dict (explicitly set)
        if let Some(annotations) = dict.inner_getitem_opt(identifier!(vm, __annotations__), vm)? {
            return Ok(annotations);
        }

        // Check if module is initializing
        let spec = zelf
            .dict()
            .inner_getitem_opt(vm.ctx.intern_str("__spec__"), vm)?;
        let is_initializing = is_spec_initializing(spec.as_deref(), vm)?;
        drop(spec);

        // PEP 649: Get __annotate__ and call it if callable
        let annotations = if let Some(annotate) =
            dict.inner_getitem_opt(identifier!(vm, __annotate__), vm)?
            && annotate.is_callable()
        {
            // Call __annotate__(1) where 1 is FORMAT_VALUE
            let result = annotate.call((1i32,), vm)?;
            if !result.downcastable::<PyDict>() {
                let error = result.class().fully_qualified_name(vm).map(|name| {
                    vm.new_type_error(wtf8_concat!(
                        "__annotate__() must return a dict, not ",
                        name
                    ))
                });
                drop(annotate);
                drop(result);
                return Err(error?);
            }
            result
        } else {
            vm.ctx.new_dict().into()
        };

        // Cache result unless module is initializing
        if !is_initializing {
            dict.inner_setitem(identifier!(vm, __annotations__), annotations.clone(), vm)?;
        }

        Ok(annotations)
    }

    #[pygetset(setter)]
    fn set___annotations__(
        zelf: &Py<Self>,
        value: PySetterValue,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let dict = Self::load_dict(zelf, vm)?;
        match &value {
            PySetterValue::Assign(value) => {
                dict.inner_setitem(identifier!(vm, __annotations__), value.clone(), vm)?;
            }
            PySetterValue::Delete => {
                if dict
                    .inner_pop_item(identifier!(vm, __annotations__), vm)?
                    .is_none()
                {
                    return Err(vm.new_attribute_error("__annotations__"));
                }
            }
        }
        dict.inner_pop_item(identifier!(vm, __annotate__), vm)?;
        Ok(())
    }
}

impl Initializer for PyModule {
    type Args = ModuleInitArgs;

    fn init(zelf: &Py<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
        debug_assert!(
            zelf.class()
                .slots
                .flags
                .has_feature(crate::types::PyTypeFlags::HAS_DICT)
        );
        let ModuleInitArgs { name, doc } = args;
        zelf.init_dict(&name, doc.as_deref(), vm)
    }
}

impl GetAttr for PyModule {
    fn getattro(zelf: &Py<Self>, name: &Py<PyStr>, vm: &VirtualMachine) -> PyResult {
        zelf.getattr_inner(name, false, vm)
    }
}

impl Representable for PyModule {
    #[inline]
    fn repr(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        // Use cached importlib reference (like interp->importlib)
        let module_repr = vm.importlib.get_attr("_module_repr", vm)?;
        let repr = module_repr.call((zelf.to_owned(),), vm)?;
        repr.downcast()
            .map_err(|_| vm.new_type_error("_module_repr did not return a string"))
    }

    #[cold]
    fn repr_str(_zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
        unreachable!("use repr instead")
    }
}

pub(crate) fn init(context: &'static Context) {
    PyModule::extend_class(context, context.types.module_type);
}

/// Check the parent module's spec for an uninitialized submodule.
fn is_uninitialized_submodule(
    spec: Option<&PyObject>,
    name: &Py<PyStr>,
    vm: &VirtualMachine,
) -> PyResult<bool> {
    let Some(spec) = spec else {
        return Ok(false);
    };
    match vm.get_attribute_opt(spec, "_uninitialized_submodules")? {
        Some(submodules) => submodules
            .sequence_unchecked()
            .contains(name.as_object(), vm),
        None => Ok(false),
    }
}

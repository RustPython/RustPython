use super::{PyDict, PyDictRef, PyStr, PyStrRef, PyType, PyTypeRef};
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    builtins::{PyStrInterned, pystr::AsPyStr},
    class::PyClassImpl,
    convert::ToPyObject,
    function::{FuncArgs, PyMethodDef, PySetterValue},
    import::{get_spec_file_origin, is_possibly_shadowing_path, is_stdlib_module_name},
    types::{GetAttr, Initializer, Representable},
};

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

pub type ModuleCreate =
    fn(&VirtualMachine, &PyObject, &'static PyModuleDef) -> PyResult<PyRef<PyModule>>;
pub type ModuleExec = fn(&VirtualMachine, &Py<PyModule>) -> PyResult<()>;

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
        PyModule::__init_dict_from_def(vm, &module);
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

#[allow(
    clippy::new_without_default,
    reason = "avoid a misleading Default implementation"
)]
#[pyclass(module = false, name = "module")]
#[derive(Debug)]
pub struct PyModule {
    // PyObject *md_dict;
    pub def: Option<&'static PyModuleDef>,
    // state: Any
    // weaklist
    // for logging purposes after md_dict is cleared
    pub name: Option<&'static PyStrInterned>,
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
    doc: Option<PyStrRef>,
}

impl PyModule {
    #[allow(clippy::new_without_default)]
    pub const fn new() -> Self {
        Self {
            def: None,
            name: None,
        }
    }

    pub const fn from_def(def: &'static PyModuleDef) -> Self {
        Self {
            def: Some(def),
            name: Some(def.name),
        }
    }

    pub fn __init_dict_from_def(vm: &VirtualMachine, module: &Py<Self>) {
        let doc = module.def.unwrap().doc.map(|doc| doc.to_owned());
        module.init_dict(module.name.unwrap(), doc, vm);
    }
}

impl Py<PyModule> {
    pub fn __init_methods(&self, vm: &VirtualMachine) -> PyResult<()> {
        debug_assert!(self.def.is_some());
        for method in self.def.unwrap().methods {
            let func = method
                .to_function()
                .with_module(self.name.unwrap())
                .into_ref(&vm.ctx);
            vm.__module_set_attr(self, vm.ctx.intern_str(method.name), func)?;
        }
        Ok(())
    }

    fn getattr_inner(&self, name: &Py<PyStr>, vm: &VirtualMachine) -> PyResult {
        if let Some(attr) = self.as_object().generic_getattr_opt(name, None, vm)? {
            return Ok(attr);
        }
        if let Ok(getattr) = self.dict().get_item(identifier!(vm, __getattr__), vm) {
            return getattr.call((name.to_owned(),), vm);
        }
        let dict = self.dict();

        // Get the raw __name__ object (may be a str subclass)
        let mod_name_obj = dict
            .get_item_opt(identifier!(vm, __name__), vm)
            .ok()
            .flatten();
        let mod_name_str = mod_name_obj
            .as_ref()
            .and_then(|n| n.downcast_ref::<PyStr>().map(|s| s.as_str().to_owned()));

        // If __name__ is not set or not a string, use a simpler error message
        let mod_display = match mod_name_str.as_deref() {
            Some(s) => s,
            None => {
                return Err(vm.new_attribute_error(format!("module has no attribute '{name}'")));
            }
        };

        let spec = dict
            .get_item_opt(vm.ctx.intern_str("__spec__"), vm)
            .ok()
            .flatten()
            .filter(|s| !vm.is_none(s));

        let origin = get_spec_file_origin(&spec, vm);

        let is_possibly_shadowing = origin
            .as_ref()
            .map(|o| is_possibly_shadowing_path(o, vm))
            .unwrap_or(false);
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
            let is_initializing = PyModule::is_initializing(&dict, vm);
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
                let submodule_initializing =
                    is_uninitialized_submodule(mod_name_str.as_ref(), name, vm);
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
        name: &'static PyStrInterned,
        doc: Option<PyStrRef>,
        vm: &VirtualMachine,
    ) {
        let dict = self.dict();
        dict.set_item(identifier!(vm, __name__), name.to_object(), vm)
            .expect("Failed to set __name__ on module");
        dict.set_item(identifier!(vm, __doc__), doc.to_pyobject(vm), vm)
            .expect("Failed to set __doc__ on module");
        dict.set_item("__package__", vm.ctx.none(), vm)
            .expect("Failed to set __package__ on module");
        dict.set_item("__loader__", vm.ctx.none(), vm)
            .expect("Failed to set __loader__ on module");
        dict.set_item("__spec__", vm.ctx.none(), vm)
            .expect("Failed to set __spec__ on module");
    }

    pub fn get_attr<'a>(&self, attr_name: impl AsPyStr<'a>, vm: &VirtualMachine) -> PyResult {
        let attr_name = attr_name.as_pystr(&vm.ctx);
        self.getattr_inner(attr_name, vm)
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

#[pyclass(with(GetAttr, Initializer, Representable), flags(BASETYPE, HAS_DICT))]
impl PyModule {
    #[pyslot]
    fn slot_new(cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        Self::new().into_ref_with_type(vm, cls).map(Into::into)
    }

    #[pymethod]
    fn __dir__(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
        // First check if __dict__ attribute exists and is actually a dictionary
        let dict_attr = zelf.as_object().get_attr(identifier!(vm, __dict__), vm)?;
        let dict = dict_attr
            .downcast::<PyDict>()
            .map_err(|_| vm.new_type_error("<module>.__dict__ is not a dictionary"))?;
        let attrs = dict.into_iter().map(|(k, _v)| k).collect();
        Ok(attrs)
    }

    #[pygetset]
    fn __annotate__(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let dict = zelf.dict();
        // Get __annotate__ from dict; if not present, insert None and return it
        // See: module_get_annotate()
        if let Some(annotate) = dict.get_item_opt(identifier!(vm, __annotate__), vm)? {
            Ok(annotate)
        } else {
            let none = vm.ctx.none();
            dict.set_item(identifier!(vm, __annotate__), none.clone(), vm)?;
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
                if !vm.is_none(&value) && !value.is_callable() {
                    return Err(vm.new_type_error("__annotate__ must be callable or None"));
                }
                let dict = zelf.dict();
                dict.set_item(identifier!(vm, __annotate__), value.clone(), vm)?;
                // Clear __annotations__ if value is not None
                if !vm.is_none(&value) {
                    dict.del_item(identifier!(vm, __annotations__), vm).ok();
                }
                Ok(())
            }
            PySetterValue::Delete => Err(vm.new_type_error("cannot delete __annotate__ attribute")),
        }
    }

    #[pygetset]
    fn __annotations__(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let dict = zelf.dict();

        // Check if __annotations__ is already in dict (explicitly set)
        if let Some(annotations) = dict.get_item_opt(identifier!(vm, __annotations__), vm)? {
            return Ok(annotations);
        }

        // Check if module is initializing
        let is_initializing = Self::is_initializing(&dict, vm);

        // PEP 649: Get __annotate__ and call it if callable
        let annotations = if let Some(annotate) =
            dict.get_item_opt(identifier!(vm, __annotate__), vm)?
            && annotate.is_callable()
        {
            // Call __annotate__(1) where 1 is FORMAT_VALUE
            let result = annotate.call((1i32,), vm)?;
            if !result.class().is(vm.ctx.types.dict_type) {
                return Err(vm.new_type_error(format!(
                    "__annotate__ returned non-dict of type '{}'",
                    result.class().name()
                )));
            }
            result
        } else {
            vm.ctx.new_dict().into()
        };

        // Cache result unless module is initializing
        if !is_initializing {
            dict.set_item(identifier!(vm, __annotations__), annotations.clone(), vm)?;
        }

        Ok(annotations)
    }

    /// Check if module is initializing via __spec__._initializing
    fn is_initializing(dict: &PyDictRef, vm: &VirtualMachine) -> bool {
        if let Ok(Some(spec)) = dict.get_item_opt(vm.ctx.intern_str("__spec__"), vm)
            && let Ok(initializing) = spec.get_attr(vm.ctx.intern_str("_initializing"), vm)
        {
            return initializing.try_to_bool(vm).unwrap_or(false);
        }
        false
    }

    #[pygetset(setter)]
    fn set___annotations__(
        zelf: &Py<Self>,
        value: PySetterValue,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let dict = zelf.dict();
        match value {
            PySetterValue::Assign(value) => {
                dict.set_item(identifier!(vm, __annotations__), value, vm)?;
                // Clear __annotate__ from dict
                dict.del_item(identifier!(vm, __annotate__), vm).ok();
                Ok(())
            }
            PySetterValue::Delete => {
                if dict.del_item(identifier!(vm, __annotations__), vm).is_err() {
                    return Err(vm.new_attribute_error("__annotations__".to_owned()));
                }
                // Also clear __annotate__
                dict.del_item(identifier!(vm, __annotate__), vm).ok();
                Ok(())
            }
        }
    }
}

impl Initializer for PyModule {
    type Args = ModuleInitArgs;

    fn init(zelf: PyRef<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
        debug_assert!(
            zelf.class()
                .slots
                .flags
                .has_feature(crate::types::PyTypeFlags::HAS_DICT)
        );
        zelf.init_dict(vm.ctx.intern_str(args.name.as_str()), args.doc, vm);
        Ok(())
    }
}

impl GetAttr for PyModule {
    fn getattro(zelf: &Py<Self>, name: &Py<PyStr>, vm: &VirtualMachine) -> PyResult {
        zelf.getattr_inner(name, vm)
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

pub(crate) fn init(context: &Context) {
    PyModule::extend_class(context, context.types.module_type);
}

/// Check if {module_name}.{name} is an uninitialized submodule in sys.modules.
fn is_uninitialized_submodule(
    module_name: Option<&String>,
    name: &Py<PyStr>,
    vm: &VirtualMachine,
) -> bool {
    let mod_name = match module_name {
        Some(n) => n.as_str(),
        None => return false,
    };
    let full_name = format!("{mod_name}.{name}");
    let sys_modules = match vm.sys_module.get_attr("modules", vm).ok() {
        Some(m) => m,
        None => return false,
    };
    let sub_mod = match sys_modules.get_item(&full_name, vm).ok() {
        Some(m) => m,
        None => return false,
    };
    let spec = match sub_mod.get_attr("__spec__", vm).ok() {
        Some(s) if !vm.is_none(&s) => s,
        _ => return false,
    };
    spec.get_attr("_initializing", vm)
        .ok()
        .and_then(|v| v.try_to_bool(vm).ok())
        .unwrap_or(false)
}

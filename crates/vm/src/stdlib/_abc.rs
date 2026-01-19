//! Implementation of the `_abc` module.
//!
//! This module provides the C implementation of Abstract Base Classes (ABCs)
//! as defined in PEP 3119.

pub(crate) use _abc::make_module;

#[pymodule]
mod _abc {
    use crate::{
        AsObject, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
        builtins::{PyFrozenSet, PyList, PySet, PyStr, PyTupleRef, PyTypeRef, PyWeak},
        common::lock::PyRwLock,
        convert::ToPyObject,
        protocol::PyIterReturn,
        types::Constructor,
    };
    use std::sync::atomic::{AtomicU64, Ordering};

    // Global invalidation counter
    static ABC_INVALIDATION_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn get_invalidation_counter() -> u64 {
        ABC_INVALIDATION_COUNTER.load(Ordering::SeqCst)
    }

    fn increment_invalidation_counter() {
        ABC_INVALIDATION_COUNTER.fetch_add(1, Ordering::SeqCst);
    }

    /// Internal state held by ABC machinery.
    #[pyattr]
    #[pyclass(name = "_abc_data", module = "_abc")]
    #[derive(Debug, PyPayload)]
    struct AbcData {
        // WeakRef sets for registry and caches
        registry: PyRwLock<Option<PyRef<PySet>>>,
        cache: PyRwLock<Option<PyRef<PySet>>>,
        negative_cache: PyRwLock<Option<PyRef<PySet>>>,
        negative_cache_version: AtomicU64,
    }

    #[pyclass(with(Constructor))]
    impl AbcData {
        fn new() -> Self {
            AbcData {
                registry: PyRwLock::new(None),
                cache: PyRwLock::new(None),
                negative_cache: PyRwLock::new(None),
                negative_cache_version: AtomicU64::new(get_invalidation_counter()),
            }
        }

        fn get_cache_version(&self) -> u64 {
            self.negative_cache_version.load(Ordering::SeqCst)
        }

        fn set_cache_version(&self, version: u64) {
            self.negative_cache_version.store(version, Ordering::SeqCst);
        }
    }

    impl Constructor for AbcData {
        type Args = ();

        fn py_new(
            _cls: &crate::Py<crate::builtins::PyType>,
            _args: Self::Args,
            _vm: &VirtualMachine,
        ) -> PyResult<Self> {
            Ok(AbcData::new())
        }
    }

    /// Get the _abc_impl attribute from an ABC class
    fn get_impl(cls: &PyObject, vm: &VirtualMachine) -> PyResult<PyRef<AbcData>> {
        let impl_obj = cls.get_attr("_abc_impl", vm)?;
        impl_obj
            .downcast::<AbcData>()
            .map_err(|_| vm.new_type_error("_abc_impl is set to a wrong type".to_owned()))
    }

    /// Check if obj is in the weak set
    fn in_weak_set(
        set_lock: &PyRwLock<Option<PyRef<PySet>>>,
        obj: &PyObject,
        vm: &VirtualMachine,
    ) -> PyResult<bool> {
        let set_opt = set_lock.read();
        let set = match &*set_opt {
            Some(s) if !s.elements().is_empty() => s.clone(),
            _ => return Ok(false),
        };
        drop(set_opt);

        // Create a weak reference to the object
        let weak_ref = match obj.downgrade(None, vm) {
            Ok(w) => w,
            Err(e) => {
                // If we can't create a weakref (e.g., TypeError), the object can't be in the set
                if e.class().is(vm.ctx.exceptions.type_error) {
                    return Ok(false);
                }
                return Err(e);
            }
        };

        // Use vm.call_method to call __contains__
        let weak_ref_obj: PyObjectRef = weak_ref.into();
        vm.call_method(set.as_ref(), "__contains__", (weak_ref_obj,))?
            .try_to_bool(vm)
    }

    /// Add obj to the weak set
    fn add_to_weak_set(
        set_lock: &PyRwLock<Option<PyRef<PySet>>>,
        obj: &PyObject,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let mut set_opt = set_lock.write();
        let set = match &*set_opt {
            Some(s) => s.clone(),
            None => {
                let new_set = PySet::default().into_ref(&vm.ctx);
                *set_opt = Some(new_set.clone());
                new_set
            }
        };
        drop(set_opt);

        // Create a weak reference to the object
        let weak_ref = obj.downgrade(None, vm)?;
        set.add(weak_ref.into(), vm)?;
        Ok(())
    }

    /// Returns the current ABC cache token.
    #[pyfunction]
    fn get_cache_token() -> u64 {
        get_invalidation_counter()
    }

    /// Compute set of abstract method names.
    fn compute_abstract_methods(cls: &PyObject, vm: &VirtualMachine) -> PyResult<()> {
        let mut abstracts = Vec::new();

        // Stage 1: direct abstract methods
        let ns = cls.get_attr("__dict__", vm)?;
        let items = vm.call_method(&ns, "items", ())?;
        let iter = items.get_iter(vm)?;

        while let PyIterReturn::Return(item) = iter.next(vm)? {
            let tuple: PyTupleRef = item
                .downcast()
                .map_err(|_| vm.new_type_error("items() returned non-tuple".to_owned()))?;
            let elements = tuple.as_slice();
            if elements.len() != 2 {
                return Err(
                    vm.new_type_error("items() returned item which size is not 2".to_owned())
                );
            }
            let key = &elements[0];
            let value = &elements[1];

            // Check if value has __isabstractmethod__ = True
            if let Ok(is_abstract) = value.get_attr("__isabstractmethod__", vm)
                && is_abstract.try_to_bool(vm)?
            {
                abstracts.push(key.clone());
            }
        }

        // Stage 2: inherited abstract methods
        let bases: PyTupleRef = cls
            .get_attr("__bases__", vm)?
            .downcast()
            .map_err(|_| vm.new_type_error("__bases__ is not a tuple".to_owned()))?;

        for base in bases.iter() {
            if let Ok(base_abstracts) = base.get_attr("__abstractmethods__", vm) {
                let iter = base_abstracts.get_iter(vm)?;
                while let PyIterReturn::Return(key) = iter.next(vm)? {
                    // Try to get the attribute from cls - key should be a string
                    if let Some(key_str) = key.downcast_ref::<PyStr>()
                        && let Some(value) = vm.get_attribute_opt(cls.to_owned(), key_str)?
                        && let Ok(is_abstract) = value.get_attr("__isabstractmethod__", vm)
                        && is_abstract.try_to_bool(vm)?
                    {
                        abstracts.push(key);
                    }
                }
            }
        }

        // Set __abstractmethods__
        let abstracts_set = PyFrozenSet::from_iter(vm, abstracts.into_iter())?;
        cls.set_attr("__abstractmethods__", abstracts_set.into_pyobject(vm), vm)?;

        Ok(())
    }

    /// Internal ABC helper for class set-up. Should be never used outside abc module.
    #[pyfunction]
    fn _abc_init(cls: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        compute_abstract_methods(&cls, vm)?;

        // Set up inheritance registry
        let data = AbcData::new();
        cls.set_attr("_abc_impl", data.to_pyobject(vm), vm)?;

        Ok(())
    }

    /// Internal ABC helper for subclass registration. Should be never used outside abc module.
    #[pyfunction]
    fn _abc_register(
        cls: PyObjectRef,
        subclass: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        // Type check
        if !subclass.class().fast_issubclass(vm.ctx.types.type_type) {
            return Err(vm.new_type_error("Can only register classes".to_owned()));
        }

        // Check if already a subclass
        if subclass.is_subclass(&cls, vm)? {
            return Ok(subclass);
        }

        // Check for cycles
        if cls.is_subclass(&subclass, vm)? {
            return Err(vm.new_runtime_error("Refusing to create an inheritance cycle".to_owned()));
        }

        // Add to registry
        let impl_data = get_impl(&cls, vm)?;
        add_to_weak_set(&impl_data.registry, &subclass, vm)?;

        // Invalidate negative cache
        increment_invalidation_counter();

        Ok(subclass)
    }

    /// Internal ABC helper for instance checks. Should be never used outside abc module.
    #[pyfunction]
    fn _abc_instancecheck(
        cls: PyObjectRef,
        instance: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        let impl_data = get_impl(&cls, vm)?;

        // Get instance.__class__
        let subclass = instance.get_attr("__class__", vm)?;

        // Check cache
        if in_weak_set(&impl_data.cache, &subclass, vm)? {
            return Ok(vm.ctx.true_value.clone().into());
        }

        let subtype: PyObjectRef = instance.class().to_owned().into();
        if subtype.is(&subclass) {
            let invalidation_counter = get_invalidation_counter();
            if impl_data.get_cache_version() == invalidation_counter
                && in_weak_set(&impl_data.negative_cache, &subclass, vm)?
            {
                return Ok(vm.ctx.false_value.clone().into());
            }
            // Fall back to __subclasscheck__
            return vm.call_method(&cls, "__subclasscheck__", (subclass,));
        }

        // Call __subclasscheck__ on subclass
        let result = vm.call_method(&cls, "__subclasscheck__", (subclass.clone(),))?;

        match result.clone().try_to_bool(vm) {
            Ok(true) => Ok(result),
            Ok(false) => {
                // Also try with subtype
                vm.call_method(&cls, "__subclasscheck__", (subtype,))
            }
            Err(e) => Err(e),
        }
    }

    /// Check if subclass is in registry (recursive)
    fn subclasscheck_check_registry(
        impl_data: &AbcData,
        subclass: &PyObject,
        vm: &VirtualMachine,
    ) -> PyResult<Option<bool>> {
        // Fast path: check if subclass is in weakref directly
        if in_weak_set(&impl_data.registry, subclass, vm)? {
            return Ok(Some(true));
        }

        let registry_opt = impl_data.registry.read();
        let registry = match &*registry_opt {
            Some(s) => s.clone(),
            None => return Ok(None),
        };
        drop(registry_opt);

        // Make a local copy to protect against concurrent modifications
        let registry_copy = PyFrozenSet::from_iter(vm, registry.elements().into_iter())?;

        for weak_ref_obj in registry_copy.elements() {
            if let Ok(weak_ref) = weak_ref_obj.downcast::<PyWeak>()
                && let Some(rkey) = weak_ref.upgrade()
                && subclass.to_owned().is_subclass(&rkey, vm)?
            {
                add_to_weak_set(&impl_data.cache, subclass, vm)?;
                return Ok(Some(true));
            }
        }

        Ok(None)
    }

    /// Internal ABC helper for subclass checks. Should be never used outside abc module.
    #[pyfunction]
    fn _abc_subclasscheck(
        cls: PyObjectRef,
        subclass: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<bool> {
        // Type check
        if !subclass.class().fast_issubclass(vm.ctx.types.type_type) {
            return Err(vm.new_type_error("issubclass() arg 1 must be a class".to_owned()));
        }

        let impl_data = get_impl(&cls, vm)?;

        // 1. Check cache
        if in_weak_set(&impl_data.cache, &subclass, vm)? {
            return Ok(true);
        }

        // 2. Check negative cache; may have to invalidate
        let invalidation_counter = get_invalidation_counter();
        if impl_data.get_cache_version() < invalidation_counter {
            // Invalidate the negative cache
            // Clone set ref and drop lock before calling into VM to avoid reentrancy
            let set = impl_data.negative_cache.read().clone();
            if let Some(ref set) = set {
                vm.call_method(set.as_ref(), "clear", ())?;
            }
            impl_data.set_cache_version(invalidation_counter);
        } else if in_weak_set(&impl_data.negative_cache, &subclass, vm)? {
            return Ok(false);
        }

        // 3. Check the subclass hook
        let ok = vm.call_method(&cls, "__subclasshook__", (subclass.clone(),))?;
        if ok.is(&vm.ctx.true_value) {
            add_to_weak_set(&impl_data.cache, &subclass, vm)?;
            return Ok(true);
        }
        if ok.is(&vm.ctx.false_value) {
            add_to_weak_set(&impl_data.negative_cache, &subclass, vm)?;
            return Ok(false);
        }
        if !ok.is(&vm.ctx.not_implemented) {
            return Err(vm.new_exception_msg(
                vm.ctx.exceptions.assertion_error.to_owned(),
                "__subclasshook__ must return either False, True, or NotImplemented".to_owned(),
            ));
        }

        // 4. Check if it's a direct subclass
        let subclass_type: PyTypeRef = subclass
            .clone()
            .downcast()
            .map_err(|_| vm.new_type_error("expected a type object".to_owned()))?;
        let cls_type: PyTypeRef = cls
            .clone()
            .downcast()
            .map_err(|_| vm.new_type_error("expected a type object".to_owned()))?;
        if subclass_type.fast_issubclass(&cls_type) {
            add_to_weak_set(&impl_data.cache, &subclass, vm)?;
            return Ok(true);
        }

        // 5. Check if it's a subclass of a registered class (recursive)
        if let Some(result) = subclasscheck_check_registry(&impl_data, &subclass, vm)? {
            return Ok(result);
        }

        // 6. Check if it's a subclass of a subclass (recursive)
        let subclasses: PyRef<PyList> = vm
            .call_method(&cls, "__subclasses__", ())?
            .downcast()
            .map_err(|_| vm.new_type_error("__subclasses__() must return a list".to_owned()))?;

        for scls in subclasses.borrow_vec().iter() {
            if subclass.is_subclass(scls, vm)? {
                add_to_weak_set(&impl_data.cache, &subclass, vm)?;
                return Ok(true);
            }
        }

        // No dice; update negative cache
        add_to_weak_set(&impl_data.negative_cache, &subclass, vm)?;
        Ok(false)
    }

    /// Internal ABC helper for cache and registry debugging.
    #[pyfunction]
    fn _get_dump(cls: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
        let impl_data = get_impl(&cls, vm)?;

        let registry = {
            let r = impl_data.registry.read();
            match &*r {
                Some(s) => {
                    // Use copy method to get a shallow copy
                    vm.call_method(s.as_ref(), "copy", ())?
                }
                None => PySet::default().to_pyobject(vm),
            }
        };

        let cache = {
            let c = impl_data.cache.read();
            match &*c {
                Some(s) => vm.call_method(s.as_ref(), "copy", ())?,
                None => PySet::default().to_pyobject(vm),
            }
        };

        let negative_cache = {
            let nc = impl_data.negative_cache.read();
            match &*nc {
                Some(s) => vm.call_method(s.as_ref(), "copy", ())?,
                None => PySet::default().to_pyobject(vm),
            }
        };

        let version = impl_data.get_cache_version();

        Ok(vm.ctx.new_tuple(vec![
            registry,
            cache,
            negative_cache,
            vm.ctx.new_int(version).into(),
        ]))
    }

    /// Internal ABC helper to reset registry of a given class.
    #[pyfunction]
    fn _reset_registry(cls: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let impl_data = get_impl(&cls, vm)?;
        // Clone set ref and drop lock before calling into VM to avoid reentrancy
        let set = impl_data.registry.read().clone();
        if let Some(ref set) = set {
            vm.call_method(set.as_ref(), "clear", ())?;
        }
        Ok(())
    }

    /// Internal ABC helper to reset both caches of a given class.
    #[pyfunction]
    fn _reset_caches(cls: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let impl_data = get_impl(&cls, vm)?;

        // Clone set refs and drop locks before calling into VM to avoid reentrancy
        let cache = impl_data.cache.read().clone();
        if let Some(ref set) = cache {
            vm.call_method(set.as_ref(), "clear", ())?;
        }

        let negative_cache = impl_data.negative_cache.read().clone();
        if let Some(ref set) = negative_cache {
            vm.call_method(set.as_ref(), "clear", ())?;
        }

        Ok(())
    }
}

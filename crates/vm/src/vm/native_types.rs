//! Interpreter-owned Python namespaces for immutable native type definitions.

use crate::{
    Context, Py, PyRef, VirtualMachine,
    builtins::{PyType, PyTypeRef, PyWeak, type_::TypeNamespace},
    class::NativeOperators,
    common::{
        lock::{PyMutex, PyRwLock},
        rc::PyRc,
    },
    types::PyTypeFlags,
};
use core::{any::TypeId, cell::RefCell};
use std::collections::HashMap;

pub struct NativeNamespaceDefinition {
    pub(crate) class: PyTypeRef,
    pub(crate) operators: NativeOperators,
    pub(crate) populate: fn(&Context, &Py<PyType>, &NativeOperators),
}

pub(crate) const RUNTIME_FLAGS: PyTypeFlags = PyTypeFlags::from_slice(&[
    PyTypeFlags::SEQUENCE,
    PyTypeFlags::MAPPING,
    PyTypeFlags::IS_ABSTRACT,
]);

#[derive(Default)]
pub(crate) struct NativeTypes {
    state: PyMutex<NativeTypeState>,
}

#[derive(Default)]
struct NativeTypeState {
    namespaces: HashMap<usize, PyRc<TypeNamespace>>,
    classes: HashMap<TypeId, PyTypeRef>,
    subclasses: HashMap<usize, PyRc<PyRwLock<Vec<PyRef<PyWeak>>>>>,
    runtime_flags: HashMap<usize, PyTypeFlags>,
    closed: bool,
}

thread_local! {
    static INITIALIZATION_OWNERS: RefCell<Vec<i64>> = const { RefCell::new(Vec::new()) };
    // Recursive construction sees its own candidate. Other threads only see
    // fully initialized namespaces, published under the interpreter type lock.
    static INITIALIZING: RefCell<Vec<(i64, usize, PyRc<TypeNamespace>)>> = const {
        RefCell::new(Vec::new())
    };
    static INITIALIZING_CLASSES: RefCell<Vec<(i64, TypeId, PyTypeRef)>> = const {
        RefCell::new(Vec::new())
    };
}

pub(crate) fn initializing(owner: i64) -> bool {
    INITIALIZATION_OWNERS.with(|owners| owners.borrow().contains(&owner))
}

struct InitializationGuard(i64);

impl InitializationGuard {
    fn new(owner: i64) -> Self {
        INITIALIZATION_OWNERS.with(|owners| owners.borrow_mut().push(owner));
        Self(owner)
    }
}

impl Drop for InitializationGuard {
    fn drop(&mut self) {
        let previous = INITIALIZATION_OWNERS.with(|owners| owners.borrow_mut().pop());
        debug_assert_eq!(previous, Some(self.0));
    }
}

impl NativeTypes {
    pub(crate) fn runtime_flags(&self, class: &PyType) -> Option<PyTypeFlags> {
        self.state
            .lock()
            .runtime_flags
            .get(&core::ptr::from_ref(class).addr())
            .copied()
    }

    pub(crate) fn set_runtime_flags(&self, class: &PyType, mask: PyTypeFlags, flags: PyTypeFlags) {
        let mut state = self.state.lock();
        let current = state
            .runtime_flags
            .entry(core::ptr::from_ref(class).addr())
            .or_insert_with(|| class.slots.flags.load() & RUNTIME_FLAGS);
        *current = (*current & !mask) | (flags & mask);
    }

    pub(crate) fn subclasses(&self, class: &PyType) -> PyRc<PyRwLock<Vec<PyRef<PyWeak>>>> {
        let key = core::ptr::from_ref(class).addr();
        let mut state = self.state.lock();
        if state.closed {
            return PyRc::default();
        }
        state.subclasses.entry(key).or_default().clone()
    }

    pub(crate) fn class<T: 'static>(
        &self,
        vm: &VirtualMachine,
        allocate: impl FnOnce() -> PyTypeRef,
        populate: impl FnOnce(&Py<PyType>),
    ) -> PyTypeRef {
        let key = TypeId::of::<T>();
        let cached = self.state.lock().classes.get(&key).cloned();
        if let Some(class) = cached {
            return class;
        }
        let owner = vm.state.interpreter_id;
        if let Some(class) = INITIALIZING_CLASSES.with(|entries| {
            entries
                .borrow()
                .iter()
                .rev()
                .find_map(|(id, candidate, class)| {
                    (*id == owner && *candidate == key).then(|| class.clone())
                })
        }) {
            return class;
        }
        PyType::with_type_lock(vm, || {
            let cached = self.state.lock().classes.get(&key).cloned();
            if let Some(class) = cached {
                return class;
            }
            let _allocation = InitializationGuard::new(owner);
            let class = allocate();
            INITIALIZING_CLASSES
                .with(|entries| entries.borrow_mut().push((owner, key, class.clone())));
            let _initializing = scopeguard::guard((), |()| {
                INITIALIZING_CLASSES.with(|entries| {
                    let previous = entries.borrow_mut().pop().unwrap();
                    debug_assert_eq!((previous.0, previous.1), (owner, key));
                    drop(previous);
                });
            });
            populate(&class);
            let mut state = self.state.lock();
            if !state.closed {
                state.classes.insert(key, class.clone());
            }
            class
        })
    }

    fn get(&self, key: usize) -> Option<PyRc<TypeNamespace>> {
        self.state.lock().namespaces.get(&key).cloned()
    }

    pub(crate) fn namespace(
        &self,
        definition: &NativeNamespaceDefinition,
        vm: &VirtualMachine,
    ) -> PyRc<TypeNamespace> {
        let key = core::ptr::from_ref(definition).addr();
        if let Some(namespace) = self.get(key) {
            return namespace;
        }
        let owner = vm.state.interpreter_id;
        if let Some(namespace) = INITIALIZING.with(|entries| {
            entries
                .borrow()
                .iter()
                .rev()
                .find_map(|(id, candidate, namespace)| {
                    (*id == owner && *candidate == key).then(|| namespace.clone())
                })
        }) {
            return namespace;
        }
        PyType::with_type_lock(vm, || {
            if let Some(namespace) = self.get(key) {
                return namespace;
            }
            let _allocation = InitializationGuard::new(owner);
            let namespace = PyRc::new(TypeNamespace::default());
            INITIALIZING.with(|entries| {
                entries.borrow_mut().push((owner, key, namespace.clone()));
            });
            let _initializing = scopeguard::guard((), |()| {
                INITIALIZING.with(|entries| {
                    let previous = entries.borrow_mut().pop().unwrap();
                    debug_assert_eq!((previous.0, previous.1), (owner, key));
                    drop(previous);
                });
            });
            (definition.populate)(&vm.ctx, &definition.class, &definition.operators);
            let mut state = self.state.lock();
            if !state.closed {
                state.namespaces.insert(key, namespace.clone());
            }
            namespace
        })
    }

    pub(super) fn close(&self) {
        let roots = {
            let mut state = self.state.lock();
            state.closed = true;
            (
                core::mem::take(&mut state.namespaces),
                core::mem::take(&mut state.classes),
                core::mem::take(&mut state.subclasses),
            )
        };
        drop(roots);
    }

    #[cfg(all(unix, feature = "threading"))]
    pub(super) unsafe fn reinit_after_fork(&self) {
        unsafe { crate::common::lock::reinit_mutex_after_fork(&self.state) };
        #[expect(
            clippy::iter_over_hash_type,
            reason = "independent locks have no reset order"
        )]
        for registry in self.state.lock().subclasses.values() {
            unsafe { crate::common::lock::reinit_rwlock_after_fork(registry) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AsObject, Interpreter, PyPayload, class::PyClassImpl};

    #[pyclass(interpreter_local, module = false, name = "LocalNative")]
    #[derive(Debug, PyPayload)]
    struct LocalNative;

    #[pyclass]
    impl LocalNative {
        #[extend_class]
        fn populate(ctx: &Context, class: &Py<PyType>) {
            assert!(Self::make_class(ctx).is(class));
            class.set_str_attr("state", ctx.new_list(Vec::new()), ctx);
        }
    }

    #[pyclass(module = false, name = "ColdNativeAttributes")]
    #[derive(Debug, PyPayload)]
    struct ColdNativeAttributes;

    #[pyclass]
    impl ColdNativeAttributes {
        #[extend_class]
        fn populate(ctx: &Context, class: &Py<PyType>) {
            class.set_str_attr("cold_native_attribute", ctx.new_int(42), ctx);
        }
    }

    #[pyclass(module = false, name = "ColdNativeMethod")]
    #[derive(Debug, PyPayload)]
    struct ColdNativeMethod;

    #[pyclass]
    impl ColdNativeMethod {
        #[pymethod]
        fn cold_native_method(_zelf: &Py<Self>) -> i32 {
            43
        }
    }

    #[test]
    fn lazy_native_names_are_resolved_on_first_lookup() {
        Interpreter::without_stdlib(Default::default()).enter_raw(|vm| {
            let _ = ColdNativeAttributes::make_class(&vm.ctx);
            let object = ColdNativeAttributes.into_ref(&vm.ctx);
            let name = vm.ctx.new_str("cold_native_attribute");
            assert!(vm.ctx.interned_str(&*name).is_none());
            let value = object.as_object().get_attr(&*name, vm).unwrap();
            assert_eq!(value.try_to_value::<i32>(vm).unwrap(), 42);

            let _ = ColdNativeMethod::make_class(&vm.ctx);
            let object = ColdNativeMethod.into_ref(&vm.ctx);
            assert!(vm.ctx.interned_str("cold_native_method").is_none());
            let value = vm
                .call_method(object.as_object(), "cold_native_method", ())
                .unwrap();
            assert_eq!(value.try_to_value::<i32>(vm).unwrap(), 43);

            let missing = vm.ctx.new_str("cold_native_missing_attribute");
            assert!(object.class().lookup_ref(&missing, vm).is_none());
            assert!(vm.ctx.interned_str(&*missing).is_none());
        });
    }

    #[test]
    fn local_types_preserve_owner_identity_and_exact_downcasts() {
        let first = Interpreter::without_stdlib(Default::default());
        let second = first.create_subinterpreter();
        first.enter_raw(|vm| {
            let class = LocalNative::make_class(&vm.ctx);
            let object = LocalNative.into_ref(&vm.ctx);
            assert!(object.class().is(&class));
            assert!(
                object
                    .as_object()
                    .downcast_ref_if_exact::<LocalNative>(vm)
                    .is_some()
            );
            second.enter_raw(|other| {
                let other_class = LocalNative::make_class(&other.ctx);
                assert!(!class.is(&other_class));
                assert!(LocalNative::make_class(&other.ctx).is(&other_class));
                assert!(LocalNative.into_ref(&other.ctx).class().is(&other_class));
            });
            assert!(LocalNative::make_class(&vm.ctx).is(&class));
        });
    }

    #[cfg(feature = "threading")]
    #[test]
    fn competing_local_type_initializers_publish_one_class() {
        let interpreter = Interpreter::without_stdlib(Default::default());
        let first = interpreter.new_thread();
        let second = interpreter.new_thread();
        let (first, second) = std::thread::scope(|scope| {
            let first = scope.spawn(move || first.run_raw(|vm| LocalNative::make_class(&vm.ctx)));
            let second = scope.spawn(move || second.run_raw(|vm| LocalNative::make_class(&vm.ctx)));
            (first.join().unwrap(), second.join().unwrap())
        });
        interpreter.enter_raw(|vm| {
            assert!(first.is(&second));
            assert!(first.is(&LocalNative::make_class(&vm.ctx)));
            drop((first, second));
        });
    }

    #[test]
    fn failed_factory_unwinds_initialization_state() {
        struct Failed;
        let interpreter = Interpreter::without_stdlib(Default::default());
        interpreter.enter_raw(|vm| {
            let make =
                || PyType::new_simple_heap("Retry", vm.ctx.types.object_type, &vm.ctx).unwrap();
            let failed = std::panic::catch_unwind(core::panic::AssertUnwindSafe(|| {
                vm.state
                    .native_types
                    .class::<Failed>(vm, make, |_| panic!("factory failed"));
            }));
            assert!(failed.is_err());
            assert!(!initializing(vm.state.interpreter_id));
            let class = vm.state.native_types.class::<Failed>(vm, make, |_| {});
            let again =
                vm.state
                    .native_types
                    .class::<Failed>(vm, || unreachable!(), |_| unreachable!());
            assert!(class.is(&again));
        });
    }
}

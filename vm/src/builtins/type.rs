use super::{
    PyClassMethod, PyDictRef, PyList, PyStr, PyStrInterned, PyStrRef, PyTupleRef, PyWeak,
    mappingproxy::PyMappingProxy, object, union_,
};
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject,
    VirtualMachine,
    builtins::{
        PyBaseExceptionRef,
        descriptor::{
            MemberGetter, MemberKind, MemberSetter, PyDescriptorOwned, PyMemberDef,
            PyMemberDescriptor,
        },
        function::PyCellRef,
        tuple::{IntoPyTuple, PyTuple},
    },
    class::{PyClassImpl, StaticType},
    common::{
        ascii,
        borrow::BorrowedValue,
        lock::{PyRwLock, PyRwLockReadGuard},
    },
    convert::ToPyResult,
    function::{FuncArgs, KwArgs, OptionalArg, PyMethodDef, PySetterValue},
    identifier,
    object::{Traverse, TraverseFn},
    protocol::{PyIterReturn, PyMappingMethods, PyNumberMethods, PySequenceMethods},
    types::{
        AsNumber, Callable, Constructor, GetAttr, PyTypeFlags, PyTypeSlots, Representable, SetAttr,
    },
};
use indexmap::{IndexMap, map::Entry};
use itertools::Itertools;
use std::{borrow::Borrow, collections::HashSet, ops::Deref, pin::Pin, ptr::NonNull};

#[pyclass(module = false, name = "type", traverse = "manual")]
pub struct PyType {
    pub base: Option<PyTypeRef>,
    pub bases: PyRwLock<Vec<PyTypeRef>>,
    pub mro: PyRwLock<Vec<PyTypeRef>>,
    pub subclasses: PyRwLock<Vec<PyRef<PyWeak>>>,
    pub attributes: PyRwLock<PyAttributes>,
    pub slots: PyTypeSlots,
    pub heaptype_ext: Option<Pin<Box<HeapTypeExt>>>,
}

unsafe impl crate::object::Traverse for PyType {
    fn traverse(&self, tracer_fn: &mut crate::object::TraverseFn<'_>) {
        self.base.traverse(tracer_fn);
        self.bases.traverse(tracer_fn);
        self.mro.traverse(tracer_fn);
        self.subclasses.traverse(tracer_fn);
        self.attributes
            .read_recursive()
            .iter()
            .map(|(_, v)| v.traverse(tracer_fn))
            .count();
    }
}

// PyHeapTypeObject in CPython
pub struct HeapTypeExt {
    pub name: PyRwLock<PyStrRef>,
    pub qualname: PyRwLock<PyStrRef>,
    pub slots: Option<PyRef<PyTuple<PyStrRef>>>,
    pub sequence_methods: PySequenceMethods,
    pub mapping_methods: PyMappingMethods,
}

pub struct PointerSlot<T>(NonNull<T>);

unsafe impl<T> Sync for PointerSlot<T> {}
unsafe impl<T> Send for PointerSlot<T> {}

impl<T> PointerSlot<T> {
    pub const unsafe fn borrow_static(&self) -> &'static T {
        unsafe { self.0.as_ref() }
    }
}

impl<T> Clone for PointerSlot<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for PointerSlot<T> {}

impl<T> From<&'static T> for PointerSlot<T> {
    fn from(x: &'static T) -> Self {
        Self(NonNull::from(x))
    }
}

impl<T> AsRef<T> for PointerSlot<T> {
    fn as_ref(&self) -> &T {
        unsafe { self.0.as_ref() }
    }
}

impl<T> PointerSlot<T> {
    pub unsafe fn from_heaptype<F>(typ: &PyType, f: F) -> Option<Self>
    where
        F: FnOnce(&HeapTypeExt) -> &T,
    {
        typ.heaptype_ext
            .as_ref()
            .map(|ext| Self(NonNull::from(f(ext))))
    }
}

pub type PyTypeRef = PyRef<PyType>;

cfg_if::cfg_if! {
    if #[cfg(feature = "threading")] {
        unsafe impl Send for PyType {}
        unsafe impl Sync for PyType {}
    }
}

/// For attributes we do not use a dict, but an IndexMap, which is an Hash Table
/// that maintains order and is compatible with the standard HashMap  This is probably
/// faster and only supports strings as keys.
pub type PyAttributes = IndexMap<&'static PyStrInterned, PyObjectRef, ahash::RandomState>;

unsafe impl Traverse for PyAttributes {
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        self.values().for_each(|v| v.traverse(tracer_fn));
    }
}

impl std::fmt::Display for PyType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.name(), f)
    }
}

impl std::fmt::Debug for PyType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[PyType {}]", &self.name())
    }
}

impl PyPayload for PyType {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.type_type
    }
}

fn downcast_qualname(value: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyRef<PyStr>> {
    match value.downcast::<PyStr>() {
        Ok(value) => Ok(value),
        Err(value) => Err(vm.new_type_error(format!(
            "can only assign string to __qualname__, not '{}'",
            value.class().name()
        ))),
    }
}

fn is_subtype_with_mro(a_mro: &[PyTypeRef], a: &Py<PyType>, b: &Py<PyType>) -> bool {
    if a.is(b) {
        return true;
    }
    for item in a_mro {
        if item.is(b) {
            return true;
        }
    }
    false
}

impl PyType {
    pub fn new_simple_heap(
        name: &str,
        base: &PyTypeRef,
        ctx: &Context,
    ) -> Result<PyRef<Self>, String> {
        Self::new_heap(
            name,
            vec![base.clone()],
            Default::default(),
            Default::default(),
            Self::static_type().to_owned(),
            ctx,
        )
    }
    pub fn new_heap(
        name: &str,
        bases: Vec<PyRef<Self>>,
        attrs: PyAttributes,
        slots: PyTypeSlots,
        metaclass: PyRef<Self>,
        ctx: &Context,
    ) -> Result<PyRef<Self>, String> {
        // TODO: ensure clean slot name
        // assert_eq!(slots.name.borrow(), "");

        let name = ctx.new_str(name);
        let heaptype_ext = HeapTypeExt {
            name: PyRwLock::new(name.clone()),
            qualname: PyRwLock::new(name),
            slots: None,
            sequence_methods: PySequenceMethods::default(),
            mapping_methods: PyMappingMethods::default(),
        };
        let base = bases[0].clone();

        Self::new_heap_inner(base, bases, attrs, slots, heaptype_ext, metaclass, ctx)
    }

    /// Equivalent to CPython's PyType_Check macro
    /// Checks if obj is an instance of type (or its subclass)
    pub(crate) fn check(obj: &PyObject) -> Option<&Py<Self>> {
        obj.downcast_ref::<Self>()
    }

    fn resolve_mro(bases: &[PyRef<Self>]) -> Result<Vec<PyTypeRef>, String> {
        // Check for duplicates in bases.
        let mut unique_bases = HashSet::new();
        for base in bases {
            if !unique_bases.insert(base.get_id()) {
                return Err(format!("duplicate base class {}", base.name()));
            }
        }

        let mros = bases
            .iter()
            .map(|base| base.mro_map_collect(|t| t.to_owned()))
            .collect();
        linearise_mro(mros)
    }

    #[allow(clippy::too_many_arguments)]
    fn new_heap_inner(
        base: PyRef<Self>,
        bases: Vec<PyRef<Self>>,
        attrs: PyAttributes,
        mut slots: PyTypeSlots,
        heaptype_ext: HeapTypeExt,
        metaclass: PyRef<Self>,
        ctx: &Context,
    ) -> Result<PyRef<Self>, String> {
        let mro = Self::resolve_mro(&bases)?;

        if base.slots.flags.has_feature(PyTypeFlags::HAS_DICT) {
            slots.flags |= PyTypeFlags::HAS_DICT
        }
        if slots.basicsize == 0 {
            slots.basicsize = base.slots.basicsize;
        }

        if let Some(qualname) = attrs.get(identifier!(ctx, __qualname__)) {
            if !qualname.fast_isinstance(ctx.types.str_type) {
                return Err(format!(
                    "type __qualname__ must be a str, not {}",
                    qualname.class().name()
                ));
            }
        }

        let new_type = PyRef::new_ref(
            Self {
                base: Some(base),
                bases: PyRwLock::new(bases),
                mro: PyRwLock::new(mro),
                subclasses: PyRwLock::default(),
                attributes: PyRwLock::new(attrs),
                slots,
                heaptype_ext: Some(Pin::new(Box::new(heaptype_ext))),
            },
            metaclass,
            None,
        );

        new_type.init_slots(ctx);

        let weakref_type = super::PyWeak::static_type();
        for base in new_type.bases.read().iter() {
            base.subclasses.write().push(
                new_type
                    .as_object()
                    .downgrade_with_weakref_typ_opt(None, weakref_type.to_owned())
                    .unwrap(),
            );
        }

        Ok(new_type)
    }

    pub fn new_static(
        base: PyRef<Self>,
        attrs: PyAttributes,
        mut slots: PyTypeSlots,
        metaclass: PyRef<Self>,
    ) -> Result<PyRef<Self>, String> {
        if base.slots.flags.has_feature(PyTypeFlags::HAS_DICT) {
            slots.flags |= PyTypeFlags::HAS_DICT
        }
        if slots.basicsize == 0 {
            slots.basicsize = base.slots.basicsize;
        }

        let bases = PyRwLock::new(vec![base.clone()]);
        let mro = base.mro_map_collect(|x| x.to_owned());

        let new_type = PyRef::new_ref(
            Self {
                base: Some(base),
                bases,
                mro: PyRwLock::new(mro),
                subclasses: PyRwLock::default(),
                attributes: PyRwLock::new(attrs),
                slots,
                heaptype_ext: None,
            },
            metaclass,
            None,
        );

        let weakref_type = super::PyWeak::static_type();
        for base in new_type.bases.read().iter() {
            base.subclasses.write().push(
                new_type
                    .as_object()
                    .downgrade_with_weakref_typ_opt(None, weakref_type.to_owned())
                    .unwrap(),
            );
        }

        Ok(new_type)
    }

    pub(crate) fn init_slots(&self, ctx: &Context) {
        #[allow(clippy::mutable_key_type)]
        let mut slot_name_set = std::collections::HashSet::new();

        for cls in self.mro.read().iter() {
            for &name in cls.attributes.read().keys() {
                if name == identifier!(ctx, __new__) {
                    continue;
                }
                if name.as_str().starts_with("__") && name.as_str().ends_with("__") {
                    slot_name_set.insert(name);
                }
            }
        }
        for &name in self.attributes.read().keys() {
            if name.as_str().starts_with("__") && name.as_str().ends_with("__") {
                slot_name_set.insert(name);
            }
        }
        for attr_name in slot_name_set {
            self.update_slot::<true>(attr_name, ctx);
        }
    }

    // This is used for class initialization where the vm is not yet available.
    pub fn set_str_attr<V: Into<PyObjectRef>>(
        &self,
        attr_name: &str,
        value: V,
        ctx: impl AsRef<Context>,
    ) {
        let ctx = ctx.as_ref();
        let attr_name = ctx.intern_str(attr_name);
        self.set_attr(attr_name, value.into())
    }

    pub fn set_attr(&self, attr_name: &'static PyStrInterned, value: PyObjectRef) {
        self.attributes.write().insert(attr_name, value);
    }

    /// This is the internal get_attr implementation for fast lookup on a class.
    pub fn get_attr(&self, attr_name: &'static PyStrInterned) -> Option<PyObjectRef> {
        flame_guard!(format!("class_get_attr({:?})", attr_name));

        self.get_direct_attr(attr_name)
            .or_else(|| self.get_super_attr(attr_name))
    }

    pub fn get_direct_attr(&self, attr_name: &'static PyStrInterned) -> Option<PyObjectRef> {
        self.attributes.read().get(attr_name).cloned()
    }

    /// Equivalent to CPython's find_name_in_mro
    /// Look in tp_dict of types in MRO - bypasses descriptors and other attribute access machinery
    fn find_name_in_mro(&self, name: &'static PyStrInterned) -> Option<PyObjectRef> {
        // First check in our own dict
        if let Some(value) = self.attributes.read().get(name) {
            return Some(value.clone());
        }

        // Then check in MRO
        for base in self.mro.read().iter() {
            if let Some(value) = base.attributes.read().get(name) {
                return Some(value.clone());
            }
        }

        None
    }

    /// Equivalent to CPython's _PyType_LookupRef
    /// Looks up a name through the MRO without setting an exception
    pub fn lookup_ref(&self, name: &Py<PyStr>, vm: &VirtualMachine) -> Option<PyObjectRef> {
        // Get interned name for efficient lookup
        let interned_name = vm.ctx.interned_str(name)?;

        // Use find_name_in_mro which matches CPython's behavior
        // This bypasses descriptors and other attribute access machinery
        self.find_name_in_mro(interned_name)
    }

    pub fn get_super_attr(&self, attr_name: &'static PyStrInterned) -> Option<PyObjectRef> {
        self.mro
            .read()
            .iter()
            .find_map(|class| class.attributes.read().get(attr_name).cloned())
    }

    // This is the internal has_attr implementation for fast lookup on a class.
    pub fn has_attr(&self, attr_name: &'static PyStrInterned) -> bool {
        self.attributes.read().contains_key(attr_name)
            || self
                .mro
                .read()
                .iter()
                .any(|c| c.attributes.read().contains_key(attr_name))
    }

    pub fn get_attributes(&self) -> PyAttributes {
        // Gather all members here:
        let mut attributes = PyAttributes::default();

        for bc in std::iter::once(self)
            .chain(self.mro.read().iter().map(|cls| -> &Self { cls }))
            .rev()
        {
            for (name, value) in bc.attributes.read().iter() {
                attributes.insert(name.to_owned(), value.clone());
            }
        }

        attributes
    }

    // bound method for every type
    pub(crate) fn __new__(zelf: PyRef<Self>, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        let (subtype, args): (PyRef<Self>, FuncArgs) = args.bind(vm)?;
        if !subtype.fast_issubclass(&zelf) {
            return Err(vm.new_type_error(format!(
                "{zelf}.__new__({subtype}): {subtype} is not a subtype of {zelf}",
                zelf = zelf.name(),
                subtype = subtype.name(),
            )));
        }
        call_slot_new(zelf, subtype, args, vm)
    }

    fn name_inner<'a, R: 'a>(
        &'a self,
        static_f: impl FnOnce(&'static str) -> R,
        heap_f: impl FnOnce(&'a HeapTypeExt) -> R,
    ) -> R {
        if !self.slots.flags.has_feature(PyTypeFlags::HEAPTYPE) {
            static_f(self.slots.name)
        } else {
            heap_f(self.heaptype_ext.as_ref().unwrap())
        }
    }

    pub fn slot_name(&self) -> BorrowedValue<'_, str> {
        self.name_inner(
            |name| name.into(),
            |ext| PyRwLockReadGuard::map(ext.name.read(), |name| name.as_str()).into(),
        )
    }

    pub fn name(&self) -> BorrowedValue<'_, str> {
        self.name_inner(
            |name| name.rsplit_once('.').map_or(name, |(_, name)| name).into(),
            |ext| PyRwLockReadGuard::map(ext.name.read(), |name| name.as_str()).into(),
        )
    }
}

impl Py<PyType> {
    pub(crate) fn is_subtype(&self, other: &Self) -> bool {
        is_subtype_with_mro(&self.mro.read(), self, other)
    }

    /// Equivalent to CPython's PyType_CheckExact macro
    /// Checks if obj is exactly a type (not a subclass)
    pub fn check_exact<'a>(obj: &'a PyObject, vm: &VirtualMachine) -> Option<&'a Self> {
        obj.downcast_ref_if_exact::<PyType>(vm)
    }

    /// Determines if `subclass` is actually a subclass of `cls`, this doesn't call __subclasscheck__,
    /// so only use this if `cls` is known to have not overridden the base __subclasscheck__ magic
    /// method.
    pub fn fast_issubclass(&self, cls: &impl Borrow<PyObject>) -> bool {
        self.as_object().is(cls.borrow()) || self.mro.read().iter().any(|c| c.is(cls.borrow()))
    }

    pub fn mro_map_collect<F, R>(&self, f: F) -> Vec<R>
    where
        F: Fn(&Self) -> R,
    {
        std::iter::once(self)
            .chain(self.mro.read().iter().map(|x| x.deref()))
            .map(f)
            .collect()
    }

    pub fn mro_collect(&self) -> Vec<PyRef<PyType>> {
        std::iter::once(self)
            .chain(self.mro.read().iter().map(|x| x.deref()))
            .map(|x| x.to_owned())
            .collect()
    }

    pub(crate) fn mro_find_map<F, R>(&self, f: F) -> Option<R>
    where
        F: Fn(&Self) -> Option<R>,
    {
        // the hot path will be primitive types which usually hit the result from itself.
        // try std::intrinsics::likely once it is stabilized
        if let Some(r) = f(self) {
            Some(r)
        } else {
            self.mro.read().iter().find_map(|cls| f(cls))
        }
    }

    pub fn iter_base_chain(&self) -> impl Iterator<Item = &Self> {
        std::iter::successors(Some(self), |cls| cls.base.as_deref())
    }

    pub fn extend_methods(&'static self, method_defs: &'static [PyMethodDef], ctx: &Context) {
        for method_def in method_defs {
            let method = method_def.to_proper_method(self, ctx);
            self.set_attr(ctx.intern_str(method_def.name), method);
        }
    }
}

#[pyclass(
    with(Py, Constructor, GetAttr, SetAttr, Callable, AsNumber, Representable),
    flags(BASETYPE)
)]
impl PyType {
    #[pygetset]
    fn __bases__(&self, vm: &VirtualMachine) -> PyTupleRef {
        vm.ctx.new_tuple(
            self.bases
                .read()
                .iter()
                .map(|x| x.as_object().to_owned())
                .collect(),
        )
    }
    #[pygetset(setter, name = "__bases__")]
    fn set_bases(zelf: &Py<Self>, bases: Vec<PyTypeRef>, vm: &VirtualMachine) -> PyResult<()> {
        // TODO: Assigning to __bases__ is only used in typing.NamedTupleMeta.__new__
        // Rather than correctly re-initializing the class, we are skipping a few steps for now
        if zelf.slots.flags.has_feature(PyTypeFlags::IMMUTABLETYPE) {
            return Err(vm.new_type_error(format!(
                "cannot set '__bases__' attribute of immutable type '{}'",
                zelf.name()
            )));
        }
        if bases.is_empty() {
            return Err(vm.new_type_error(format!(
                "can only assign non-empty tuple to %s.__bases__, not {}",
                zelf.name()
            )));
        }

        // TODO: check for mro cycles

        // TODO: Remove this class from all subclass lists
        // for base in self.bases.read().iter() {
        //     let subclasses = base.subclasses.write();
        //     // TODO: how to uniquely identify the subclasses to remove?
        // }

        *zelf.bases.write() = bases;
        // Recursively update the mros of this class and all subclasses
        fn update_mro_recursively(cls: &PyType, vm: &VirtualMachine) -> PyResult<()> {
            *cls.mro.write() =
                PyType::resolve_mro(&cls.bases.read()).map_err(|msg| vm.new_type_error(msg))?;
            for subclass in cls.subclasses.write().iter() {
                let subclass = subclass.upgrade().unwrap();
                let subclass: &Py<PyType> = subclass.downcast_ref().unwrap();
                update_mro_recursively(subclass, vm)?;
            }
            Ok(())
        }
        update_mro_recursively(zelf, vm)?;

        // TODO: do any old slots need to be cleaned up first?
        zelf.init_slots(&vm.ctx);

        // Register this type as a subclass of its new bases
        let weakref_type = super::PyWeak::static_type();
        for base in zelf.bases.read().iter() {
            base.subclasses.write().push(
                zelf.as_object()
                    .downgrade_with_weakref_typ_opt(None, weakref_type.to_owned())
                    .unwrap(),
            );
        }

        Ok(())
    }

    #[pygetset]
    fn __base__(&self) -> Option<PyTypeRef> {
        self.base.clone()
    }

    #[pygetset]
    const fn __flags__(&self) -> u64 {
        self.slots.flags.bits()
    }

    #[pygetset]
    const fn __basicsize__(&self) -> usize {
        self.slots.basicsize
    }

    #[pygetset]
    pub fn __name__(&self, vm: &VirtualMachine) -> PyStrRef {
        self.name_inner(
            |name| {
                vm.ctx
                    .interned_str(name.rsplit_once('.').map_or(name, |(_, name)| name))
                    .unwrap_or_else(|| {
                        panic!(
                            "static type name must be already interned but {} is not",
                            self.slot_name()
                        )
                    })
                    .to_owned()
            },
            |ext| ext.name.read().clone(),
        )
    }

    #[pygetset]
    pub fn __qualname__(&self, vm: &VirtualMachine) -> PyObjectRef {
        if let Some(ref heap_type) = self.heaptype_ext {
            heap_type.qualname.read().clone().into()
        } else {
            // For static types, return the name
            vm.ctx.new_str(self.name().deref()).into()
        }
    }

    #[pygetset(setter)]
    fn set___qualname__(&self, value: PySetterValue, vm: &VirtualMachine) -> PyResult<()> {
        // TODO: we should replace heaptype flag check to immutable flag check
        if !self.slots.flags.has_feature(PyTypeFlags::HEAPTYPE) {
            return Err(vm.new_type_error(format!(
                "cannot set '__qualname__' attribute of immutable type '{}'",
                self.name()
            )));
        };
        let value = value.ok_or_else(|| {
            vm.new_type_error(format!(
                "cannot delete '__qualname__' attribute of immutable type '{}'",
                self.name()
            ))
        })?;

        let str_value = downcast_qualname(value, vm)?;

        let heap_type = self
            .heaptype_ext
            .as_ref()
            .expect("HEAPTYPE should have heaptype_ext");

        // Use std::mem::replace to swap the new value in and get the old value out,
        // then drop the old value after releasing the lock
        let _old_qualname = {
            let mut qualname_guard = heap_type.qualname.write();
            std::mem::replace(&mut *qualname_guard, str_value)
        };
        // old_qualname is dropped here, outside the lock scope

        Ok(())
    }

    #[pygetset]
    fn __annotations__(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        if !self.slots.flags.has_feature(PyTypeFlags::HEAPTYPE) {
            return Err(vm.new_attribute_error(format!(
                "type object '{}' has no attribute '__annotations__'",
                self.name()
            )));
        }

        let __annotations__ = identifier!(vm, __annotations__);
        let annotations = self.attributes.read().get(__annotations__).cloned();

        let annotations = if let Some(annotations) = annotations {
            annotations
        } else {
            let annotations: PyObjectRef = vm.ctx.new_dict().into();
            let removed = self
                .attributes
                .write()
                .insert(__annotations__, annotations.clone());
            debug_assert!(removed.is_none());
            annotations
        };
        Ok(annotations)
    }

    #[pygetset(setter)]
    fn set___annotations__(&self, value: Option<PyObjectRef>, vm: &VirtualMachine) -> PyResult<()> {
        if self.slots.flags.has_feature(PyTypeFlags::IMMUTABLETYPE) {
            return Err(vm.new_type_error(format!(
                "cannot set '__annotations__' attribute of immutable type '{}'",
                self.name()
            )));
        }

        let __annotations__ = identifier!(vm, __annotations__);
        if let Some(value) = value {
            self.attributes.write().insert(__annotations__, value);
        } else {
            self.attributes
                .read()
                .get(__annotations__)
                .cloned()
                .ok_or_else(|| {
                    vm.new_attribute_error(format!(
                        "'{}' object has no attribute '__annotations__'",
                        self.name()
                    ))
                })?;
        }

        Ok(())
    }

    #[pygetset]
    pub fn __module__(&self, vm: &VirtualMachine) -> PyObjectRef {
        self.attributes
            .read()
            .get(identifier!(vm, __module__))
            .cloned()
            // We need to exclude this method from going into recursion:
            .and_then(|found| {
                if found.fast_isinstance(vm.ctx.types.getset_type) {
                    None
                } else {
                    Some(found)
                }
            })
            .unwrap_or_else(|| vm.ctx.new_str(ascii!("builtins")).into())
    }

    #[pygetset(setter)]
    fn set___module__(&self, value: PyObjectRef, vm: &VirtualMachine) {
        self.attributes
            .write()
            .insert(identifier!(vm, __module__), value);
    }

    #[pyclassmethod]
    fn __prepare__(
        _cls: PyTypeRef,
        _name: OptionalArg<PyObjectRef>,
        _bases: OptionalArg<PyObjectRef>,
        _kwargs: KwArgs,
        vm: &VirtualMachine,
    ) -> PyDictRef {
        vm.ctx.new_dict()
    }

    #[pymethod]
    fn __subclasses__(&self) -> PyList {
        let mut subclasses = self.subclasses.write();
        subclasses.retain(|x| x.upgrade().is_some());
        PyList::from(
            subclasses
                .iter()
                .map(|x| x.upgrade().unwrap())
                .collect::<Vec<_>>(),
        )
    }

    #[pymethod]
    pub fn __ror__(zelf: PyObjectRef, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        or_(other, zelf, vm)
    }

    #[pymethod]
    pub fn __or__(zelf: PyObjectRef, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        or_(zelf, other, vm)
    }

    #[pygetset]
    fn __dict__(zelf: PyRef<Self>) -> PyMappingProxy {
        PyMappingProxy::from(zelf)
    }

    #[pygetset(setter)]
    fn set___dict__(&self, _value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        Err(vm.new_not_implemented_error(
            "Setting __dict__ attribute on a type isn't yet implemented",
        ))
    }

    fn check_set_special_type_attr(
        &self,
        _value: &PyObject,
        name: &PyStrInterned,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        if self.slots.flags.has_feature(PyTypeFlags::IMMUTABLETYPE) {
            return Err(vm.new_type_error(format!(
                "cannot set '{}' attribute of immutable type '{}'",
                name,
                self.slot_name()
            )));
        }
        Ok(())
    }

    #[pygetset(setter)]
    fn set___name__(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.check_set_special_type_attr(&value, identifier!(vm, __name__), vm)?;
        let name = value.downcast::<PyStr>().map_err(|value| {
            vm.new_type_error(format!(
                "can only assign string to {}.__name__, not '{}'",
                self.slot_name(),
                value.class().slot_name(),
            ))
        })?;
        if name.as_bytes().contains(&0) {
            return Err(vm.new_value_error("type name must not contain null characters"));
        }

        // Use std::mem::replace to swap the new value in and get the old value out,
        // then drop the old value after releasing the lock (similar to CPython's Py_SETREF)
        let _old_name = {
            let mut name_guard = self.heaptype_ext.as_ref().unwrap().name.write();
            std::mem::replace(&mut *name_guard, name)
        };
        // old_name is dropped here, outside the lock scope

        Ok(())
    }

    #[pygetset]
    fn __text_signature__(&self) -> Option<String> {
        self.slots
            .doc
            .and_then(|doc| get_text_signature_from_internal_doc(&self.name(), doc))
            .map(|signature| signature.to_string())
    }

    #[pygetset]
    fn __type_params__(&self, vm: &VirtualMachine) -> PyTupleRef {
        let attrs = self.attributes.read();
        let key = identifier!(vm, __type_params__);
        if let Some(params) = attrs.get(&key) {
            if let Ok(tuple) = params.clone().downcast::<PyTuple>() {
                return tuple;
            }
        }
        // Return empty tuple if not found or not a tuple
        vm.ctx.empty_tuple.clone()
    }

    #[pygetset(setter)]
    fn set___type_params__(
        &self,
        value: PySetterValue<PyTupleRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        match value {
            PySetterValue::Assign(ref val) => {
                let key = identifier!(vm, __type_params__);
                self.check_set_special_type_attr(val.as_ref(), key, vm)?;
                let mut attrs = self.attributes.write();
                attrs.insert(key, val.clone().into());
            }
            PySetterValue::Delete => {
                // For delete, we still need to check if the type is immutable
                if self.slots.flags.has_feature(PyTypeFlags::IMMUTABLETYPE) {
                    return Err(vm.new_type_error(format!(
                        "cannot delete '__type_params__' attribute of immutable type '{}'",
                        self.slot_name()
                    )));
                }
                let mut attrs = self.attributes.write();
                let key = identifier!(vm, __type_params__);
                attrs.shift_remove(&key);
            }
        }
        Ok(())
    }
}

impl Constructor for PyType {
    type Args = FuncArgs;

    fn py_new(metatype: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        vm_trace!("type.__new__ {:?}", args);

        let is_type_type = metatype.is(vm.ctx.types.type_type);
        if is_type_type && args.args.len() == 1 && args.kwargs.is_empty() {
            return Ok(args.args[0].class().to_owned().into());
        }

        if args.args.len() != 3 {
            return Err(vm.new_type_error(if is_type_type {
                "type() takes 1 or 3 arguments".to_owned()
            } else {
                format!(
                    "type.__new__() takes exactly 3 arguments ({} given)",
                    args.args.len()
                )
            }));
        }

        let (name, bases, dict, kwargs): (PyStrRef, PyTupleRef, PyDictRef, KwArgs) =
            args.clone().bind(vm)?;

        if name.as_bytes().contains(&0) {
            return Err(vm.new_value_error("type name must not contain null characters"));
        }

        let (metatype, base, bases, base_is_type) = if bases.is_empty() {
            let base = vm.ctx.types.object_type.to_owned();
            (metatype, base.clone(), vec![base], false)
        } else {
            let bases = bases
                .iter()
                .map(|obj| {
                    obj.clone().downcast::<Self>().or_else(|obj| {
                        if vm
                            .get_attribute_opt(obj, identifier!(vm, __mro_entries__))?
                            .is_some()
                        {
                            Err(vm.new_type_error(
                                "type() doesn't support MRO entry resolution; \
                                 use types.new_class()",
                            ))
                        } else {
                            Err(vm.new_type_error("bases must be types"))
                        }
                    })
                })
                .collect::<PyResult<Vec<_>>>()?;

            // Search the bases for the proper metatype to deal with this:
            let winner = calculate_meta_class(metatype.clone(), &bases, vm)?;
            let metatype = if !winner.is(&metatype) {
                if let Some(ref slot_new) = winner.slots.new.load() {
                    // Pass it to the winner
                    return slot_new(winner, args, vm);
                }
                winner
            } else {
                metatype
            };

            let base = best_base(&bases, vm)?;
            let base_is_type = base.is(vm.ctx.types.type_type);

            (metatype, base.to_owned(), bases, base_is_type)
        };

        let qualname = dict
            .pop_item(identifier!(vm, __qualname__).as_object(), vm)?
            .map(|obj| downcast_qualname(obj, vm))
            .transpose()?
            .unwrap_or_else(|| {
                // If __qualname__ is not provided, we can use the name as default
                name.clone()
            });
        let mut attributes = dict.to_attributes(vm);

        if let Some(f) = attributes.get_mut(identifier!(vm, __init_subclass__)) {
            if f.class().is(vm.ctx.types.function_type) {
                *f = PyClassMethod::from(f.clone()).into_pyobject(vm);
            }
        }

        if let Some(f) = attributes.get_mut(identifier!(vm, __class_getitem__)) {
            if f.class().is(vm.ctx.types.function_type) {
                *f = PyClassMethod::from(f.clone()).into_pyobject(vm);
            }
        }

        if let Some(current_frame) = vm.current_frame() {
            let entry = attributes.entry(identifier!(vm, __module__));
            if matches!(entry, Entry::Vacant(_)) {
                let module_name = vm.unwrap_or_none(
                    current_frame
                        .globals
                        .get_item_opt(identifier!(vm, __name__), vm)?,
                );
                entry.or_insert(module_name);
            }
        }

        if attributes.get(identifier!(vm, __eq__)).is_some()
            && attributes.get(identifier!(vm, __hash__)).is_none()
        {
            // if __eq__ exists but __hash__ doesn't, overwrite it with None so it doesn't inherit the default hash
            // https://docs.python.org/3/reference/datamodel.html#object.__hash__
            attributes.insert(identifier!(vm, __hash__), vm.ctx.none.clone().into());
        }

        let (heaptype_slots, add_dict): (Option<PyRef<PyTuple<PyStrRef>>>, bool) =
            if let Some(x) = attributes.get(identifier!(vm, __slots__)) {
                let slots = if x.class().is(vm.ctx.types.str_type) {
                    let x = unsafe { x.downcast_unchecked_ref::<PyStr>() };
                    PyTuple::new_ref_typed(vec![x.to_owned()], &vm.ctx)
                } else {
                    let iter = x.get_iter(vm)?;
                    let elements = {
                        let mut elements = Vec::new();
                        while let PyIterReturn::Return(element) = iter.next(vm)? {
                            elements.push(element);
                        }
                        elements
                    };
                    let tuple = elements.into_pytuple(vm);
                    tuple.try_into_typed(vm)?
                };

                // Check if __dict__ is in slots
                let dict_name = "__dict__";
                let has_dict = slots.iter().any(|s| s.as_str() == dict_name);

                // Filter out __dict__ from slots
                let filtered_slots = if has_dict {
                    let filtered: Vec<PyStrRef> = slots
                        .iter()
                        .filter(|s| s.as_str() != dict_name)
                        .cloned()
                        .collect();
                    PyTuple::new_ref_typed(filtered, &vm.ctx)
                } else {
                    slots
                };

                (Some(filtered_slots), has_dict)
            } else {
                (None, false)
            };

        // FIXME: this is a temporary fix. multi bases with multiple slots will break object
        let base_member_count = bases
            .iter()
            .map(|base| base.slots.member_count)
            .max()
            .unwrap();
        let heaptype_member_count = heaptype_slots.as_ref().map(|x| x.len()).unwrap_or(0);
        let member_count: usize = base_member_count + heaptype_member_count;

        let mut flags = PyTypeFlags::heap_type_flags();
        // Add HAS_DICT and MANAGED_DICT if:
        // 1. __slots__ is not defined, OR
        // 2. __dict__ is in __slots__
        if heaptype_slots.is_none() || add_dict {
            flags |= PyTypeFlags::HAS_DICT | PyTypeFlags::MANAGED_DICT;
        }

        let (slots, heaptype_ext) = {
            let slots = PyTypeSlots {
                flags,
                member_count,
                ..PyTypeSlots::heap_default()
            };
            let heaptype_ext = HeapTypeExt {
                name: PyRwLock::new(name),
                qualname: PyRwLock::new(qualname),
                slots: heaptype_slots.clone(),
                sequence_methods: PySequenceMethods::default(),
                mapping_methods: PyMappingMethods::default(),
            };
            (slots, heaptype_ext)
        };

        let typ = Self::new_heap_inner(
            base,
            bases,
            attributes,
            slots,
            heaptype_ext,
            metatype,
            &vm.ctx,
        )
        .map_err(|e| vm.new_type_error(e))?;

        if let Some(ref slots) = heaptype_slots {
            let mut offset = base_member_count;
            for member in slots.as_slice() {
                let member_def = PyMemberDef {
                    name: member.to_string(),
                    kind: MemberKind::ObjectEx,
                    getter: MemberGetter::Offset(offset),
                    setter: MemberSetter::Offset(offset),
                    doc: None,
                };
                let member_descriptor: PyRef<PyMemberDescriptor> =
                    vm.ctx.new_pyref(PyMemberDescriptor {
                        common: PyDescriptorOwned {
                            typ: typ.clone(),
                            name: vm.ctx.intern_str(member.as_str()),
                            qualname: PyRwLock::new(None),
                        },
                        member: member_def,
                    });

                let attr_name = vm.ctx.intern_str(member.to_string());
                if !typ.has_attr(attr_name) {
                    typ.set_attr(attr_name, member_descriptor.into());
                }

                offset += 1;
            }
        }

        if let Some(cell) = typ.attributes.write().get(identifier!(vm, __classcell__)) {
            let cell = PyCellRef::try_from_object(vm, cell.clone()).map_err(|_| {
                vm.new_type_error(format!(
                    "__classcell__ must be a nonlocal cell, not {}",
                    cell.class().name()
                ))
            })?;
            cell.set(Some(typ.clone().into()));
        };

        // All *classes* should have a dict. Exceptions are *instances* of
        // classes that define __slots__ and instances of built-in classes
        // (with exceptions, e.g function)
        // Also, type subclasses don't need their own __dict__ descriptor
        // since they inherit it from type

        // Add __dict__ descriptor after type creation to ensure correct __objclass__
        if !base_is_type {
            let __dict__ = identifier!(vm, __dict__);
            if !typ.attributes.read().contains_key(&__dict__) {
                unsafe {
                    let descriptor =
                        vm.ctx
                            .new_getset("__dict__", &typ, subtype_get_dict, subtype_set_dict);
                    typ.attributes.write().insert(__dict__, descriptor.into());
                }
            }
        }

        // avoid deadlock
        let attributes = typ
            .attributes
            .read()
            .iter()
            .filter_map(|(name, obj)| {
                vm.get_method(obj.clone(), identifier!(vm, __set_name__))
                    .map(|res| res.map(|meth| (obj.clone(), name.to_owned(), meth)))
            })
            .collect::<PyResult<Vec<_>>>()?;
        for (obj, name, set_name) in attributes {
            set_name.call((typ.clone(), name), vm).map_err(|e| {
                let err = vm.new_runtime_error(format!(
                    "Error calling __set_name__ on '{}' instance {} in '{}'",
                    obj.class().name(),
                    name,
                    typ.name()
                ));
                err.set___cause__(Some(e));
                err
            })?;
        }

        if let Some(init_subclass) = typ.get_super_attr(identifier!(vm, __init_subclass__)) {
            let init_subclass = vm
                .call_get_descriptor_specific(&init_subclass, None, Some(typ.clone().into()))
                .unwrap_or(Ok(init_subclass))?;
            init_subclass.call(kwargs, vm)?;
        };

        Ok(typ.into())
    }
}

const SIGNATURE_END_MARKER: &str = ")\n--\n\n";
fn get_signature(doc: &str) -> Option<&str> {
    doc.find(SIGNATURE_END_MARKER).map(|index| &doc[..=index])
}

fn find_signature<'a>(name: &str, doc: &'a str) -> Option<&'a str> {
    let name = name.rsplit('.').next().unwrap();
    let doc = doc.strip_prefix(name)?;
    doc.starts_with('(').then_some(doc)
}

pub(crate) fn get_text_signature_from_internal_doc<'a>(
    name: &str,
    internal_doc: &'a str,
) -> Option<&'a str> {
    find_signature(name, internal_doc).and_then(get_signature)
}

// _PyType_GetDocFromInternalDoc in CPython
fn get_doc_from_internal_doc<'a>(name: &str, internal_doc: &'a str) -> &'a str {
    // Similar to CPython's _PyType_DocWithoutSignature
    // If the doc starts with the type name and a '(', it's a signature
    if let Some(doc_without_sig) = find_signature(name, internal_doc) {
        // Find where the signature ends
        if let Some(sig_end_pos) = doc_without_sig.find(SIGNATURE_END_MARKER) {
            let after_sig = &doc_without_sig[sig_end_pos + SIGNATURE_END_MARKER.len()..];
            // Return the documentation after the signature, or empty string if none
            return after_sig;
        }
    }
    // If no signature found, return the whole doc
    internal_doc
}

impl GetAttr for PyType {
    fn getattro(zelf: &Py<Self>, name_str: &Py<PyStr>, vm: &VirtualMachine) -> PyResult {
        #[cold]
        fn attribute_error(
            zelf: &Py<PyType>,
            name: &str,
            vm: &VirtualMachine,
        ) -> PyBaseExceptionRef {
            vm.new_attribute_error(format!(
                "type object '{}' has no attribute '{}'",
                zelf.slot_name(),
                name,
            ))
        }

        let Some(name) = vm.ctx.interned_str(name_str) else {
            return Err(attribute_error(zelf, name_str.as_str(), vm));
        };
        vm_trace!("type.__getattribute__({:?}, {:?})", zelf, name);
        let mcl = zelf.class();
        let mcl_attr = mcl.get_attr(name);

        if let Some(ref attr) = mcl_attr {
            let attr_class = attr.class();
            let has_descr_set = attr_class
                .mro_find_map(|cls| cls.slots.descr_set.load())
                .is_some();
            if has_descr_set {
                let descr_get = attr_class.mro_find_map(|cls| cls.slots.descr_get.load());
                if let Some(descr_get) = descr_get {
                    let mcl = mcl.to_owned().into();
                    return descr_get(attr.clone(), Some(zelf.to_owned().into()), Some(mcl), vm);
                }
            }
        }

        let zelf_attr = zelf.get_attr(name);

        if let Some(attr) = zelf_attr {
            let descr_get = attr.class().mro_find_map(|cls| cls.slots.descr_get.load());
            if let Some(descr_get) = descr_get {
                descr_get(attr, None, Some(zelf.to_owned().into()), vm)
            } else {
                Ok(attr)
            }
        } else if let Some(attr) = mcl_attr {
            vm.call_if_get_descriptor(&attr, zelf.to_owned().into())
        } else {
            Err(attribute_error(zelf, name_str.as_str(), vm))
        }
    }
}

#[pyclass]
impl Py<PyType> {
    #[pygetset(name = "__mro__")]
    fn get_mro(&self) -> PyTuple {
        let elements: Vec<PyObjectRef> = self.mro_map_collect(|x| x.as_object().to_owned());
        PyTuple::new_unchecked(elements.into_boxed_slice())
    }

    #[pygetset]
    fn __doc__(&self, vm: &VirtualMachine) -> PyResult {
        // Similar to CPython's type_get_doc
        // For non-heap types (static types), check if there's an internal doc
        if !self.slots.flags.has_feature(PyTypeFlags::HEAPTYPE) {
            if let Some(internal_doc) = self.slots.doc {
                // Process internal doc, removing signature if present
                let doc_str = get_doc_from_internal_doc(&self.name(), internal_doc);
                return Ok(vm.ctx.new_str(doc_str).into());
            }
        }

        // Check if there's a __doc__ in the type's dict
        if let Some(doc_attr) = self.get_attr(vm.ctx.intern_str("__doc__")) {
            // If it's a descriptor, call its __get__ method
            let descr_get = doc_attr
                .class()
                .mro_find_map(|cls| cls.slots.descr_get.load());
            if let Some(descr_get) = descr_get {
                descr_get(doc_attr, None, Some(self.to_owned().into()), vm)
            } else {
                Ok(doc_attr)
            }
        } else {
            Ok(vm.ctx.none())
        }
    }

    #[pygetset(setter)]
    fn set___doc__(&self, value: PySetterValue, vm: &VirtualMachine) -> PyResult<()> {
        // Similar to CPython's type_set_doc
        let value = value.ok_or_else(|| {
            vm.new_type_error(format!(
                "cannot delete '__doc__' attribute of type '{}'",
                self.name()
            ))
        })?;

        // Check if we can set this special type attribute
        self.check_set_special_type_attr(&value, identifier!(vm, __doc__), vm)?;

        // Set the __doc__ in the type's dict
        self.attributes
            .write()
            .insert(identifier!(vm, __doc__), value);

        Ok(())
    }

    #[pymethod]
    fn __dir__(&self) -> PyList {
        let attributes: Vec<PyObjectRef> = self
            .get_attributes()
            .into_iter()
            .map(|(k, _)| k.to_object())
            .collect();
        PyList::from(attributes)
    }

    #[pymethod]
    fn __instancecheck__(&self, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        // Use real_is_instance to avoid infinite recursion, matching CPython's behavior
        obj.real_is_instance(self.as_object(), vm)
    }

    #[pymethod]
    fn __subclasscheck__(&self, subclass: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        // Use real_is_subclass to avoid going through __subclasscheck__ recursion
        // This matches CPython's type___subclasscheck___impl which calls _PyObject_RealIsSubclass
        subclass.real_is_subclass(self.as_object(), vm)
    }

    #[pyclassmethod]
    fn __subclasshook__(_args: FuncArgs, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.not_implemented()
    }

    #[pymethod]
    fn mro(&self) -> Vec<PyObjectRef> {
        self.mro_map_collect(|cls| cls.to_owned().into())
    }
}

impl SetAttr for PyType {
    fn setattro(
        zelf: &Py<Self>,
        attr_name: &Py<PyStr>,
        value: PySetterValue,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        // TODO: pass PyRefExact instead of &str
        let attr_name = vm.ctx.intern_str(attr_name.as_str());
        if let Some(attr) = zelf.get_class_attr(attr_name) {
            let descr_set = attr.class().mro_find_map(|cls| cls.slots.descr_set.load());
            if let Some(descriptor) = descr_set {
                return descriptor(&attr, zelf.to_owned().into(), value, vm);
            }
        }
        let assign = value.is_assign();

        if let PySetterValue::Assign(value) = value {
            zelf.attributes.write().insert(attr_name, value);
        } else {
            let prev_value = zelf.attributes.write().shift_remove(attr_name); // TODO: swap_remove applicable?
            if prev_value.is_none() {
                return Err(vm.new_attribute_error(format!(
                    "type object '{}' has no attribute '{}'",
                    zelf.name(),
                    attr_name.as_str(),
                )));
            }
        }
        if attr_name.as_str().starts_with("__") && attr_name.as_str().ends_with("__") {
            if assign {
                zelf.update_slot::<true>(attr_name, &vm.ctx);
            } else {
                zelf.update_slot::<false>(attr_name, &vm.ctx);
            }
        }
        Ok(())
    }
}

impl Callable for PyType {
    type Args = FuncArgs;
    fn call(zelf: &Py<Self>, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        vm_trace!("type_call: {:?}", zelf);
        let obj = call_slot_new(zelf.to_owned(), zelf.to_owned(), args.clone(), vm)?;

        if (zelf.is(vm.ctx.types.type_type) && args.kwargs.is_empty()) || !obj.fast_isinstance(zelf)
        {
            return Ok(obj);
        }

        let init = obj.class().mro_find_map(|cls| cls.slots.init.load());
        if let Some(init_method) = init {
            init_method(obj.clone(), args, vm)?;
        }
        Ok(obj)
    }
}

impl AsNumber for PyType {
    fn as_number() -> &'static PyNumberMethods {
        static AS_NUMBER: PyNumberMethods = PyNumberMethods {
            or: Some(|a, b, vm| or_(a.to_owned(), b.to_owned(), vm).to_pyresult(vm)),
            ..PyNumberMethods::NOT_IMPLEMENTED
        };
        &AS_NUMBER
    }
}

impl Representable for PyType {
    #[inline]
    fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
        let module = zelf.__module__(vm);
        let module = module.downcast_ref::<PyStr>().map(|m| m.as_str());

        let repr = match module {
            Some(module) if module != "builtins" => {
                let name = zelf.name();
                format!(
                    "<class '{}.{}'>",
                    module,
                    zelf.__qualname__(vm)
                        .downcast_ref::<PyStr>()
                        .map(|n| n.as_str())
                        .unwrap_or_else(|| &name)
                )
            }
            _ => format!("<class '{}'>", zelf.slot_name()),
        };
        Ok(repr)
    }
}

// = get_builtin_base_with_dict
fn get_builtin_base_with_dict(typ: &Py<PyType>, vm: &VirtualMachine) -> Option<PyTypeRef> {
    let mut current = Some(typ.to_owned());
    while let Some(t) = current {
        // In CPython: type->tp_dictoffset != 0 && !(type->tp_flags & Py_TPFLAGS_HEAPTYPE)
        // Special case: type itself is a builtin with dict support
        if t.is(vm.ctx.types.type_type) {
            return Some(t);
        }
        // We check HAS_DICT flag (equivalent to tp_dictoffset != 0) and HEAPTYPE
        if t.slots.flags.contains(PyTypeFlags::HAS_DICT)
            && !t.slots.flags.contains(PyTypeFlags::HEAPTYPE)
        {
            return Some(t);
        }
        current = t.__base__();
    }
    None
}

// = get_dict_descriptor
fn get_dict_descriptor(base: &Py<PyType>, vm: &VirtualMachine) -> Option<PyObjectRef> {
    let dict_attr = identifier!(vm, __dict__);
    // Use _PyType_Lookup (which is lookup_ref in RustPython)
    base.lookup_ref(dict_attr, vm)
}

// = raise_dict_descr_error
fn raise_dict_descriptor_error(obj: &PyObject, vm: &VirtualMachine) -> PyBaseExceptionRef {
    vm.new_type_error(format!(
        "this __dict__ descriptor does not support '{}' objects",
        obj.class().name()
    ))
}

fn subtype_get_dict(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    let base = get_builtin_base_with_dict(obj.class(), vm);

    if let Some(base_type) = base {
        if let Some(descr) = get_dict_descriptor(&base_type, vm) {
            // Call the descriptor's tp_descr_get
            vm.call_get_descriptor(&descr, obj.clone())
                .unwrap_or_else(|| Err(raise_dict_descriptor_error(&obj, vm)))
        } else {
            Err(raise_dict_descriptor_error(&obj, vm))
        }
    } else {
        // PyObject_GenericGetDict
        object::object_get_dict(obj, vm).map(Into::into)
    }
}

// = subtype_setdict
fn subtype_set_dict(obj: PyObjectRef, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
    let base = get_builtin_base_with_dict(obj.class(), vm);

    if let Some(base_type) = base {
        if let Some(descr) = get_dict_descriptor(&base_type, vm) {
            // Call the descriptor's tp_descr_set
            let descr_set = descr
                .class()
                .mro_find_map(|cls| cls.slots.descr_set.load())
                .ok_or_else(|| raise_dict_descriptor_error(&obj, vm))?;
            descr_set(&descr, obj, PySetterValue::Assign(value), vm)
        } else {
            Err(raise_dict_descriptor_error(&obj, vm))
        }
    } else {
        // PyObject_GenericSetDict
        object::object_set_dict(obj, value.try_into_value(vm)?, vm)?;
        Ok(())
    }
}

/*
 * The magical type type
 */

pub(crate) fn init(ctx: &Context) {
    PyType::extend_class(ctx, ctx.types.type_type);
}

pub(crate) fn call_slot_new(
    typ: PyTypeRef,
    subtype: PyTypeRef,
    args: FuncArgs,
    vm: &VirtualMachine,
) -> PyResult {
    let slot_new = typ
        .deref()
        .mro_find_map(|cls| cls.slots.new.load())
        .expect("Should be able to find a new slot somewhere in the mro");
    slot_new(subtype, args, vm)
}

pub(super) fn or_(zelf: PyObjectRef, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
    if !union_::is_unionable(zelf.clone(), vm) || !union_::is_unionable(other.clone(), vm) {
        return vm.ctx.not_implemented();
    }

    let tuple = PyTuple::new_ref(vec![zelf, other], &vm.ctx);
    union_::make_union(&tuple, vm)
}

fn take_next_base(bases: &mut [Vec<PyTypeRef>]) -> Option<PyTypeRef> {
    for base in bases.iter() {
        let head = base[0].clone();
        if !bases.iter().any(|x| x[1..].iter().any(|x| x.is(&head))) {
            // Remove from other heads.
            for item in bases.iter_mut() {
                if item[0].is(&head) {
                    item.remove(0);
                }
            }

            return Some(head);
        }
    }

    None
}

fn linearise_mro(mut bases: Vec<Vec<PyTypeRef>>) -> Result<Vec<PyTypeRef>, String> {
    vm_trace!("Linearise MRO: {:?}", bases);
    // Python requires that the class direct bases are kept in the same order.
    // This is called local precedence ordering.
    // This means we must verify that for classes A(), B(A) we must reject C(A, B) even though this
    // algorithm will allow the mro ordering of [C, B, A, object].
    // To verify this, we make sure non of the direct bases are in the mro of bases after them.
    for (i, base_mro) in bases.iter().enumerate() {
        let base = &base_mro[0]; // MROs cannot be empty.
        for later_mro in &bases[i + 1..] {
            // We start at index 1 to skip direct bases.
            // This will not catch duplicate bases, but such a thing is already tested for.
            if later_mro[1..].iter().any(|cls| cls.is(base)) {
                return Err(
                    "Unable to find mro order which keeps local precedence ordering".to_owned(),
                );
            }
        }
    }

    let mut result = vec![];
    while !bases.is_empty() {
        let head = take_next_base(&mut bases).ok_or_else(|| {
            // Take the head class of each class here. Now that we have reached the problematic bases.
            // Because this failed, we assume the lists cannot be empty.
            format!(
                "Cannot create a consistent method resolution order (MRO) for bases {}",
                bases.iter().map(|x| x.first().unwrap()).format(", ")
            )
        })?;

        result.push(head);

        bases.retain(|x| !x.is_empty());
    }
    Ok(result)
}

fn calculate_meta_class(
    metatype: PyTypeRef,
    bases: &[PyTypeRef],
    vm: &VirtualMachine,
) -> PyResult<PyTypeRef> {
    // = _PyType_CalculateMetaclass
    let mut winner = metatype;
    for base in bases {
        let base_type = base.class();
        if winner.fast_issubclass(base_type) {
            continue;
        } else if base_type.fast_issubclass(&winner) {
            winner = base_type.to_owned();
            continue;
        }

        return Err(vm.new_type_error(
            "metaclass conflict: the metaclass of a derived class must be a (non-strict) subclass \
             of the metaclasses of all its bases",
        ));
    }
    Ok(winner)
}

fn solid_base<'a>(typ: &'a Py<PyType>, vm: &VirtualMachine) -> &'a Py<PyType> {
    let base = if let Some(base) = &typ.base {
        solid_base(base, vm)
    } else {
        vm.ctx.types.object_type
    };

    // TODO: requires itemsize comparison too
    if typ.__basicsize__() != base.__basicsize__() {
        typ
    } else {
        base
    }
}

fn best_base<'a>(bases: &'a [PyTypeRef], vm: &VirtualMachine) -> PyResult<&'a Py<PyType>> {
    let mut base: Option<&Py<PyType>> = None;
    let mut winner: Option<&Py<PyType>> = None;

    for base_i in bases {
        // if !base_i.fast_issubclass(vm.ctx.types.type_type) {
        //     println!("base_i type : {}", base_i.name());
        //     return Err(vm.new_type_error("best must be types".into()));
        // }

        if !base_i.slots.flags.has_feature(PyTypeFlags::BASETYPE) {
            return Err(vm.new_type_error(format!(
                "type '{}' is not an acceptable base type",
                base_i.slot_name()
            )));
        }

        let candidate = solid_base(base_i, vm);
        if winner.is_none() {
            winner = Some(candidate);
            base = Some(base_i.deref());
        } else if winner.unwrap().fast_issubclass(candidate) {
            // Do nothing
        } else if candidate.fast_issubclass(winner.unwrap()) {
            winner = Some(candidate);
            base = Some(base_i.deref());
        } else {
            return Err(vm.new_type_error("multiple bases have instance layout conflict"));
        }
    }

    debug_assert!(base.is_some());
    Ok(base.unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map_ids(obj: Result<Vec<PyTypeRef>, String>) -> Result<Vec<usize>, String> {
        Ok(obj?.into_iter().map(|x| x.get_id()).collect())
    }

    #[test]
    fn test_linearise() {
        let context = Context::genesis();
        let object = context.types.object_type.to_owned();
        let type_type = context.types.type_type.to_owned();

        let a = PyType::new_heap(
            "A",
            vec![object.clone()],
            PyAttributes::default(),
            Default::default(),
            type_type.clone(),
            context,
        )
        .unwrap();
        let b = PyType::new_heap(
            "B",
            vec![object.clone()],
            PyAttributes::default(),
            Default::default(),
            type_type,
            context,
        )
        .unwrap();

        assert_eq!(
            map_ids(linearise_mro(vec![
                vec![object.clone()],
                vec![object.clone()]
            ])),
            map_ids(Ok(vec![object.clone()]))
        );
        assert_eq!(
            map_ids(linearise_mro(vec![
                vec![a.clone(), object.clone()],
                vec![b.clone(), object.clone()],
            ])),
            map_ids(Ok(vec![a, b, object]))
        );
    }
}

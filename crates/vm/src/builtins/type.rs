use super::{
    PyClassMethod, PyDictRef, PyList, PyStaticMethod, PyStr, PyStrInterned, PyStrRef, PyTupleRef,
    PyWeak, mappingproxy::PyMappingProxy, object, union_,
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
    object::{Traverse, TraverseFn},
    protocol::{PyIterReturn, PyNumberMethods},
    types::{
        AsNumber, Callable, Constructor, GetAttr, Initializer, PyTypeFlags, PyTypeSlots,
        Representable, SLOT_DEFS, SetAttr, TypeDataRef, TypeDataRefMut, TypeDataSlot,
    },
};
use core::{any::Any, borrow::Borrow, ops::Deref, pin::Pin, ptr::NonNull};
use indexmap::{IndexMap, map::Entry};
use itertools::Itertools;
use num_traits::ToPrimitive;
use std::collections::HashSet;

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
        // mro contains self as mro[0], so skip traversing to avoid circular reference
        // self.mro.traverse(tracer_fn);
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
    pub type_data: PyRwLock<Option<TypeDataSlot>>,
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

impl core::fmt::Display for PyType {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Display::fmt(&self.name(), f)
    }
}

impl core::fmt::Debug for PyType {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
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
        base: &Py<PyType>,
        ctx: &Context,
    ) -> Result<PyRef<Self>, String> {
        Self::new_heap(
            name,
            vec![base.to_owned()],
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
        mut slots: PyTypeSlots,
        metaclass: PyRef<Self>,
        ctx: &Context,
    ) -> Result<PyRef<Self>, String> {
        // TODO: ensure clean slot name
        // assert_eq!(slots.name.borrow(), "");

        // Set HEAPTYPE flag for heap-allocated types
        slots.flags |= PyTypeFlags::HEAPTYPE;

        let name = ctx.new_str(name);
        let heaptype_ext = HeapTypeExt {
            name: PyRwLock::new(name.clone()),
            qualname: PyRwLock::new(name),
            slots: None,
            type_data: PyRwLock::new(None),
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

    /// Inherit SEQUENCE and MAPPING flags from base classes
    /// Check all bases in order and inherit the first SEQUENCE or MAPPING flag found
    fn inherit_patma_flags(slots: &mut PyTypeSlots, bases: &[PyRef<Self>]) {
        const COLLECTION_FLAGS: PyTypeFlags = PyTypeFlags::from_bits_truncate(
            PyTypeFlags::SEQUENCE.bits() | PyTypeFlags::MAPPING.bits(),
        );

        // If flags are already set, don't override
        if slots.flags.intersects(COLLECTION_FLAGS) {
            return;
        }

        // Check each base in order and inherit the first collection flag found
        for base in bases {
            let base_flags = base.slots.flags & COLLECTION_FLAGS;
            if !base_flags.is_empty() {
                slots.flags |= base_flags;
                return;
            }
        }
    }

    /// Check for __abc_tpflags__ and set the appropriate flags
    /// This checks in attrs and all base classes for __abc_tpflags__
    fn check_abc_tpflags(
        slots: &mut PyTypeSlots,
        attrs: &PyAttributes,
        bases: &[PyRef<Self>],
        ctx: &Context,
    ) {
        const COLLECTION_FLAGS: PyTypeFlags = PyTypeFlags::from_bits_truncate(
            PyTypeFlags::SEQUENCE.bits() | PyTypeFlags::MAPPING.bits(),
        );

        // Don't override if flags are already set
        if slots.flags.intersects(COLLECTION_FLAGS) {
            return;
        }

        // First check in our own attributes
        let abc_tpflags_name = ctx.intern_str("__abc_tpflags__");
        if let Some(abc_tpflags_obj) = attrs.get(abc_tpflags_name)
            && let Some(int_obj) = abc_tpflags_obj.downcast_ref::<crate::builtins::int::PyInt>()
        {
            let flags_val = int_obj.as_bigint().to_i64().unwrap_or(0);
            let abc_flags = PyTypeFlags::from_bits_truncate(flags_val as u64);
            slots.flags |= abc_flags & COLLECTION_FLAGS;
            return;
        }

        // Then check in base classes
        for base in bases {
            if let Some(abc_tpflags_obj) = base.find_name_in_mro(abc_tpflags_name)
                && let Some(int_obj) = abc_tpflags_obj.downcast_ref::<crate::builtins::int::PyInt>()
            {
                let flags_val = int_obj.as_bigint().to_i64().unwrap_or(0);
                let abc_flags = PyTypeFlags::from_bits_truncate(flags_val as u64);
                slots.flags |= abc_flags & COLLECTION_FLAGS;
                return;
            }
        }
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

        // Inherit HAS_DICT from any base in MRO that has it
        // (not just the first base, as any base with __dict__ means subclass needs it too)
        if mro
            .iter()
            .any(|b| b.slots.flags.has_feature(PyTypeFlags::HAS_DICT))
        {
            slots.flags |= PyTypeFlags::HAS_DICT
        }

        // Inherit SEQUENCE and MAPPING flags from base classes
        Self::inherit_patma_flags(&mut slots, &bases);

        // Check for __abc_tpflags__ from ABCMeta (for collections.abc.Sequence, Mapping, etc.)
        Self::check_abc_tpflags(&mut slots, &attrs, &bases, ctx);

        if slots.basicsize == 0 {
            slots.basicsize = base.slots.basicsize;
        }

        Self::inherit_readonly_slots(&mut slots, &base);

        if let Some(qualname) = attrs.get(identifier!(ctx, __qualname__))
            && !qualname.fast_isinstance(ctx.types.str_type)
        {
            return Err(format!(
                "type __qualname__ must be a str, not {}",
                qualname.class().name()
            ));
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
        new_type.mro.write().insert(0, new_type.clone());

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

        // Inherit SEQUENCE and MAPPING flags from base class
        // For static types, we only have a single base
        Self::inherit_patma_flags(&mut slots, core::slice::from_ref(&base));

        if slots.basicsize == 0 {
            slots.basicsize = base.slots.basicsize;
        }

        Self::inherit_readonly_slots(&mut slots, &base);

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
        new_type.mro.write().insert(0, new_type.clone());

        // Note: inherit_slots is called in PyClassImpl::init_class after
        // slots are fully initialized by make_slots()

        Self::set_new(&new_type.slots, &new_type.base);

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
        // Inherit slots from MRO (mro[0] is self, so skip it)
        let mro: Vec<_> = self.mro.read()[1..].to_vec();
        for base in mro.iter() {
            self.inherit_slots(base);
        }

        // Wire dunder methods to slots
        #[allow(clippy::mutable_key_type)]
        let mut slot_name_set = std::collections::HashSet::new();

        // mro[0] is self, so skip it; self.attributes is checked separately below
        for cls in self.mro.read()[1..].iter() {
            for &name in cls.attributes.read().keys() {
                if name.as_bytes().starts_with(b"__") && name.as_bytes().ends_with(b"__") {
                    slot_name_set.insert(name);
                }
            }
        }
        for &name in self.attributes.read().keys() {
            if name.as_bytes().starts_with(b"__") && name.as_bytes().ends_with(b"__") {
                slot_name_set.insert(name);
            }
        }
        // Sort for deterministic iteration order (important for slot processing)
        let mut slot_names: Vec<_> = slot_name_set.into_iter().collect();
        slot_names.sort_by_key(|name| name.as_str());
        for attr_name in slot_names {
            self.update_slot::<true>(attr_name, ctx);
        }

        Self::set_new(&self.slots, &self.base);
    }

    fn set_new(slots: &PyTypeSlots, base: &Option<PyTypeRef>) {
        if slots.flags.contains(PyTypeFlags::DISALLOW_INSTANTIATION) {
            slots.new.store(None)
        } else if slots.new.load().is_none() {
            slots.new.store(
                base.as_ref()
                    .map(|base| base.slots.new.load())
                    .unwrap_or(None),
            )
        }
    }

    /// Inherit readonly slots from base type at creation time.
    /// These slots are not AtomicCell and must be set before the type is used.
    fn inherit_readonly_slots(slots: &mut PyTypeSlots, base: &Self) {
        if slots.as_buffer.is_none() {
            slots.as_buffer = base.slots.as_buffer;
        }
    }

    /// Inherit slots from base type. inherit_slots
    pub(crate) fn inherit_slots(&self, base: &Self) {
        // Use SLOT_DEFS to iterate all slots
        // Note: as_buffer is handled in inherit_readonly_slots (not AtomicCell)
        for def in SLOT_DEFS {
            def.accessor.copyslot_if_none(self, base);
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
        // mro[0] is self, so we just iterate through the entire MRO
        for cls in self.mro.read().iter() {
            if let Some(value) = cls.attributes.read().get(name) {
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
        self.mro.read()[1..]
            .iter()
            .find_map(|class| class.attributes.read().get(attr_name).cloned())
    }

    // This is the internal has_attr implementation for fast lookup on a class.
    pub fn has_attr(&self, attr_name: &'static PyStrInterned) -> bool {
        self.attributes.read().contains_key(attr_name)
            || self.mro.read()[1..]
                .iter()
                .any(|c| c.attributes.read().contains_key(attr_name))
    }

    pub fn get_attributes(&self) -> PyAttributes {
        // Gather all members here:
        let mut attributes = PyAttributes::default();

        // mro[0] is self, so we iterate through the entire MRO in reverse
        for bc in self.mro.read().iter().map(|cls| -> &Self { cls }).rev() {
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

    // Type Data Slot API - CPython's PyObject_GetTypeData equivalent

    /// Initialize type data for this type. Can only be called once.
    /// Returns an error if the type is not a heap type or if data is already initialized.
    pub fn init_type_data<T: Any + Send + Sync + 'static>(&self, data: T) -> Result<(), String> {
        let ext = self
            .heaptype_ext
            .as_ref()
            .ok_or_else(|| "Cannot set type data on non-heap types".to_string())?;

        let mut type_data = ext.type_data.write();
        if type_data.is_some() {
            return Err("Type data already initialized".to_string());
        }
        *type_data = Some(TypeDataSlot::new(data));
        Ok(())
    }

    /// Get a read guard to the type data.
    /// Returns None if the type is not a heap type, has no data, or the data type doesn't match.
    pub fn get_type_data<T: Any + 'static>(&self) -> Option<TypeDataRef<'_, T>> {
        self.heaptype_ext
            .as_ref()
            .and_then(|ext| TypeDataRef::try_new(ext.type_data.read()))
    }

    /// Get a write guard to the type data.
    /// Returns None if the type is not a heap type, has no data, or the data type doesn't match.
    pub fn get_type_data_mut<T: Any + 'static>(&self) -> Option<TypeDataRefMut<'_, T>> {
        self.heaptype_ext
            .as_ref()
            .and_then(|ext| TypeDataRefMut::try_new(ext.type_data.write()))
    }

    /// Check if this type has type data of the given type.
    pub fn has_type_data<T: Any + 'static>(&self) -> bool {
        self.heaptype_ext.as_ref().is_some_and(|ext| {
            ext.type_data
                .read()
                .as_ref()
                .is_some_and(|slot| slot.get::<T>().is_some())
        })
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
        self.as_object().is(cls.borrow()) || self.mro.read()[1..].iter().any(|c| c.is(cls.borrow()))
    }

    pub fn mro_map_collect<F, R>(&self, f: F) -> Vec<R>
    where
        F: Fn(&Self) -> R,
    {
        self.mro.read().iter().map(|x| x.deref()).map(f).collect()
    }

    pub fn mro_collect(&self) -> Vec<PyRef<PyType>> {
        self.mro
            .read()
            .iter()
            .map(|x| x.deref())
            .map(|x| x.to_owned())
            .collect()
    }

    pub fn iter_base_chain(&self) -> impl Iterator<Item = &Self> {
        core::iter::successors(Some(self), |cls| cls.base.as_deref())
    }

    pub fn extend_methods(&'static self, method_defs: &'static [PyMethodDef], ctx: &Context) {
        for method_def in method_defs {
            let method = method_def.to_proper_method(self, ctx);
            self.set_attr(ctx.intern_str(method_def.name), method);
        }
    }
}

#[pyclass(
    with(
        Py,
        Constructor,
        Initializer,
        GetAttr,
        SetAttr,
        Callable,
        AsNumber,
        Representable
    ),
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
            let mut mro =
                PyType::resolve_mro(&cls.bases.read()).map_err(|msg| vm.new_type_error(msg))?;
            // Preserve self (mro[0]) when updating MRO
            mro.insert(0, cls.mro.read()[0].to_owned());
            *cls.mro.write() = mro;
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
    fn __basicsize__(&self) -> usize {
        crate::object::SIZEOF_PYOBJECT_HEAD + self.slots.basicsize
    }

    #[pygetset]
    fn __itemsize__(&self) -> usize {
        self.slots.itemsize
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
            core::mem::replace(&mut *qualname_guard, str_value)
        };
        // old_qualname is dropped here, outside the lock scope

        Ok(())
    }

    #[pygetset]
    fn __annotate__(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        if !self.slots.flags.has_feature(PyTypeFlags::HEAPTYPE) {
            return Err(vm.new_attribute_error(format!(
                "type object '{}' has no attribute '__annotate__'",
                self.name()
            )));
        }

        let mut attrs = self.attributes.write();
        // First try __annotate__, in case that's been set explicitly
        if let Some(annotate) = attrs.get(identifier!(vm, __annotate__)).cloned() {
            return Ok(annotate);
        }
        // Then try __annotate_func__
        if let Some(annotate) = attrs.get(identifier!(vm, __annotate_func__)).cloned() {
            // TODO: Apply descriptor tp_descr_get if needed
            return Ok(annotate);
        }
        // Set __annotate_func__ = None and return None
        let none = vm.ctx.none();
        attrs.insert(identifier!(vm, __annotate_func__), none.clone());
        Ok(none)
    }

    #[pygetset(setter)]
    fn set___annotate__(&self, value: Option<PyObjectRef>, vm: &VirtualMachine) -> PyResult<()> {
        if value.is_none() {
            return Err(vm.new_type_error("cannot delete __annotate__ attribute".to_owned()));
        }
        let value = value.unwrap();

        if self.slots.flags.has_feature(PyTypeFlags::IMMUTABLETYPE) {
            return Err(vm.new_type_error(format!(
                "cannot set '__annotate__' attribute of immutable type '{}'",
                self.name()
            )));
        }

        if !vm.is_none(&value) && !value.is_callable() {
            return Err(vm.new_type_error("__annotate__ must be callable or None".to_owned()));
        }

        let mut attrs = self.attributes.write();
        // Store to __annotate_func__
        attrs.insert(identifier!(vm, __annotate_func__), value.clone());
        // Always clear cached annotations when __annotate__ is updated
        attrs.swap_remove(identifier!(vm, __annotations_cache__));

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

        // First try __annotations__ (e.g. for "from __future__ import annotations")
        let attrs = self.attributes.read();
        if let Some(annotations) = attrs.get(identifier!(vm, __annotations__)).cloned() {
            return Ok(annotations);
        }
        // Then try __annotations_cache__
        if let Some(annotations) = attrs.get(identifier!(vm, __annotations_cache__)).cloned() {
            return Ok(annotations);
        }
        drop(attrs);

        // Get __annotate__ and call it if callable
        let annotate = self.__annotate__(vm)?;
        let annotations = if annotate.is_callable() {
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

        // Cache the result in __annotations_cache__
        self.attributes
            .write()
            .insert(identifier!(vm, __annotations_cache__), annotations.clone());
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

        let mut attrs = self.attributes.write();
        // conditional update based on __annotations__ presence
        let has_annotations = attrs.contains_key(identifier!(vm, __annotations__));

        if has_annotations {
            // If __annotations__ is in dict, update it
            if let Some(value) = value {
                attrs.insert(identifier!(vm, __annotations__), value);
            } else if attrs
                .swap_remove(identifier!(vm, __annotations__))
                .is_none()
            {
                return Err(vm.new_attribute_error("__annotations__".to_owned()));
            }
            // Also clear __annotations_cache__
            attrs.swap_remove(identifier!(vm, __annotations_cache__));
        } else {
            // Otherwise update only __annotations_cache__
            if let Some(value) = value {
                attrs.insert(identifier!(vm, __annotations_cache__), value);
            } else if attrs
                .swap_remove(identifier!(vm, __annotations_cache__))
                .is_none()
            {
                return Err(vm.new_attribute_error("__annotations__".to_owned()));
            }
        }
        // Always clear __annotate_func__ and __annotate__
        attrs.swap_remove(identifier!(vm, __annotate_func__));
        attrs.swap_remove(identifier!(vm, __annotate__));

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
    fn set___module__(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.check_set_special_type_attr(identifier!(vm, __module__), vm)?;
        self.attributes
            .write()
            .insert(identifier!(vm, __module__), value);
        Ok(())
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

    pub fn __ror__(zelf: PyObjectRef, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        or_(other, zelf, vm)
    }

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
        self.check_set_special_type_attr(identifier!(vm, __name__), vm)?;
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
        name.ensure_valid_utf8(vm)?;

        // Use std::mem::replace to swap the new value in and get the old value out,
        // then drop the old value after releasing the lock (similar to CPython's Py_SETREF)
        let _old_name = {
            let mut name_guard = self.heaptype_ext.as_ref().unwrap().name.write();
            core::mem::replace(&mut *name_guard, name)
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
        if let Some(params) = attrs.get(&key)
            && let Ok(tuple) = params.clone().downcast::<PyTuple>()
        {
            return tuple;
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
                self.check_set_special_type_attr(key, vm)?;
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

    fn slot_new(metatype: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
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
        name.ensure_valid_utf8(vm)?;

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

        // Check __doc__ for surrogates - raises UnicodeEncodeError during type creation
        if let Some(doc) = attributes.get(identifier!(vm, __doc__))
            && let Some(doc_str) = doc.downcast_ref::<PyStr>()
        {
            doc_str.ensure_valid_utf8(vm)?;
        }

        if let Some(f) = attributes.get_mut(identifier!(vm, __init_subclass__))
            && f.class().is(vm.ctx.types.function_type)
        {
            *f = PyClassMethod::from(f.clone()).into_pyobject(vm);
        }

        if let Some(f) = attributes.get_mut(identifier!(vm, __class_getitem__))
            && f.class().is(vm.ctx.types.function_type)
        {
            *f = PyClassMethod::from(f.clone()).into_pyobject(vm);
        }

        if let Some(f) = attributes.get_mut(identifier!(vm, __new__))
            && f.class().is(vm.ctx.types.function_type)
        {
            *f = PyStaticMethod::from(f.clone()).into_pyobject(vm);
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

        let (heaptype_slots, add_dict): (Option<PyRef<PyTuple<PyStrRef>>>, bool) = if let Some(x) =
            attributes.get(identifier!(vm, __slots__))
        {
            // Check if __slots__ is bytes - not allowed
            if x.class().is(vm.ctx.types.bytes_type) {
                return Err(
                    vm.new_type_error("__slots__ items must be strings, not 'bytes'".to_owned())
                );
            }

            let slots = if x.class().is(vm.ctx.types.str_type) {
                let x = unsafe { x.downcast_unchecked_ref::<PyStr>() };
                PyTuple::new_ref_typed(vec![x.to_owned()], &vm.ctx)
            } else {
                let iter = x.get_iter(vm)?;
                let elements = {
                    let mut elements = Vec::new();
                    while let PyIterReturn::Return(element) = iter.next(vm)? {
                        // Check if any slot item is bytes
                        if element.class().is(vm.ctx.types.bytes_type) {
                            return Err(vm.new_type_error(
                                "__slots__ items must be strings, not 'bytes'".to_owned(),
                            ));
                        }
                        elements.push(element);
                    }
                    elements
                };
                let tuple = elements.into_pytuple(vm);
                tuple.try_into_typed(vm)?
            };

            // Check if base has itemsize > 0 - can't add arbitrary slots to variable-size types
            // Types like int, bytes, tuple have itemsize > 0 and don't allow custom slots
            // But types like weakref.ref have itemsize = 0 and DO allow slots
            let has_custom_slots = slots
                .iter()
                .any(|s| s.as_str() != "__dict__" && s.as_str() != "__weakref__");
            if has_custom_slots && base.slots.itemsize > 0 {
                return Err(vm.new_type_error(format!(
                    "nonempty __slots__ not supported for subtype of '{}'",
                    base.name()
                )));
            }

            // Validate slot names and track duplicates
            let mut seen_dict = false;
            let mut seen_weakref = false;
            for slot in slots.iter() {
                // Use isidentifier for validation (handles Unicode properly)
                if !slot.isidentifier() {
                    return Err(vm.new_type_error("__slots__ must be identifiers".to_owned()));
                }

                let slot_name = slot.as_str();

                // Check for duplicate __dict__
                if slot_name == "__dict__" {
                    if seen_dict {
                        return Err(vm.new_type_error(
                            "__dict__ slot disallowed: we already got one".to_owned(),
                        ));
                    }
                    seen_dict = true;
                }

                // Check for duplicate __weakref__
                if slot_name == "__weakref__" {
                    if seen_weakref {
                        return Err(vm.new_type_error(
                            "__weakref__ slot disallowed: we already got one".to_owned(),
                        ));
                    }
                    seen_weakref = true;
                }

                // Check if slot name conflicts with class attributes
                if attributes.contains_key(vm.ctx.intern_str(slot_name)) {
                    return Err(vm.new_value_error(format!(
                        "'{}' in __slots__ conflicts with a class variable",
                        slot_name
                    )));
                }
            }

            // Check if base class already has __dict__ - can't redefine it
            if seen_dict && base.slots.flags.has_feature(PyTypeFlags::HAS_DICT) {
                return Err(
                    vm.new_type_error("__dict__ slot disallowed: we already got one".to_owned())
                );
            }

            // Check if base class already has __weakref__ - can't redefine it
            // A base has weakref support if:
            // 1. It's a heap type without explicit __slots__ (automatic weakref), OR
            // 2. It's a heap type with __weakref__ in its __slots__
            if seen_weakref {
                let base_has_weakref = if let Some(ref ext) = base.heaptype_ext {
                    match &ext.slots {
                        // Heap type without __slots__ - has automatic weakref
                        None => true,
                        // Heap type with __slots__ - check if __weakref__ is in slots
                        Some(base_slots) => base_slots.iter().any(|s| s.as_str() == "__weakref__"),
                    }
                } else {
                    // Builtin type - check if it has __weakref__ descriptor
                    let weakref_name = vm.ctx.intern_str("__weakref__");
                    base.attributes.read().contains_key(weakref_name)
                };

                if base_has_weakref {
                    return Err(vm.new_type_error(
                        "__weakref__ slot disallowed: we already got one".to_owned(),
                    ));
                }
            }

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

        // Check if we may add dict
        // We can only add a dict if the primary base class doesn't already have one
        // In CPython, this checks tp_dictoffset == 0
        let may_add_dict = !base.slots.flags.has_feature(PyTypeFlags::HAS_DICT);

        // Add HAS_DICT and MANAGED_DICT if:
        // 1. __slots__ is not defined AND base doesn't have dict, OR
        // 2. __dict__ is in __slots__
        if (heaptype_slots.is_none() && may_add_dict) || add_dict {
            flags |= PyTypeFlags::HAS_DICT | PyTypeFlags::MANAGED_DICT;
        }

        let (slots, heaptype_ext) = {
            let slots = PyTypeSlots {
                flags,
                member_count,
                itemsize: base.slots.itemsize,
                ..PyTypeSlots::heap_default()
            };
            let heaptype_ext = HeapTypeExt {
                name: PyRwLock::new(name),
                qualname: PyRwLock::new(qualname),
                slots: heaptype_slots.clone(),
                type_data: PyRwLock::new(None),
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
            let class_name = typ.name().to_string();
            for member in slots.as_slice() {
                // Apply name mangling for private attributes (__x -> _ClassName__x)
                let mangled_name = mangle_name(&class_name, member.as_str());
                let member_def = PyMemberDef {
                    name: mangled_name.clone(),
                    kind: MemberKind::ObjectEx,
                    getter: MemberGetter::Offset(offset),
                    setter: MemberSetter::Offset(offset),
                    doc: None,
                };
                let attr_name = vm.ctx.intern_str(mangled_name.as_str());
                let member_descriptor: PyRef<PyMemberDescriptor> =
                    vm.ctx.new_pyref(PyMemberDescriptor {
                        common: PyDescriptorOwned {
                            typ: typ.clone(),
                            name: attr_name,
                            qualname: PyRwLock::new(None),
                        },
                        member: member_def,
                    });
                // __slots__ attributes always get a member descriptor
                // (this overrides any inherited attribute from MRO)
                typ.set_attr(attr_name, member_descriptor.into());
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
        // Only add if:
        // 1. base is not type (type subclasses inherit __dict__ from type)
        // 2. the class has HAS_DICT flag (i.e., __slots__ was not defined or __dict__ is in __slots__)
        // 3. no base class in MRO already provides __dict__ descriptor
        if !base_is_type && typ.slots.flags.has_feature(PyTypeFlags::HAS_DICT) {
            let __dict__ = identifier!(vm, __dict__);
            let has_inherited_dict = typ
                .mro
                .read()
                .iter()
                .any(|base| base.attributes.read().contains_key(&__dict__));
            if !typ.attributes.read().contains_key(&__dict__) && !has_inherited_dict {
                unsafe {
                    let descriptor =
                        vm.ctx
                            .new_getset("__dict__", &typ, subtype_get_dict, subtype_set_dict);
                    typ.attributes.write().insert(__dict__, descriptor.into());
                }
            }
        }

        // Set __doc__ to None if not already present in the type's dict
        // This matches CPython's behavior in type_dict_set_doc (typeobject.c)
        // which ensures every type has a __doc__ entry in its dict
        {
            let __doc__ = identifier!(vm, __doc__);
            if !typ.attributes.read().contains_key(&__doc__) {
                typ.attributes.write().insert(__doc__, vm.ctx.none());
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

    fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
        unimplemented!("use slot_new")
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

impl Initializer for PyType {
    type Args = FuncArgs;

    // type_init
    fn slot_init(_zelf: PyObjectRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
        // type.__init__() takes 1 or 3 arguments
        if args.args.len() == 1 && !args.kwargs.is_empty() {
            return Err(vm.new_type_error("type.__init__() takes no keyword arguments".to_owned()));
        }
        if args.args.len() != 1 && args.args.len() != 3 {
            return Err(vm.new_type_error("type.__init__() takes 1 or 3 arguments".to_owned()));
        }
        Ok(())
    }

    fn init(_zelf: PyRef<Self>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<()> {
        unreachable!("slot_init is defined")
    }
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
            let has_descr_set = attr_class.slots.descr_set.load().is_some();
            if has_descr_set {
                let descr_get = attr_class.slots.descr_get.load();
                if let Some(descr_get) = descr_get {
                    let mcl = mcl.to_owned().into();
                    return descr_get(attr.clone(), Some(zelf.to_owned().into()), Some(mcl), vm);
                }
            }
        }

        let zelf_attr = zelf.get_attr(name);

        if let Some(attr) = zelf_attr {
            let descr_get = attr.class().slots.descr_get.load();
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
    #[pygetset]
    fn __mro__(&self) -> PyTuple {
        let elements: Vec<PyObjectRef> = self.mro_map_collect(|x| x.as_object().to_owned());
        PyTuple::new_unchecked(elements.into_boxed_slice())
    }

    #[pygetset]
    fn __doc__(&self, vm: &VirtualMachine) -> PyResult {
        // Similar to CPython's type_get_doc
        // For non-heap types (static types), check if there's an internal doc
        if !self.slots.flags.has_feature(PyTypeFlags::HEAPTYPE)
            && let Some(internal_doc) = self.slots.doc
        {
            // Process internal doc, removing signature if present
            let doc_str = get_doc_from_internal_doc(&self.name(), internal_doc);
            return Ok(vm.ctx.new_str(doc_str).into());
        }

        // Check if there's a __doc__ in THIS type's dict only (not MRO)
        // CPython returns None if __doc__ is not in the type's own dict
        if let Some(doc_attr) = self.get_direct_attr(vm.ctx.intern_str("__doc__")) {
            // If it's a descriptor, call its __get__ method
            let descr_get = doc_attr.class().slots.descr_get.load();
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
        self.check_set_special_type_attr(identifier!(vm, __doc__), vm)?;

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
            let descr_set = attr.class().slots.descr_set.load();
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

        if zelf.is(vm.ctx.types.type_type) {
            let num_args = args.args.len();
            if num_args == 1 && args.kwargs.is_empty() {
                return Ok(args.args[0].obj_type());
            }
            if num_args != 3 {
                return Err(vm.new_type_error("type() takes 1 or 3 arguments".to_owned()));
            }
        }

        let obj = if let Some(slot_new) = zelf.slots.new.load() {
            slot_new(zelf.to_owned(), args.clone(), vm)?
        } else {
            return Err(vm.new_type_error(format!("cannot create '{}' instances", zelf.slots.name)));
        };

        if !obj.class().fast_issubclass(zelf) {
            return Ok(obj);
        }

        if let Some(init_method) = obj.class().slots.init.load() {
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
                .slots
                .descr_set
                .load()
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
    // Check DISALLOW_INSTANTIATION flag on subtype (the type being instantiated)
    if subtype
        .slots
        .flags
        .has_feature(PyTypeFlags::DISALLOW_INSTANTIATION)
    {
        return Err(vm.new_type_error(format!("cannot create '{}' instances", subtype.slot_name())));
    }

    // "is not safe" check (tp_new_wrapper logic)
    // Check that the user doesn't do something silly and unsafe like
    // object.__new__(dict). To do this, we check that the most derived base
    // that's not a heap type is this type.
    let mut staticbase = subtype.clone();
    while staticbase.slots.flags.has_feature(PyTypeFlags::HEAPTYPE) {
        if let Some(base) = staticbase.base.as_ref() {
            staticbase = base.clone();
        } else {
            break;
        }
    }

    // Check if staticbase's tp_new differs from typ's tp_new
    let typ_new = typ.slots.new.load();
    let staticbase_new = staticbase.slots.new.load();
    if typ_new.map(|f| f as usize) != staticbase_new.map(|f| f as usize) {
        return Err(vm.new_type_error(format!(
            "{}.__new__({}) is not safe, use {}.__new__()",
            typ.slot_name(),
            subtype.slot_name(),
            staticbase.slot_name()
        )));
    }

    let slot_new = typ
        .slots
        .new
        .load()
        .expect("Should be able to find a new slot somewhere in the mro");
    slot_new(subtype, args, vm)
}

pub(crate) fn or_(zelf: PyObjectRef, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
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

        // First try fast_issubclass for PyType instances
        if winner.fast_issubclass(base_type) {
            continue;
        } else if base_type.fast_issubclass(&winner) {
            winner = base_type.to_owned();
            continue;
        }

        // If fast_issubclass didn't work, fall back to general is_subclass
        // This handles cases where metaclasses are not PyType subclasses
        let winner_is_subclass = winner.as_object().is_subclass(base_type.as_object(), vm)?;
        if winner_is_subclass {
            continue;
        }

        let base_type_is_subclass = base_type.as_object().is_subclass(winner.as_object(), vm)?;
        if base_type_is_subclass {
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

/// Returns true if the two types have different instance layouts.
fn shape_differs(t1: &Py<PyType>, t2: &Py<PyType>) -> bool {
    t1.__basicsize__() != t2.__basicsize__() || t1.slots.itemsize != t2.slots.itemsize
}

fn solid_base<'a>(typ: &'a Py<PyType>, vm: &VirtualMachine) -> &'a Py<PyType> {
    let base = if let Some(base) = &typ.base {
        solid_base(base, vm)
    } else {
        vm.ctx.types.object_type
    };

    if shape_differs(typ, base) { typ } else { base }
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

/// Apply Python name mangling for private attributes.
/// `__x` becomes `_ClassName__x` if inside a class.
fn mangle_name(class_name: &str, name: &str) -> String {
    // Only mangle names starting with __ and not ending with __
    if !name.starts_with("__") || name.ends_with("__") || name.contains('.') {
        return name.to_string();
    }
    // Strip leading underscores from class name
    let class_name = class_name.trim_start_matches('_');
    format!("_{}{}", class_name, name)
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

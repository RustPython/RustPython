use super::{
    mappingproxy::PyMappingProxy, object, union_, PyClassMethod, PyDictRef, PyList, PyStr,
    PyStrInterned, PyStrRef, PyTuple, PyTupleRef, PyWeak,
};
use crate::{
    builtins::{
        descriptor::{
            MemberGetter, MemberKind, MemberSetter, PyDescriptorOwned, PyMemberDef,
            PyMemberDescriptor,
        },
        function::PyCellRef,
        tuple::{IntoPyTuple, PyTupleTyped},
        PyBaseExceptionRef,
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
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject,
    VirtualMachine,
};
use indexmap::{map::Entry, IndexMap};
use itertools::Itertools;
use std::{borrow::Borrow, collections::HashSet, fmt, ops::Deref, pin::Pin, ptr::NonNull};

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
    fn traverse(&self, tracer_fn: &mut crate::object::TraverseFn) {
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

pub struct HeapTypeExt {
    pub name: PyRwLock<PyStrRef>,
    pub slots: Option<PyTupleTyped<PyStrRef>>,
    pub sequence_methods: PySequenceMethods,
    pub mapping_methods: PyMappingMethods,
}

pub struct PointerSlot<T>(NonNull<T>);

impl<T> PointerSlot<T> {
    pub unsafe fn borrow_static(&self) -> &'static T {
        self.0.as_ref()
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
    fn traverse(&self, tracer_fn: &mut TraverseFn) {
        self.values().for_each(|v| v.traverse(tracer_fn));
    }
}

impl fmt::Display for PyType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.name(), f)
    }
}

impl fmt::Debug for PyType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "[PyType {}]", &self.name())
    }
}

impl PyPayload for PyType {
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.type_type
    }
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
            name: PyRwLock::new(name),
            slots: None,
            sequence_methods: PySequenceMethods::default(),
            mapping_methods: PyMappingMethods::default(),
        };
        let base = bases[0].clone();

        Self::new_heap_inner(base, bases, attrs, slots, heaptype_ext, metaclass, ctx)
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
            PyType {
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
            PyType {
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

    // This is used for class initialisation where the vm is not yet available.
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
            .chain(self.mro.read().iter().map(|cls| -> &PyType { cls }))
            .rev()
        {
            for (name, value) in bc.attributes.read().iter() {
                attributes.insert(name.to_owned(), value.clone());
            }
        }

        attributes
    }

    // bound method for every type
    pub(crate) fn __new__(zelf: PyRef<PyType>, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
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

    pub fn slot_name(&self) -> BorrowedValue<str> {
        self.name_inner(
            |name| name.into(),
            |ext| PyRwLockReadGuard::map(ext.name.read(), |name| name.as_str()).into(),
        )
    }

    pub fn name(&self) -> BorrowedValue<str> {
        self.name_inner(
            |name| name.rsplit_once('.').map_or(name, |(_, name)| name).into(),
            |ext| PyRwLockReadGuard::map(ext.name.read(), |name| name.as_str()).into(),
        )
    }
}

impl Py<PyType> {
    /// Determines if `subclass` is actually a subclass of `cls`, this doesn't call __subclasscheck__,
    /// so only use this if `cls` is known to have not overridden the base __subclasscheck__ magic
    /// method.
    pub fn fast_issubclass(&self, cls: &impl Borrow<crate::PyObject>) -> bool {
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
        // try std::intrinsics::likely once it is stablized
        if let Some(r) = f(self) {
            Some(r)
        } else {
            self.mro.read().iter().find_map(|cls| f(cls))
        }
    }

    pub fn iter_base_chain(&self) -> impl Iterator<Item = &Py<PyType>> {
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
    #[pygetset(magic)]
    fn bases(&self, vm: &VirtualMachine) -> PyTupleRef {
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
        // Rather than correctly reinitializing the class, we are skipping a few steps for now
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
                let subclass: &PyType = subclass.payload().unwrap();
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

    #[pygetset(magic)]
    fn base(&self) -> Option<PyTypeRef> {
        self.base.clone()
    }

    #[pygetset(magic)]
    fn flags(&self) -> u64 {
        self.slots.flags.bits()
    }

    #[pygetset(magic)]
    fn basicsize(&self) -> usize {
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

    #[pygetset(magic)]
    pub fn qualname(&self, vm: &VirtualMachine) -> PyObjectRef {
        self.attributes
            .read()
            .get(identifier!(vm, __qualname__))
            .cloned()
            // We need to exclude this method from going into recursion:
            .and_then(|found| {
                if found.fast_isinstance(vm.ctx.types.getset_type) {
                    None
                } else {
                    Some(found)
                }
            })
            .unwrap_or_else(|| vm.ctx.new_str(self.name().deref()).into())
    }

    #[pygetset(magic, setter)]
    fn set_qualname(&self, value: PySetterValue, vm: &VirtualMachine) -> PyResult<()> {
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
        if !value.class().fast_issubclass(vm.ctx.types.str_type) {
            return Err(vm.new_type_error(format!(
                "can only assign string to {}.__qualname__, not '{}'",
                self.name(),
                value.class().name()
            )));
        }
        self.attributes
            .write()
            .insert(identifier!(vm, __qualname__), value);
        Ok(())
    }

    #[pygetset(magic)]
    fn annotations(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
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

    #[pygetset(magic, setter)]
    fn set_annotations(&self, value: Option<PyObjectRef>, vm: &VirtualMachine) -> PyResult<()> {
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

    #[pygetset(magic)]
    pub fn module(&self, vm: &VirtualMachine) -> PyObjectRef {
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

    #[pygetset(magic, setter)]
    fn set_module(&self, value: PyObjectRef, vm: &VirtualMachine) {
        self.attributes
            .write()
            .insert(identifier!(vm, __module__), value);
    }

    #[pyclassmethod(magic)]
    fn prepare(
        _cls: PyTypeRef,
        _name: OptionalArg<PyObjectRef>,
        _bases: OptionalArg<PyObjectRef>,
        _kwargs: KwArgs,
        vm: &VirtualMachine,
    ) -> PyDictRef {
        vm.ctx.new_dict()
    }

    #[pymethod(magic)]
    fn subclasses(&self) -> PyList {
        let mut subclasses = self.subclasses.write();
        subclasses.retain(|x| x.upgrade().is_some());
        PyList::from(
            subclasses
                .iter()
                .map(|x| x.upgrade().unwrap())
                .collect::<Vec<_>>(),
        )
    }

    #[pymethod(magic)]
    pub fn ror(zelf: PyObjectRef, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        or_(other, zelf, vm)
    }

    #[pymethod(magic)]
    pub fn or(zelf: PyObjectRef, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        or_(zelf, other, vm)
    }

    #[pygetset(magic)]
    fn dict(zelf: PyRef<Self>) -> PyMappingProxy {
        PyMappingProxy::from(zelf)
    }

    #[pygetset(magic, setter)]
    fn set_dict(&self, _value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        Err(vm.new_not_implemented_error(
            "Setting __dict__ attribute on a type isn't yet implemented".to_owned(),
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

    #[pygetset(magic, setter)]
    fn set_name(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.check_set_special_type_attr(&value, identifier!(vm, __name__), vm)?;
        let name = value.downcast::<PyStr>().map_err(|value| {
            vm.new_type_error(format!(
                "can only assign string to {}.__name__, not '{}'",
                self.slot_name(),
                value.class().slot_name(),
            ))
        })?;
        if name.as_str().as_bytes().contains(&0) {
            return Err(vm.new_value_error("type name must not contain null characters".to_owned()));
        }

        *self.heaptype_ext.as_ref().unwrap().name.write() = name;

        Ok(())
    }

    #[pygetset(magic)]
    fn text_signature(&self) -> Option<String> {
        self.slots
            .doc
            .and_then(|doc| get_text_signature_from_internal_doc(&self.name(), doc))
            .map(|signature| signature.to_string())
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

        if name.as_str().as_bytes().contains(&0) {
            return Err(vm.new_value_error("type name must not contain null characters".to_owned()));
        }

        let (metatype, base, bases) = if bases.is_empty() {
            let base = vm.ctx.types.object_type.to_owned();
            (metatype, base.clone(), vec![base])
        } else {
            let bases = bases
                .iter()
                .map(|obj| {
                    obj.clone().downcast::<PyType>().or_else(|obj| {
                        if vm
                            .get_attribute_opt(obj, identifier!(vm, __mro_entries__))?
                            .is_some()
                        {
                            Err(vm.new_type_error(
                                "type() doesn't support MRO entry resolution; \
                                 use types.new_class()"
                                    .to_owned(),
                            ))
                        } else {
                            Err(vm.new_type_error("bases must be types".to_owned()))
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

            (metatype, base.to_owned(), bases)
        };

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

        attributes
            .entry(identifier!(vm, __qualname__))
            .or_insert_with(|| vm.ctx.new_str(name.as_str()).into());

        if attributes.get(identifier!(vm, __eq__)).is_some()
            && attributes.get(identifier!(vm, __hash__)).is_none()
        {
            // if __eq__ exists but __hash__ doesn't, overwrite it with None so it doesn't inherit the default hash
            // https://docs.python.org/3/reference/datamodel.html#object.__hash__
            attributes.insert(identifier!(vm, __hash__), vm.ctx.none.clone().into());
        }

        // All *classes* should have a dict. Exceptions are *instances* of
        // classes that define __slots__ and instances of built-in classes
        // (with exceptions, e.g function)
        let __dict__ = identifier!(vm, __dict__);
        attributes.entry(__dict__).or_insert_with(|| {
            vm.ctx
                .new_getset(
                    "__dict__",
                    vm.ctx.types.object_type,
                    subtype_get_dict,
                    subtype_set_dict,
                )
                .into()
        });

        // TODO: Flags is currently initialized with HAS_DICT. Should be
        // updated when __slots__ are supported (toggling the flag off if
        // a class has __slots__ defined).
        let heaptype_slots: Option<PyTupleTyped<PyStrRef>> =
            if let Some(x) = attributes.get(identifier!(vm, __slots__)) {
                Some(if x.to_owned().class().is(vm.ctx.types.str_type) {
                    PyTupleTyped::<PyStrRef>::try_from_object(
                        vm,
                        vec![x.to_owned()].into_pytuple(vm).into(),
                    )?
                } else {
                    let iter = x.to_owned().get_iter(vm)?;
                    let elements = {
                        let mut elements = Vec::new();
                        while let PyIterReturn::Return(element) = iter.next(vm)? {
                            elements.push(element);
                        }
                        elements
                    };
                    PyTupleTyped::<PyStrRef>::try_from_object(vm, elements.into_pytuple(vm).into())?
                })
            } else {
                None
            };

        // FIXME: this is a temporary fix. multi bases with multiple slots will break object
        let base_member_count = bases
            .iter()
            .map(|base| base.slots.member_count)
            .max()
            .unwrap();
        let heaptype_member_count = heaptype_slots.as_ref().map(|x| x.len()).unwrap_or(0);
        let member_count: usize = base_member_count + heaptype_member_count;

        let flags = PyTypeFlags::heap_type_flags() | PyTypeFlags::HAS_DICT;
        let (slots, heaptype_ext) = {
            let slots = PyTypeSlots {
                member_count,
                flags,
                ..PyTypeSlots::heap_default()
            };
            let heaptype_ext = HeapTypeExt {
                name: PyRwLock::new(name),
                slots: heaptype_slots.to_owned(),
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
                err.set_cause(Some(e));
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

    #[pymethod(magic)]
    fn dir(&self) -> PyList {
        let attributes: Vec<PyObjectRef> = self
            .get_attributes()
            .into_iter()
            .map(|(k, _)| k.to_object())
            .collect();
        PyList::from(attributes)
    }

    #[pymethod(magic)]
    fn instancecheck(&self, obj: PyObjectRef) -> bool {
        obj.fast_isinstance(self)
    }

    #[pymethod(magic)]
    fn subclasscheck(&self, subclass: PyTypeRef) -> bool {
        subclass.fast_issubclass(self)
    }

    #[pyclassmethod(magic)]
    fn subclasshook(_args: FuncArgs, vm: &VirtualMachine) -> PyObjectRef {
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
        let module = zelf.module(vm);
        let module = module.downcast_ref::<PyStr>().map(|m| m.as_str());

        let repr = match module {
            Some(module) if module != "builtins" => {
                let name = zelf.name();
                format!(
                    "<class '{}.{}'>",
                    module,
                    zelf.qualname(vm)
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

fn find_base_dict_descr(cls: &Py<PyType>, vm: &VirtualMachine) -> Option<PyObjectRef> {
    cls.iter_base_chain().skip(1).find_map(|cls| {
        // TODO: should actually be some translation of:
        // cls.slot_dictoffset != 0 && !cls.flags.contains(HEAPTYPE)
        if cls.is(vm.ctx.types.type_type) {
            cls.get_attr(identifier!(vm, __dict__))
        } else {
            None
        }
    })
}

fn subtype_get_dict(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    // TODO: obj.class().as_pyref() need to be supported
    let ret = match find_base_dict_descr(obj.class(), vm) {
        Some(descr) => vm.call_get_descriptor(&descr, obj).unwrap_or_else(|| {
            Err(vm.new_type_error(format!(
                "this __dict__ descriptor does not support '{}' objects",
                descr.class()
            )))
        })?,
        None => object::object_get_dict(obj, vm)?.into(),
    };
    Ok(ret)
}

fn subtype_set_dict(obj: PyObjectRef, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
    let cls = obj.class();
    match find_base_dict_descr(cls, vm) {
        Some(descr) => {
            let descr_set = descr
                .class()
                .mro_find_map(|cls| cls.slots.descr_set.load())
                .ok_or_else(|| {
                    vm.new_type_error(format!(
                        "this __dict__ descriptor does not support '{}' objects",
                        cls.name()
                    ))
                })?;
            descr_set(&descr, obj, PySetterValue::Assign(value), vm)
        }
        None => {
            object::object_set_dict(obj, value.try_into_value(vm)?, vm)?;
            Ok(())
        }
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
             of the metaclasses of all its bases"
                .to_owned(),
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
    if typ.basicsize() != base.basicsize() {
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
                base_i.name()
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
            return Err(
                vm.new_type_error("multiple bases have instance layout conflict".to_string())
            );
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

use super::{
    mappingproxy::PyMappingProxy, object, union_, PyClassMethod, PyDictRef, PyList, PyStaticMethod,
    PyStr, PyStrRef, PyTuple, PyTupleRef, PyWeak,
};
use crate::common::{
    ascii,
    borrow::BorrowedValue,
    lock::{PyRwLock, PyRwLockReadGuard},
};
use crate::{
    class::{PyClassImpl, StaticType},
    function::{FuncArgs, KwArgs, OptionalArg},
    types::{Callable, GetAttr, PyTypeFlags, PyTypeSlots, SetAttr},
    AsObject, Context, Py, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
};
use indexmap::{map::Entry, IndexMap};
use itertools::Itertools;
use std::{borrow::Borrow, collections::HashSet, fmt, ops::Deref};

/// type(object_or_name, bases, dict)
/// type(object) -> the object's type
/// type(name, bases, dict) -> a new type
#[pyclass(module = false, name = "type")]
pub struct PyType {
    pub base: Option<PyTypeRef>,
    pub bases: Vec<PyTypeRef>,
    pub mro: Vec<PyTypeRef>,
    pub subclasses: PyRwLock<Vec<PyRef<PyWeak>>>,
    pub attributes: PyRwLock<PyAttributes>,
    pub slots: PyTypeSlots,
}

pub type PyTypeRef = PyRef<PyType>;

/// For attributes we do not use a dict, but an IndexMap, which is an Hash Table
/// that maintains order and is compatible with the standard HashMap  This is probably
/// faster and only supports strings as keys.
pub type PyAttributes = IndexMap<String, PyObjectRef, ahash::RandomState>;

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
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.type_type
    }
}

impl PyType {
    pub fn new_simple_ref(name: &str, base: &PyTypeRef) -> Result<PyRef<Self>, String> {
        Self::new_ref(
            name,
            vec![base.clone()],
            Default::default(),
            Default::default(),
            Self::static_type().clone(),
        )
    }
    pub fn new_ref(
        name: &str,
        bases: Vec<PyRef<Self>>,
        attrs: PyAttributes,
        slots: PyTypeSlots,
        metaclass: PyRef<Self>,
    ) -> Result<PyRef<Self>, String> {
        Self::new_verbose_ref(name, bases[0].clone(), bases, attrs, slots, metaclass)
    }
    fn new_verbose_ref(
        name: &str,
        base: PyRef<Self>,
        bases: Vec<PyRef<Self>>,
        attrs: PyAttributes,
        mut slots: PyTypeSlots,
        metaclass: PyRef<Self>,
    ) -> Result<PyRef<Self>, String> {
        // Check for duplicates in bases.
        let mut unique_bases = HashSet::new();
        for base in &bases {
            if !unique_bases.insert(base.get_id()) {
                return Err(format!("duplicate base class {}", base.name()));
            }
        }

        let mros = bases
            .iter()
            .map(|x| x.iter_mro().cloned().collect())
            .collect();
        let mro = linearise_mro(mros)?;
        slots.inherits(&mro.iter().map(|t| -> &PyType { &t }).collect::<Vec<_>>());
        debug_assert!(slots.hash.load().is_some(), "{}", name);

        if base.slots.flags.has_feature(PyTypeFlags::HAS_DICT) {
            slots.flags |= PyTypeFlags::HAS_DICT
        }

        *slots.name.get_mut() = Some(String::from(name));

        let new_type = PyRef::new_ref(
            PyType {
                base: Some(base),
                bases,
                mro,
                subclasses: PyRwLock::default(),
                attributes: PyRwLock::new(attrs),
                slots,
            },
            metaclass,
            None,
        );

        for attr_name in new_type.attributes.read().keys() {
            if attr_name.starts_with("__") && attr_name.ends_with("__") {
                new_type.update_slot(attr_name, true);
            }
        }
        let weakref_type = super::PyWeak::static_type();
        for base in &new_type.bases {
            base.subclasses.write().push(
                new_type
                    .as_object()
                    .downgrade_with_weakref_typ_opt(None, weakref_type.clone())
                    .unwrap(),
            );
        }

        Ok(new_type)
    }

    pub fn slot_name(&self) -> String {
        self.slots.name.read().as_ref().unwrap().to_string()
    }

    pub fn iter_mro(&self) -> impl Iterator<Item = &PyType> + DoubleEndedIterator {
        std::iter::once(self).chain(self.mro.iter().map(|cls| -> &PyType { cls }))
    }

    // This is used for class initialisation where the vm is not yet available.
    pub fn set_str_attr<V: Into<PyObjectRef>>(&self, attr_name: &str, value: V) {
        self._set_str_attr(attr_name, value.into())
    }

    fn _set_str_attr(&self, attr_name: &str, value: PyObjectRef) {
        self.attributes.write().insert(attr_name.to_owned(), value);
    }

    /// This is the internal get_attr implementation for fast lookup on a class.
    pub fn get_attr(&self, attr_name: &str) -> Option<PyObjectRef> {
        flame_guard!(format!("class_get_attr({:?})", attr_name));

        self.get_direct_attr(attr_name)
            .or_else(|| self.get_super_attr(attr_name))
    }

    pub fn get_direct_attr(&self, attr_name: &str) -> Option<PyObjectRef> {
        self.attributes.read().get(attr_name).cloned()
    }

    pub fn get_super_attr(&self, attr_name: &str) -> Option<PyObjectRef> {
        self.mro
            .iter()
            .find_map(|class| class.attributes.read().get(attr_name).cloned())
    }

    // This is the internal has_attr implementation for fast lookup on a class.
    pub fn has_attr(&self, attr_name: &str) -> bool {
        self.attributes.read().contains_key(attr_name)
            || self
                .mro
                .iter()
                .any(|c| c.attributes.read().contains_key(attr_name))
    }

    pub fn get_attributes(&self) -> PyAttributes {
        // Gather all members here:
        let mut attributes = PyAttributes::default();

        for bc in self.iter_mro().rev() {
            for (name, value) in bc.attributes.read().iter() {
                attributes.insert(name.to_owned(), value.clone());
            }
        }

        attributes
    }
}

impl PyTypeRef {
    /// Determines if `subclass` is actually a subclass of `cls`, this doesn't call __subclasscheck__,
    /// so only use this if `cls` is known to have not overridden the base __subclasscheck__ magic
    /// method.
    pub fn fast_issubclass(&self, cls: &impl Borrow<crate::PyObject>) -> bool {
        self.as_object().is(cls.borrow()) || self.mro.iter().any(|c| c.is(cls.borrow()))
    }

    pub fn iter_mro(&self) -> impl Iterator<Item = &PyTypeRef> + DoubleEndedIterator {
        std::iter::once(self).chain(self.mro.iter())
    }

    pub fn iter_base_chain(&self) -> impl Iterator<Item = &PyTypeRef> {
        std::iter::successors(Some(self), |cls| cls.base.as_ref())
    }
}

#[pyimpl(with(GetAttr, SetAttr, Callable), flags(BASETYPE))]
impl PyType {
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

    #[pyproperty(name = "__mro__")]
    fn get_mro(zelf: PyRef<Self>) -> PyTuple {
        let elements: Vec<PyObjectRef> =
            zelf.iter_mro().map(|x| x.as_object().to_owned()).collect();
        PyTuple::new_unchecked(elements.into_boxed_slice())
    }

    #[pyproperty(magic)]
    fn bases(&self, vm: &VirtualMachine) -> PyTupleRef {
        vm.ctx.new_tuple(
            self.bases
                .iter()
                .map(|x| x.as_object().to_owned())
                .collect(),
        )
    }

    #[pyproperty(magic)]
    fn base(&self) -> Option<PyTypeRef> {
        self.base.clone()
    }

    #[pyproperty(magic)]
    fn flags(&self) -> u64 {
        self.slots.flags.bits()
    }

    #[pymethod(magic)]
    fn dir(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyList {
        let attributes: Vec<PyObjectRef> = zelf
            .get_attributes()
            .into_iter()
            .map(|(k, _)| vm.ctx.new_str(k).into())
            .collect();
        PyList::from(attributes)
    }

    #[pymethod(magic)]
    fn instancecheck(zelf: PyRef<Self>, obj: PyObjectRef) -> bool {
        obj.fast_isinstance(&zelf)
    }

    #[pymethod(magic)]
    fn subclasscheck(zelf: PyRef<Self>, subclass: PyTypeRef) -> bool {
        subclass.fast_issubclass(&zelf)
    }

    #[pyclassmethod(magic)]
    fn subclasshook(_args: FuncArgs, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.not_implemented()
    }

    #[pyproperty]
    fn __name__(&self) -> String {
        self.name().to_string()
    }

    pub fn name(&self) -> BorrowedValue<str> {
        PyRwLockReadGuard::map(self.slots.name.read(), |slot_name| {
            let name = slot_name.as_ref().unwrap();
            if self.slots.flags.has_feature(PyTypeFlags::HEAPTYPE) {
                name.as_str()
            } else {
                name.rsplit('.').next().unwrap()
            }
        })
        .into()
    }

    #[pymethod(magic)]
    fn repr(&self, vm: &VirtualMachine) -> String {
        let module = self.module(vm);
        let module = module.downcast_ref::<PyStr>().map(|m| m.as_str());

        match module {
            Some(module) if module != "builtins" => {
                let name = self.name();
                format!(
                    "<class '{}.{}'>",
                    module,
                    self.qualname(vm)
                        .downcast_ref::<PyStr>()
                        .map(|n| n.as_str())
                        .unwrap_or_else(|| &name)
                )
            }
            _ => format!("<class '{}'>", self.slot_name()),
        }
    }

    #[pyproperty(magic)]
    pub fn qualname(&self, vm: &VirtualMachine) -> PyObjectRef {
        self.attributes
            .read()
            .get("__qualname__")
            .cloned()
            // We need to exclude this method from going into recursion:
            .and_then(|found| {
                if found.fast_isinstance(&vm.ctx.types.getset_type) {
                    None
                } else {
                    Some(found)
                }
            })
            .unwrap_or_else(|| vm.ctx.new_str(self.name().deref()).into())
    }

    #[pyproperty(magic)]
    pub fn module(&self, vm: &VirtualMachine) -> PyObjectRef {
        self.attributes
            .read()
            .get("__module__")
            .cloned()
            // We need to exclude this method from going into recursion:
            .and_then(|found| {
                if found.fast_isinstance(&vm.ctx.types.getset_type) {
                    None
                } else {
                    Some(found)
                }
            })
            .unwrap_or_else(|| vm.ctx.new_str(ascii!("builtins")).into())
    }

    #[pyproperty(magic, setter)]
    fn set_module(&self, value: PyObjectRef) {
        self.attributes
            .write()
            .insert("__module__".to_owned(), value);
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

    #[pymethod]
    fn mro(zelf: PyRef<Self>) -> Vec<PyObjectRef> {
        zelf.iter_mro().map(|cls| cls.clone().into()).collect()
    }

    #[pymethod(name = "__ror__")]
    #[pymethod(magic)]
    pub fn or(zelf: PyObjectRef, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if !union_::is_unionable(zelf.clone(), vm) || !union_::is_unionable(other.clone(), vm) {
            return vm.ctx.not_implemented();
        }

        let tuple = PyTuple::new_ref(vec![zelf, other], &vm.ctx);
        union_::make_union(tuple, vm)
    }

    #[pyslot]
    fn slot_new(metatype: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        vm_trace!("type.__new__ {:?}", args);

        let is_type_type = metatype.is(&vm.ctx.types.type_type);
        if is_type_type && args.args.len() == 1 && args.kwargs.is_empty() {
            return Ok(args.args[0].class().clone().into());
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

        if name.as_str().contains(char::from(0)) {
            return Err(vm.new_value_error("type name must not contain null characters".to_owned()));
        }

        let (metatype, base, bases) = if bases.is_empty() {
            let base = vm.ctx.types.object_type.clone();
            (metatype, base.clone(), vec![base])
        } else {
            let bases = bases
                .iter()
                .map(|obj| {
                    obj.clone().downcast::<PyType>().or_else(|obj| {
                        if vm.get_attribute_opt(obj, "__mro_entries__")?.is_some() {
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

            (metatype, base, bases)
        };

        let mut attributes = dict.to_attributes();
        if let Some(f) = attributes.get_mut("__new__") {
            if f.class().is(&vm.ctx.types.function_type) {
                *f = PyStaticMethod::from(f.clone()).into_pyobject(vm);
            }
        }

        if let Some(f) = attributes.get_mut("__init_subclass__") {
            if f.class().is(&vm.ctx.types.function_type) {
                *f = PyClassMethod::from(f.clone()).into_pyobject(vm);
            }
        }

        if let Some(f) = attributes.get_mut("__class_getitem__") {
            if f.class().is(&vm.ctx.types.function_type) {
                *f = PyClassMethod::from(f.clone()).into_pyobject(vm);
            }
        }

        if let Some(current_frame) = vm.current_frame() {
            let entry = attributes.entry("__module__".to_owned());
            if matches!(entry, Entry::Vacant(_)) {
                let module_name =
                    vm.unwrap_or_none(current_frame.globals.get_item_opt("__name__", vm)?);
                entry.or_insert(module_name);
            }
        }

        attributes
            .entry("__qualname__".to_owned())
            .or_insert_with(|| vm.ctx.new_str(name.as_str()).into());

        // All *classes* should have a dict. Exceptions are *instances* of
        // classes that define __slots__ and instances of built-in classes
        // (with exceptions, e.g function)
        attributes.entry("__dict__".to_owned()).or_insert_with(|| {
            vm.ctx
                .new_getset(
                    "__dict__",
                    vm.ctx.types.object_type.clone(),
                    subtype_get_dict,
                    subtype_set_dict,
                )
                .into()
        });

        // TODO: Flags is currently initialized with HAS_DICT. Should be
        // updated when __slots__ are supported (toggling the flag off if
        // a class has __slots__ defined).
        let flags = PyTypeFlags::heap_type_flags() | PyTypeFlags::HAS_DICT;
        let slots = PyTypeSlots::with_flags(flags);

        let typ = Self::new_verbose_ref(name.as_str(), base, bases, attributes, slots, metatype)
            .map_err(|e| vm.new_type_error(e))?;

        // avoid deadlock
        let attributes = typ
            .attributes
            .read()
            .iter()
            .filter_map(|(name, obj)| {
                vm.get_method(obj.clone(), "__set_name__").map(|res| {
                    res.map(|meth| (obj.clone(), PyStr::from(name.clone()).into_ref(vm), meth))
                })
            })
            .collect::<PyResult<Vec<_>>>()?;
        for (obj, name, set_name) in attributes {
            vm.invoke(&set_name, (typ.clone(), name.clone()))
                .map_err(|e| {
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

        if let Some(initter) = typ.get_super_attr("__init_subclass__") {
            let initter = vm
                .call_get_descriptor_specific(initter.clone(), None, Some(typ.clone().into()))
                .unwrap_or(Ok(initter))?;
            vm.invoke(&initter, kwargs)?;
        };

        Ok(typ.into())
    }

    #[pyproperty(magic)]
    fn dict(zelf: PyRef<Self>) -> PyMappingProxy {
        PyMappingProxy::new(zelf)
    }

    #[pyproperty(magic, setter)]
    fn set_dict(&self, _value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        Err(vm.new_not_implemented_error(
            "Setting __dict__ attribute on a type isn't yet implemented".to_owned(),
        ))
    }

    #[pyproperty(magic, setter)]
    fn set_name(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        if !self.slots.flags.has_feature(PyTypeFlags::HEAPTYPE) {
            return Err(vm.new_type_error(format!(
                "cannot set '{}' attribute of immutable type '{}'",
                "__name__",
                self.name()
            )));
        }
        let name = value.downcast_ref::<PyStr>().ok_or_else(|| {
            vm.new_type_error(format!(
                "can only assign string to {}.__name__, not '{}'",
                self.name(),
                value.class().name()
            ))
        })?;
        if name.as_str().contains(char::from(0)) {
            return Err(vm.new_value_error("type name must not contain null characters".to_owned()));
        }
        *self.slots.name.write() = Some(name.as_str().to_string());
        Ok(())
    }

    #[pyproperty(magic)]
    fn text_signature(&self) -> Option<String> {
        self.slots
            .doc
            .and_then(|doc| get_text_signature_from_internal_doc(&self.name(), doc))
            .map(|signature| signature.to_string())
    }
}

const SIGNATURE_END_MARKER: &str = ")\n--\n\n";
fn get_signature(doc: &str) -> Option<&str> {
    doc.find(SIGNATURE_END_MARKER).map(|index| &doc[..=index])
}

fn find_signature<'a>(name: &str, doc: &'a str) -> Option<&'a str> {
    let name = name.rsplit('.').next().unwrap();
    let doc = doc.strip_prefix(name)?;
    if !doc.starts_with('(') {
        None
    } else {
        Some(doc)
    }
}

pub(crate) fn get_text_signature_from_internal_doc<'a>(
    name: &str,
    internal_doc: &'a str,
) -> Option<&'a str> {
    find_signature(name, internal_doc).and_then(get_signature)
}

impl GetAttr for PyType {
    fn getattro(zelf: &Py<Self>, name_str: PyStrRef, vm: &VirtualMachine) -> PyResult {
        let name = name_str.as_str();
        vm_trace!("type.__getattribute__({:?}, {:?})", zelf, name);
        let mcl = zelf.class();

        let mcl_attr = mcl.get_attr(name);

        if let Some(ref attr) = mcl_attr {
            let attr_class = attr.class();
            if attr_class.slots.descr_set.load().is_some() {
                if let Some(descr_get) = attr_class.slots.descr_get.load() {
                    let mcl = mcl.into_owned().into();
                    return descr_get(attr.clone(), Some(zelf.to_owned().into()), Some(mcl), vm);
                }
            }
        }

        let zelf_attr = zelf.get_attr(name);

        if let Some(ref attr) = zelf_attr {
            if let Some(descr_get) = attr.class().slots.descr_get.load() {
                drop(mcl);
                return descr_get(attr.clone(), None, Some(zelf.to_owned().into()), vm);
            }
        }

        if let Some(cls_attr) = zelf_attr {
            Ok(cls_attr)
        } else if let Some(attr) = mcl_attr {
            drop(mcl);
            vm.call_if_get_descriptor(attr, zelf.to_owned().into())
        } else {
            Err(vm.new_attribute_error(format!(
                "type object '{}' has no attribute '{}'",
                zelf.slot_name(),
                name
            )))
        }
    }
}

impl SetAttr for PyType {
    fn setattro(
        zelf: &crate::Py<Self>,
        attr_name: PyStrRef,
        value: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        if let Some(attr) = zelf.get_class_attr(attr_name.as_str()) {
            let descr_set = attr.class().slots.descr_set.load();
            if let Some(descriptor) = descr_set {
                return descriptor(attr, zelf.to_owned().into(), value, vm);
            }
        }
        let assign = value.is_some();

        let mut attributes = zelf.attributes.write();
        if let Some(value) = value {
            attributes.insert(attr_name.as_str().to_owned(), value);
        } else {
            let prev_value = attributes.remove(attr_name.as_str());
            if prev_value.is_none() {
                return Err(vm.new_exception(
                    vm.ctx.exceptions.attribute_error.clone(),
                    vec![attr_name.into()],
                ));
            }
        }
        let attr_name = attr_name.as_str();
        if attr_name.starts_with("__") && attr_name.ends_with("__") {
            zelf.update_slot(attr_name, assign);
        }
        Ok(())
    }
}

impl Callable for PyType {
    type Args = FuncArgs;
    fn call(zelf: &crate::Py<Self>, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        vm_trace!("type_call: {:?}", zelf);
        let obj = call_slot_new(zelf.to_owned(), zelf.to_owned(), args.clone(), vm)?;

        if (zelf.is(&vm.ctx.types.type_type) && args.kwargs.is_empty())
            || !obj.fast_isinstance(zelf)
        {
            return Ok(obj);
        }

        if let Some(init_method) = obj.class().slots.init.load() {
            init_method(obj.clone(), args, vm)?;
        }
        Ok(obj)
    }
}

fn find_base_dict_descr(cls: &PyTypeRef, vm: &VirtualMachine) -> Option<PyObjectRef> {
    cls.iter_base_chain().skip(1).find_map(|cls| {
        // TODO: should actually be some translation of:
        // cls.slot_dictoffset != 0 && !cls.flags.contains(HEAPTYPE)
        if cls.is(&vm.ctx.types.type_type) {
            cls.get_attr("__dict__")
        } else {
            None
        }
    })
}

fn subtype_get_dict(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    // TODO: obj.class().as_pyref() need to be supported
    let cls = obj.class().clone();
    let ret = match find_base_dict_descr(&cls, vm) {
        Some(descr) => vm.call_get_descriptor(descr, obj).unwrap_or_else(|_| {
            Err(vm.new_type_error(format!(
                "this __dict__ descriptor does not support '{}' objects",
                cls.name()
            )))
        })?,
        None => object::object_get_dict(obj, vm)?.into(),
    };
    Ok(ret)
}

fn subtype_set_dict(obj: PyObjectRef, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
    let cls = obj.class().clone();
    match find_base_dict_descr(&cls, vm) {
        Some(descr) => {
            let descr_set = descr.class().slots.descr_set.load().ok_or_else(|| {
                vm.new_type_error(format!(
                    "this __dict__ descriptor does not support '{}' objects",
                    cls.name()
                ))
            })?;
            descr_set(descr, obj, Some(value), vm)
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
    PyType::extend_class(ctx, &ctx.types.type_type);
}

pub(crate) fn call_slot_new(
    typ: PyTypeRef,
    subtype: PyTypeRef,
    args: FuncArgs,
    vm: &VirtualMachine,
) -> PyResult {
    for cls in typ.deref().iter_mro() {
        if let Some(slot_new) = cls.slots.new.load() {
            return slot_new(subtype, args, vm);
        }
    }
    unreachable!("Should be able to find a new slot somewhere in the mro")
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
    vm_trace!("Linearising MRO: {:?}", bases);
    // Python requires that the class direct bases are kept in the same order.
    // This is called local precedence ordering.
    // This means we must verify that for classes A(), B(A) we must reject C(A, B) even though this
    // algorithm will allow the mro ordering of [C, B, A, object].
    // To verify this, we make sure non of the direct bases are in the mro of bases after them.
    for (i, base_mro) in bases.iter().enumerate() {
        let base = &base_mro[0]; // Mros cannot be empty.
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
        if winner.fast_issubclass(&base_type) {
            continue;
        } else if base_type.fast_issubclass(&winner) {
            winner = base_type.into_owned();
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

fn best_base(bases: &[PyTypeRef], vm: &VirtualMachine) -> PyResult<PyTypeRef> {
    // let mut base = None;
    // let mut winner = None;

    for base_i in bases {
        // base_proto = PyTuple_GET_ITEM(bases, i);
        // if (!PyType_Check(base_proto)) {
        //     PyErr_SetString(
        //         PyExc_TypeError,
        //         "bases must be types");
        //     return NULL;
        // }
        // base_i = (PyTypeObject *)base_proto;
        // if (base_i->slot_dict == NULL) {
        //     if (PyType_Ready(base_i) < 0)
        //         return NULL;
        // }

        if !base_i.slots.flags.has_feature(PyTypeFlags::BASETYPE) {
            return Err(vm.new_type_error(format!(
                "type '{}' is not an acceptable base type",
                base_i.name()
            )));
        }
        // candidate = solid_base(base_i);
        // if (winner == NULL) {
        //     winner = candidate;
        //     base = base_i;
        // }
        // else if (PyType_IsSubtype(winner, candidate))
        //     ;
        // else if (PyType_IsSubtype(candidate, winner)) {
        //     winner = candidate;
        //     base = base_i;
        // }
        // else {
        //     PyErr_SetString(
        //         PyExc_TypeError,
        //         "multiple bases have "
        //         "instance lay-out conflict");
        //     return NULL;
        // }
    }

    // FIXME: Ok(base.unwrap()) is expected
    Ok(bases[0].clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map_ids(obj: Result<Vec<PyTypeRef>, String>) -> Result<Vec<usize>, String> {
        Ok(obj?.into_iter().map(|x| x.get_id()).collect())
    }

    #[test]
    fn test_linearise() {
        let context = Context::default();
        let object = &context.types.object_type;
        let type_type = &context.types.type_type;

        let a = PyType::new_ref(
            "A",
            vec![object.clone()],
            PyAttributes::default(),
            Default::default(),
            type_type.clone(),
        )
        .unwrap();
        let b = PyType::new_ref(
            "B",
            vec![object.clone()],
            PyAttributes::default(),
            Default::default(),
            type_type.clone(),
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
            map_ids(Ok(vec![a.clone(), b.clone(), object.clone()]))
        );
    }
}

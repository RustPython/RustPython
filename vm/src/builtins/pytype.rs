use crate::common::lock::{
    PyMappedRwLockReadGuard, PyRwLock, PyRwLockReadGuard, PyRwLockUpgradableReadGuard,
    PyRwLockWriteGuard,
};
use std::collections::HashSet;
use std::fmt;

use super::classmethod::PyClassMethod;
use super::dict::PyDictRef;
use super::int::PyInt;
use super::list::PyList;
use super::mappingproxy::PyMappingProxy;
use super::object;
use super::pystr::{PyStr, PyStrRef};
use super::staticmethod::PyStaticMethod;
use super::tuple::PyTuple;
use super::weakref::PyWeak;
use crate::builtins::tuple::PyTupleTyped;
use crate::function::{FuncArgs, KwArgs};
use crate::pyobject::{
    BorrowValue, Either, IdProtocol, PyAttributes, PyClassImpl, PyContext, PyLease, PyObjectRef,
    PyRef, PyResult, PyValue, TryFromObject, TypeProtocol,
};
use crate::slots::{self, Callable, PyTpFlags, PyTypeSlots, SlotGetattro, SlotSetattro};
use crate::vm::VirtualMachine;
use itertools::Itertools;
use std::ops::Deref;

/// type(object_or_name, bases, dict)
/// type(object) -> the object's type
/// type(name, bases, dict) -> a new type
#[pyclass(module = false, name = "type")]
pub struct PyType {
    pub name: String,
    pub base: Option<PyTypeRef>,
    pub bases: Vec<PyTypeRef>,
    pub mro: Vec<PyTypeRef>,
    pub subclasses: PyRwLock<Vec<PyWeak>>,
    pub attributes: PyRwLock<PyAttributes>,
    pub slots: PyTypeSlots,
}

impl fmt::Display for PyType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.name, f)
    }
}

impl fmt::Debug for PyType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "[PyType {}]", &self.name)
    }
}

pub type PyTypeRef = PyRef<PyType>;

impl PyValue for PyType {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.type_type
    }
}

impl PyType {
    pub fn tp_name(&self) -> PyMappedRwLockReadGuard<str> {
        let opt_name = self.slots.name.upgradable_read();
        let read_guard = if opt_name.is_some() {
            PyRwLockUpgradableReadGuard::downgrade(opt_name)
        } else {
            let mut opt_name = PyRwLockUpgradableReadGuard::upgrade(opt_name);
            let module = self.attributes.read().get("__module__").cloned();
            let module: Option<PyStrRef> = module.and_then(|m| m.downcast().ok());
            let new_name = if let Some(module) = module {
                format!("{}.{}", module, &self.name)
            } else {
                self.name.clone()
            };
            *opt_name = Some(new_name);
            PyRwLockWriteGuard::downgrade(opt_name)
        };
        PyRwLockReadGuard::map(read_guard, |opt| opt.as_deref().unwrap())
    }

    pub fn iter_mro(&self) -> impl Iterator<Item = &PyType> + DoubleEndedIterator {
        std::iter::once(self).chain(self.mro.iter().map(|cls| cls.deref()))
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
            self.mro.iter().find_map(|cls| f(&cls))
        }
    }

    // This is used for class initialisation where the vm is not yet available.
    pub fn set_str_attr<V: Into<PyObjectRef>>(&self, attr_name: &str, value: V) {
        self.attributes
            .write()
            .insert(attr_name.to_owned(), value.into());
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

    pub(crate) fn update_slot(&self, name: &str, add: bool) {
        macro_rules! update_slot {
            ($name:ident, $func:expr) => {{
                self.slots.$name.store(if add { Some($func) } else { None });
            }};
        }
        match name {
            "__call__" => {
                let func: slots::GenericMethod =
                    |zelf, args, vm| vm.call_special_method(zelf.clone(), "__call__", args);
                update_slot!(call, func);
            }
            "__get__" => {
                let func: slots::DescrGetFunc =
                    |zelf, obj, cls, vm| vm.call_special_method(zelf, "__get__", (obj, cls));
                update_slot!(descr_get, func);
            }
            "__set__" | "__delete__" => {
                let func: slots::DescrSetFunc = |zelf, obj, value, vm| {
                    match value {
                        Some(val) => vm.call_special_method(zelf, "__set__", (obj, val)),
                        None => vm.call_special_method(zelf, "__delete__", (obj,)),
                    }
                    .map(drop)
                };
                update_slot!(descr_set, func);
            }
            "__hash__" => {
                let func: slots::HashFunc = |zelf, vm| {
                    let hash_obj = vm.call_special_method(zelf.clone(), "__hash__", ())?;
                    match hash_obj.payload_if_subclass::<PyInt>(vm) {
                        Some(py_int) => {
                            Ok(rustpython_common::hash::hash_bigint(py_int.borrow_value()))
                        }
                        None => Err(vm
                            .new_type_error("__hash__ method should return an integer".to_owned())),
                    }
                } as _;
                update_slot!(hash, func);
            }
            "__del__" => {
                let func: slots::DelFunc = |zelf, vm| {
                    vm.call_special_method(zelf.clone(), "__del__", ())?;
                    Ok(())
                } as _;
                update_slot!(del, func);
            }
            "__eq__" | "__ne__" | "__le__" | "__lt__" | "__ge__" | "__gt__" => {
                let func: slots::CmpFunc = |zelf, other, op, vm| {
                    vm.call_special_method(zelf.clone(), op.method_name(), (other.clone(),))
                        .map(Either::A)
                } as _;
                update_slot!(cmp, func);
            }
            "__getattribute__" => {
                let func: slots::GetattroFunc =
                    |zelf, name, vm| vm.call_special_method(zelf, "__getattribute__", (name,));
                update_slot!(getattro, func);
            }
            "__setattr__" => {
                let func: slots::SetattroFunc = |zelf, name, value, vm| {
                    match value {
                        Some(value) => {
                            vm.call_special_method(zelf.clone(), "__setattr__", (name, value))?;
                        }
                        None => {
                            vm.call_special_method(zelf.clone(), "__delattr__", (name,))?;
                        }
                    };
                    Ok(())
                };
                update_slot!(setattro, func);
            }
            "__iter__" => {
                let func: slots::IterFunc = |zelf, vm| vm.call_special_method(zelf, "__iter__", ());
                update_slot!(iter, func);
            }
            "__next__" => {
                let func: slots::IterNextFunc =
                    |zelf, vm| vm.call_special_method(zelf.clone(), "__next__", ());
                update_slot!(iternext, func);
            }
            _ => {}
        }
    }
}

impl PyTypeRef {
    pub fn issubclass<R: IdProtocol>(&self, cls: R) -> bool {
        self._issubclass(cls)
    }

    pub fn iter_mro(&self) -> impl Iterator<Item = &PyTypeRef> + DoubleEndedIterator {
        std::iter::once(self).chain(self.mro.iter())
    }

    pub fn iter_base_chain(&self) -> impl Iterator<Item = &PyTypeRef> {
        std::iter::successors(Some(self), |cls| cls.base.as_ref())
    }
}

#[pyimpl(with(SlotGetattro, SlotSetattro, Callable), flags(BASETYPE))]
impl PyType {
    #[pyproperty(name = "__mro__")]
    fn get_mro(zelf: PyRef<Self>) -> PyTuple {
        let elements: Vec<PyObjectRef> = zelf.iter_mro().map(|x| x.as_object().clone()).collect();
        PyTuple::_new(elements.into_boxed_slice())
    }

    #[pyproperty(magic)]
    fn bases(&self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx
            .new_tuple(self.bases.iter().map(|x| x.as_object().clone()).collect())
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
            .drain()
            .map(|(k, _)| vm.ctx.new_str(k))
            .collect();
        PyList::from(attributes)
    }

    #[pymethod(magic)]
    fn instancecheck(zelf: PyRef<Self>, obj: PyObjectRef) -> bool {
        obj.isinstance(&zelf)
    }

    #[pymethod(magic)]
    fn subclasscheck(zelf: PyRef<Self>, subclass: PyTypeRef) -> bool {
        subclass.issubclass(&zelf)
    }

    #[pyproperty(magic)]
    fn name(&self) -> String {
        self.name.clone()
    }

    #[pymethod(magic)]
    fn repr(&self) -> String {
        format!("<class '{}'>", self.tp_name())
    }

    #[pyproperty(magic)]
    fn qualname(&self, vm: &VirtualMachine) -> PyObjectRef {
        self.attributes
            .read()
            .get("__qualname__")
            .cloned()
            .unwrap_or_else(|| vm.ctx.new_str(self.name.clone()))
    }

    #[pyproperty(magic)]
    fn module(&self, vm: &VirtualMachine) -> PyObjectRef {
        // TODO: Implement getting the actual module a builtin type is from
        self.attributes
            .read()
            .get("__module__")
            .cloned()
            .unwrap_or_else(|| vm.ctx.new_str("builtins"))
    }

    #[pyproperty(magic, setter)]
    fn set_module(&self, value: PyObjectRef) {
        *self.slots.name.write() = None;
        self.attributes
            .write()
            .insert("__module__".to_owned(), value);
    }

    #[pymethod(magic)]
    fn prepare(
        _name: PyStrRef,
        _bases: PyObjectRef,
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
    fn mro(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_list(
            zelf.iter_mro()
                .map(|cls| cls.clone().into_object())
                .collect(),
        )
    }
    #[pyslot]
    fn tp_new(metatype: PyTypeRef, mut args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        vm_trace!("type.__new__ {:?}", args);

        let is_type_type = metatype.is(&vm.ctx.types.type_type);
        if is_type_type && args.args.len() == 1 && args.kwargs.is_empty() {
            return Ok(args.args[0].clone_class().into_object());
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

        let (name, bases, dict, kwargs): (PyStrRef, PyTupleTyped<PyTypeRef>, PyDictRef, KwArgs) =
            args.clone().bind(vm)?;

        let bases = bases.borrow_value().to_vec();
        let (metatype, base, bases) = if bases.is_empty() {
            let base = vm.ctx.types.object_type.clone();
            (metatype, base.clone(), vec![base])
        } else {
            // TODO
            // for base in &bases {
            //   if PyType_Check(base) { continue; }
            //   _PyObject_LookupAttrId(base, PyId___mro_entries__, &base)?
            //   Err(new_type_error( "type() doesn't support MRO entry resolution; "
            //                       "use types.new_class()"))
            // }

            // Search the bases for the proper metatype to deal with this:
            let winner = calculate_meta_class(metatype.clone(), &bases, vm)?;
            let metatype = if !winner.is(&metatype) {
                #[allow(clippy::redundant_clone)] // false positive
                if let Some(ref tp_new) = winner.clone().slots.new {
                    // Pass it to the winner
                    args.prepend_arg(winner.into_object());
                    return tp_new(vm, args);
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
                *f = PyStaticMethod::from(f.clone()).into_object(vm);
            }
        }

        if let Some(f) = attributes.get_mut("__init_subclass__") {
            if f.class().is(&vm.ctx.types.function_type) {
                *f = PyClassMethod::from(f.clone()).into_object(vm);
            }
        }

        if !attributes.contains_key("__dict__") {
            attributes.insert(
                "__dict__".to_owned(),
                vm.ctx
                    .new_getset("__dict__", subtype_get_dict, subtype_set_dict),
            );
        }

        // TODO: how do we know if it should have a dict?
        let flags = base.slots.flags | PyTpFlags::HAS_DICT;

        let slots = PyTypeSlots::from_flags(flags);

        let typ = new(
            metatype,
            name.borrow_value(),
            base,
            bases,
            attributes,
            slots,
        )
        .map_err(|e| vm.new_type_error(e))?;

        vm.ctx.add_slot_wrappers(&typ);

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
                        obj.class().name,
                        name,
                        typ.name
                    ));
                    err.set_cause(Some(e));
                    err
                })?;
        }

        if let Some(initter) = typ.get_super_attr("__init_subclass__") {
            let initter = vm
                .call_get_descriptor_specific(
                    initter.clone(),
                    None,
                    Some(typ.clone().into_object()),
                )
                .unwrap_or(Ok(initter))?;
            vm.invoke(&initter, kwargs)?;
        };

        Ok(typ.into_object())
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
}

impl SlotGetattro for PyType {
    fn getattro(zelf: PyRef<Self>, name_str: PyStrRef, vm: &VirtualMachine) -> PyResult {
        let name = name_str.borrow_value();
        vm_trace!("type.__getattribute__({:?}, {:?})", zelf, name);
        let mcl = zelf.class();

        let mcl_attr = mcl.get_attr(name);

        if let Some(ref attr) = mcl_attr {
            let attr_class = attr.class();
            if attr_class
                .mro_find_map(|cls| cls.slots.descr_set.load())
                .is_some()
            {
                if let Some(descr_get) = attr_class.mro_find_map(|cls| cls.slots.descr_get.load()) {
                    let mcl = PyLease::into_pyref(mcl).into_object();
                    return descr_get(attr.clone(), Some(zelf.into_object()), Some(mcl), vm);
                }
            }
        }

        let zelf_attr = zelf.get_attr(name);

        if let Some(ref attr) = zelf_attr {
            if let Some(descr_get) = attr.class().mro_find_map(|cls| cls.slots.descr_get.load()) {
                drop(mcl);
                return descr_get(attr.clone(), None, Some(zelf.into_object()), vm);
            }
        }

        if let Some(cls_attr) = zelf_attr {
            Ok(cls_attr)
        } else if let Some(attr) = mcl_attr {
            drop(mcl);
            vm.call_if_get_descriptor(attr, zelf.into_object())
        } else {
            Err(vm.new_attribute_error(format!(
                "type object '{}' has no attribute '{}'",
                zelf, name
            )))
        }
    }
}

impl SlotSetattro for PyType {
    fn setattro(
        zelf: &PyRef<Self>,
        attr_name: PyStrRef,
        value: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        if let Some(attr) = zelf.get_class_attr(attr_name.borrow_value()) {
            let descr_set = attr.class().mro_find_map(|cls| cls.slots.descr_set.load());
            if let Some(descriptor) = descr_set {
                return descriptor(attr, zelf.clone().into_object(), value, vm);
            }
        }
        let assign = value.is_some();

        let mut attributes = zelf.attributes.write();
        if let Some(value) = value {
            attributes.insert(attr_name.borrow_value().to_owned(), value);
        } else {
            let prev_value = attributes.remove(attr_name.borrow_value());
            if prev_value.is_none() {
                return Err(vm.new_exception(
                    vm.ctx.exceptions.attribute_error.clone(),
                    vec![attr_name.into_object()],
                ));
            }
        }
        let attr_name = attr_name.borrow_value();
        if attr_name.starts_with("__") && attr_name.ends_with("__") {
            zelf.update_slot(attr_name, assign);
        }
        Ok(())
    }
}

impl Callable for PyType {
    fn call(zelf: &PyRef<Self>, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        vm_trace!("type_call: {:?}", zelf);
        let obj = call_tp_new(zelf.clone(), zelf.clone(), args.clone(), vm)?;

        if (zelf.is(&vm.ctx.types.type_type) && args.kwargs.is_empty()) || !obj.isinstance(&zelf) {
            return Ok(obj);
        }

        if let Some(init_method_or_err) = vm.get_method(obj.clone(), "__init__") {
            let init_method = init_method_or_err?;
            let res = vm.invoke(&init_method, args)?;
            if !vm.is_none(&res) {
                return Err(vm.new_type_error("__init__ must return None".to_owned()));
            }
        }
        Ok(obj)
    }
}

fn find_base_dict_descr(cls: &PyTypeRef, vm: &VirtualMachine) -> Option<PyObjectRef> {
    cls.iter_base_chain().skip(1).find_map(|cls| {
        // TODO: should actually be some translation of:
        // cls.tp_dictoffset != 0 && !cls.flags.contains(HEAPTYPE)
        if cls.is(&vm.ctx.types.type_type) {
            cls.get_attr("__dict__")
        } else {
            None
        }
    })
}

fn subtype_get_dict(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    // TODO: obj.class().as_pyref() need to be supported
    let cls = obj.clone_class();
    let ret = match find_base_dict_descr(&cls, vm) {
        Some(descr) => vm.call_get_descriptor(descr, obj).unwrap_or_else(|_| {
            Err(vm.new_type_error(format!(
                "this __dict__ descriptor does not support '{}' objects",
                cls.name
            )))
        })?,
        None => object::object_get_dict(obj, vm)?.into_object(),
    };
    Ok(ret)
}

fn subtype_set_dict(obj: PyObjectRef, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
    let cls = obj.clone_class();
    match find_base_dict_descr(&cls, vm) {
        Some(descr) => {
            let descr_set = descr
                .class()
                .mro_find_map(|cls| cls.slots.descr_set.load())
                .ok_or_else(|| {
                    vm.new_type_error(format!(
                        "this __dict__ descriptor does not support '{}' objects",
                        cls.name
                    ))
                })?;
            descr_set(descr, obj, Some(value), vm)
        }
        None => {
            object::object_set_dict(obj, PyDictRef::try_from_object(vm, value)?, vm)?;
            Ok(())
        }
    }
}

/*
 * The magical type type
 */

pub(crate) fn init(ctx: &PyContext) {
    PyType::extend_class(ctx, &ctx.types.type_type);
}

impl PyLease<'_, PyType> {
    pub fn issubclass<R: IdProtocol>(&self, cls: R) -> bool {
        self._issubclass(cls)
    }
}

pub trait DerefToPyType {
    fn deref_to_type(&self) -> &PyType;

    /// Determines if `subclass` is actually a subclass of `cls`, this doesn't call __subclasscheck__,
    /// so only use this if `cls` is known to have not overridden the base __subclasscheck__ magic
    /// method.
    fn _issubclass<R: IdProtocol>(&self, cls: R) -> bool
    where
        Self: IdProtocol,
    {
        self.is(&cls) || self.deref_to_type().mro.iter().any(|c| c.is(&cls))
    }
}

impl DerefToPyType for PyTypeRef {
    fn deref_to_type(&self) -> &PyType {
        self.deref()
    }
}

impl<'a> DerefToPyType for PyLease<'a, PyType> {
    fn deref_to_type(&self) -> &PyType {
        self.deref()
    }
}

impl<T: DerefToPyType> DerefToPyType for &'_ T {
    fn deref_to_type(&self) -> &PyType {
        (&**self).deref_to_type()
    }
}

fn call_tp_new(
    typ: PyTypeRef,
    subtype: PyTypeRef,
    mut args: FuncArgs,
    vm: &VirtualMachine,
) -> PyResult {
    for cls in typ.deref().iter_mro() {
        if let Some(new_meth) = cls.get_attr("__new__") {
            if !vm.ctx.is_tp_new_wrapper(&new_meth) {
                let new_meth = vm.call_if_get_descriptor(new_meth, typ.clone().into_object())?;
                args.prepend_arg(typ.clone().into_object());
                return vm.invoke(&new_meth, args);
            }
        }
        if let Some(tp_new) = cls.slots.new.as_ref() {
            args.prepend_arg(subtype.into_object());
            return tp_new(vm, args);
        }
    }
    unreachable!("Should be able to find a new slot somewhere in the mro")
}

pub fn tp_new_wrapper(
    zelf: PyTypeRef,
    cls: PyTypeRef,
    args: FuncArgs,
    vm: &VirtualMachine,
) -> PyResult {
    if !cls.issubclass(&zelf) {
        return Err(vm.new_type_error(format!(
            "{zelf}.__new__({cls}): {cls} is not a subtype of {zelf}",
            zelf = zelf.name,
            cls = cls.name,
        )));
    }
    call_tp_new(zelf, cls, args, vm)
}

fn take_next_base(bases: &mut Vec<Vec<PyTypeRef>>) -> Option<PyTypeRef> {
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
        for later_mro in bases[i + 1..].iter() {
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

pub fn new(
    typ: PyTypeRef,
    name: &str,
    base: PyTypeRef,
    bases: Vec<PyTypeRef>,
    attrs: PyAttributes,
    mut slots: PyTypeSlots,
) -> Result<PyTypeRef, String> {
    // Check for duplicates in bases.
    let mut unique_bases = HashSet::new();
    for base in bases.iter() {
        if !unique_bases.insert(base.get_id()) {
            return Err(format!("duplicate base class {}", base.name));
        }
    }

    let mros = bases
        .iter()
        .map(|x| x.iter_mro().cloned().collect())
        .collect();
    let mro = linearise_mro(mros)?;

    if base.slots.flags.has_feature(PyTpFlags::HAS_DICT) {
        slots.flags |= PyTpFlags::HAS_DICT
    }
    let new_type = PyRef::new_ref(
        PyType {
            name: String::from(name),
            base: Some(base),
            bases,
            mro,
            subclasses: PyRwLock::default(),
            attributes: PyRwLock::new(attrs),
            slots,
        },
        typ,
        None,
    );

    for attr_name in new_type.attributes.read().keys() {
        if attr_name.starts_with("__") && attr_name.ends_with("__") {
            new_type.update_slot(attr_name, true);
        }
    }
    for base in &new_type.bases {
        base.subclasses
            .write()
            .push(PyWeak::downgrade(new_type.as_object()));
    }

    Ok(new_type)
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
        if winner.issubclass(&base_type) {
            continue;
        } else if base_type.issubclass(&winner) {
            winner = PyLease::into_pyref(base_type);
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
        // if (base_i->tp_dict == NULL) {
        //     if (PyType_Ready(base_i) < 0)
        //         return NULL;
        // }

        if !base_i.slots.flags.has_feature(PyTpFlags::BASETYPE) {
            return Err(vm.new_type_error(format!(
                "type '{}' is not an acceptable base type",
                base_i.name
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
        let context = PyContext::new();
        let object = &context.types.object_type;
        let type_type = &context.types.type_type;

        let a = new(
            type_type.clone(),
            "A",
            object.clone(),
            vec![object.clone()],
            PyAttributes::default(),
            Default::default(),
        )
        .unwrap();
        let b = new(
            type_type.clone(),
            "B",
            object.clone(),
            vec![object.clone()],
            PyAttributes::default(),
            Default::default(),
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

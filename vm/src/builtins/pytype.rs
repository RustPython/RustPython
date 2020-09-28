use crate::common::lock::PyRwLock;
use std::collections::{HashMap, HashSet};
use std::fmt;

use super::classmethod::PyClassMethod;
use super::dict::PyDictRef;
use super::int::PyInt;
use super::list::PyList;
use super::mappingproxy::PyMappingProxy;
use super::object;
use super::pystr::PyStrRef;
use super::staticmethod::PyStaticMethod;
use super::tuple::PyTuple;
use super::weakref::PyWeak;
use crate::function::{FuncArgs, KwArgs};
use crate::pyobject::{
    BorrowValue, Either, IdProtocol, PyAttributes, PyClassImpl, PyContext, PyIterable, PyLease,
    PyObjectRef, PyRef, PyResult, PyValue, TryFromObject, TypeProtocol,
};
use crate::slots::{self, Callable, PyTpFlags, PyTypeSlots, SlotGetattro};
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
    fn tp_name(&self, vm: &VirtualMachine) -> String {
        let opt_name = self.slots.name.read().clone();
        opt_name.unwrap_or_else(|| {
            let module = self.attributes.read().get("__module__").cloned();
            let new_name = if let Some(module) = module {
                // FIXME: "unknown" case is a bug.
                let module_str = PyStrRef::try_from_object(vm, module)
                    .map_or("<unknown>".to_owned(), |m| m.borrow_value().to_owned());
                format!("{}.{}", module_str, &self.name)
            } else {
                self.name.clone()
            };
            *self.slots.name.write() = Some(new_name.clone());
            new_name
        })
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
        let mut attributes = PyAttributes::new();

        for bc in self.iter_mro().rev() {
            for (name, value) in bc.attributes.read().iter() {
                attributes.insert(name.to_owned(), value.clone());
            }
        }

        attributes
    }

    pub(crate) fn update_slot(&self, name: &str) {
        // self is the resolved class in get_class_magic
        match name {
            "__call__" => {
                let func: slots::GenericMethod = |zelf, args, vm| {
                    let magic = get_class_magic(&zelf, "__call__");
                    let magic = vm.call_if_get_descriptor(magic, zelf.clone())?;
                    vm.invoke(&magic, args)
                } as _;
                self.slots.call.store(Some(func))
            }
            "__get__" => {
                let func: slots::DescrGetFunc = |zelf, obj, cls, vm| {
                    let magic = get_class_magic(&zelf, "__get__");
                    vm.invoke(&magic, (zelf, obj, cls))
                } as _;
                self.slots.descr_get.store(Some(func))
            }
            "__hash__" => {
                let func: slots::HashFunc = |zelf, vm| {
                    let magic = get_class_magic(&zelf, "__hash__");
                    let hash_obj = vm.invoke(&magic, vec![zelf.clone()])?;
                    match hash_obj.payload_if_subclass::<PyInt>(vm) {
                        Some(py_int) => {
                            Ok(rustpython_common::hash::hash_bigint(py_int.borrow_value()))
                        }
                        None => Err(vm
                            .new_type_error("__hash__ method should return an integer".to_owned())),
                    }
                } as _;
                self.slots.hash.store(Some(func));
            }
            "__del__" => {
                let func: slots::DelFunc = |zelf, vm| {
                    let magic = get_class_magic(&zelf, "__del__");
                    let _ = vm.invoke(&magic, vec![zelf.clone()])?;
                    Ok(())
                } as _;
                self.slots.del.store(Some(func));
            }
            "__eq__" | "__ne__" | "__le__" | "__lt__" | "__ge__" | "__gt__" => {
                let func: slots::CmpFunc = |zelf, other, op, vm| {
                    let magic = get_class_magic(&zelf, op.method_name());
                    vm.invoke(&magic, vec![zelf.clone(), other.clone()])
                        .map(Either::A)
                } as _;
                self.slots.cmp.store(Some(func))
            }
            "__getattribute__" => {
                let func: slots::GetattroFunc = |zelf, name, vm| {
                    let magic = get_class_magic(&zelf, "__getattribute__");
                    vm.invoke(&magic, (zelf, name))
                };
                self.slots.getattro.store(Some(func))
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

#[inline]
fn get_class_magic(zelf: &PyObjectRef, name: &str) -> PyObjectRef {
    zelf.get_class_attr(name).unwrap()

    // TODO: we already looked up the matching class but lost the information here
    // let cls = zelf.class();
    // let attrs = cls.attributes.read();
    // attrs.get(name).unwrap().clone()
}

#[pyimpl(with(SlotGetattro, Callable), flags(BASETYPE))]
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
    fn repr(&self, vm: &VirtualMachine) -> String {
        format!("<class '{}'>", self.tp_name(vm))
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
    fn prepare(_name: PyStrRef, _bases: PyObjectRef, vm: &VirtualMachine) -> PyDictRef {
        vm.ctx.new_dict()
    }

    #[pymethod(magic)]
    fn setattr(
        zelf: PyRef<Self>,
        attr_name: PyStrRef,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        if let Some(attr) = zelf.get_class_attr(attr_name.borrow_value()) {
            if let Some(ref descriptor) = attr.get_class_attr("__set__") {
                vm.invoke(descriptor, (attr, zelf, value))?;
                return Ok(());
            }
        }
        let attr_name = attr_name.borrow_value();
        if attr_name.starts_with("__") && attr_name.ends_with("__") {
            zelf.update_slot(attr_name);
        }
        zelf.attributes.write().insert(attr_name.to_owned(), value);
        Ok(())
    }

    #[pymethod(magic)]
    fn delattr(zelf: PyRef<Self>, attr_name: PyStrRef, vm: &VirtualMachine) -> PyResult<()> {
        if let Some(attr) = zelf.get_class_attr(attr_name.borrow_value()) {
            if let Some(ref descriptor) = attr.get_class_attr("__delete__") {
                return vm.invoke(descriptor, (attr, zelf)).map(|_| ());
            }
        }

        zelf.get_attr(attr_name.borrow_value())
            .ok_or_else(|| vm.new_attribute_error(attr_name.borrow_value().to_owned()))?;
        zelf.attributes.write().remove(attr_name.borrow_value());
        Ok(())
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
    fn tp_new(metatype: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
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

        let (name, bases, dict, kwargs): (PyStrRef, PyIterable<PyTypeRef>, PyDictRef, KwArgs) =
            args.clone().bind(vm)?;

        let bases: Vec<PyTypeRef> = bases.iter(vm)?.collect::<Result<Vec<_>, _>>()?;
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

                    return tp_new(vm, args.insert(winner.into_object()));
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

        vm.ctx.add_tp_new_wrapper(&typ);

        for (name, obj) in typ.attributes.read().clone().iter() {
            if let Some(meth) = vm.get_method(obj.clone(), "__set_name__") {
                let set_name = meth?;
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
            if attr_class.has_attr("__set__") {
                if let Some(ref descr_get) =
                    attr_class.mro_find_map(|cls| cls.slots.descr_get.load())
                {
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
        } else if let Some(ref getter) = zelf.get_attr("__getattr__") {
            vm.invoke(getter, (PyLease::into_pyref(mcl), name_str))
        } else {
            Err(vm.new_attribute_error(format!(
                "type object '{}' has no attribute '{}'",
                zelf, name
            )))
        }
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
        Some(descr) => vm.call_get_descriptor(descr, obj).unwrap_or_else(|| {
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
            descr
                .get_class_attr("__set__")
                .map(|set| vm.invoke(&set, vec![descr, obj, value]))
                .unwrap_or_else(|| {
                    Err(vm.new_type_error(format!(
                        "this __dict__ descriptor does not support '{}' objects",
                        cls.name
                    )))
                })?;
        }
        None => object::object_set_dict(obj, PyDictRef::try_from_object(vm, value)?, vm)?,
    }
    Ok(())
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
    args: FuncArgs,
    vm: &VirtualMachine,
) -> PyResult {
    for cls in typ.deref().iter_mro() {
        if let Some(new_meth) = cls.get_attr("__new__") {
            if !vm.ctx.is_tp_new_wrapper(&new_meth) {
                let new_meth = vm.call_if_get_descriptor(new_meth, typ.clone().into_object())?;
                return vm.invoke(&new_meth, args.insert(typ.clone().into_object()));
            }
        }
        if let Some(tp_new) = cls.slots.new.as_ref() {
            return tp_new(vm, args.insert(subtype.into_object()));
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

fn take_next_base(mut bases: Vec<Vec<PyTypeRef>>) -> (Option<PyTypeRef>, Vec<Vec<PyTypeRef>>) {
    bases = bases.into_iter().filter(|x| !x.is_empty()).collect();

    for base in &bases {
        let head = base[0].clone();
        if !(&bases).iter().any(|x| x[1..].iter().any(|x| x.is(&head))) {
            // Remove from other heads.
            for item in &mut bases {
                if item[0].is(&head) {
                    item.remove(0);
                }
            }

            return (Some(head), bases);
        }
    }

    (None, bases)
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
    loop {
        if (&bases).iter().all(Vec::is_empty) {
            break;
        }
        let (head, new_bases) = take_next_base(bases);
        let head = head.ok_or_else(|| {
            // Take the head class of each class here. Now that we have reached the problematic bases.
            // Because this failed, we assume the lists cannot be empty.
            format!(
                "Cannot create a consistent method resolution order (MRO) for bases {}",
                new_bases.iter().map(|x| x.first().unwrap()).join(", ")
            )
        });

        result.push(head.unwrap());
        bases = new_bases;
    }
    Ok(result)
}

pub fn new(
    typ: PyTypeRef,
    name: &str,
    base: PyTypeRef,
    bases: Vec<PyTypeRef>,
    attrs: HashMap<String, PyObjectRef>,
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
            new_type.update_slot(attr_name);
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

fn best_base<'a>(bases: &'a [PyTypeRef], vm: &VirtualMachine) -> PyResult<PyTypeRef> {
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
    use super::{linearise_mro, new};
    use super::{HashMap, IdProtocol, PyContext, PyTypeRef};

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
            HashMap::new(),
            Default::default(),
        )
        .unwrap();
        let b = new(
            type_type.clone(),
            "B",
            object.clone(),
            vec![object.clone()],
            HashMap::new(),
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

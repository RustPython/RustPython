use crate::common::cell::PyRwLock;
use std::collections::{HashMap, HashSet};
use std::fmt;

use super::objclassmethod::PyClassMethod;
use super::objdict::PyDictRef;
use super::objlist::PyList;
use super::objmappingproxy::PyMappingProxy;
use super::objobject;
use super::objstaticmethod::PyStaticMethod;
use super::objstr::PyStringRef;
use super::objtuple::PyTuple;
use super::objweakref::PyWeak;
use crate::function::{KwArgs, OptionalArg, PyFuncArgs};
use crate::pyobject::{
    BorrowValue, IdProtocol, PyAttributes, PyClassImpl, PyContext, PyIterable, PyLease,
    PyObjectRef, PyRef, PyResult, PyValue, TypeProtocol,
};
use crate::slots::{PyClassSlots, PyTpFlags};
use crate::vm::VirtualMachine;
use itertools::Itertools;
use std::ops::Deref;

/// type(object_or_name, bases, dict)
/// type(object) -> the object's type
/// type(name, bases, dict) -> a new type
#[pyclass(name = "type")]
pub struct PyClass {
    pub name: String,
    pub bases: Vec<PyClassRef>,
    pub mro: Vec<PyClassRef>,
    pub subclasses: PyRwLock<Vec<PyWeak>>,
    pub attributes: PyRwLock<PyAttributes>,
    pub slots: PyRwLock<PyClassSlots>,
}

impl fmt::Display for PyClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.name, f)
    }
}

impl fmt::Debug for PyClass {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "[PyClass {}]", &self.name)
    }
}

pub type PyClassRef = PyRef<PyClass>;

impl PyValue for PyClass {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.type_type()
    }
}

#[pyimpl(flags(BASETYPE))]
impl PyClassRef {
    pub fn iter_mro(&self) -> impl Iterator<Item = &PyClassRef> + DoubleEndedIterator {
        std::iter::once(self).chain(self.mro.iter())
    }

    #[pyproperty(name = "__mro__")]
    fn get_mro(self) -> PyTuple {
        let elements: Vec<PyObjectRef> = self.iter_mro().map(|x| x.as_object().clone()).collect();
        PyTuple::from(elements)
    }

    #[pyproperty(magic)]
    fn bases(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx
            .new_tuple(self.bases.iter().map(|x| x.as_object().clone()).collect())
    }

    #[pymethod(magic)]
    fn dir(self, vm: &VirtualMachine) -> PyList {
        let attributes: Vec<PyObjectRef> = self
            .get_attributes()
            .drain()
            .map(|(k, _)| vm.ctx.new_str(k))
            .collect();
        PyList::from(attributes)
    }

    #[pymethod(magic)]
    fn instancecheck(self, obj: PyObjectRef) -> bool {
        isinstance(&obj, &self)
    }

    #[pymethod(magic)]
    fn subclasscheck(self, subclass: PyClassRef) -> bool {
        issubclass(&subclass, &self)
    }

    #[pyproperty(magic)]
    fn name(self) -> String {
        self.name.clone()
    }

    #[pymethod(magic)]
    fn repr(self) -> String {
        format!("<class '{}'>", self.name)
    }

    #[pyproperty(magic)]
    fn qualname(self, vm: &VirtualMachine) -> PyObjectRef {
        self.attributes
            .read()
            .get("__qualname__")
            .cloned()
            .unwrap_or_else(|| vm.ctx.new_str(self.name.clone()))
    }

    #[pyproperty(magic)]
    fn module(self, vm: &VirtualMachine) -> PyObjectRef {
        // TODO: Implement getting the actual module a builtin type is from
        self.attributes
            .read()
            .get("__module__")
            .cloned()
            .unwrap_or_else(|| vm.ctx.new_str("builtins"))
    }

    #[pyproperty(magic, setter)]
    fn set_module(self, value: PyObjectRef) {
        self.attributes
            .write()
            .insert("__module__".to_owned(), value);
    }

    #[pymethod(magic)]
    fn prepare(_name: PyStringRef, _bases: PyObjectRef, vm: &VirtualMachine) -> PyDictRef {
        vm.ctx.new_dict()
    }

    #[pymethod(magic)]
    fn getattribute(self, name_ref: PyStringRef, vm: &VirtualMachine) -> PyResult {
        let name = name_ref.borrow_value();
        vm_trace!("type.__getattribute__({:?}, {:?})", self, name);
        let mcl = self.lease_class();

        if let Some(attr) = mcl.get_attr(&name) {
            let attr_class = attr.lease_class();
            if attr_class.has_attr("__set__") {
                if let Some(ref descriptor) = attr_class.get_attr("__get__") {
                    drop(attr_class);
                    let mcl = PyLease::into_pyref(mcl).into_object();
                    return vm.invoke(descriptor, vec![attr, self.into_object(), mcl]);
                }
            }
        }

        if let Some(attr) = self.get_attr(&name) {
            let attr_class = attr.class();
            let slots = attr_class.slots.read();
            if let Some(ref descr_get) = slots.descr_get {
                drop(mcl);
                return descr_get(vm, attr, None, OptionalArg::Present(self.into_object()));
            } else if let Some(ref descriptor) = attr_class.get_attr("__get__") {
                drop(mcl);
                // TODO: is this nessessary?
                return vm.invoke(descriptor, vec![attr, vm.get_none(), self.into_object()]);
            }
        }

        if let Some(cls_attr) = self.get_attr(&name) {
            Ok(cls_attr)
        } else if let Some(attr) = mcl.get_attr(&name) {
            drop(mcl);
            vm.call_if_get_descriptor(attr, self.into_object())
        } else if let Some(ref getter) = self.get_attr("__getattr__") {
            vm.invoke(
                getter,
                vec![
                    PyLease::into_pyref(mcl).into_object(),
                    name_ref.into_object(),
                ],
            )
        } else {
            Err(vm.new_attribute_error(format!(
                "type object '{}' has no attribute '{}'",
                self, name
            )))
        }
    }

    #[pymethod(magic)]
    fn setattr(
        self,
        attr_name: PyStringRef,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        if let Some(attr) = self.get_class_attr(attr_name.borrow_value()) {
            if let Some(ref descriptor) = attr.get_class_attr("__set__") {
                vm.invoke(descriptor, vec![attr, self.into_object(), value])?;
                return Ok(());
            }
        }

        self.attributes.write().insert(attr_name.to_string(), value);
        Ok(())
    }

    #[pymethod(magic)]
    fn delattr(self, attr_name: PyStringRef, vm: &VirtualMachine) -> PyResult<()> {
        if let Some(attr) = self.get_class_attr(attr_name.borrow_value()) {
            if let Some(ref descriptor) = attr.get_class_attr("__delete__") {
                return vm
                    .invoke(descriptor, vec![attr, self.into_object()])
                    .map(|_| ());
            }
        }

        if self.get_attr(attr_name.borrow_value()).is_some() {
            self.attributes.write().remove(attr_name.borrow_value());
            Ok(())
        } else {
            Err(vm.new_attribute_error(attr_name.borrow_value().to_owned()))
        }
    }

    // This is used for class initialisation where the vm is not yet available.
    pub fn set_str_attr<V: Into<PyObjectRef>>(&self, attr_name: &str, value: V) {
        self.attributes
            .write()
            .insert(attr_name.to_owned(), value.into());
    }

    #[pymethod(magic)]
    fn subclasses(self) -> PyList {
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
    fn mro(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_list(
            self.iter_mro()
                .map(|cls| cls.clone().into_object())
                .collect(),
        )
    }
    #[pyslot]
    fn tp_new(metatype: PyClassRef, args: PyFuncArgs, vm: &VirtualMachine) -> PyResult {
        vm_trace!("type.__new__ {:?}", args);

        let is_type_type = metatype.is(&vm.ctx.types.type_type);
        if is_type_type && args.args.len() == 1 && args.kwargs.is_empty() {
            return Ok(args.args[0].class().into_object());
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

        let (name, bases, dict, kwargs): (PyStringRef, PyIterable<PyClassRef>, PyDictRef, KwArgs) =
            args.clone().bind(vm)?;

        let bases: Vec<PyClassRef> = bases.iter(vm)?.collect::<Result<Vec<_>, _>>()?;
        let (metatype, base, bases) = if bases.is_empty() {
            let base = vm.ctx.object();
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
                {
                    if let Some(ref tp_new) = winner.clone().slots.read().new {
                        // Pass it to the winner

                        return tp_new(vm, args.insert(winner.into_object()));
                    }
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
            if f.class().is(&vm.ctx.function_type()) {
                *f = PyStaticMethod::from(f.clone()).into_ref(vm).into_object();
            }
        }

        if let Some(f) = attributes.get_mut("__init_subclass__") {
            if f.class().is(&vm.ctx.function_type()) {
                *f = PyClassMethod::from(f.clone()).into_ref(vm).into_object();
            }
        }

        if !attributes.contains_key("__dict__") {
            attributes.insert(
                "__dict__".to_owned(),
                vm.ctx.new_getset(
                    "__dict__",
                    objobject::object_get_dict,
                    objobject::object_set_dict,
                ),
            );
        }

        let slots = PyClassSlots {
            // TODO: how do we know if it should have a dict?
            flags: base.slots.read().flags | PyTpFlags::HAS_DICT,
            ..Default::default()
        };

        // TODO: is this correct behavior?
        let cls_dict = if metatype.is(&vm.ctx.types.type_type) {
            None
        } else {
            Some(vm.ctx.new_dict())
        };

        let typ = new(
            metatype,
            name.borrow_value(),
            base,
            bases,
            attributes,
            slots,
            cls_dict,
        )
        .map_err(|e| vm.new_type_error(e))?;

        vm.ctx.add_tp_new_wrapper(&typ);

        for (name, obj) in typ.attributes.read().clone().iter() {
            if let Some(meth) = vm.get_method(obj.clone(), "__set_name__") {
                let set_name = meth?;
                vm.invoke(
                    &set_name,
                    vec![typ.clone().into_object(), vm.ctx.new_str(name.clone())],
                )
                .map_err(|e| {
                    let err = vm.new_runtime_error(format!(
                        "Error calling __set_name__ on '{}' instance {} in '{}'",
                        obj.lease_class().name,
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

    #[pyslot]
    #[pymethod(magic)]
    fn call(self, args: PyFuncArgs, vm: &VirtualMachine) -> PyResult {
        vm_trace!("type_call: {:?}", self);
        let obj = call_tp_new(self.clone(), self.clone(), args.clone(), vm)?;

        if (self.is(&vm.ctx.types.type_type) && args.kwargs.is_empty()) || !isinstance(&obj, &self)
        {
            return Ok(obj);
        }

        if let Some(init_method_or_err) = vm.get_method(obj.clone(), "__init__") {
            let init_method = init_method_or_err?;
            let res = vm.invoke(&init_method, args)?;
            if !res.is(&vm.get_none()) {
                return Err(vm.new_type_error("__init__ must return None".to_owned()));
            }
        }
        Ok(obj)
    }

    #[pyproperty(magic)]
    fn dict(self) -> PyMappingProxy {
        PyMappingProxy::new(self)
    }

    #[pyproperty(magic, setter)]
    fn set_dict(self, _value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        Err(vm.new_not_implemented_error(
            "Setting __dict__ attribute on a type isn't yet implemented".to_owned(),
        ))
    }
}

/*
 * The magical type type
 */

pub(crate) fn init(ctx: &PyContext) {
    PyClassRef::extend_class(ctx, &ctx.types.type_type);
}

pub trait DerefToPyClass {
    fn deref_to_class(&self) -> &PyClass;
}

impl DerefToPyClass for PyClassRef {
    fn deref_to_class(&self) -> &PyClass {
        self.deref()
    }
}

impl<'a> DerefToPyClass for PyLease<'a, PyClass> {
    fn deref_to_class(&self) -> &PyClass {
        self.deref()
    }
}

impl<T: DerefToPyClass> DerefToPyClass for &'_ T {
    fn deref_to_class(&self) -> &PyClass {
        (&**self).deref_to_class()
    }
}

/// Determines if `obj` actually an instance of `cls`, this doesn't call __instancecheck__, so only
/// use this if `cls` is known to have not overridden the base __instancecheck__ magic method.
#[inline]
pub fn isinstance<T: TypeProtocol>(obj: &T, cls: &PyClassRef) -> bool {
    issubclass(obj.lease_class(), &cls)
}

/// Determines if `subclass` is actually a subclass of `cls`, this doesn't call __subclasscheck__,
/// so only use this if `cls` is known to have not overridden the base __subclasscheck__ magic
/// method.
pub fn issubclass<T: DerefToPyClass + IdProtocol, R: IdProtocol>(subclass: T, cls: R) -> bool {
    subclass.is(&cls) || subclass.deref_to_class().mro.iter().any(|c| c.is(&cls))
}

fn call_tp_new(
    typ: PyClassRef,
    subtype: PyClassRef,
    args: PyFuncArgs,
    vm: &VirtualMachine,
) -> PyResult {
    for cls in typ.iter_mro() {
        if let Some(new_meth) = cls.get_attr("__new__") {
            if !vm.ctx.is_tp_new_wrapper(&new_meth) {
                let new_meth = vm.call_if_get_descriptor(new_meth, typ.clone().into_object())?;
                return vm.invoke(&new_meth, args.insert(typ.clone().into_object()));
            }
        }
    }
    let class_with_new_slot = typ
        .iter_mro()
        .cloned()
        .find(|cls| cls.slots.read().new.is_some())
        .expect("Should be able to find a new slot somewhere in the mro");
    let slots = class_with_new_slot.slots.read();
    let new_slot = slots.new.as_ref().unwrap();
    new_slot(vm, args.insert(subtype.into_object()))
}

pub fn tp_new_wrapper(
    zelf: PyClassRef,
    cls: PyClassRef,
    args: PyFuncArgs,
    vm: &VirtualMachine,
) -> PyResult {
    if !issubclass(&cls, &zelf) {
        return Err(vm.new_type_error(format!(
            "{zelf}.__new__({cls}): {cls} is not a subtype of {zelf}",
            zelf = zelf.name,
            cls = cls.name,
        )));
    }
    call_tp_new(zelf, cls, args, vm)
}

impl PyClass {
    /// This is the internal get_attr implementation for fast lookup on a class.
    pub fn get_attr(&self, attr_name: &str) -> Option<PyObjectRef> {
        flame_guard!(format!("class_get_attr({:?})", attr_name));

        self.attributes
            .read()
            .get(attr_name)
            .cloned()
            .or_else(|| self.get_super_attr(attr_name))
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
}

impl PyClassRef {
    pub fn get_attributes(self) -> PyAttributes {
        // Gather all members here:
        let mut attributes = PyAttributes::new();

        for bc in self.iter_mro().rev() {
            for (name, value) in bc.attributes.read().clone().iter() {
                attributes.insert(name.to_owned(), value.clone());
            }
        }

        attributes
    }
}

fn take_next_base(mut bases: Vec<Vec<PyClassRef>>) -> (Option<PyClassRef>, Vec<Vec<PyClassRef>>) {
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

fn linearise_mro(mut bases: Vec<Vec<PyClassRef>>) -> Result<Vec<PyClassRef>, String> {
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
        if head.is_none() {
            // Take the head class of each class here. Now that we have reached the problematic bases.
            // Because this failed, we assume the lists cannot be empty.
            return Err(format!(
                "Cannot create a consistent method resolution order (MRO) for bases {}",
                new_bases.iter().map(|x| x.first().unwrap()).join(", ")
            ));
        }

        result.push(head.unwrap());
        bases = new_bases;
    }
    Ok(result)
}

pub fn new(
    typ: PyClassRef,
    name: &str,
    base: PyClassRef,
    bases: Vec<PyClassRef>,
    attrs: HashMap<String, PyObjectRef>,
    mut slots: PyClassSlots,
    dict: Option<PyDictRef>,
) -> Result<PyClassRef, String> {
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
    if base.slots.read().flags.has_feature(PyTpFlags::HAS_DICT) {
        slots.flags |= PyTpFlags::HAS_DICT
    }
    let new_type = PyRef::new_ref(
        PyClass {
            name: String::from(name),
            bases,
            mro,
            subclasses: PyRwLock::default(),
            attributes: PyRwLock::new(attrs),
            slots: PyRwLock::new(slots),
        },
        typ,
        dict,
    );

    for base in &new_type.bases {
        base.subclasses
            .write()
            .push(PyWeak::downgrade(new_type.as_object()));
    }

    Ok(new_type)
}

fn calculate_meta_class(
    metatype: PyClassRef,
    bases: &[PyClassRef],
    vm: &VirtualMachine,
) -> PyResult<PyClassRef> {
    // = _PyType_CalculateMetaclass
    let mut winner = metatype;
    for base in bases {
        let base_type = base.lease_class();
        if issubclass(&winner, &base_type) {
            continue;
        } else if issubclass(&base_type, &winner) {
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

fn best_base<'a>(bases: &'a [PyClassRef], vm: &VirtualMachine) -> PyResult<PyClassRef> {
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

        if !base_i.slots.read().flags.has_feature(PyTpFlags::BASETYPE) {
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
    use super::{HashMap, IdProtocol, PyClassRef, PyContext};

    fn map_ids(obj: Result<Vec<PyClassRef>, String>) -> Result<Vec<usize>, String> {
        Ok(obj?.into_iter().map(|x| x.get_id()).collect())
    }

    #[test]
    fn test_linearise() {
        let context = PyContext::new();
        let object: PyClassRef = context.object();
        let type_type = &context.types.type_type;

        let a = new(
            type_type.clone(),
            "A",
            object.clone(),
            vec![object.clone()],
            HashMap::new(),
            Default::default(),
            None,
        )
        .unwrap();
        let b = new(
            type_type.clone(),
            "B",
            object.clone(),
            vec![object.clone()],
            HashMap::new(),
            Default::default(),
            None,
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

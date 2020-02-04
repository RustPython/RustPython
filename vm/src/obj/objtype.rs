use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;

use super::objdict::PyDictRef;
use super::objlist::PyList;
use super::objmappingproxy::PyMappingProxy;
use super::objproperty::PropertyBuilder;
use super::objstr::PyStringRef;
use super::objtuple::PyTuple;
use super::objweakref::PyWeak;
use crate::function::{OptionalArg, PyFuncArgs};
use crate::pyobject::{
    IdProtocol, PyAttributes, PyClassImpl, PyContext, PyIterable, PyObject, PyObjectRef, PyRef,
    PyResult, PyValue, TypeProtocol,
};
use crate::slots::{PyClassSlots, PyTpFlags};
use crate::vm::VirtualMachine;

/// type(object_or_name, bases, dict)
/// type(object) -> the object's type
/// type(name, bases, dict) -> a new type
#[pyclass(name = "type")]
#[derive(Debug)]
pub struct PyClass {
    pub name: String,
    pub bases: Vec<PyClassRef>,
    pub mro: Vec<PyClassRef>,
    pub subclasses: RefCell<Vec<PyWeak>>,
    pub attributes: RefCell<PyAttributes>,
    pub slots: RefCell<PyClassSlots>,
}

impl fmt::Display for PyClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.name, f)
    }
}

pub type PyClassRef = PyRef<PyClass>;

impl PyValue for PyClass {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.type_type()
    }
}

struct IterMro<'a> {
    cls: &'a PyClassRef,
    offset: Option<usize>,
}

impl<'a> Iterator for IterMro<'a> {
    type Item = &'a PyClassRef;

    fn next(&mut self) -> Option<Self::Item> {
        match self.offset {
            None => {
                self.offset = Some(0);
                Some(&self.cls)
            }
            Some(offset) => {
                if offset < self.cls.mro.len() {
                    self.offset = Some(offset + 1);
                    Some(&self.cls.mro[offset])
                } else {
                    None
                }
            }
        }
    }
}

#[pyimpl(flags(BASETYPE))]
impl PyClassRef {
    fn iter_mro(&self) -> IterMro {
        IterMro {
            cls: self,
            offset: None,
        }
    }

    fn _mro(self, _vm: &VirtualMachine) -> PyTuple {
        let elements: Vec<PyObjectRef> =
            _mro(&self).iter().map(|x| x.as_object().clone()).collect();
        PyTuple::from(elements)
    }

    fn _set_mro(self, _value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        Err(vm.new_attribute_error("read-only attribute".to_owned()))
    }

    #[pyproperty(magic)]
    fn bases(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx
            .new_tuple(self.bases.iter().map(|x| x.as_object().clone()).collect())
    }

    #[pymethod(magic)]
    fn dir(self, vm: &VirtualMachine) -> PyList {
        let attributes = self.get_attributes();
        let attributes: Vec<PyObjectRef> = attributes
            .keys()
            .map(|k| vm.ctx.new_str(k.to_owned()))
            .collect();
        PyList::from(attributes)
    }

    #[pymethod(magic)]
    fn instancecheck(self, obj: PyObjectRef, _vm: &VirtualMachine) -> bool {
        isinstance(&obj, &self)
    }

    #[pymethod(magic)]
    fn subclasscheck(self, subclass: PyClassRef, _vm: &VirtualMachine) -> bool {
        issubclass(&subclass, &self)
    }

    #[pyproperty(magic)]
    fn name(self, _vm: &VirtualMachine) -> String {
        self.name.clone()
    }

    #[pymethod(magic)]
    fn repr(self, _vm: &VirtualMachine) -> String {
        format!("<class '{}'>", self.name)
    }

    #[pyproperty(magic)]
    fn qualname(self, vm: &VirtualMachine) -> PyObjectRef {
        self.attributes
            .borrow()
            .get("__qualname__")
            .cloned()
            .unwrap_or_else(|| vm.ctx.new_str(self.name.clone()))
    }

    #[pyproperty(magic)]
    fn module(self, vm: &VirtualMachine) -> PyObjectRef {
        // TODO: Implement getting the actual module a builtin type is from
        self.attributes
            .borrow()
            .get("__module__")
            .cloned()
            .unwrap_or_else(|| vm.ctx.new_str("builtins".to_owned()))
    }

    #[pymethod(magic)]
    fn prepare(_name: PyStringRef, _bases: PyObjectRef, vm: &VirtualMachine) -> PyDictRef {
        vm.ctx.new_dict()
    }

    #[pymethod(magic)]
    fn getattribute(self, name_ref: PyStringRef, vm: &VirtualMachine) -> PyResult {
        let name = name_ref.as_str();
        vm_trace!("type.__getattribute__({:?}, {:?})", self, name);
        let mcl = self.class();

        if let Some(attr) = mcl.get_attr(&name) {
            let attr_class = attr.class();
            if attr_class.has_attr("__set__") {
                if let Some(ref descriptor) = attr_class.get_attr("__get__") {
                    return vm.invoke(
                        descriptor,
                        vec![attr, self.into_object(), mcl.into_object()],
                    );
                }
            }
        }

        if let Some(attr) = self.get_attr(&name) {
            let attr_class = attr.class();
            let slots = attr_class.slots.borrow();
            if let Some(ref descr_get) = slots.descr_get {
                return descr_get(vm, attr, None, OptionalArg::Present(self.into_object()));
            } else if let Some(ref descriptor) = attr_class.get_attr("__get__") {
                // TODO: is this nessessary?
                return vm.invoke(descriptor, vec![attr, vm.get_none(), self.into_object()]);
            }
        }

        if let Some(cls_attr) = self.get_attr(&name) {
            Ok(cls_attr)
        } else if let Some(attr) = mcl.get_attr(&name) {
            vm.call_get_descriptor(attr, self.into_object())
        } else if let Some(ref getter) = self.get_attr("__getattr__") {
            vm.invoke(getter, vec![mcl.into_object(), name_ref.into_object()])
        } else {
            Err(vm.new_attribute_error(format!("{} has no attribute '{}'", self, name)))
        }
    }

    #[pymethod(magic)]
    fn setattr(
        self,
        attr_name: PyStringRef,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        if let Some(attr) = self.class().get_attr(attr_name.as_str()) {
            if let Some(ref descriptor) = attr.class().get_attr("__set__") {
                vm.invoke(descriptor, vec![attr, self.into_object(), value])?;
                return Ok(());
            }
        }

        self.attributes
            .borrow_mut()
            .insert(attr_name.to_string(), value);
        Ok(())
    }

    #[pymethod(magic)]
    fn delattr(self, attr_name: PyStringRef, vm: &VirtualMachine) -> PyResult<()> {
        if let Some(attr) = self.class().get_attr(attr_name.as_str()) {
            if let Some(ref descriptor) = attr.class().get_attr("__delete__") {
                return vm
                    .invoke(descriptor, vec![attr, self.into_object()])
                    .map(|_| ());
            }
        }

        if self.get_attr(attr_name.as_str()).is_some() {
            self.attributes.borrow_mut().remove(attr_name.as_str());
            Ok(())
        } else {
            Err(vm.new_attribute_error(attr_name.as_str().to_owned()))
        }
    }

    // This is used for class initialisation where the vm is not yet available.
    pub fn set_str_attr<V: Into<PyObjectRef>>(&self, attr_name: &str, value: V) {
        self.attributes
            .borrow_mut()
            .insert(attr_name.to_owned(), value.into());
    }

    #[pymethod(magic)]
    fn subclasses(self, _vm: &VirtualMachine) -> PyList {
        let mut subclasses = self.subclasses.borrow_mut();
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
        let mut mro = vec![self.clone().into_object()];
        mro.extend(self.mro.iter().map(|x| x.clone().into_object()));
        vm.ctx.new_list(mro)
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

        let (name, bases, dict): (PyStringRef, PyIterable<PyClassRef>, PyDictRef) =
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
                if let Some(ref tp_new) = winner.clone().slots.borrow().new {
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

        let attributes = dict.to_attributes();
        let typ = new(metatype, name.as_str(), base.clone(), bases, attributes)?;
        typ.slots.borrow_mut().flags = base.slots.borrow().flags;
        Ok(typ.into())
    }

    #[pyslot]
    #[pymethod(magic)]
    fn call(self, args: PyFuncArgs, vm: &VirtualMachine) -> PyResult {
        vm_trace!("type_call: {:?}", self);
        let new = vm.get_attribute(self.as_object().clone(), "__new__")?;
        let new_args = args.insert(self.into_object());
        let obj = vm.invoke(&new, new_args)?;

        if let Some(init_method_or_err) = vm.get_method(obj.clone(), "__init__") {
            let init_method = init_method_or_err?;
            let res = vm.invoke(&init_method, args)?;
            if !res.is(&vm.get_none()) {
                return Err(vm.new_type_error("__init__ must return None".to_owned()));
            }
        }
        Ok(obj)
    }
}

/*
 * The magical type type
 */

pub(crate) fn init(ctx: &PyContext) {
    PyClassRef::extend_class(ctx, &ctx.types.type_type);
    extend_class!(&ctx, &ctx.types.type_type, {
        "__dict__" =>
        PropertyBuilder::new(ctx)
                .add_getter(type_dict)
                .add_setter(type_dict_setter)
                .create(),
        "__mro__" =>
            PropertyBuilder::new(ctx)
                .add_getter(PyClassRef::_mro)
                .add_setter(PyClassRef::_set_mro)
                .create(),
    });
}

fn _mro(cls: &PyClassRef) -> Vec<PyClassRef> {
    cls.iter_mro().cloned().collect()
}

/// Determines if `obj` actually an instance of `cls`, this doesn't call __instancecheck__, so only
/// use this if `cls` is known to have not overridden the base __instancecheck__ magic method.
#[inline]
pub fn isinstance<T: TypeProtocol>(obj: &T, cls: &PyClassRef) -> bool {
    issubclass(&obj.class(), &cls)
}

/// Determines if `subclass` is actually a subclass of `cls`, this doesn't call __subclasscheck__,
/// so only use this if `cls` is known to have not overridden the base __subclasscheck__ magic
/// method.
pub fn issubclass(subclass: &PyClassRef, cls: &PyClassRef) -> bool {
    let mro = &subclass.mro;
    subclass.is(cls) || mro.iter().any(|c| c.is(cls.as_object()))
}

pub fn type_new(
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

    let class_with_new_slot = if cls.slots.borrow().new.is_some() {
        cls.clone()
    } else {
        cls.mro
            .iter()
            .cloned()
            .find(|cls| cls.slots.borrow().new.is_some())
            .expect("Should be able to find a new slot somewhere in the mro")
    };

    let slots = class_with_new_slot.slots.borrow();
    let new = slots.new.as_ref().unwrap();

    new(vm, args.insert(cls.into_object()))
}

fn type_dict(class: PyClassRef, _vm: &VirtualMachine) -> PyMappingProxy {
    PyMappingProxy::new(class)
}

fn type_dict_setter(
    _instance: PyClassRef,
    _value: PyObjectRef,
    vm: &VirtualMachine,
) -> PyResult<()> {
    Err(vm.new_not_implemented_error(
        "Setting __dict__ attribute on a type isn't yet implemented".to_owned(),
    ))
}

impl PyClassRef {
    /// This is the internal get_attr implementation for fast lookup on a class.
    pub fn get_attr(&self, attr_name: &str) -> Option<PyObjectRef> {
        flame_guard!(format!("class_get_attr({:?})", attr_name));

        self.attributes
            .borrow()
            .get(attr_name)
            .cloned()
            .or_else(|| self.get_super_attr(attr_name))
    }

    pub fn get_super_attr(&self, attr_name: &str) -> Option<PyObjectRef> {
        self.mro
            .iter()
            .find_map(|class| class.attributes.borrow().get(attr_name).cloned())
    }

    // This is the internal has_attr implementation for fast lookup on a class.
    pub fn has_attr(&self, attr_name: &str) -> bool {
        self.attributes.borrow().contains_key(attr_name)
            || self
                .mro
                .iter()
                .any(|c| c.attributes.borrow().contains_key(attr_name))
    }

    pub fn get_attributes(self) -> PyAttributes {
        // Gather all members here:
        let mut attributes = PyAttributes::new();

        let mut base_classes: Vec<&PyClassRef> = self.iter_mro().collect();
        base_classes.reverse();

        for bc in base_classes {
            for (name, value) in bc.attributes.borrow().iter() {
                attributes.insert(name.to_owned(), value.clone());
            }
        }

        attributes
    }
}

fn take_next_base(mut bases: Vec<Vec<PyClassRef>>) -> Option<(PyClassRef, Vec<Vec<PyClassRef>>)> {
    let mut next = None;

    bases = bases.into_iter().filter(|x| !x.is_empty()).collect();

    for base in &bases {
        let head = base[0].clone();
        if !(&bases).iter().any(|x| x[1..].iter().any(|x| x.is(&head))) {
            next = Some(head);
            break;
        }
    }

    if let Some(head) = next {
        for item in &mut bases {
            if item[0].is(&head) {
                item.remove(0);
            }
        }
        return Some((head, bases));
    }
    None
}

fn linearise_mro(mut bases: Vec<Vec<PyClassRef>>) -> Option<Vec<PyClassRef>> {
    vm_trace!("Linearising MRO: {:?}", bases);
    let mut result = vec![];
    loop {
        if (&bases).iter().all(Vec::is_empty) {
            break;
        }
        let (head, new_bases) = take_next_base(bases)?;

        result.push(head);
        bases = new_bases;
    }
    Some(result)
}

pub fn new(
    typ: PyClassRef,
    name: &str,
    _base: PyClassRef,
    bases: Vec<PyClassRef>,
    dict: HashMap<String, PyObjectRef>,
) -> PyResult<PyClassRef> {
    let mros = bases.iter().map(|x| _mro(&x)).collect();
    let mro = linearise_mro(mros).unwrap();
    let new_type = PyObject {
        payload: PyClass {
            name: String::from(name),
            bases,
            mro,
            subclasses: RefCell::default(),
            attributes: RefCell::new(dict),
            slots: RefCell::default(),
        },
        dict: None,
        typ,
    }
    .into_ref();

    let new_type: PyClassRef = new_type.downcast().unwrap();

    for base in &new_type.bases {
        base.subclasses
            .borrow_mut()
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
        let base_type = base.class();
        if issubclass(&winner, &base_type) {
            continue;
        } else if issubclass(&base_type, &winner) {
            winner = base_type.clone();
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

        if !base_i.slots.borrow().flags.has_feature(PyTpFlags::BASETYPE) {
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

    fn map_ids(obj: Option<Vec<PyClassRef>>) -> Option<Vec<usize>> {
        match obj {
            Some(vec) => Some(vec.into_iter().map(|x| x.get_id()).collect()),
            None => None,
        }
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
        )
        .unwrap();
        let b = new(
            type_type.clone(),
            "B",
            object.clone(),
            vec![object.clone()],
            HashMap::new(),
        )
        .unwrap();

        assert_eq!(
            map_ids(linearise_mro(vec![
                vec![object.clone()],
                vec![object.clone()]
            ])),
            map_ids(Some(vec![object.clone()]))
        );
        assert_eq!(
            map_ids(linearise_mro(vec![
                vec![a.clone(), object.clone()],
                vec![b.clone(), object.clone()],
            ])),
            map_ids(Some(vec![a.clone(), b.clone(), object.clone()]))
        );
    }
}

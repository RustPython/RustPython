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
use crate::function::{PyFuncArgs, PyNativeFunc};
use crate::pyobject::{
    IdProtocol, PyAttributes, PyContext, PyIterable, PyObject, PyObjectRef, PyRef, PyResult,
    PyValue, TypeProtocol,
};
use crate::vm::VirtualMachine;

#[derive(Debug)]
pub struct PyClass {
    pub name: String,
    pub bases: Vec<PyClassRef>,
    pub mro: Vec<PyClassRef>,
    pub subclasses: RefCell<Vec<PyWeak>>,
    pub attributes: RefCell<PyAttributes>,
    pub slots: RefCell<PyClassSlots>,
}

#[derive(Default)]
pub struct PyClassSlots {
    pub new: Option<PyNativeFunc>,
}
impl fmt::Debug for PyClassSlots {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PyClassSlots")
    }
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

impl PyClassRef {
    fn iter_mro(&self) -> IterMro {
        IterMro {
            cls: self,
            offset: None,
        }
    }

    fn mro(self, _vm: &VirtualMachine) -> PyTuple {
        let elements: Vec<PyObjectRef> =
            _mro(&self).iter().map(|x| x.as_object().clone()).collect();
        PyTuple::from(elements)
    }

    fn set_mro(self, _value: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_attribute_error("read-only attribute".to_string()))
    }

    fn bases(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx
            .new_tuple(self.bases.iter().map(|x| x.as_object().clone()).collect())
    }

    fn dir(self, vm: &VirtualMachine) -> PyList {
        let attributes = get_attributes(self);
        let attributes: Vec<PyObjectRef> = attributes
            .keys()
            .map(|k| vm.ctx.new_str(k.to_string()))
            .collect();
        PyList::from(attributes)
    }

    fn instance_check(self, obj: PyObjectRef, _vm: &VirtualMachine) -> bool {
        isinstance(&obj, &self)
    }

    fn subclass_check(self, subclass: PyClassRef, _vm: &VirtualMachine) -> bool {
        issubclass(&subclass, &self)
    }

    fn name(self, _vm: &VirtualMachine) -> String {
        self.name.clone()
    }

    fn repr(self, _vm: &VirtualMachine) -> String {
        format!("<class '{}'>", self.name)
    }

    fn qualname(self, vm: &VirtualMachine) -> PyObjectRef {
        self.attributes
            .borrow()
            .get("__qualname__")
            .cloned()
            .unwrap_or_else(|| vm.ctx.new_str(self.name.clone()))
    }

    fn module(self, vm: &VirtualMachine) -> PyObjectRef {
        // TODO: Implement getting the actual module a builtin type is from
        self.attributes
            .borrow()
            .get("__module__")
            .cloned()
            .unwrap_or_else(|| vm.ctx.new_str("builtins".to_owned()))
    }

    fn prepare(_name: PyStringRef, _bases: PyObjectRef, vm: &VirtualMachine) -> PyDictRef {
        vm.ctx.new_dict()
    }

    fn getattribute(self, name_ref: PyStringRef, vm: &VirtualMachine) -> PyResult {
        let name = name_ref.as_str();
        vm_trace!("type.__getattribute__({:?}, {:?})", self, name);
        let mcl = self.class();

        if let Some(attr) = class_get_attr(&mcl, &name) {
            let attr_class = attr.class();
            if class_has_attr(&attr_class, "__set__") {
                if let Some(ref descriptor) = class_get_attr(&attr_class, "__get__") {
                    return vm.invoke(
                        descriptor,
                        vec![attr, self.into_object(), mcl.into_object()],
                    );
                }
            }
        }

        if let Some(attr) = class_get_attr(&self, &name) {
            let attr_class = attr.class();
            if let Some(ref descriptor) = class_get_attr(&attr_class, "__get__") {
                return vm.invoke(descriptor, vec![attr, vm.get_none(), self.into_object()]);
            }
        }

        if let Some(cls_attr) = class_get_attr(&self, &name) {
            Ok(cls_attr)
        } else if let Some(attr) = class_get_attr(&mcl, &name) {
            vm.call_get_descriptor(attr, self.into_object())
        } else if let Some(ref getter) = class_get_attr(&self, "__getattr__") {
            vm.invoke(getter, vec![mcl.into_object(), name_ref.into_object()])
        } else {
            Err(vm.new_attribute_error(format!("{} has no attribute '{}'", self, name)))
        }
    }

    fn set_attr(
        self,
        attr_name: PyStringRef,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        if let Some(attr) = class_get_attr(&self.class(), attr_name.as_str()) {
            if let Some(ref descriptor) = class_get_attr(&attr.class(), "__set__") {
                vm.invoke(descriptor, vec![attr, self.into_object(), value])?;
                return Ok(());
            }
        }

        self.attributes
            .borrow_mut()
            .insert(attr_name.to_string(), value);
        Ok(())
    }

    fn del_attr(self, attr_name: PyStringRef, vm: &VirtualMachine) -> PyResult<()> {
        if let Some(attr) = class_get_attr(&self.class(), attr_name.as_str()) {
            if let Some(ref descriptor) = class_get_attr(&attr.class(), "__delete__") {
                return vm
                    .invoke(descriptor, vec![attr, self.into_object()])
                    .map(|_| ());
            }
        }

        if class_get_attr(&self, attr_name.as_str()).is_some() {
            self.attributes.borrow_mut().remove(attr_name.as_str());
            Ok(())
        } else {
            Err(vm.new_attribute_error(attr_name.as_str().to_string()))
        }
    }

    // This is used for class initialisation where the vm is not yet available.
    pub fn set_str_attr<V: Into<PyObjectRef>>(&self, attr_name: &str, value: V) {
        self.attributes
            .borrow_mut()
            .insert(attr_name.to_string(), value.into());
    }

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
}

fn type_mro(cls: PyClassRef, vm: &VirtualMachine) -> PyObjectRef {
    let mut mro = vec![cls.clone().into_object()];
    mro.extend(cls.mro.iter().map(|x| x.clone().into_object()));
    vm.ctx.new_list(mro)
}

/*
 * The magical type type
 */

pub fn init(ctx: &PyContext) {
    let type_doc = "type(object_or_name, bases, dict)\n\
                    type(object) -> the object's type\n\
                    type(name, bases, dict) -> a new type";

    extend_class!(&ctx, &ctx.types.type_type, {
        "mro" => ctx.new_rustfunc(type_mro),
        "__call__" => ctx.new_rustfunc(type_call),
        "__dict__" =>
        PropertyBuilder::new(ctx)
                .add_getter(type_dict)
                .add_setter(type_dict_setter)
                .create(),
        (slot new) => type_new_slot,
        "__mro__" =>
            PropertyBuilder::new(ctx)
                .add_getter(PyClassRef::mro)
                .add_setter(PyClassRef::set_mro)
                .create(),
        "__bases__" => ctx.new_property(PyClassRef::bases),
        "__name__" => ctx.new_property(PyClassRef::name),
        "__repr__" => ctx.new_rustfunc(PyClassRef::repr),
        "__qualname__" => ctx.new_property(PyClassRef::qualname),
        "__module__" => ctx.new_property(PyClassRef::module),
        "__prepare__" => ctx.new_rustfunc(PyClassRef::prepare),
        "__getattribute__" => ctx.new_rustfunc(PyClassRef::getattribute),
        "__setattr__" => ctx.new_rustfunc(PyClassRef::set_attr),
        "__delattr__" => ctx.new_rustfunc(PyClassRef::del_attr),
        "__subclasses__" => ctx.new_rustfunc(PyClassRef::subclasses),
        "__instancecheck__" => ctx.new_rustfunc(PyClassRef::instance_check),
        "__subclasscheck__" => ctx.new_rustfunc(PyClassRef::subclass_check),
        "__doc__" => ctx.new_str(type_doc.to_string()),
        "__dir__" => ctx.new_rustfunc(PyClassRef::dir),
    });
}

fn _mro(cls: &PyClassRef) -> Vec<PyClassRef> {
    cls.iter_mro().cloned().collect()
}

/// Determines if `obj` actually an instance of `cls`, this doesn't call __instancecheck__, so only
/// use this if `cls` is known to have not overridden the base __instancecheck__ magic method.
#[cfg_attr(feature = "flame-it", flame("objtype"))]
pub fn isinstance(obj: &PyObjectRef, cls: &PyClassRef) -> bool {
    issubclass(&obj.class(), &cls)
}

/// Determines if `subclass` is actually a subclass of `cls`, this doesn't call __subclasscheck__,
/// so only use this if `cls` is known to have not overridden the base __subclasscheck__ magic
/// method.
pub fn issubclass(subclass: &PyClassRef, cls: &PyClassRef) -> bool {
    let mro = &subclass.mro;
    subclass.is(cls) || mro.iter().any(|c| c.is(cls.as_object()))
}

fn type_new_slot(metatype: PyClassRef, args: PyFuncArgs, vm: &VirtualMachine) -> PyResult {
    vm_trace!("type.__new__ {:?}", args);

    if metatype.is(&vm.ctx.types.type_type) {
        if args.args.len() == 1 && args.kwargs.is_empty() {
            return Ok(args.args[0].class().into_object());
        }
        if args.args.len() != 3 {
            return Err(vm.new_type_error("type() takes 1 or 3 arguments".to_string()));
        }
    }

    let (name, bases, dict): (PyStringRef, PyIterable<PyClassRef>, PyDictRef) = args.bind(vm)?;

    let bases: Vec<PyClassRef> = bases.iter(vm)?.collect::<Result<Vec<_>, _>>()?;
    let bases = if bases.is_empty() {
        vec![vm.ctx.object()]
    } else {
        bases
    };

    let attributes = dict.to_attributes();

    let mut winner = metatype.clone();
    for base in &bases {
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
                .to_string(),
        ));
    }

    new(winner, name.as_str(), bases, attributes).map(Into::into)
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

pub fn type_call(class: PyClassRef, args: PyFuncArgs, vm: &VirtualMachine) -> PyResult {
    vm_trace!("type_call: {:?}", class);
    let new = vm.get_attribute(class.as_object().clone(), "__new__")?;
    let new_args = args.insert(class.into_object());
    let obj = vm.invoke(&new, new_args)?;

    if let Some(init_method_or_err) = vm.get_method(obj.clone(), "__init__") {
        let init_method = init_method_or_err?;
        let res = vm.invoke(&init_method, args)?;
        if !res.is(&vm.get_none()) {
            return Err(vm.new_type_error("__init__ must return None".to_string()));
        }
    }
    Ok(obj)
}

fn type_dict(class: PyClassRef, _vm: &VirtualMachine) -> PyMappingProxy {
    PyMappingProxy::new(class)
}

fn type_dict_setter(_instance: PyClassRef, _value: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    Err(vm.new_not_implemented_error(
        "Setting __dict__ attribute on a type isn't yet implemented".to_string(),
    ))
}

/// This is the internal get_attr implementation for fast lookup on a class.
pub fn class_get_attr(class: &PyClassRef, attr_name: &str) -> Option<PyObjectRef> {
    flame_guard!(format!("class_get_attr({:?})", attr_name));

    class
        .attributes
        .borrow()
        .get(attr_name)
        .cloned()
        .or_else(|| class_get_super_attr(class, attr_name))
}

pub fn class_get_super_attr(class: &PyClassRef, attr_name: &str) -> Option<PyObjectRef> {
    class
        .mro
        .iter()
        .find_map(|class| class.attributes.borrow().get(attr_name).cloned())
}

// This is the internal has_attr implementation for fast lookup on a class.
pub fn class_has_attr(class: &PyClassRef, attr_name: &str) -> bool {
    class.attributes.borrow().contains_key(attr_name)
        || class
            .mro
            .iter()
            .any(|c| c.attributes.borrow().contains_key(attr_name))
}

pub fn get_attributes(cls: PyClassRef) -> PyAttributes {
    // Gather all members here:
    let mut attributes = PyAttributes::new();

    let mut base_classes: Vec<&PyClassRef> = cls.iter_mro().collect();
    base_classes.reverse();

    for bc in base_classes {
        for (name, value) in bc.attributes.borrow().iter() {
            attributes.insert(name.to_string(), value.clone());
        }
    }

    attributes
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

        let a = new(type_type.clone(), "A", vec![object.clone()], HashMap::new()).unwrap();
        let b = new(type_type.clone(), "B", vec![object.clone()], HashMap::new()).unwrap();

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

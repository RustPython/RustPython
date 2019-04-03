use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;

use crate::function::{Args, KwArgs, PyFuncArgs};
use crate::pyobject::{
    IdProtocol, PyAttributes, PyContext, PyIterable, PyObject, PyObjectRef, PyRef, PyResult,
    PyValue, TypeProtocol,
};
use crate::vm::VirtualMachine;

use super::objdict::PyDictRef;
use super::objlist::PyList;
use super::objproperty::PropertyBuilder;
use super::objstr::PyStringRef;
use super::objtuple::PyTuple;
use super::objweakref::PyWeak;

#[derive(Debug)]
pub struct PyClass {
    pub name: String,
    pub mro: Vec<PyClassRef>,
    pub subclasses: RefCell<Vec<PyWeak>>,
    pub attributes: RefCell<PyAttributes>,
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

    fn repr(self, _vm: &VirtualMachine) -> String {
        format!("<class '{}'>", self.name)
    }

    fn prepare(_name: PyStringRef, _bases: PyObjectRef, vm: &VirtualMachine) -> PyDictRef {
        vm.ctx.new_dict()
    }

    fn getattribute(self, name_ref: PyStringRef, vm: &VirtualMachine) -> PyResult {
        let name = &name_ref.value;
        trace!("type.__getattribute__({:?}, {:?})", self, name);
        let mcl = self.class();

        if let Some(attr) = class_get_attr(&mcl, &name) {
            let attr_class = attr.class();
            if class_has_attr(&attr_class, "__set__") {
                if let Some(descriptor) = class_get_attr(&attr_class, "__get__") {
                    return vm.invoke(
                        descriptor,
                        vec![attr, self.into_object(), mcl.into_object()],
                    );
                }
            }
        }

        if let Some(attr) = class_get_attr(&self, &name) {
            let attr_class = attr.class();
            if let Some(descriptor) = class_get_attr(&attr_class, "__get__") {
                let none = vm.get_none();
                return vm.invoke(descriptor, vec![attr, none, self.into_object()]);
            }
        }

        if let Some(cls_attr) = class_get_attr(&self, &name) {
            Ok(cls_attr)
        } else if let Some(attr) = class_get_attr(&mcl, &name) {
            vm.call_get_descriptor(attr, self.into_object())
        } else if let Some(getter) = class_get_attr(&self, "__getattr__") {
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
        if let Some(attr) = class_get_attr(&self.class(), &attr_name.value) {
            if let Some(descriptor) = class_get_attr(&attr.class(), "__set__") {
                vm.invoke(descriptor, vec![attr, self.into_object(), value])?;
                return Ok(());
            }
        }

        self.attributes
            .borrow_mut()
            .insert(attr_name.to_string(), value);
        Ok(())
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

/*
 * The magical type type
 */

pub fn init(ctx: &PyContext) {
    let type_doc = "type(object_or_name, bases, dict)\n\
                    type(object) -> the object's type\n\
                    type(name, bases, dict) -> a new type";

    extend_class!(&ctx, &ctx.type_type, {
        "__call__" => ctx.new_rustfunc(type_call),
        "__new__" => ctx.new_rustfunc(type_new),
        "__mro__" =>
            PropertyBuilder::new(ctx)
                .add_getter(PyClassRef::mro)
                .add_setter(PyClassRef::set_mro)
                .create(),
        "__repr__" => ctx.new_rustfunc(PyClassRef::repr),
        "__prepare__" => ctx.new_rustfunc(PyClassRef::prepare),
        "__getattribute__" => ctx.new_rustfunc(PyClassRef::getattribute),
        "__setattr__" => ctx.new_rustfunc(PyClassRef::set_attr),
        "__subclasses__" => ctx.new_rustfunc(PyClassRef::subclasses),
        "__getattribute__" => ctx.new_rustfunc(PyClassRef::getattribute),
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

pub fn type_new(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    debug!("type.__new__ {:?}", args);
    if args.args.len() == 2 {
        Ok(args.args[1].class().into_object())
    } else if args.args.len() == 4 {
        let (typ, name, bases, dict) = args.bind(vm)?;
        type_new_class(vm, typ, name, bases, dict).map(|x| x.into_object())
    } else {
        Err(vm.new_type_error(format!(": type_new: {:?}", args)))
    }
}

pub fn type_new_class(
    vm: &VirtualMachine,
    typ: PyClassRef,
    name: PyStringRef,
    bases: PyIterable<PyClassRef>,
    dict: PyDictRef,
) -> PyResult<PyClassRef> {
    let mut bases: Vec<PyClassRef> = bases.iter(vm)?.collect::<Result<Vec<_>, _>>()?;
    bases.push(vm.ctx.object());
    new(typ.clone(), &name.value, bases, dict.to_attributes())
}

pub fn type_call(class: PyClassRef, args: Args, kwargs: KwArgs, vm: &VirtualMachine) -> PyResult {
    debug!("type_call: {:?}", class);
    let new = class_get_attr(&class, "__new__").expect("All types should have a __new__.");
    let new_wrapped = vm.call_get_descriptor(new, class.into_object())?;
    let obj = vm.invoke(new_wrapped, (&args, &kwargs))?;

    if let Ok(init) = vm.get_method(obj.clone(), "__init__") {
        let res = vm.invoke(init, (&args, &kwargs))?;
        if !res.is(&vm.get_none()) {
            return Err(vm.new_type_error("__init__ must return None".to_string()));
        }
    }
    Ok(obj)
}

// This is the internal get_attr implementation for fast lookup on a class.
pub fn class_get_attr(class: &PyClassRef, attr_name: &str) -> Option<PyObjectRef> {
    if let Some(item) = class.attributes.borrow().get(attr_name).cloned() {
        return Some(item);
    }
    for class in &class.mro {
        if let Some(item) = class.attributes.borrow().get(attr_name).cloned() {
            return Some(item);
        }
    }
    None
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
    debug!("Linearising MRO: {:?}", bases);
    let mut result = vec![];
    loop {
        if (&bases).iter().all(Vec::is_empty) {
            break;
        }
        match take_next_base(bases) {
            Some((head, new_bases)) => {
                result.push(head);
                bases = new_bases;
            }
            None => return None,
        }
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
            mro,
            subclasses: RefCell::new(vec![]),
            attributes: RefCell::new(dict),
        },
        dict: None,
        typ,
    }
    .into_ref();
    for base in bases {
        base.subclasses
            .borrow_mut()
            .push(PyWeak::downgrade(&new_type));
    }

    Ok(new_type.downcast().unwrap())
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
        let object: PyClassRef = context.object.clone();
        let type_type = &context.type_type;

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

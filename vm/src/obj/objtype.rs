use std::cell::RefCell;
use std::collections::HashMap;

use crate::function::PyFuncArgs;
use crate::pyobject::{
    AttributeProtocol, FromPyObjectRef, IdProtocol, PyAttributes, PyContext, PyObject, PyObjectRef,
    PyRef, PyResult, PyValue, TypeProtocol,
};
use crate::vm::VirtualMachine;

use super::objdict;
use super::objlist::PyList;
use super::objproperty::PropertyBuilder;
use super::objstr::{self, PyStringRef};
use super::objtuple::PyTuple;

#[derive(Clone, Debug)]
pub struct PyClass {
    pub name: String,
    pub mro: Vec<PyClassRef>,
}

pub type PyClassRef = PyRef<PyClass>;

impl PyValue for PyClass {
    fn class(vm: &mut VirtualMachine) -> PyObjectRef {
        vm.ctx.type_type()
    }
}

impl IdProtocol for PyClassRef {
    fn get_id(&self) -> usize {
        self.as_object().get_id()
    }
}

impl TypeProtocol for PyClassRef {
    fn type_ref(&self) -> &PyObjectRef {
        &self.as_object().type_ref()
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

    fn mro(self, _vm: &mut VirtualMachine) -> PyTuple {
        let elements: Vec<PyObjectRef> =
            _mro(&self).iter().map(|x| x.as_object().clone()).collect();
        PyTuple::from(elements)
    }

    fn set_mro(self, _value: PyObjectRef, vm: &mut VirtualMachine) -> PyResult {
        Err(vm.new_attribute_error("read-only attribute".to_string()))
    }

    fn dir(self, vm: &mut VirtualMachine) -> PyList {
        let attributes = get_attributes(self);
        let attributes: Vec<PyObjectRef> = attributes
            .keys()
            .map(|k| vm.ctx.new_str(k.to_string()))
            .collect();
        PyList::from(attributes)
    }

    fn instance_check(self, obj: PyObjectRef, _vm: &mut VirtualMachine) -> bool {
        isinstance(&obj, self.as_object())
    }

    fn subclass_check(self, subclass: PyObjectRef, _vm: &mut VirtualMachine) -> bool {
        issubclass(&subclass, self.as_object())
    }

    fn repr(self, _vm: &mut VirtualMachine) -> String {
        format!("<class '{}'>", self.name)
    }

    fn prepare(_name: PyStringRef, _bases: PyObjectRef, vm: &mut VirtualMachine) -> PyObjectRef {
        vm.new_dict()
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
        "__getattribute__" => ctx.new_rustfunc(type_getattribute),
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
pub fn isinstance(obj: &PyObjectRef, cls: &PyObjectRef) -> bool {
    issubclass(obj.type_ref(), &cls)
}

/// Determines if `subclass` is actually a subclass of `cls`, this doesn't call __subclasscheck__,
/// so only use this if `cls` is known to have not overridden the base __subclasscheck__ magic
/// method.
pub fn issubclass(subclass: &PyObjectRef, cls: &PyObjectRef) -> bool {
    let ref mro = subclass.payload::<PyClass>().unwrap().mro;
    subclass.is(cls) || mro.iter().any(|c| c.is(cls))
}

pub fn get_type_name(typ: &PyObjectRef) -> String {
    if let Some(PyClass { name, .. }) = &typ.payload::<PyClass>() {
        name.clone()
    } else {
        panic!("Cannot get type_name of non-type type {:?}", typ);
    }
}

pub fn type_new(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    debug!("type.__new__ {:?}", args);
    if args.args.len() == 2 {
        arg_check!(
            vm,
            args,
            required = [(_typ, Some(vm.ctx.type_type())), (obj, None)]
        );
        Ok(obj.typ())
    } else if args.args.len() == 4 {
        arg_check!(
            vm,
            args,
            required = [
                (typ, Some(vm.ctx.type_type())),
                (name, Some(vm.ctx.str_type())),
                (bases, None),
                (dict, Some(vm.ctx.dict_type()))
            ]
        );
        type_new_class(vm, typ, name, bases, dict)
    } else {
        Err(vm.new_type_error(format!(": type_new: {:?}", args)))
    }
}

pub fn type_new_class(
    vm: &mut VirtualMachine,
    typ: &PyObjectRef,
    name: &PyObjectRef,
    bases: &PyObjectRef,
    dict: &PyObjectRef,
) -> PyResult {
    let mut bases: Vec<PyClassRef> = vm
        .extract_elements(bases)?
        .iter()
        .map(|x| FromPyObjectRef::from_pyobj(x))
        .collect();
    bases.push(FromPyObjectRef::from_pyobj(&vm.ctx.object()));
    let name = objstr::get_value(name);
    new(
        typ.clone(),
        &name,
        bases,
        objdict::py_dict_to_attributes(dict),
    )
}

pub fn type_call(vm: &mut VirtualMachine, mut args: PyFuncArgs) -> PyResult {
    debug!("type_call: {:?}", args);
    let cls = args.shift();
    let new = cls.get_attr("__new__").unwrap();
    let new_wrapped = vm.call_get_descriptor(new, cls)?;
    let obj = vm.invoke(new_wrapped, args.clone())?;

    if let Ok(init) = vm.get_method(obj.clone(), "__init__") {
        let res = vm.invoke(init, args)?;
        if !res.is(&vm.get_none()) {
            return Err(vm.new_type_error("__init__ must return None".to_string()));
        }
    }
    Ok(obj)
}

pub fn type_getattribute(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [
            (cls, Some(vm.ctx.object())),
            (name_str, Some(vm.ctx.str_type()))
        ]
    );
    let name = objstr::get_value(&name_str);
    trace!("type.__getattribute__({:?}, {:?})", cls, name);
    let mcl = cls.typ();

    if let Some(attr) = mcl.get_attr(&name) {
        let attr_class = attr.typ();
        if attr_class.has_attr("__set__") {
            if let Some(descriptor) = attr_class.get_attr("__get__") {
                return vm.invoke(descriptor, vec![attr, cls.clone(), mcl]);
            }
        }
    }

    if let Some(attr) = cls.get_attr(&name) {
        let attr_class = attr.typ();
        if let Some(descriptor) = attr_class.get_attr("__get__") {
            let none = vm.get_none();
            return vm.invoke(descriptor, vec![attr, none, cls.clone()]);
        }
    }

    if let Some(cls_attr) = cls.get_attr(&name) {
        Ok(cls_attr)
    } else if let Some(attr) = mcl.get_attr(&name) {
        vm.call_get_descriptor(attr, cls.clone())
    } else if let Some(getter) = cls.get_attr("__getattr__") {
        vm.invoke(getter, vec![mcl, name_str.clone()])
    } else {
        Err(vm.new_attribute_error(format!("{} has no attribute '{}'", cls, name)))
    }
}

pub fn get_attributes(cls: PyClassRef) -> PyAttributes {
    // Gather all members here:
    let mut attributes = PyAttributes::new();

    let mut base_classes: Vec<&PyClassRef> = cls.iter_mro().collect();
    base_classes.reverse();

    for bc in base_classes {
        if let Some(ref dict) = &bc.as_object().dict {
            for (name, value) in dict.borrow().iter() {
                attributes.insert(name.to_string(), value.clone());
            }
        }
    }

    attributes
}

fn take_next_base(mut bases: Vec<Vec<PyClassRef>>) -> Option<(PyClassRef, Vec<Vec<PyClassRef>>)> {
    let mut next = None;

    bases = bases.into_iter().filter(|x| !x.is_empty()).collect();

    for base in &bases {
        let head = base[0].clone();
        if !(&bases)
            .iter()
            .any(|x| x[1..].iter().any(|x| x.get_id() == head.get_id()))
        {
            next = Some(head);
            break;
        }
    }

    if let Some(head) = next {
        for item in &mut bases {
            if item[0].get_id() == head.get_id() {
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
    typ: PyObjectRef,
    name: &str,
    bases: Vec<PyClassRef>,
    dict: HashMap<String, PyObjectRef>,
) -> PyResult {
    let mros = bases.into_iter().map(|x| _mro(&x)).collect();
    let mro = linearise_mro(mros).unwrap();
    Ok(PyObject {
        payload: Box::new(PyClass {
            name: String::from(name),
            mro,
        }),
        dict: Some(RefCell::new(dict)),
        typ,
    }
    .into_ref())
}

#[cfg(test)]
mod tests {
    use super::FromPyObjectRef;
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
        let object: PyClassRef = FromPyObjectRef::from_pyobj(&context.object);
        let type_type = &context.type_type;

        let a = new(type_type.clone(), "A", vec![object.clone()], HashMap::new()).unwrap();
        let b = new(type_type.clone(), "B", vec![object.clone()], HashMap::new()).unwrap();

        let a: PyClassRef = FromPyObjectRef::from_pyobj(&a);
        let b: PyClassRef = FromPyObjectRef::from_pyobj(&b);

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

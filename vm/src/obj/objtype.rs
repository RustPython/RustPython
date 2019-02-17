use super::super::pyobject::{
    AttributeProtocol, IdProtocol, PyAttributes, PyContext, PyFuncArgs, PyObject, PyObjectPayload,
    PyObjectRef, PyResult, TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objdict;
use super::objstr;
use super::objtype; // Required for arg_check! to use isinstance
use std::cell::RefCell;
use std::collections::HashMap;

/*
 * The magical type type
 */

pub fn create_type(type_type: PyObjectRef, object_type: PyObjectRef, _dict_type: PyObjectRef) {
    (*type_type.borrow_mut()).payload = PyObjectPayload::Class {
        name: String::from("type"),
        dict: RefCell::new(PyAttributes::new()),
        mro: vec![object_type],
    };
    (*type_type.borrow_mut()).typ = Some(type_type.clone());
}

pub fn init(context: &PyContext) {
    let type_type = &context.type_type;

    let type_doc = "type(object_or_name, bases, dict)\n\
                    type(object) -> the object's type\n\
                    type(name, bases, dict) -> a new type";

    context.set_attr(&type_type, "__call__", context.new_rustfunc(type_call));
    context.set_attr(&type_type, "__new__", context.new_rustfunc(type_new));
    context.set_attr(
        &type_type,
        "__mro__",
        context.new_member_descriptor(type_mro),
    );
    context.set_attr(
        &type_type,
        "__class__",
        context.new_member_descriptor(type_new),
    );
    context.set_attr(&type_type, "__repr__", context.new_rustfunc(type_repr));
    context.set_attr(
        &type_type,
        "__prepare__",
        context.new_rustfunc(type_prepare),
    );
    context.set_attr(
        &type_type,
        "__getattribute__",
        context.new_rustfunc(type_getattribute),
    );
    context.set_attr(&type_type, "__doc__", context.new_str(type_doc.to_string()));
}

fn type_mro(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [
            (cls, Some(vm.ctx.type_type())),
            (_typ, Some(vm.ctx.type_type()))
        ]
    );
    match _mro(cls.clone()) {
        Some(mro) => Ok(vm.context().new_tuple(mro)),
        None => Err(vm.new_type_error("Only classes have an MRO.".to_string())),
    }
}

fn _mro(cls: PyObjectRef) -> Option<Vec<PyObjectRef>> {
    match cls.borrow().payload {
        PyObjectPayload::Class { ref mro, .. } => {
            let mut mro = mro.clone();
            mro.insert(0, cls.clone());
            Some(mro)
        }
        _ => None,
    }
}

pub fn base_classes(obj: &PyObjectRef) -> Vec<PyObjectRef> {
    _mro(obj.typ()).unwrap()
}

pub fn isinstance(obj: &PyObjectRef, cls: &PyObjectRef) -> bool {
    let mro = _mro(obj.typ()).unwrap();
    mro.into_iter().any(|c| c.is(&cls))
}

pub fn issubclass(typ: &PyObjectRef, cls: &PyObjectRef) -> bool {
    let mro = _mro(typ.clone()).unwrap();
    mro.into_iter().any(|c| c.is(&cls))
}

pub fn get_type_name(typ: &PyObjectRef) -> String {
    if let PyObjectPayload::Class { name, .. } = &typ.borrow().payload {
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
        let mut bases = vm.extract_elements(bases)?;
        bases.push(vm.context().object());
        let name = objstr::get_value(name);
        new(
            typ.clone(),
            &name,
            bases,
            objdict::py_dict_to_attributes(dict),
        )
    } else {
        Err(vm.new_type_error(format!(": type_new: {:?}", args)))
    }
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
                return vm.invoke(
                    descriptor,
                    PyFuncArgs {
                        args: vec![attr, cls.clone(), mcl],
                        kwargs: vec![],
                    },
                );
            }
        }
    }

    if let Some(attr) = cls.get_attr(&name) {
        let attr_class = attr.typ();
        if let Some(descriptor) = attr_class.get_attr("__get__") {
            let none = vm.get_none();
            return vm.invoke(
                descriptor,
                PyFuncArgs {
                    args: vec![attr, none, cls.clone()],
                    kwargs: vec![],
                },
            );
        }
    }

    if let Some(cls_attr) = cls.get_attr(&name) {
        Ok(cls_attr)
    } else if let Some(attr) = mcl.get_attr(&name) {
        vm.call_get_descriptor(attr, cls.clone())
    } else if let Some(getter) = cls.get_attr("__getattr__") {
        vm.invoke(
            getter,
            PyFuncArgs {
                args: vec![mcl, name_str.clone()],
                kwargs: vec![],
            },
        )
    } else {
        let attribute_error = vm.context().exceptions.attribute_error.clone();
        Err(vm.new_exception(
            attribute_error,
            format!("{} has no attribute '{}'", cls.borrow(), name),
        ))
    }
}

pub fn get_attributes(obj: &PyObjectRef) -> PyAttributes {
    // Gather all members here:
    let mut attributes = PyAttributes::new();

    // Get class attributes:
    let mut base_classes = objtype::base_classes(obj);
    base_classes.reverse();
    for bc in base_classes {
        if let PyObjectPayload::Class { dict, .. } = &bc.borrow().payload {
            for (name, value) in dict.borrow().iter() {
                attributes.insert(name.to_string(), value.clone());
            }
        }
    }

    // Get instance attributes:
    if let PyObjectPayload::Instance { dict } = &obj.borrow().payload {
        for (name, value) in dict.borrow().iter() {
            attributes.insert(name.to_string(), value.clone());
        }
    }
    attributes
}

fn take_next_base(
    mut bases: Vec<Vec<PyObjectRef>>,
) -> Option<(PyObjectRef, Vec<Vec<PyObjectRef>>)> {
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

fn linearise_mro(mut bases: Vec<Vec<PyObjectRef>>) -> Option<Vec<PyObjectRef>> {
    debug!("Linearising MRO: {:?}", bases);
    let mut result = vec![];
    loop {
        if (&bases).iter().all(|x| x.is_empty()) {
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
    bases: Vec<PyObjectRef>,
    dict: HashMap<String, PyObjectRef>,
) -> PyResult {
    let mros = bases.into_iter().map(|x| _mro(x).unwrap()).collect();
    let mro = linearise_mro(mros).unwrap();
    Ok(PyObject::new(
        PyObjectPayload::Class {
            name: String::from(name),
            dict: RefCell::new(dict),
            mro,
        },
        typ,
    ))
}

fn type_repr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(obj, Some(vm.ctx.type_type()))]);
    let type_name = get_type_name(&obj);
    Ok(vm.new_str(format!("<class '{}'>", type_name)))
}

fn type_prepare(vm: &mut VirtualMachine, _args: PyFuncArgs) -> PyResult {
    Ok(vm.new_dict())
}

#[cfg(test)]
mod tests {
    use super::{linearise_mro, new};
    use super::{HashMap, IdProtocol, PyContext, PyObjectRef};

    fn map_ids(obj: Option<Vec<PyObjectRef>>) -> Option<Vec<usize>> {
        match obj {
            Some(vec) => Some(vec.into_iter().map(|x| x.get_id()).collect()),
            None => None,
        }
    }

    #[test]
    fn test_linearise() {
        let context = PyContext::new();
        let object = context.object;
        let type_type = context.type_type;

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

use super::pyobject::{
    AttributeProtocol, IdProtocol, PyContext, PyFuncArgs, PyObject, PyObjectKind, PyObjectRef,
    PyResult, ToRust, TypeProtocol,
};
use super::vm::VirtualMachine;
use super::objdict;

/*
 * The magical type type
 */

pub fn create_type(type_type: PyObjectRef, object_type: PyObjectRef, dict_type: PyObjectRef) {
    (*type_type.borrow_mut()).kind = PyObjectKind::Class {
        name: String::from("type"),
        dict: objdict::new(dict_type),
        mro: vec![object_type],
    };
    (*type_type.borrow_mut()).typ = Some(type_type.clone());
}

pub fn init(context: &PyContext) {
    let ref type_type = context.type_type;
    type_type.set_attr("__call__", context.new_rustfunc(type_call));
    type_type.set_attr("__new__", context.new_rustfunc(type_new));
    type_type.set_attr("__mro__", context.new_member_descriptor(type_mro));
    type_type.set_attr("__class__", context.new_member_descriptor(type_new));
    type_type.set_attr("__dict__", context.new_member_descriptor(type_dict));
}

fn type_mro(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    match _mro(args.args[0].clone()) {
        Some(mro) => Ok(vm.context().new_tuple(mro)),
        None => Err(vm.new_exception("Only classes have an MRO.".to_string())),
    }
}

fn _mro(cls: PyObjectRef) -> Option<Vec<PyObjectRef>> {
    match cls.borrow().kind {
        PyObjectKind::Class { ref mro, .. } => {
            let mut mro = mro.clone();
            mro.insert(0, cls.clone());
            Some(mro)
        }
        _ => None,
    }
}

fn type_dict(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    match args.args[0].borrow().kind {
        PyObjectKind::Class { ref dict, .. } => Ok(dict.clone()),
        _ => Err(vm.new_exception("type_dict must be called on a class.".to_string())),
    }
}

pub fn type_new(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    debug!("type.__new__{:?}", args);
    if args.args.len() == 2 {
        Ok(args.args[1].typ())
    } else if args.args.len() == 4 {
        let typ = args.args[0].clone();
        let name = args.args[1].to_str().unwrap();
        let mut bases = args.args[2].to_vec().unwrap();
        bases.push(vm.context().object.clone());
        let dict = args.args[3].clone();
        new(typ, &name, bases, dict)
    } else {
        Err(vm.new_exception(format!("TypeError: type_new: {:?}", args)))
    }
}

pub fn type_call(vm: &mut VirtualMachine, mut args: PyFuncArgs) -> PyResult {
    debug!("type_call: {:?}", args);
    let typ = args.shift();
    let new = typ.get_attr(&String::from("__new__"));
    let obj = vm.invoke(new, args.insert(typ.clone()))?;

    match get_attribute(vm, obj.typ(), &String::from("__init__")) {
        Ok(init) => {
            vm.invoke(init, args.insert(obj.clone()))?;
        }
        Err(_) => return Ok(obj),
    }

    Ok(obj)
}

pub fn get_attribute(vm: &mut VirtualMachine, obj: PyObjectRef, name: &String) -> PyResult {
    let cls = obj.typ();
    trace!("get_attribute: {:?}, {:?}, {:?}", cls, obj, name);
    if cls.has_attr(name) {
        let attr = cls.get_attr(name);
        let attr_class = attr.typ();
        if attr_class.has_attr(&String::from("__get__")) {
            return vm.invoke(
                attr_class.get_attr(&String::from("__get__")),
                PyFuncArgs {
                    args: vec![attr, obj, cls],
                },
            );
        }
    }

    if obj.has_attr(name) {
        Ok(obj.get_attr(name))
    } else if cls.has_attr(name) {
        Ok(cls.get_attr(name))
    } else {
        Err(vm.new_exception(format!(
            "AttributeError: {:?} object has no attribute {}",
            cls, name
        )))
    }
}

fn take_next_base(
    mut bases: Vec<Vec<PyObjectRef>>,
) -> Option<(PyObjectRef, Vec<Vec<PyObjectRef>>)> {
    let mut next = None;

    bases = bases.into_iter().filter(|x| !x.is_empty()).collect();

    for base in &bases {
        let head = base[0].clone();
        if !(&bases)
            .into_iter()
            .any(|x| x[1..].into_iter().any(|x| x.get_id() == head.get_id()))
        {
            next = Some(head);
            break;
        }
    }

    if let Some(head) = next {
        for ref mut item in &mut bases {
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
        if (&bases).into_iter().all(|x| x.is_empty()) {
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

pub fn new(typ: PyObjectRef, name: &str, bases: Vec<PyObjectRef>, dict: PyObjectRef) -> PyResult {
    let mros = bases.into_iter().map(|x| _mro(x).unwrap()).collect();
    let mro = linearise_mro(mros).unwrap();
    Ok(PyObject::new(
        PyObjectKind::Class {
            name: String::from(name),
            dict: dict,
            mro: mro,
        },
        typ,
    ))
}

pub fn call(vm: &mut VirtualMachine, typ: PyObjectRef, args: PyFuncArgs) -> PyResult {
    let function = get_attribute(vm, typ, &String::from("__call__"))?;
    vm.invoke(function, args)
}

#[cfg(test)]
mod tests {
    use super::{create_type, linearise_mro, new};
    use super::{IdProtocol, PyContext, PyObjectRef};

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
        let type_type = create_type();

        let a = new(
            type_type.clone(),
            String::from("A"),
            vec![object.clone()],
            type_type.clone(),
        ).unwrap();
        let b = new(
            type_type.clone(),
            String::from("B"),
            vec![object.clone()],
            type_type.clone(),
        ).unwrap();

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

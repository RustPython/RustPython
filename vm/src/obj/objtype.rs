use super::super::pyobject::{
    AttributeProtocol, IdProtocol, PyContext, PyFuncArgs, PyObject, PyObjectKind, PyObjectRef,
    PyResult, ToRust, TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objdict;
use super::objtype; // Required for arg_check! to use isinstance

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
    type_type.set_attr("__str__", context.new_rustfunc(type_str));
}

fn type_mro(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    println!("{:?}", args);
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
    match cls.borrow().kind {
        PyObjectKind::Class { ref mro, .. } => {
            let mut mro = mro.clone();
            mro.insert(0, cls.clone());
            Some(mro)
        }
        _ => None,
    }
}

pub fn isinstance(obj: PyObjectRef, cls: PyObjectRef) -> bool {
    let mro = _mro(obj.typ()).unwrap();
    mro.into_iter().any(|c| c.is(&cls))
}

pub fn issubclass(typ: PyObjectRef, cls: PyObjectRef) -> bool {
    let mro = _mro(typ).unwrap();
    mro.into_iter().any(|c| c.is(&cls))
}

pub fn get_type_name(typ: &PyObjectRef) -> String {
    if let PyObjectKind::Class {
        name,
        dict: _,
        mro: _,
    } = &typ.borrow().kind
    {
        name.clone()
    } else {
        panic!("Cannot get type_name of non-type type");
    }
}

pub fn type_new(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    debug!("type.__new__{:?}", args);
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
                // bases needs to be mutable, which arg_check! doesn't support, so we just check
                // the type and extract it again below
                // TODO: arg_check! should support specifying iterables
                (_bases, None),
                (dict, Some(vm.ctx.dict_type()))
            ]
        );
        let mut bases = args.args[2].to_vec().unwrap();
        bases.push(vm.context().object());
        new(typ.clone(), &name.to_str().unwrap(), bases, dict.clone())
    } else {
        Err(vm.new_type_error(format!(": type_new: {:?}", args)))
    }
}

pub fn type_call(vm: &mut VirtualMachine, mut args: PyFuncArgs) -> PyResult {
    debug!("type_call: {:?}", args);
    let typ = args.shift();
    let new = typ.get_attr("__new__").unwrap();
    let obj = vm.invoke(new, args.insert(typ.clone()))?;

    if let Some(init) = obj.typ().get_attr("__init__") {
        let _ = vm.invoke(init, args.insert(obj.clone())).unwrap();
    }
    Ok(obj)
}

pub fn get_attribute(vm: &mut VirtualMachine, obj: PyObjectRef, name: &String) -> PyResult {
    let cls = obj.typ();
    trace!("get_attribute: {:?}, {:?}, {:?}", cls, obj, name);
    if let Some(attr) = cls.get_attr(name) {
        let attr_class = attr.typ();
        if let Some(descriptor) = attr_class.get_attr("__get__") {
            return vm.invoke(
                descriptor,
                PyFuncArgs {
                    args: vec![attr, obj, cls],
                },
            );
        }
    }

    if let Some(obj_attr) = obj.get_attr(name) {
        Ok(obj_attr)
    } else if let Some(cls_attr) = cls.get_attr(name) {
        Ok(cls_attr)
    } else {
        let attribute_error = vm.context().exceptions.attribute_error.clone();
        Err(vm.new_exception(
            attribute_error,
            format!("{:?} object has no attribute {}", cls, name),
        ))
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

fn type_str(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(obj, Some(vm.ctx.type_type()))]);
    let type_name = get_type_name(&obj);
    Ok(vm.new_str(format!("<class '{}'>", type_name)))
}

pub fn call(vm: &mut VirtualMachine, typ: PyObjectRef, args: PyFuncArgs) -> PyResult {
    let function = get_attribute(vm, typ, &String::from("__call__"))?;
    vm.invoke(function, args)
}

#[cfg(test)]
mod tests {
    use super::{linearise_mro, new};
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
        let type_type = context.type_type;

        let a = new(
            type_type.clone(),
            "A",
            vec![object.clone()],
            type_type.clone(),
        ).unwrap();
        let b = new(
            type_type.clone(),
            "B",
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

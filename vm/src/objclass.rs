use super::pyobject::AttributeProtocol;
use super::pyobject::{PyFuncArgs, PyObject, PyObjectKind, PyObjectRef, PyResult, TypeProtocol};
use super::vm::VirtualMachine;

pub fn get_attribute(
    vm: &mut VirtualMachine,
    cls: PyObjectRef,
    obj: PyObjectRef,
    name: &String,
) -> PyResult {
    if obj.has_attr(name) {
        Ok(obj.get_attr(name))
    } else if cls.has_attr(name) {
        let attr = cls.get_attr(name);
        let attr_class = attr.typ();
        if attr_class.has_attr(&String::from("__get__")) {
            vm.invoke(
                attr_class.get_attr(&String::from("__get__")),
                PyFuncArgs {
                    args: vec![attr, obj, cls],
                },
            )
        } else {
            Ok(attr)
        }
    } else {
        Err(vm.new_exception(format!(
            "AttributeError: {:?} object has no attribute {}",
            cls, name
        )))
    }
}

pub fn new_instance(vm: &mut VirtualMachine, mut args: PyFuncArgs) -> PyResult {
    // more or less __new__ operator
    let type_ref = args.shift();
    let dict = vm.new_dict();
    let obj = PyObject::new(PyObjectKind::Instance { dict: dict }, type_ref.clone());
    let init = get_attribute(vm, type_ref, obj.clone(), &String::from("__init__"))?;
    vm.invoke(init, args)?;
    // TODO Raise TypeError if init returns not None.
    Ok(obj)
}

pub fn call(vm: &mut VirtualMachine, mut args: PyFuncArgs) -> PyResult {
    let instance = args.shift();
    let function = get_attribute(vm, instance.typ(), instance, &String::from("__call__"))?;
    vm.invoke(function, args)
}

use super::pyobject::AttributeProtocol;
use super::pyobject::{PyFuncArgs, PyObjectKind, PyObjectRef, PyResult, TypeProtocol};
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
                    args: vec![attr, cls],
                },
            )
        } else {
            Ok(attr)
        }
    } else {
        Err(vm.new_exception(String::from("TypeError goes here!")))
    }
}

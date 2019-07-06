use super::objdict::PyDictRef;
use super::objlist::PyList;
use super::objstr::PyStringRef;
use super::objtype;
use crate::function::PyFuncArgs;
use crate::obj::objproperty::PropertyBuilder;
use crate::obj::objtype::PyClassRef;
use crate::pyhash;
use crate::pyobject::{
    IdProtocol, ItemProtocol, PyAttributes, PyContext, PyObject, PyObjectRef, PyResult, PyValue,
    TryFromObject, TypeProtocol,
};
use crate::vm::VirtualMachine;

#[derive(Debug)]
pub struct PyInstance;

impl PyValue for PyInstance {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.object()
    }
}

pub fn new_instance(vm: &VirtualMachine, mut args: PyFuncArgs) -> PyResult {
    // more or less __new__ operator
    let cls = PyClassRef::try_from_object(vm, args.shift())?;
    let dict = if cls.is(&vm.ctx.object) {
        None
    } else {
        Some(vm.ctx.new_dict())
    };
    Ok(PyObject::new(PyInstance, cls, dict))
}

fn object_eq(_zelf: PyObjectRef, _other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
    vm.ctx.not_implemented()
}

fn object_ne(_zelf: PyObjectRef, _other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
    vm.ctx.not_implemented()
}

fn object_lt(_zelf: PyObjectRef, _other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
    vm.ctx.not_implemented()
}

fn object_le(_zelf: PyObjectRef, _other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
    vm.ctx.not_implemented()
}

fn object_gt(_zelf: PyObjectRef, _other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
    vm.ctx.not_implemented()
}

fn object_ge(_zelf: PyObjectRef, _other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
    vm.ctx.not_implemented()
}

fn object_hash(zelf: PyObjectRef, _vm: &VirtualMachine) -> pyhash::PyHash {
    zelf.get_id() as pyhash::PyHash
}

fn object_setattr(
    obj: PyObjectRef,
    attr_name: PyStringRef,
    value: PyObjectRef,
    vm: &VirtualMachine,
) -> PyResult<()> {
    trace!("object.__setattr__({:?}, {}, {:?})", obj, attr_name, value);
    let cls = obj.class();

    if let Some(attr) = objtype::class_get_attr(&cls, &attr_name.value) {
        if let Some(descriptor) = objtype::class_get_attr(&attr.class(), "__set__") {
            return vm
                .invoke(descriptor, vec![attr, obj.clone(), value])
                .map(|_| ());
        }
    }

    if let Some(ref dict) = obj.clone().dict {
        dict.set_item(attr_name, value, vm)?;
        Ok(())
    } else {
        Err(vm.new_attribute_error(format!(
            "'{}' object has no attribute '{}'",
            obj.class().name,
            &attr_name.value
        )))
    }
}

fn object_delattr(obj: PyObjectRef, attr_name: PyStringRef, vm: &VirtualMachine) -> PyResult<()> {
    let cls = obj.class();

    if let Some(attr) = objtype::class_get_attr(&cls, &attr_name.value) {
        if let Some(descriptor) = objtype::class_get_attr(&attr.class(), "__delete__") {
            return vm.invoke(descriptor, vec![attr, obj.clone()]).map(|_| ());
        }
    }

    if let Some(ref dict) = obj.dict {
        dict.del_item(attr_name, vm)?;
        Ok(())
    } else {
        Err(vm.new_attribute_error(format!(
            "'{}' object has no attribute '{}'",
            obj.class().name,
            &attr_name.value
        )))
    }
}

fn object_str(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    vm.call_method(&zelf, "__repr__", vec![])
}

fn object_repr(zelf: PyObjectRef, _vm: &VirtualMachine) -> String {
    format!("<{} object at 0x{:x}>", zelf.class().name, zelf.get_id())
}

pub fn object_dir(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyList> {
    let attributes: PyAttributes = objtype::get_attributes(obj.class());

    let dict = PyDictRef::from_attributes(attributes, vm)?;

    // Get instance attributes:
    if let Some(object_dict) = &obj.dict {
        vm.invoke(
            vm.get_attribute(dict.clone().into_object(), "update")?,
            object_dict.clone().into_object(),
        )?;
    }

    let attributes: Vec<_> = dict.into_iter().map(|(k, _v)| k.clone()).collect();

    Ok(PyList::from(attributes))
}

fn object_format(
    obj: PyObjectRef,
    format_spec: PyStringRef,
    vm: &VirtualMachine,
) -> PyResult<PyStringRef> {
    if format_spec.value.is_empty() {
        vm.to_str(&obj)
    } else {
        Err(vm.new_type_error("unsupported format string passed to object.__format__".to_string()))
    }
}

pub fn init(context: &PyContext) {
    let object = &context.object;
    let object_doc = "The most base type";

    extend_class!(context, object, {
        "__new__" => context.new_rustfunc(new_instance),
        "__init__" => context.new_rustfunc(object_init),
        "__class__" =>
        PropertyBuilder::new(context)
            .add_getter(object_class)
            .add_setter(object_class_setter)
            .create(),
        "__eq__" => context.new_rustfunc(object_eq),
        "__ne__" => context.new_rustfunc(object_ne),
        "__lt__" => context.new_rustfunc(object_lt),
        "__le__" => context.new_rustfunc(object_le),
        "__gt__" => context.new_rustfunc(object_gt),
        "__ge__" => context.new_rustfunc(object_ge),
        "__setattr__" => context.new_rustfunc(object_setattr),
        "__delattr__" => context.new_rustfunc(object_delattr),
        "__dict__" =>
        PropertyBuilder::new(context)
                .add_getter(object_dict)
                .add_setter(object_dict_setter)
                .create(),
        "__dir__" => context.new_rustfunc(object_dir),
        "__hash__" => context.new_rustfunc(object_hash),
        "__str__" => context.new_rustfunc(object_str),
        "__repr__" => context.new_rustfunc(object_repr),
        "__format__" => context.new_rustfunc(object_format),
        "__getattribute__" => context.new_rustfunc(object_getattribute),
        "__doc__" => context.new_str(object_doc.to_string())
    });
}

fn object_init(vm: &VirtualMachine, _args: PyFuncArgs) -> PyResult {
    Ok(vm.ctx.none())
}

fn object_class(obj: PyObjectRef, _vm: &VirtualMachine) -> PyObjectRef {
    obj.class().into_object()
}

fn object_class_setter(
    instance: PyObjectRef,
    _value: PyObjectRef,
    vm: &VirtualMachine,
) -> PyResult {
    let type_repr = vm.to_pystr(&instance.class())?;
    Err(vm.new_type_error(format!("can't change class of type '{}'", type_repr)))
}

fn object_dict(object: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyDictRef> {
    if let Some(ref dict) = object.dict {
        Ok(dict.clone())
    } else {
        Err(vm.new_type_error("TypeError: no dictionary.".to_string()))
    }
}

fn object_dict_setter(
    _instance: PyObjectRef,
    _value: PyObjectRef,
    vm: &VirtualMachine,
) -> PyResult {
    Err(vm.new_not_implemented_error(
        "Setting __dict__ attribute on an object isn't yet implemented".to_string(),
    ))
}

fn object_getattribute(obj: PyObjectRef, name_str: PyStringRef, vm: &VirtualMachine) -> PyResult {
    let name = &name_str.value;
    trace!("object.__getattribute__({:?}, {:?})", obj, name);
    let cls = obj.class();

    if let Some(attr) = objtype::class_get_attr(&cls, &name) {
        let attr_class = attr.class();
        if objtype::class_has_attr(&attr_class, "__set__") {
            if let Some(descriptor) = objtype::class_get_attr(&attr_class, "__get__") {
                return vm.invoke(descriptor, vec![attr, obj, cls.into_object()]);
            }
        }
    }

    if let Some(obj_attr) = object_getattr(&obj, &name, &vm)? {
        Ok(obj_attr)
    } else if let Some(attr) = objtype::class_get_attr(&cls, &name) {
        vm.call_get_descriptor(attr, obj)
    } else if let Some(getter) = objtype::class_get_attr(&cls, "__getattr__") {
        vm.invoke(getter, vec![obj, name_str.into_object()])
    } else {
        Err(vm.new_attribute_error(format!("{} has no attribute '{}'", obj, name)))
    }
}

fn object_getattr(
    obj: &PyObjectRef,
    attr_name: &str,
    vm: &VirtualMachine,
) -> PyResult<Option<PyObjectRef>> {
    if let Some(ref dict) = obj.dict {
        dict.get_item_option(attr_name, vm)
    } else {
        Ok(None)
    }
}

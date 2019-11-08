use super::objdict::PyDictRef;
use super::objlist::PyList;
use super::objproperty::PropertyBuilder;
use super::objstr::PyStringRef;
use super::objtype::{self, PyClassRef};
use crate::function::{OptionalArg, PyFuncArgs};
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
    let dict = if cls.is(&vm.ctx.object()) {
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
    vm_trace!("object.__setattr__({:?}, {}, {:?})", obj, attr_name, value);
    let cls = obj.class();

    if let Some(attr) = objtype::class_get_attr(&cls, attr_name.as_str()) {
        if let Some(descriptor) = objtype::class_get_attr(&attr.class(), "__set__") {
            return vm
                .invoke(&descriptor, vec![attr, obj.clone(), value])
                .map(|_| ());
        }
    }

    if let Some(ref dict) = obj.clone().dict {
        dict.borrow().set_item(attr_name.as_str(), value, vm)?;
        Ok(())
    } else {
        Err(vm.new_attribute_error(format!(
            "'{}' object has no attribute '{}'",
            obj.class().name,
            attr_name.as_str()
        )))
    }
}

fn object_delattr(obj: PyObjectRef, attr_name: PyStringRef, vm: &VirtualMachine) -> PyResult<()> {
    let cls = obj.class();

    if let Some(attr) = objtype::class_get_attr(&cls, attr_name.as_str()) {
        if let Some(descriptor) = objtype::class_get_attr(&attr.class(), "__delete__") {
            return vm.invoke(&descriptor, vec![attr, obj.clone()]).map(|_| ());
        }
    }

    if let Some(ref dict) = obj.dict {
        dict.borrow().del_item(attr_name.as_str(), vm)?;
        Ok(())
    } else {
        Err(vm.new_attribute_error(format!(
            "'{}' object has no attribute '{}'",
            obj.class().name,
            attr_name.as_str()
        )))
    }
}

fn object_str(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    vm.call_method(&zelf, "__repr__", vec![])
}

fn object_repr(zelf: PyObjectRef, _vm: &VirtualMachine) -> String {
    format!("<{} object at 0x{:x}>", zelf.class().name, zelf.get_id())
}

fn object_subclasshook(vm: &VirtualMachine, _args: PyFuncArgs) -> PyResult {
    Ok(vm.ctx.not_implemented())
}

pub fn object_dir(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyList> {
    let attributes: PyAttributes = objtype::get_attributes(obj.class());

    let dict = PyDictRef::from_attributes(attributes, vm)?;

    // Get instance attributes:
    if let Some(object_dict) = &obj.dict {
        vm.invoke(
            &vm.get_attribute(dict.clone().into_object(), "update")?,
            object_dict.borrow().clone().into_object(),
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
    if format_spec.as_str().is_empty() {
        vm.to_str(&obj)
    } else {
        Err(vm.new_type_error("unsupported format string passed to object.__format__".to_string()))
    }
}

pub fn init(context: &PyContext) {
    let object = &context.types.object_type;
    let object_doc = "The most base type";

    extend_class!(context, object, {
        (slot new) => new_instance,
        // yeah, it's `type_new`, but we're putting here so it's available on every object
        "__new__" => context.new_classmethod(objtype::type_new),
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
        "__subclasshook__" => context.new_classmethod(object_subclasshook),
        "__reduce__" => context.new_rustfunc(object_reduce),
        "__reduce_ex__" => context.new_rustfunc(object_reduce_ex),
        "__doc__" => context.new_str(object_doc.to_string()),
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
        Ok(dict.borrow().clone())
    } else {
        Err(vm.new_attribute_error("no dictionary.".to_string()))
    }
}

fn object_dict_setter(instance: PyObjectRef, value: PyDictRef, vm: &VirtualMachine) -> PyResult {
    if let Some(dict) = &instance.dict {
        *dict.borrow_mut() = value;
        Ok(vm.get_none())
    } else {
        Err(vm.new_attribute_error(format!(
            "'{}' object has no attribute '__dict__'",
            instance.class().name
        )))
    }
}

fn object_getattribute(obj: PyObjectRef, name: PyStringRef, vm: &VirtualMachine) -> PyResult {
    vm_trace!("object.__getattribute__({:?}, {:?})", obj, name);
    vm.generic_getattribute(obj.clone(), name.clone())?
        .ok_or_else(|| vm.new_attribute_error(format!("{} has no attribute '{}'", obj, name)))
}

fn object_reduce(obj: PyObjectRef, proto: OptionalArg<usize>, vm: &VirtualMachine) -> PyResult {
    common_reduce(obj, proto.unwrap_or(0), vm)
}

fn object_reduce_ex(obj: PyObjectRef, proto: usize, vm: &VirtualMachine) -> PyResult {
    let cls = obj.class();
    if let Some(reduce) = objtype::class_get_attr(&cls, "__reduce__") {
        let object_reduce =
            objtype::class_get_attr(&vm.ctx.types.object_type, "__reduce__").unwrap();
        if !reduce.is(&object_reduce) {
            return vm.invoke(&reduce, vec![]);
        }
    }
    common_reduce(obj, proto, vm)
}

fn common_reduce(obj: PyObjectRef, proto: usize, vm: &VirtualMachine) -> PyResult {
    if proto >= 2 {
        let reducelib = vm.import("__reducelib", &[], 0)?;
        let reduce_2 = vm.get_attribute(reducelib, "reduce_2")?;
        vm.invoke(&reduce_2, vec![obj])
    } else {
        let copyreg = vm.import("copyreg", &[], 0)?;
        let reduce_ex = vm.get_attribute(copyreg, "_reduce_ex")?;
        vm.invoke(&reduce_ex, vec![obj, vm.new_int(proto)])
    }
}

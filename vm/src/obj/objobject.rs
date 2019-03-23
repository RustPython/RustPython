use super::objlist::PyList;
use super::objmodule::PyModule;
use super::objstr::{self, PyStringRef};
use super::objtype;
use crate::function::PyFuncArgs;
use crate::obj::objproperty::PropertyBuilder;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{
    DictProtocol, IdProtocol, PyAttributes, PyContext, PyObject, PyObjectRef, PyResult, PyValue,
    TypeProtocol,
};
use crate::vm::VirtualMachine;

#[derive(Clone, Debug)]
pub struct PyInstance;

impl PyValue for PyInstance {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.object()
    }
}

pub fn new_instance(vm: &VirtualMachine, mut args: PyFuncArgs) -> PyResult {
    // more or less __new__ operator
    let cls = args.shift();
    Ok(if cls.is(&vm.ctx.object) {
        PyObject::new_without_dict(PyInstance, cls)
    } else {
        PyObject::new(PyInstance, cls)
    })
}

fn object_eq(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(_zelf, Some(vm.ctx.object())), (_other, None)]
    );
    Ok(vm.ctx.not_implemented())
}

fn object_ne(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(_zelf, Some(vm.ctx.object())), (_other, None)]
    );

    Ok(vm.ctx.not_implemented())
}

fn object_lt(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(_zelf, Some(vm.ctx.object())), (_other, None)]
    );

    Ok(vm.ctx.not_implemented())
}

fn object_le(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(_zelf, Some(vm.ctx.object())), (_other, None)]
    );

    Ok(vm.ctx.not_implemented())
}

fn object_gt(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(_zelf, Some(vm.ctx.object())), (_other, None)]
    );

    Ok(vm.ctx.not_implemented())
}

fn object_ge(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(_zelf, Some(vm.ctx.object())), (_other, None)]
    );

    Ok(vm.ctx.not_implemented())
}

fn object_hash(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(_zelf, Some(vm.ctx.object()))]);

    // For now default to non hashable
    Err(vm.new_type_error("unhashable type".to_string()))
}

fn object_setattr(
    obj: PyObjectRef,
    attr_name: PyStringRef,
    value: PyObjectRef,
    vm: &VirtualMachine,
) -> PyResult<()> {
    trace!("object.__setattr__({:?}, {}, {:?})", obj, attr_name, value);
    let cls = obj.type_pyref();

    if let Some(attr) = objtype::class_get_attr(&cls, &attr_name.value) {
        if let Some(descriptor) = objtype::class_get_attr(&attr.type_pyref(), "__set__") {
            return vm
                .invoke(descriptor, vec![attr, obj.clone(), value])
                .map(|_| ());
        }
    }

    if let Some(ref dict) = obj.clone().dict {
        dict.borrow_mut().insert(attr_name.value.clone(), value);
        Ok(())
    } else {
        let type_name = objtype::get_type_name(obj.type_ref());
        Err(vm.new_attribute_error(format!(
            "'{}' object has no attribute '{}'",
            type_name, &attr_name.value
        )))
    }
}

fn object_delattr(obj: PyObjectRef, attr_name: PyStringRef, vm: &VirtualMachine) -> PyResult<()> {
    let cls = obj.type_pyref();

    if let Some(attr) = objtype::class_get_attr(&cls, &attr_name.value) {
        if let Some(descriptor) = objtype::class_get_attr(&attr.type_pyref(), "__delete__") {
            return vm.invoke(descriptor, vec![attr, obj.clone()]).map(|_| ());
        }
    }

    if let Some(ref dict) = obj.dict {
        dict.borrow_mut().remove(&attr_name.value);
        Ok(())
    } else {
        let type_name = objtype::get_type_name(obj.type_ref());
        Err(vm.new_attribute_error(format!(
            "'{}' object has no attribute '{}'",
            type_name, &attr_name.value
        )))
    }
}

fn object_str(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, Some(vm.ctx.object()))]);
    vm.call_method(zelf, "__repr__", vec![])
}

fn object_repr(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(obj, Some(vm.ctx.object()))]);
    let type_name = objtype::get_type_name(&obj.typ());
    let address = obj.get_id();
    Ok(vm.new_str(format!("<{} object at 0x{:x}>", type_name, address)))
}

pub fn object_dir(obj: PyObjectRef, vm: &VirtualMachine) -> PyList {
    let attributes = get_attributes(&obj);
    let attributes: Vec<PyObjectRef> = attributes
        .keys()
        .map(|k| vm.ctx.new_str(k.to_string()))
        .collect();
    PyList::from(attributes)
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

    context.set_attr(object, "__new__", context.new_rustfunc(new_instance));
    context.set_attr(object, "__init__", context.new_rustfunc(object_init));
    context.set_attr(
        object,
        "__class__",
        PropertyBuilder::new(context)
            .add_getter(object_class)
            .add_setter(object_class_setter)
            .create(),
    );
    context.set_attr(object, "__eq__", context.new_rustfunc(object_eq));
    context.set_attr(object, "__ne__", context.new_rustfunc(object_ne));
    context.set_attr(object, "__lt__", context.new_rustfunc(object_lt));
    context.set_attr(object, "__le__", context.new_rustfunc(object_le));
    context.set_attr(object, "__gt__", context.new_rustfunc(object_gt));
    context.set_attr(object, "__ge__", context.new_rustfunc(object_ge));
    context.set_attr(object, "__setattr__", context.new_rustfunc(object_setattr));
    context.set_attr(object, "__delattr__", context.new_rustfunc(object_delattr));
    context.set_attr(object, "__dict__", context.new_property(object_dict));
    context.set_attr(object, "__dir__", context.new_rustfunc(object_dir));
    context.set_attr(object, "__hash__", context.new_rustfunc(object_hash));
    context.set_attr(object, "__str__", context.new_rustfunc(object_str));
    context.set_attr(object, "__repr__", context.new_rustfunc(object_repr));
    context.set_attr(object, "__format__", context.new_rustfunc(object_format));
    context.set_attr(
        object,
        "__getattribute__",
        context.new_rustfunc(object_getattribute),
    );
    context.set_attr(object, "__doc__", context.new_str(object_doc.to_string()));
}

fn object_init(vm: &VirtualMachine, _args: PyFuncArgs) -> PyResult {
    Ok(vm.ctx.none())
}

fn object_class(obj: PyObjectRef, _vm: &VirtualMachine) -> PyObjectRef {
    obj.typ()
}

fn object_class_setter(
    instance: PyObjectRef,
    _value: PyObjectRef,
    vm: &VirtualMachine,
) -> PyResult {
    let type_repr = vm.to_pystr(&instance.typ())?;
    Err(vm.new_type_error(format!("can't change class of type '{}'", type_repr)))
}

fn object_dict(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    if let Some(ref dict) = args.args[0].dict {
        let new_dict = vm.new_dict();
        for (attr, value) in dict.borrow().iter() {
            new_dict.set_item(&vm.ctx, &attr, value.clone());
        }
        Ok(new_dict)
    } else {
        Err(vm.new_type_error("TypeError: no dictionary.".to_string()))
    }
}

fn object_getattribute(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [
            (obj, Some(vm.ctx.object())),
            (name_str, Some(vm.ctx.str_type()))
        ]
    );
    let name = objstr::get_value(&name_str);
    trace!("object.__getattribute__({:?}, {:?})", obj, name);
    let cls = obj.type_pyref();

    if let Some(attr) = objtype::class_get_attr(&cls, &name) {
        let attr_class = attr.type_pyref();
        if objtype::class_has_attr(&attr_class, "__set__") {
            if let Some(descriptor) = objtype::class_get_attr(&attr_class, "__get__") {
                return vm.invoke(descriptor, vec![attr, obj.clone(), cls.into_object()]);
            }
        }
    }

    if let Some(obj_attr) = object_getattr(&obj, &name) {
        Ok(obj_attr)
    } else if let Some(attr) = objtype::class_get_attr(&cls, &name) {
        vm.call_get_descriptor(attr, obj.clone())
    } else if let Some(getter) = objtype::class_get_attr(&cls, "__getattr__") {
        vm.invoke(getter, vec![obj.clone(), name_str.clone()])
    } else {
        Err(vm.new_attribute_error(format!("{} has no attribute '{}'", obj, name)))
    }
}

fn object_getattr(obj: &PyObjectRef, attr_name: &str) -> Option<PyObjectRef> {
    // TODO:
    // This is an all kinds of wrong work-around for the temporary difference in
    // shape between modules and object. It will disappear once that is fixed.
    if let Some(PyModule { ref dict, .. }) = obj.payload::<PyModule>() {
        return dict.get_item(attr_name);
    }

    if let Some(ref dict) = obj.dict {
        dict.borrow().get(attr_name).cloned()
    } else {
        None
    }
}

pub fn get_attributes(obj: &PyObjectRef) -> PyAttributes {
    // Get class attributes:
    let mut attributes = objtype::get_attributes(obj.type_pyref());

    // Get instance attributes:
    if let Some(dict) = &obj.dict {
        for (name, value) in dict.borrow().iter() {
            attributes.insert(name.to_string(), value.clone());
        }
    }

    attributes
}

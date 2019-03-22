/*! Python `super` class.

See also:

https://github.com/python/cpython/blob/50b48572d9a90c5bb36e2bef6179548ea927a35a/Objects/typeobject.c#L7663

*/

use crate::function::PyFuncArgs;
use crate::obj::objtype::PyClass;
use crate::pyobject::{
    DictProtocol, PyContext, PyObject, PyObjectRef, PyResult, PyValue, TypeProtocol,
};
use crate::vm::VirtualMachine;

use super::objtype;

#[derive(Debug)]
pub struct PySuper {
    obj: PyObjectRef,
}

impl PyValue for PySuper {
    fn class(vm: &VirtualMachine) -> Vec<PyObjectRef> {
        vec![vm.ctx.super_type()]
    }
}

pub fn init(context: &PyContext) {
    let super_type = &context.super_type;

    let super_doc = "super() -> same as super(__class__, <first argument>)\n\
                     super(type) -> unbound super object\n\
                     super(type, obj) -> bound super object; requires isinstance(obj, type)\n\
                     super(type, type2) -> bound super object; requires issubclass(type2, type)\n\
                     Typical use to call a cooperative superclass method:\n\
                     class C(B):\n    \
                     def meth(self, arg):\n        \
                     super().meth(arg)\n\
                     This works for class methods too:\n\
                     class C(B):\n    \
                     @classmethod\n    \
                     def cmeth(cls, arg):\n        \
                     super().cmeth(arg)\n";

    context.set_attr(&super_type, "__new__", context.new_rustfunc(super_new));
    context.set_attr(
        &super_type,
        "__getattribute__",
        context.new_rustfunc(super_getattribute),
    );
    context.set_attr(
        &super_type,
        "__doc__",
        context.new_str(super_doc.to_string()),
    );
}

fn super_getattribute(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [
            (super_obj, Some(vm.ctx.super_type())),
            (name_str, Some(vm.ctx.str_type()))
        ]
    );

    let inst = super_obj.payload::<PySuper>().unwrap().obj.clone();

    match inst.typ().payload::<PyClass>() {
        Some(PyClass { ref mro, .. }) => {
            for class in mro {
                if let Ok(item) = vm.get_attribute(class.as_object().clone(), name_str.clone()) {
                    return Ok(vm.ctx.new_bound_method(item, inst.clone()));
                }
            }
            Err(vm.new_attribute_error(format!("{} has no attribute '{}'", inst, name_str)))
        }
        _ => panic!("not Class"),
    }
}

fn super_new(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    trace!("super.__new__ {:?}", args.args);
    arg_check!(
        vm,
        args,
        required = [(cls, None)],
        optional = [(py_type, None), (py_obj, None)]
    );

    if !objtype::issubclass(cls, &vm.ctx.super_type()) {
        return Err(vm.new_type_error(format!("{:?} is not a subtype of super", cls)));
    }

    // Get the type:
    let py_type = if let Some(ty) = py_type {
        ty.clone()
    } else {
        match vm.get_locals().get_item("self") {
            Some(obj) => obj.typ().clone(),
            _ => panic!("No self"),
        }
    };

    // Check type argument:
    if !objtype::isinstance(&py_type, &vm.get_type()) {
        let type_name = objtype::get_type_name(&py_type.typ());
        return Err(vm.new_type_error(format!(
            "super() argument 1 must be type, not {}",
            type_name
        )));
    }

    // Get the bound object:
    let py_obj = if let Some(obj) = py_obj {
        obj.clone()
    } else {
        match vm.get_locals().get_item("self") {
            Some(obj) => obj,
            _ => panic!("No self"),
        }
    };

    // Check obj type:
    if !(objtype::isinstance(&py_obj, &py_type) || objtype::issubclass(&py_obj, &py_type)) {
        return Err(vm.new_type_error(
            "super(type, obj): obj must be an instance or subtype of type".to_string(),
        ));
    }

    Ok(PyObject::new(PySuper { obj: py_obj }, cls.clone()))
}

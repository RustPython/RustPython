/*! Python `super` class.

See also:

https://github.com/python/cpython/blob/50b48572d9a90c5bb36e2bef6179548ea927a35a/Objects/typeobject.c#L7663

*/

use crate::function::{OptionalArg, PyFuncArgs};
use crate::obj::objfunction::PyMethod;
use crate::obj::objstr;
use crate::obj::objtype::{PyClass, PyClassRef};
use crate::pyobject::{
    PyContext, PyObjectRef, PyRef, PyResult, PyValue, TryFromObject, TypeProtocol,
};
use crate::scope::NameProtocol;
use crate::vm::VirtualMachine;

use super::objtype;

pub type PySuperRef = PyRef<PySuper>;

#[derive(Debug)]
pub struct PySuper {
    obj: PyObjectRef,
    typ: PyObjectRef,
    obj_type: PyObjectRef,
}

impl PyValue for PySuper {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.super_type()
    }
}

pub fn init(context: &PyContext) {
    let super_type = &context.types.super_type;

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

    extend_class!(context, super_type, {
        "__new__" => context.new_rustfunc(super_new),
        "__getattribute__" => context.new_rustfunc(super_getattribute),
        "__doc__" => context.new_str(super_doc.to_string()),
        "__str__" => context.new_rustfunc(super_str),
        "__repr__" => context.new_rustfunc(super_repr),
    });
}

fn super_str(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    vm.call_method(&zelf, "__repr__", vec![])
}

fn super_repr(zelf: PyObjectRef, _vm: &VirtualMachine) -> String {
    let super_obj = zelf.downcast::<PySuper>().unwrap();
    let class_type_str = if let Ok(type_class) = super_obj.typ.clone().downcast::<PyClass>() {
        type_class.name.clone()
    } else {
        "NONE".to_string()
    };
    match super_obj.obj_type.clone().downcast::<PyClass>() {
        Ok(obj_class_typ) => format!(
            "<super: <class '{}'>, <{} object>>",
            class_type_str, obj_class_typ.name
        ),
        _ => format!("<super: <class '{}'> NULL>", class_type_str),
    }
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
    let typ = super_obj.payload::<PySuper>().unwrap().typ.clone();

    match typ.payload::<PyClass>() {
        Some(PyClass { ref mro, .. }) => {
            for class in mro {
                if let Ok(item) = vm.get_attribute(class.as_object().clone(), name_str.clone()) {
                    if item.payload_is::<PyMethod>() {
                        // This is a classmethod
                        return Ok(item);
                    }
                    return Ok(vm.ctx.new_bound_method(item, inst.clone()));
                }
            }
            Err(vm.new_attribute_error(format!(
                "{} has no attribute '{}'",
                inst,
                objstr::get_value(name_str)
            )))
        }
        _ => panic!("not Class"),
    }
}

fn super_new(
    cls: PyClassRef,
    py_type: OptionalArg<PyClassRef>,
    py_obj: OptionalArg<PyObjectRef>,
    vm: &VirtualMachine,
) -> PyResult<PySuperRef> {
    // Get the type:
    let py_type = if let OptionalArg::Present(ty) = py_type {
        ty.clone()
    } else {
        match vm.current_scope().load_cell(vm, "__class__") {
            Some(obj) => PyClassRef::try_from_object(vm, obj)?,
            _ => {
                return Err(vm.new_type_error(
                    "super must be called with 1 argument or from inside class method".to_string(),
                ));
            }
        }
    };

    // Check type argument:
    if !objtype::isinstance(py_type.as_object(), &vm.get_type()) {
        return Err(vm.new_type_error(format!(
            "super() argument 1 must be type, not {}",
            py_type.class().name
        )));
    }

    // Get the bound object:
    let py_obj = if let OptionalArg::Present(obj) = py_obj {
        obj.clone()
    } else {
        let frame = vm.current_frame().expect("no current frame for super()");
        if let Some(first_arg) = frame.code.arg_names.get(0) {
            match vm.get_locals().get_item_option(first_arg, vm)? {
                Some(obj) => obj.clone(),
                _ => {
                    return Err(vm.new_type_error(format!(
                        "super arguement {} was not supplied",
                        first_arg
                    )));
                }
            }
        } else {
            return Err(vm.new_type_error(
                "super must be called with 1 argument or from inside class method".to_string(),
            ));
        }
    };

    // Check obj type:
    let obj_type = if !objtype::isinstance(&py_obj, &py_type) {
        let is_subclass = if let Ok(py_obj) = PyClassRef::try_from_object(vm, py_obj.clone()) {
            objtype::issubclass(&py_obj, &py_type)
        } else {
            false
        };
        if !is_subclass {
            return Err(vm.new_type_error(
                "super(type, obj): obj must be an instance or subtype of type".to_string(),
            ));
        }
        PyClassRef::try_from_object(vm, py_obj.clone())?
    } else {
        py_obj.class()
    };

    PySuper {
        obj: py_obj,
        typ: py_type.into_object(),
        obj_type: obj_type.into_object(),
    }
    .into_ref_with_type(vm, cls)
}

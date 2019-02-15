/*! Python `property` descriptor class.

*/

use super::super::pyobject::{PyContext, PyFuncArgs, PyResult, TypeProtocol};
use super::super::vm::VirtualMachine;
use super::objtype;

pub fn init(context: &PyContext) {
    let property_type = &context.property_type;

    let property_doc =
        "Property attribute.\n\n  \
         fget\n    \
         function to be used for getting an attribute value\n  \
         fset\n    \
         function to be used for setting an attribute value\n  \
         fdel\n    \
         function to be used for del\'ing an attribute\n  \
         doc\n    \
         docstring\n\n\
         Typical use is to define a managed attribute x:\n\n\
         class C(object):\n    \
         def getx(self): return self._x\n    \
         def setx(self, value): self._x = value\n    \
         def delx(self): del self._x\n    \
         x = property(getx, setx, delx, \"I\'m the \'x\' property.\")\n\n\
         Decorators make defining new properties or modifying existing ones easy:\n\n\
         class C(object):\n    \
         @property\n    \
         def x(self):\n        \"I am the \'x\' property.\"\n        \
         return self._x\n    \
         @x.setter\n    \
         def x(self, value):\n        \
         self._x = value\n    \
         @x.deleter\n    \
         def x(self):\n        \
         del self._x";

    context.set_attr(
        &property_type,
        "__get__",
        context.new_rustfunc(property_get),
    );
    context.set_attr(
        &property_type,
        "__new__",
        context.new_rustfunc(property_new),
    );
    context.set_attr(
        &property_type,
        "__doc__",
        context.new_str(property_doc.to_string()),
    );
    // TODO: how to handle __set__ ?
}

// `property` methods.
fn property_get(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    trace!("property.__get__ {:?}", args.args);
    arg_check!(
        vm,
        args,
        required = [
            (cls, Some(vm.ctx.property_type())),
            (inst, None),
            (_owner, None)
        ]
    );

    match vm.ctx.get_attr(&cls, "fget") {
        Some(getter) => {
            let py_method = vm.ctx.new_bound_method(getter, inst.clone());
            vm.invoke(py_method, PyFuncArgs::default())
        }
        None => {
            let attribute_error = vm.context().exceptions.attribute_error.clone();
            Err(vm.new_exception(
                attribute_error,
                String::from("Attribute Error: property must have 'fget' attribute"),
            ))
        }
    }
}

fn property_new(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    trace!("property.__new__ {:?}", args.args);
    arg_check!(vm, args, required = [(cls, None), (fget, None)]);

    let py_obj = vm.ctx.new_instance(cls.clone(), None);
    vm.ctx.set_attr(&py_obj, "fget", fget.clone());
    Ok(py_obj)
}

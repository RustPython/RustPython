/*! Python `super` class.

See also:

https://github.com/python/cpython/blob/50b48572d9a90c5bb36e2bef6179548ea927a35a/Objects/typeobject.c#L7663

*/

use super::super::pyobject::{PyContext, PyFuncArgs, PyResult, TypeProtocol};
use super::super::vm::VirtualMachine;
use super::objtype;

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

    context.set_attr(&super_type, "__init__", context.new_rustfunc(super_init));
    context.set_attr(
        &super_type,
        "__doc__",
        context.new_str(super_doc.to_string()),
    );
}

fn super_init(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    trace!("super.__init__ {:?}", args.args);
    arg_check!(
        vm,
        args,
        required = [(inst, None)],
        optional = [(py_type, None), (py_obj, None)]
    );

    // Get the type:
    let py_type = if let Some(ty) = py_type {
        ty.clone()
    } else {
        // TODO: implement complex logic here....
        unimplemented!("TODO: get frame and determine instance and class?");
        // let frame = vm.get_current_frame();
        //
        // vm.get_none()
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
        vm.get_none()
    };

    // Check obj type:
    if !(objtype::isinstance(&py_obj, &py_type) || objtype::issubclass(&py_obj, &py_type)) {
        return Err(vm.new_type_error(
            "super(type, obj): obj must be an instance or subtype of type".to_string(),
        ));
    }

    // TODO: how to store those types?
    vm.ctx.set_attr(inst, "type", py_type);
    vm.ctx.set_attr(inst, "obj", py_obj);

    Ok(vm.get_none())
}

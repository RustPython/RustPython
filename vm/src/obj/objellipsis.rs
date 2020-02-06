use super::objtype::{issubclass, PyClassRef};
use crate::pyobject::{PyContext, PyEllipsisRef, PyResult};
use crate::vm::VirtualMachine;

pub fn init(context: &PyContext) {
    extend_class!(context, &context.ellipsis_type, {
        (slot new) => ellipsis_new,
        "__repr__" => context.new_method(ellipsis_repr),
        "__reduce__" => context.new_method(ellipsis_reduce),
    });
}

fn ellipsis_new(cls: PyClassRef, vm: &VirtualMachine) -> PyResult {
    if issubclass(&cls, &vm.ctx.ellipsis_type) {
        Ok(vm.ctx.ellipsis())
    } else {
        Err(vm.new_type_error(format!(
            "ellipsis.__new__({ty}): {ty} is not a subtype of ellipsis",
            ty = cls,
        )))
    }
}

fn ellipsis_repr(_self: PyEllipsisRef) -> String {
    "Ellipsis".to_owned()
}

fn ellipsis_reduce(_self: PyEllipsisRef) -> String {
    "Ellipsis".to_owned()
}

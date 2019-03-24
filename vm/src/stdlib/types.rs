/*
 * Dynamic type creation and names for built in types.
 */

use crate::function::OptionalArg;
use crate::obj::objdict::PyDict;
use crate::obj::objstr::PyStringRef;
use crate::obj::objtype;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{PyContext, PyIterable, PyObjectRef, PyResult, PyValue, TryFromObject};
use crate::VirtualMachine;

fn types_new_class(
    name: PyStringRef,
    bases: OptionalArg<PyIterable<PyClassRef>>,
    vm: &VirtualMachine,
) -> PyResult<PyClassRef> {
    // TODO kwds and exec_body parameter

    let bases = match bases {
        OptionalArg::Present(bases) => bases,
        OptionalArg::Missing => PyIterable::try_from_object(vm, vm.ctx.new_tuple(vec![]))?,
    };
    let dict = PyDict::default().into_ref(vm);
    objtype::type_new_class(vm, vm.ctx.type_type(), name, bases, dict)
}

pub fn make_module(ctx: &PyContext) -> PyObjectRef {
    py_module!(ctx, "types", {
        "new_class" => ctx.new_rustfunc(types_new_class),
        "FunctionType" => ctx.function_type(),
        "LambdaType" => ctx.function_type(),
        "CodeType" => ctx.code_type(),
        "FrameType" => ctx.frame_type()
    })
}

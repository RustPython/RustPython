/*
 * Dynamic type creation and names for built in types.
 */

use crate::obj::objtype;
use crate::pyobject::{PyContext, PyFuncArgs, PyObjectRef, PyResult, TypeProtocol};
use crate::VirtualMachine;

fn types_new_class(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(name, Some(vm.ctx.str_type()))],
        optional = [(bases, None), (_kwds, None), (_exec_body, None)]
    );

    let ref type_type = vm.ctx.type_type();
    let bases: PyObjectRef = match bases {
        Some(bases) => bases.clone(),
        None => vm.ctx.new_tuple(vec![]),
    };
    let dict = vm.ctx.new_dict();
    objtype::type_new_class(vm, &type_type, name, &bases, &dict)
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

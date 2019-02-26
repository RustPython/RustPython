/*
 * Dynamic type creation and names for built in types.
 */

use crate::obj::{objsequence, objstr, objtype};
use crate::pyobject::{PyAttributes, PyContext, PyFuncArgs, PyObjectRef, PyResult, TypeProtocol};
use crate::VirtualMachine;

fn types_new_class(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(name, Some(vm.ctx.str_type()))],
        optional = [(bases, None), (_kwds, None), (_exec_body, None)]
    );

    let name = objstr::get_value(name);

    let bases = match bases {
        Some(b) => {
            if objtype::real_isinstance(b, &vm.ctx.tuple_type()) {
                objsequence::get_elements(b).to_vec()
            } else {
                return Err(vm.new_type_error("Bases must be a tuple".to_string()));
            }
        }
        None => vec![vm.ctx.object()],
    };

    objtype::new(vm.ctx.type_type(), &name, bases, PyAttributes::new())
}

pub fn mk_module(ctx: &PyContext) -> PyObjectRef {
    py_module!(ctx, "types", {
        "new_class" => ctx.new_rustfunc(types_new_class),
        "FunctionType" => ctx.function_type(),
        "LambdaType" => ctx.function_type(),
        "CodeType" => ctx.code_type(),
        "FrameType" => ctx.frame_type()
    })
}

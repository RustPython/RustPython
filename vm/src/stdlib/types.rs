/*
 * Dynamic type creation and names for built in types.
 */

use super::super::obj::{objsequence, objstr, objtype};
use super::super::pyobject::{PyContext, PyFuncArgs, PyObjectRef, PyResult, TypeProtocol};
use super::super::VirtualMachine;

fn types_new_class(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(name, Some(vm.ctx.str_type()))],
        optional = [(bases, None), (_kwds, None), (_exec_body, None)]
    );

    let name = objstr::get_value(name);
    let dict = vm.ctx.new_dict();

    let bases = match bases {
        Some(b) => {
            if objtype::isinstance(b, &vm.ctx.tuple_type()) {
                objsequence::get_elements(b).to_vec()
            } else {
                return Err(vm.new_type_error("Bases must be a tuple".to_string()));
            }
        }
        None => vec![vm.ctx.object()],
    };

    objtype::new(vm.ctx.type_type(), &name, bases, dict)
}

pub fn mk_module(ctx: &PyContext) -> PyObjectRef {
    py_item!(ctx, mod types {
        // Number theory functions:
        let new_class = ctx.new_rustfunc(types_new_class);
        let FunctionType = ctx.function_type();
        let LambdaType = ctx.function_type();
        let CodeType = ctx.code_type();
        let FrameType = ctx.frame_type();
    })
}

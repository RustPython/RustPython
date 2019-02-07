/*
 * Dynamic type creation and names for built in types.
 */

use super::super::obj::{objsequence, objstr, objtype};
use super::super::pyobject::{
    PyAttributes, PyContext, PyFuncArgs, PyObjectRef, PyResult, TypeProtocol,
};
use super::super::VirtualMachine;

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
            if objtype::isinstance(b, &vm.ctx.tuple_type()) {
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
    let py_mod = ctx.new_module("types", ctx.new_scope(None));

    // Number theory functions:
    ctx.set_attr(&py_mod, "new_class", ctx.new_rustfunc(types_new_class));
    ctx.set_attr(&py_mod, "FunctionType", ctx.function_type());
    ctx.set_attr(&py_mod, "LambdaType", ctx.function_type());
    ctx.set_attr(&py_mod, "CodeType", ctx.code_type());
    ctx.set_attr(&py_mod, "FrameType", ctx.frame_type());

    py_mod
}

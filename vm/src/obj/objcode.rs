/*! Infamous code object. The python class `code`

*/

use std::fmt;

use crate::bytecode;
use crate::function::PyFuncArgs;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{IdProtocol, PyContext, PyObjectRef, PyRef, PyResult, PyValue, TypeProtocol};
use crate::vm::VirtualMachine;

pub type PyCodeRef = PyRef<PyCode>;

pub struct PyCode {
    pub code: bytecode::CodeObject,
}

impl PyCode {
    pub fn new(code: bytecode::CodeObject) -> PyCode {
        PyCode { code }
    }
}

impl fmt::Debug for PyCode {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "code: {:?}", self.code)
    }
}

impl PyValue for PyCode {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.code_type()
    }
}

pub fn init(context: &PyContext) {
    let code_type = context.code_type.as_object();
    extend_class!(context, code_type, {
        "__new__" => context.new_rustfunc(code_new),
        "__repr__" => context.new_rustfunc(code_repr),

        "co_argcount" => context.new_property(code_co_argcount),
        "co_consts" => context.new_property(code_co_consts),
        "co_filename" => context.new_property(code_co_filename),
        "co_firstlineno" => context.new_property(code_co_firstlineno),
        "co_kwonlyargcount" => context.new_property(code_co_kwonlyargcount),
        "co_name" => context.new_property(code_co_name),
    });
}

fn code_new(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(_cls, None)]);
    Err(vm.new_type_error("Cannot directly create code object".to_string()))
}

fn code_repr(o: PyCodeRef, _vm: &VirtualMachine) -> String {
    let code = &o.code;
    format!(
        "<code object {} at 0x{:x} file {:?}, line {}>",
        code.obj_name,
        o.get_id(),
        code.source_path,
        code.first_line_number
    )
}

fn code_co_argcount(code: PyCodeRef, _vm: &VirtualMachine) -> usize {
    code.code.arg_names.len()
}

fn code_co_filename(code: PyCodeRef, _vm: &VirtualMachine) -> String {
    code.code.source_path.clone()
}

fn code_co_firstlineno(code: PyCodeRef, _vm: &VirtualMachine) -> usize {
    code.code.first_line_number
}

fn code_co_kwonlyargcount(code: PyCodeRef, _vm: &VirtualMachine) -> usize {
    code.code.kwonlyarg_names.len()
}

fn code_co_consts(code: PyCodeRef, vm: &VirtualMachine) -> PyObjectRef {
    let consts = code
        .code
        .get_constants()
        .map(|x| vm.ctx.unwrap_constant(x))
        .collect();
    vm.ctx.new_tuple(consts)
}

fn code_co_name(code: PyCodeRef, _vm: &VirtualMachine) -> String {
    code.code.obj_name.clone()
}

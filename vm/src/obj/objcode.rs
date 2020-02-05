/*! Infamous code object. The python class `code`

*/

use std::fmt;
use std::ops::Deref;

use super::objtype::PyClassRef;
use crate::bytecode;
use crate::pyobject::{IdProtocol, PyContext, PyObjectRef, PyRef, PyResult, PyValue};
use crate::vm::VirtualMachine;

pub type PyCodeRef = PyRef<PyCode>;

pub struct PyCode {
    pub code: bytecode::CodeObject,
}

impl Deref for PyCode {
    type Target = bytecode::CodeObject;
    fn deref(&self) -> &Self::Target {
        &self.code
    }
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

impl PyCodeRef {
    #[allow(clippy::new_ret_no_self)]
    fn new(_cls: PyClassRef, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_type_error("Cannot directly create code object".to_owned()))
    }

    fn repr(self, _vm: &VirtualMachine) -> String {
        let code = &self.code;
        format!(
            "<code object {} at 0x{:x} file {:?}, line {}>",
            code.obj_name,
            self.get_id(),
            code.source_path,
            code.first_line_number
        )
    }

    fn co_argcount(self, _vm: &VirtualMachine) -> usize {
        self.code.arg_names.len()
    }

    fn co_filename(self, _vm: &VirtualMachine) -> String {
        self.code.source_path.clone()
    }

    fn co_firstlineno(self, _vm: &VirtualMachine) -> usize {
        self.code.first_line_number
    }

    fn co_kwonlyargcount(self, _vm: &VirtualMachine) -> usize {
        self.code.kwonlyarg_names.len()
    }

    fn co_consts(self, vm: &VirtualMachine) -> PyObjectRef {
        let consts = self
            .code
            .get_constants()
            .map(|x| vm.ctx.unwrap_constant(x))
            .collect();
        vm.ctx.new_tuple(consts)
    }

    fn co_name(self, _vm: &VirtualMachine) -> String {
        self.code.obj_name.clone()
    }

    fn co_flags(self, _vm: &VirtualMachine) -> u8 {
        self.code.flags.bits()
    }
}

pub fn init(ctx: &PyContext) {
    extend_class!(ctx, &ctx.types.code_type, {
        (slot new) => PyCodeRef::new,
        "__repr__" => ctx.new_method(PyCodeRef::repr),

        "co_argcount" => ctx.new_readonly_getset("co_argcount", PyCodeRef::co_argcount),
        "co_consts" => ctx.new_readonly_getset("co_consts", PyCodeRef::co_consts),
        "co_filename" => ctx.new_readonly_getset("co_filename", PyCodeRef::co_filename),
        "co_firstlineno" => ctx.new_readonly_getset("co_firstlineno", PyCodeRef::co_firstlineno),
        "co_kwonlyargcount" => ctx.new_readonly_getset("co_kwonlyargcount", PyCodeRef::co_kwonlyargcount),
        "co_name" => ctx.new_readonly_getset("co_name", PyCodeRef::co_name),
        "co_flags" => ctx.new_readonly_getset("co_flags", PyCodeRef::co_flags),
    });
}

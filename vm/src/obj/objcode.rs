/*! Infamous code object. The python class `code`

*/

use std::fmt;
use std::ops::Deref;

use super::objtype::PyClassRef;
use crate::bytecode;
use crate::pyobject::{IdProtocol, PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue};
use crate::vm::VirtualMachine;

pub type PyCodeRef = PyRef<PyCode>;

#[pyclass]
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

#[pyimpl]
impl PyCodeRef {
    #[pyslot]
    fn new(_cls: PyClassRef, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        Err(vm.new_type_error("Cannot directly create code object".to_owned()))
    }

    #[pymethod(magic)]
    fn repr(self) -> String {
        let code = &self.code;
        format!(
            "<code object {} at 0x{:x} file {:?}, line {}>",
            code.obj_name,
            self.get_id(),
            code.source_path,
            code.first_line_number
        )
    }

    #[pyproperty]
    fn co_posonlyargcount(self) -> usize {
        self.code.posonlyarg_count
    }

    #[pyproperty]
    fn co_argcount(self) -> usize {
        self.code.arg_names.len()
    }

    #[pyproperty]
    fn co_filename(self) -> String {
        self.code.source_path.clone()
    }

    #[pyproperty]
    fn co_firstlineno(self) -> usize {
        self.code.first_line_number
    }

    #[pyproperty]
    fn co_kwonlyargcount(self) -> usize {
        self.code.kwonlyarg_names.len()
    }

    #[pyproperty]
    fn co_consts(self, vm: &VirtualMachine) -> PyObjectRef {
        let consts = self
            .code
            .get_constants()
            .map(|x| vm.ctx.unwrap_constant(x))
            .collect();
        vm.ctx.new_tuple(consts)
    }

    #[pyproperty]
    fn co_name(self) -> String {
        self.code.obj_name.clone()
    }

    #[pyproperty]
    fn co_flags(self) -> u16 {
        self.code.flags.bits()
    }

    #[pyproperty]
    fn co_varnames(self, vm: &VirtualMachine) -> PyObjectRef {
        let varnames = self.code.varnames().map(|s| vm.ctx.new_str(s)).collect();
        vm.ctx.new_tuple(varnames)
    }
}

pub fn init(ctx: &PyContext) {
    PyCodeRef::extend_class(ctx, &ctx.types.code_type);
}

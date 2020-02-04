/* String builtin module
 */

use crate::pyobject::PyObjectRef;
use crate::vm::VirtualMachine;

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    // let ctx = &vm.ctx;

    // Constants:
    py_module!(vm, "_string", {})
}

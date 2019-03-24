use crate::obj::objcode::PyCodeRef;
use crate::pyobject::{PyContext, PyObjectRef, PyResult, TryFromObject};
use crate::vm::VirtualMachine;

fn dis_dis(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    // Method or function:
    if let Ok(co) = vm.get_attribute(obj.clone(), "__code__") {
        return dis_disassemble(co, vm);
    }

    dis_disassemble(obj, vm)
}

fn dis_disassemble(co: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    let code = &PyCodeRef::try_from_object(vm, co)?.code;
    print!("{}", code);
    Ok(vm.get_none())
}

pub fn make_module(ctx: &PyContext) -> PyObjectRef {
    py_module!(ctx, "dis", {
        "dis" => ctx.new_rustfunc(dis_dis),
        "disassemble" => ctx.new_rustfunc(dis_disassemble)
    })
}
